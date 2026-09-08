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
- road-locked terrain coverage follows each grounded footprint's required tie-in envelope; a large
  cut / fill envelope on one road must not widen unrelated road patches, and clip-source queries
  include the same render/cell safety pad used to select grading-ray boundary patches
- ordinary grounded-road tie-ins do not emit retaining-wall mesh as a visual cleanup path
- player road bulldoze is a queued `SimCore` mutation: Godot captures an immutable target from the
  road spatial index, the simulation thread verifies and soft-deletes that exact edge, removes
  attached zoning parcels, and repairs adjacency, clips, lanes, CCH, road surface chunks, terrain
  clips, and flow-field dirtiness before publishing the next render snapshot
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

## Pedestrian Junction Crossings

One authoritative crossing record drives both pedestrian routing and zebra-marking rendering:

- an arm is crossable if and only if its lane rebuild emits a `CrosswalkMarking`
- the two directional pedestrian connections for that arm traverse the exact same asphalt-edge
  segment stored by the marking
- routes between adjacent arms follow the solved sidewalk perimeter and must not create direct
  mouth-to-mouth chords through the junction carriageway; their centerlines use the same sampled
  circular-arc / bounded-fillet policy as the compiled rounded sidewalk side joins, with straight
  sidewalk approaches between the node mouth and crosswalk inset
- every incoming sidewalk mouth has one precomputed route to each reachable outbound sidewalk
  mouth; ordinary edge routing selects the shortest route to an arm, while exact building access
  can select either sidewalk without falling back to a side-to-side lane reselection
- physical road-sidewalk lanes terminate at the configured crosswalk inset, exactly matching the
  first and last points of their junction connectors, so walkers cannot pass a zebra and backtrack
- a same-side reversal uses an explicit stationary connector at the mouth instead of crossing the
  road or reattaching farther along the reverse sidewalk
- degree-two junctions keep the deterministic single-crosswalk policy, while higher-degree
  junctions may expose one crossing per eligible arm

Crossing availability and geometry must not be independently reconstructed by the renderer,
selection queries, or agent movement.

Incremental physical-lane rebuilds include every road arm incident to the original dirty
endpoints. All connector lanes at both ends of every rebuilt arm are replaced in the same update,
and active agents on that complete rebuild closure are invalidated before the old lane IDs become
orphans. After the lane rebuild, invalidated on-road agents may reattach only to rebuilt physical
lanes in the same closure near their preserved world position; the stored `lane_distance` is
derived from the same 3D lane arc-length metric used by lane geometry. A connector must never
survive while targeting an orphaned physical lane.

## Visual Pieces

### Span

`Span` owns ordinary edge corridors between node throats:

- span sections are ordered along the edge plan polyline
- every original edge knot and every node throat appears as a section
- long spans are refined by deterministic world-space constants owned in Rust code
- span cross-section directions interpolate normalized centered secants between polyline knots;
  incident lengths are retained before normalization so short profile supports do not amplify
  coordinate roundoff. Exact node-mouth directions bound this frame, and node-owned rails retain
  their source geometry. Inserted samples use the same continuous frame, preventing tiny bands
  from folding across near-adjacent profile knots. Sampling stays O(P) per section for P edge
  points, adds two O(P) mouth samples per compiled edge, and allocates no tangent-search buffers.
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
- carriageway owner carriers and side-join candidates define asphalt; non-terminal nodes must not
  reintroduce raw source-band-none carriageway corridor fallback polygons, and those owner carriers
  must follow the same rounded adjacent-mouth boundary policy as side-join contours instead of
  preserving old miter endpoints
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

Adjacent-mouth corners in `Bend` and `JunctionN` pieces are canonical rounded geometry, not a
renderer bevel, shader mask, or test-only helper. For every non-degenerate adjacent mouth gap, the
side-join generator must round asphalt-to-curb / sidewalk material boundaries and outer
roadbed-to-terrain footprint boundaries before the ownership boolean. The emitted visible side-join
contours are adjacent-mouth boundaries: their endpoints come from the generated rail / band boundary
points, not from the shared graph endpoint or node centre. A shared endpoint / centreline point may
remain as internal owner or height support, but it must not be emitted as an exposed side-join
material or footprint boundary for the rounded corner.

Exact circular arcs are preferred when the incident rails support a shared radius; bounded
deterministic fillets are used when exact arcs are unavailable. Fillet sampling is fixed and bounded
per adjacent mouth, preserves the generated band owner / height carrier provenance through the normal
contour path, and must be clamped by the available adjacent segments so it cannot erase sidewalks,
cross another mouth, or create unsupported sliver polygons. If a split carriageway slice collapses
against the centerline, that degenerate asphalt slice must not abort the whole corner or reintroduce a
center-routed miter; its rounded outer path is carried forward so curb / shoulder and sidewalk bands
still receive rounded ownership boundaries. Render polygons, visible-surface queries, terrain clip
loops, and earthwork roots all consume this same rounded ownership output.

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

Degree-two `PassThrough` nodes own no node platform and therefore never run the `Bend` /
`JunctionN` endpoint-profile rewrite. Their incident spans preserve the already validated authored
vertical profiles. At a shallow alignment change, both spans use one deterministic bisected
cross-section axis at the shared endpoint, so `Bridge` / `Standard` and same-class handoffs meet
without a transverse cap, terrain slit, or overlapping roadbed.

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

Save/load preserves saved node positions, road grades, physical lengths, and junction clips.
It rebuilds derived road surfaces, terrain earthworks, and lanes without running terrain-authoring
resynchronization over the saved roads.

Road-touched terrain patches obey this contract:

- Rust unions grounded road-owned footprint loops for the patch before CDT input
- terrain patch rectangles, footprint loops, and source-terrain samples are inserted in canonical
  order
- road footprint constraints are clipped and split at patch boundaries before Spade input
- clipping preserves exact rectangle coordinates, including integer-overlay output for disconnected
  components; source-vertex recovery cannot move an intersection off that boundary or outside the patch
- source provenance and earthwork loop connectivity follow canonical endpoint identity, including
  edges shorter than 1 mm between distinct keys; their nonzero outward directions remain valid
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
- when a building-site footprint boundary shares an exact XZ vertex with a road-owned terrain
  constraint, the road-owned height is authoritative for that shared CDT vertex; site-owned yard
  grading must not emit a conflicting over/under terrain face at the road seam
- ordinary `Standard` span and node seam sources stay in the terrain bucket even when the authored
  terrain is steep; retaining-wall output is reserved for explicit structural bridge / tunnel /
  future retaining sources
- an omitted over-steep source sample beside a bridge abutment does not promote every face sharing
  that span boundary to retaining material; bridge faces are classified from their emitted slope,
  while a tunnel portal may still require source-wide retaining output
- no shader mask, water plane, closure carpet, guard strip, or seam strip may replace missing
  topology

`Bridge` and `Tunnel` edges do not flatten or clip ordinary midspan terrain. Their terrain support
is limited to class-owned abutment or portal regions. A bridge ramp within the ground-contact
clearance zone exports a source-owned terrain cutout only through its abutment and adjacent node
handoff; it does not stamp terrain fill, and the elevated bridge run remains outside terrain CDT.

Structural earthwork stamping remains for explicit bridge, tunnel, retaining, and future
engineered-ground cases. Its current acceleration is chunk-local: prepared triangles are bucketed
inside dirty chunks, support candidates preserve the closest-distance / lower-height tie-break
rule, and writes are applied in canonical chunk order.

Shared engineered-ground rules live in [`earthworks.md`](earthworks.md).

Building-site earthworks consume road-owned visible-surface/query heights for driveway or frontage
connections, but roads do not own lot grading, flat-site height selection, or authored yard material
regions.

## Preview, Query, And Editing

Editor preview must use the same road-surface solve rules as committed placement. Preview and
commit may differ in cache lifetime or display detail, but not in geometry ownership.

Standard-road input keeps the player's authored XZ alignment, then prepares its physical geometry
before graph commit or preview compilation: long spans are densified every few metres, samples are
grounded against source terrain or existing visible road support, true endpoints and road
connections are hard pins, and a bounded vertical-profile solve keeps the road near terrain while
targeting smooth grade and curvature changes. These profile targets shape generated geometry and
remain available as diagnostics; they are not player-facing placement limits. The dense solved
profile is stored on `physical_geometry`; section compilation, terrain clips, and earthworks
interpolate that stored profile instead of drawing a straight vertical line between sparse
endpoints. Earthworks own only the remaining local cut/fill around that solved roadbed.
When a road is extended from a degree-1 standard-road terminal, preview and commit solve the
existing terminal edge plus the new edge as one vertical corridor; the shared terminal becomes an
internal profile point, while the far old endpoint, far new endpoint, and true junctions stay pinned.
When the same edit runs from a degree-1 elevated bridge terminal down to source terrain, the dense
profile remains `Bridge` and forms a structural ramp across the full approach. It may enter the
ordinary bridge-clearance zone near its grounded landing, but it may not pass below source terrain
and it never becomes `Standard` earthwork that raises terrain to the bridge deck.

Player-facing placement rules include existing parcel conflicts and structural impossibilities:
roads shorter than `2 m`, standard roadbeds crossing authored water without bridge mode,
bridge/tunnel clearance failures, and endpoints that resolve to an impossible same-node connection.
There is no hard centerline-grade or curve-angle rejection. Same-node rejection uses the distinct
`same_node_connection` reason; it is not reported as a compiler failure.

Road cursor snapping retains a generation-checked node or edge identity. Nodes remain fixed snap
targets; edges continuously project the latest cursor onto their centerline and acquire their real
endpoints within the node capture radius. Retention releases by distance to that target, not distance
to the previous projected point. Interior polyline knots have no endpoint margin, avoiding artificial
half-metre jumps. Retained projection is allocation-free `O(edge polyline segments)`; acquisition
uses the existing node grid and edge R-tree. No additional spatial index or whole-network scan is used.

Ghost-guide snapping also queries the existing edge R-tree (`ROAD-20`). Its search bounds include
the maximum guide reach (240 m lateral offsets or 200 m outward extensions) plus capture radius,
rounded outwards. The index's internal visitor avoids heap-backed traversal storage; candidate
polylines stream the same offset segments and crossing-pair rejection without candidate or offset
buffers. Equal-distance ties resolve by world-XZ order, not index traversal order. Work depends on
the indexed local edges and their source segments; long polylines
or dense overlapping bounds still cost more. Road-tool snapshot publication no longer builds or
retires a second whole-network snapping R-tree. Visible guide generation is unchanged.

Mouse motion performs no curve baking, validation, or mesh upload in the input-event handler.
The tool resolves the current pointer once per frame, then coalesces changed position/settings into
one lightweight preview update. Camera-only movement also refreshes the preview; motion that leaves
the resolved snap unchanged preserves the displayed exact result. Exact compilation starts immediately
and continues during motion: one running job completes while the current curve replaces pending
intent, then the newest input is dispatched. The native mailbox also holds at most one pending job;
there is no idle timer or unbounded request backlog. Cache/request matching uses exact points, so fine movements cannot
redisplay an old result within the former 5 cm tolerance. Clicks resolve their current pointer before
building the committed curve, including clicks arriving before the next frame. Shift angle/length rules and capture/release
distances are unchanged by this scheduling/snap-target change.

The moving and ordinary settled stroke previews render asphalt, lane dividers, curbs, and sidewalks rather than a
uniform blue ribbon. They share committed-road texture resources and Rust's lateral band widths;
zero vehicle lanes render the actual 2 m walkway. Valid placement uses untinted materials with no
outline; pending validation is amber, and rejection is red. Lane coordinates remain in metres through curves.

Preview display positions are separate from the authoritative prepared points. Moving feedback
reuses the already prepared terrain-aware vertical profile; settled feedback uses compiled sections.
Both stroke ribbons sample current visual terrain along and across the road, at 0.25–2 m spacing
tied to terrain resolution, and lift vertices 15 cm above
the higher of the planned roadbed and terrain, preserving raised band offsets. This keeps cut areas
visible before commit excavates them, without lowering elevated roads or changing tunnel/bridge
classification, validation, cost, or committed heights. Normal depth testing stays enabled; the
preview is not an always-visible overlay through buildings. It is a sampled placement display, not
an exact visualization of the future cut/fill terrain.

Display generation is `O(longitudinal samples × lateral strips)` for the current stroke, with
constant-time terrain-grid samples and no city-wide query or junction compilation. Buffer capacity
is computed before triangle emission. Hover validation returns prepared points and its source
generation without generating or exporting a ribbon. Only an actual fallback draw requests packed
material/UV geometry from those prepared points; it performs no second snap/profile solve and rejects
a changed source generation. GDScript performs no per-vertex terrain calls. A matching exact result
is consumed before hover validation, avoiding both a redundant validity check and unused fallback
construction. Retaining a compiled pose during motion likewise builds no ribbon.

When the exact worker compiles an affected `Bend`, `JunctionN` or `PassThrough`, moving and settled
feedback show the resulting local connection and spans through the committed road renderer: asphalt, sidewalks, curb/step
faces, crosswalks and lane dividers. It exports the already-compiled validation neighborhood,
reconstructing lanes only inside that excerpt for the normal marking rules. It does not compile
the junction again, rebuild city routing or modify the live graph. Before display-only lifting,
all vertex attributes must match an independent cold commit, including unequal widths, slopes,
four-way crossings and an old terminal that becomes a pass-through without a nearby junction.
The export decision reuses node classification already required by validation; isolated strokes
retain their lightweight ribbon. Terminal-extension previews include the reprofiled existing edge
in the same changed-edge ledger as commit, keeping sloped bend heights and mouths identical.

Cached road chunks retain compact source-owner ranges alongside their existing vertex buffers.
The worker uses the existing owner-to-chunk memberships to select replacements, snapshots only
those chunk Arcs under a nonblocking, generation-checked core lock, then filters them off-lock.
Unrelated owners retain their exact vertices/material attributes. This removes the old curbs and
markings by identity, not by approximate triangle clipping or blanking neighboring roads.
The preview context is an `O(1)` Arc snapshot; expensive work holds neither its publication lock
nor `SimCore`. A newer pointer request supersedes one waiting for chunk access.

`RoadJunctionPreview` stages retained geometry and new surfaces before temporarily hiding
the corresponding resident instances. Ordinary motion keeps the last completed junction visible
until its replacement is ready. That older pose is display-only: exact click validation still requires
identical current points and context. Session, start/control, lane, mode, snap-target, query and zoning
changes retire mismatched poses/results. Rejection, cancellation, world reset and source generation
changes restore originals; incomplete/stale batches cannot replace a complete preview. A pending
parcel check retries without removing the displayed junction or authorizing a click. A completed
pose consumed while that check is pending remains eligible for display when validation recovers;
it is not lost merely because it completed on an earlier frame.
Query and source-mesh revisions are checked separately: a water-only edit invalidates validation
without requiring unchanged resident road chunks to be relabelled or rebuilt.
Unchanged request IDs do not upload meshes again. One retained CPU-mesh cache is keyed by exact
source-mesh generation, chunk grid, replacement keys and excluded owners. Matching revisions omit
retained payloads at the bridge and preserve their GPU instances; only planned geometry is replaced.
Temporary fallback detaches this retained cache; cancellation, renderer reset and tool destruction
release it. Cached dictionaries are immutable references, not per-frame deep copies. Junction results
do not build a redundant stroke ribbon. `ROAD-23` keeps replacement geometry over committed roads
at its compiled height: existing terrain is already cut out beneath those roads and cannot support a
blanket hover offset. Only new coverage receives terrain-aware clearance. Triangles transitioning
between anchored and lifted coverage are split at the current cutout boundaries, including vertical
curb/support faces; the contact vertices remain unlifted and material layers share their XZ offsets.
Inserted interior vertices are checked against existing coverage too: interpolating an offset from
new coverage must not raise a vertex inside the old road footprint.
Existing collinear terrain breakpoints are retained even on unlifted replacement edges, avoiding
raster T-junction cracks when an old terminal becomes a straight continuation.
The local old-minus-planned cutout footprint receives reversible ground infill when a bend removes
an old terminal corner. Its inner edge follows the planned road and its outer edge the existing
surface. Holes and sub-centimetre residuals use the shared constrained triangulator without the
road renderer's skinny-triangle rejection. Failure to build complete infill declines the replacement
scene, leaving the ordinary ribbon available. Prepared points, reusable topology and live terrain
patches remain untouched: this is a render-only seam treatment, not full future excavation/grading.

The added work scales with the local validation/lane excerpt and affected chunks: `O(C log M)`
for `C` chunk-cache lookups among `M` resident chunks on a retained-cache miss, then `O(R + V)` retained-range filtering,
payload copying for affected ranges/vertices. Display clearance queries reuse existing query chunks
and owner-local triangle grids. Footprint booleans/CDT depend on local boundary segments,
intersections and output only. Contact splitting takes `O(B(T + P))` boundary checks for `B` local
segments, `T` input triangles and `P` generated pieces; boundaries are filtered to the chunk and
source triangle's actual bounds before splitting, and scratch buffers are reused per chunk.
Fully anchored triangles only need collinear seam breakpoints.
Wholly lifted triangles are also checked because they can enclose a cutout without a covered
original vertex. Independent chunk work uses
Rayon. A retained-cache hit checks `O(affected owners + chunks)` dependencies and shares mesh Arcs
in `O(1)`, avoiding retained filtering, payload copying and upload. Planned geometry still scales
with the local excerpt. The request mailbox is bounded to one pending input plus one running job.
Normal mesh generation adds `O(1)` ownership bookkeeping per triangle and storage per
contiguous owner/layer/chunk run; no new spatial index or whole-network preview scan is introduced.

Road and walkway tools show the committed parcel overlay. Cheap candidate validation, synchronous
commit validation, and completed async previews query the same parcel chunk index and corridor
width as the simulation-thread commit guard. Conflicts return `parcel_overlap` with the count and
first parcel id; empty/free parcels remain parcel authority and are not silently removed. Preview
caches include the zoning revision as well as road geometry/generation. A busy simulation mutex
produces a retryable pending result, never a cached valid verdict or a blocking hover wait.

Placement validity also includes local road-surface compileability. A preview or commit replays the
candidate's local post-split topology before acceptance, including interior crossings against nearby
road edges. Any edit that would fail to compile the new span or its required endpoint `Terminal` /
`Bend` / `JunctionN` pieces is rejected before it reaches the live graph; tight switchbacks are
allowed only when the compiled surface topology can actually represent them.
The exact async preview is also a validation certificate for its matching commit. Godot may reuse
it only for identical road points, lanes, snap mode, and current surface generation; the simulation
thread independently checks the prepared points and the same generation before skipping its exact
candidate replay. The accepted certificate also carries immutable node-topology candidates from
the validation surface. Commit compilation matches them by quantized world position, piece kind,
and mouth count, then independently requires exact canonical node-local rail topology. Exact rail
height values additionally permit boolean ownership and the already validated, triangulated
arrangement to be rebound from preview-local node IDs to authoritative node IDs. Any metadata,
topology, carrier, source-authority, or exact-height mismatch takes the full deterministic compiler
path.
Preview validation also seeds that same compiler with immutable committed-node topology candidates,
remapped through the bounded validation graph's source-to-local node map. It rebuilds current node
inputs before checking reuse; removed/merged local identities are not seeded. This adds only
`O(copied neighborhood nodes)` lookups and shared references, not a resident-network scan or a
second topology cache. Cold and seeded preview/commit output must remain identical.
`surface_geometry_invalid` is an exact surface-compiler integrity failure, never a curve-angle or
grade policy. An ordinary continuation or T-junction producing it is a compiler bug. Road debug logs
must include the failed required split spans or nodes with their lengths, clips, lane counts, and
endpoints. Bounded validation graphs can also contain unchanged frontier/context nodes that are not
part of the candidate's required topology. Their cold-compile failures do not reject the candidate:
both full and incremental transient compilers retain each successful artifact while keeping the
failed-generation latch, and the gate independently requires every
candidate-owned span and changed endpoint node piece to exist. Authoritative simulation and render
compiles remain transactional, so placement cannot publish a road whose required surface is missing.
Deleting and redrawing a connection must ignore deleted edges in duplicate detection, matching the
preview graph. Bulldozing uses the existing incremental lane-rebuild closure, identical to agent
invalidation/reattachment; it must not renumber distant active lanes (`ROAD-13`). Lane construction
is local to that closure; existing surviving-lane lookup and agent scans remain `O(L + A)`.
Terrain CDT boundary ownership follows the same `1 mm` canonical vertex identity as its constraint
graph: a junction seam may be physically shorter than `1 mm` while still joining two distinct
canonical cells, and it retains its source until seam hardening deterministically merges or accepts
it. Truly unsourced road boundaries still fail the coordinated terrain/road upload.

Numeric-dust connector height recovery is local to the connected run and its two source anchors.
It must not validate every vertex of the unioned contour: a distant curb can legitimately expose
two source heights handled by the terrain cutter's top envelope. Recovery walks only numeric-dust
edges, stops at the first source anchor in each direction, and retains strict conflict checks for
every sampled point in that run. Source queries reuse `TerrainClipSourceEdgeIndex`; each point
query costs `O(log T + K log K)` for `T` indexed tiles and `K` local candidates, instead of scanning
all patch sources for every contour vertex. This is terrain-build work, with no per-agent/tick cost.
`ROAD-11` covers the iso `(22, 1)` patch failure that blocked live road publication and left terrain
absent after reload.

`ROAD-12` closes the simulation/render acceptance gap. A road command stages its local graph
edit under the simulation mutex, compiles the road surface, and runs the production terrain input,
CDT, and final-buffer builders for the affected grading patches. Constraint conflicts and
pathological final buffers reject the command. Only a complete result permits lane/agent remapping,
entrance/parcel repair, routing rebuild, and the treasury charge. Successful terrain buffers enter
the normal generation-checked cache for reuse by the renderer. A failed edit restores the local
graph/compiler checkpoint and reverses split building/frontage/occupancy mutations; it does not
invoke the ordinary undo path's global lane rebuild or consume an older undo record.
Terrain validation reuses indexed source queries and dirty 64 m tile assembly, borrowing the live
world while locked instead of copying it. Added journal storage/work is proportional to affected
split records and occupancy cells; terrain work is proportional to affected tiles and patch output.

The `(23, 1)` saved-map failure came from a 29.6 m junction boundary enclosing only 0.0013775 m²
of numeric seam dust. The single-operation area cap incorrectly made this complete boundary survive
cleanup, then opposing curb vertices with heights 92 mm apart collided on the terrain identity grid.
Complete-boundary uncertainty now accumulates the existing 0.1 mm edge strip and vertex floor
without that unrelated fixed cap. Height conflicts remain errors; a resolved hole of comparable
area still reaches normal source/height validation. Regression coverage includes cyclic/reversed
order of the logged contour, terrain rejection after an occupied road split, and cached terrain
availability before accepted-road publication.

Authored water is part of the same placement contract. A `Standard` candidate is rejected with the
stable `water_requires_bridge` reason when its complete roadbed footprint overlaps visible baseline
water. The cheap hover query, exact async preview, live simulation-thread commit, and conversion of
an existing edge to `Standard` all apply the same rule. `Bridge` spans remain legal over water;
`Tunnel` spans remain legal below it.

The query reuses the sparse baseline-water state and samples bilinear depth on a deterministic
quarter-cell lattice across the roadbed. It allocates no per query and introduces no additional
spatial index. Its bounded cost is
`O(ceil(length / step) * ceil(width / step))`, with the sample step clamped to `0.5..=3.0 m` and
early exit on the first visible-water sample.

Visible-world queries use this precedence:

1. road-owned top surface
2. intentionally surfaced road earthwork / structure
3. visual terrain
4. source terrain only for terrain-only APIs

Visible road-height sampling reuses the existing immutable owner-local triangle grids after the
query-chunk lookup. It tests only triangles in each owner's matching cell, retains the original
renderability predicate and highest-top-surface rule, and leaves structural-earthwork fallback
unchanged. Top-surface work is proportional to local owners, their node-visibility adjacency, and
cell triangle candidates rather than every triangle in those owners; sampling allocates nothing
and builds no extra index. The same sampler serves ghost guides, snapping, and other visible-height
consumers. Exact scan-parity tests cover seams, overlapping heights, width edits, deletion, bridges,
and tunnels.

Ghost-guide CPU geometry is retained per edge. Exact physical geometry, immutable road-mesh
chunk identities, and persistent terrain-patch revisions validate reuse over the bounds of the
actual guide samples (including offset curves and ticks). Terrain source brushes and world resets
invalidate the full guide cache. Multiple edits before a fetch cannot lose earlier invalidations;
removed slots drop their lines. Independent refreshes use Rayon and reuse offset/line buffers.
Full output assembly and Godot upload remain O(total guide vertices), in the original edge and
outward/offset order. Validation costs O(source points + C log M), for C dependency chunks and M
existing mesh chunks; height sampling is limited to changed edges. No new spatial index is added.
The native fetch defers until surface and mesh publications match the requested generation.

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
- bounded road undo restores the affected pre-edit surface compiler records, removes post-edit
  owners, and rebuilds only old-plus-restored chunk shells and refined tiles; unsafe or incomplete
  captures fall back to bounded owner compilation, while dense terrain-authoring undo remains an
  explicit full reset
- steady-state chunk rebuilds use sorted contributor lists; they must not scan every compiled node
  piece in the world
- committed road rendering uses the same normalized render-chunk span and world-minimum origin as
  terrain and water. This keeps ordinary central edits away from the world-zero four-chunk corner;
  the target is one chunk for a contained edit and two when it crosses one boundary. Road keys can
  extend outside the bounded terrain grid and remain a separate ownership domain even though their
  in-world boundaries align. A local edit rebuilds the sorted union of changed surface and earthwork chunks,
  collects their unique owners once, and assigns each non-indexed triangle to exactly one
  deterministic XZ-centroid home chunk. Chunk vertices are stored relative to that chunk's origin;
  unchanged mesh buffers remain immutable and shared.
- Rust accumulates changed chunk keys and removal tombstones until Godot acknowledges the exact
  road generation. Godot preflights every resident dirty terrain payload before mutating any patch,
  stages every changed road `ArrayMesh` as a detached instance, rejects stale generations before
  the swap, and commits the complete terrain/road pair back-to-back. Engineered terrain accepts
  only a current-contract `ok` payload with structurally valid clipped baked buffers; this includes
  an `ok` payload that reports already-omitted pathological faces. A missing, empty, failed,
  conflicted, still-pathological, wrong-contract, or malformed engineered payload retains the
  complete previous terrain/road pair and cannot acknowledge the current road generation. Raw
  heightmap terrain is never an engineered-patch fallback.
  The bridge rejects malformed or non-finite layer arrays atomically, and a chunk-span or grid-origin
  change is legal only in a full replacement so retained instances cannot use mixed coordinate grids.
  World replacement clears old chunks before terrain rebuild and remains an explicit full-chunk
  replacement, including the empty-road case. A recreated renderer hydrates from a full snapshot
  even when the simulation has no pending dirty revision.
- ordinary road-render work is `O(affected owners + affected triangles + changed chunk upload)`;
  immutable snapshot publication copies only accumulated unacknowledged update metadata and `Arc`
  handles, never the full occupied-chunk map or unchanged vertex buffers. A renderer-requested full
  snapshot remains `O(total chunks + total vertices)` because it necessarily uploads the network.
- `./run.sh --benchmark-road-chunks` is the reproducible scaling check. Its Rust fixture keeps one
  central 32 m two-lane local road wholly inside one terrain-aligned chunk, with its selected owners,
  target chunk, and emitted vertex signature fixed
  while occupied chunks rise through 1, 64, 256, and 1,024. `chunk_emit_only` isolates targeted
  generation, `full_network_emit` provides the former whole-network-work comparator, and
  `dirty_compile_plus_chunk_emit_diagnostic` separately exposes the known global stale-cache scans
  in dirty surface compilation. The Godot fixture replays the same immutable packed arrays with 4,
  64, 256, 1,024, and 4,096 resident instances while one 8,190-vertex chunk changes, then holds
  residency at 1,024 while 0, 1, 4, and 16 chunks change. It reports median and p95 both for command
  staging and for a render-server synchronization fence; this headless boundary covers validation,
  `ArrayMesh` construction, scene/RID mutation, and command drain, not physical GPU upload or frame
  rendering. Fixture geometry digests and retained-instance checks are correctness gates;
  wall-clock comparisons are made between release runs on the same machine rather than encoded as
  test thresholds. `./run.sh --benchmark-road-chunk-upload` runs the Godot half without inheriting
  thermal/boost state from Criterion.
- Schema-3 measurements use `./run.sh --benchmark-gameplay-roads` or
  `./run.sh --benchmark-gameplay-roads-headless`: release runs without Samply. Existing
  `--profile-gameplay-roads[-headless]` commands profile the same selectable workloads for CPU
  attribution. Old schema-2 timings are not directly comparable. The default `paired` matrix
  resets the entire flat world **before every fixture**, holds the free anchor at `(100,100)`, and
  changes only the corner variant's grid alignment. It covers the eight bend/T/four-way, oblique,
  mixed-width, curved, close-double-T, and chunk-corner layouts. One warmup plus five measured
  repetitions gives 48 fixtures; alternate measured cycles reverse case order deterministically.
  Override counts with `METRUM_GAMEPLAY_BENCHMARK_REPETITIONS` / `..._WARMUP_REPETITIONS`.
- `METRUM_GAMEPLAY_BENCHMARK_MATRIX=scaling` holds the local T edit and terrain fixed while adding
  a remote, internally connected grid. Default sides `0,4,8` contain `0,24,112` live background edges;
  `METRUM_GAMEPLAY_BENCHMARK_GRID_SIDES=0,8,16` gives `0,112,480`. Sizes must be unique, zero or
  `2..64`. Every setup road uses the ordinary async command outside measured edit phases; all grid
  junction degrees and the live edge count are verified. The local component is intentionally
  separate. This includes full edit routing/snapshot costs, not only local mesh emission.
  Intersection candidates and split batches now use stable edge-ID order, with node-ID ties
  resolved explicitly. This prevents hash/R-tree iteration from changing allocated edge IDs and
  subsequent profile authority (`ROAD-16`). Sorting is limited to indexed local candidates and
  touched edges, `O(K log K)` with `O(K)` split-batch storage, not a city-wide scan.
  Side 32 verifies 1,984 edges and all 1,024 grid-junction degrees. Its formerly rejected first
  2,790 m stroke (`ROAD-14`) now passes: terrain-query span footprints are exported in 64 m
  section-station runs so a patch union does not receive an entire multi-kilometre owner loop.
  A failed setup remains a failure, never a large-city performance result.
- `METRUM_GAMEPLAY_BENCHMARK_MATRIX=interaction` compares the same T with a completed preview,
  a 24-point moving pointer trace (60 scheduled inputs/s, one 80 ms midpoint hold), and an immediate
  click without exact preparation. This is scripted world-space motion, **not OS raycast/snapping**.
  `pointer_idle_to_ready_ms` excludes intentional trace duration; immediate clicks have no
  `preview_ready_ms`. Observed request counts and frame intervals accompany each operation.
- `METRUM_GAMEPLAY_BENCHMARK_MATRIX=saved` reloads a paused city from
  `METRUM_GAMEPLAY_BENCHMARK_SAVE_PATH` before each T fixture. It requires
  `METRUM_GAMEPLAY_BENCHMARK_ANCHORS_PATH`, a JSON object such as `{"t_90_2l":[100,100]}`.
  Use matched saved states to vary actual population/building density; the suite does not fabricate
  populations or measure running-simulation contention. Authored Kuopio `controlled`, historical
  growing `baseline`, and targeted `double_t`/`road08` matrices remain diagnostic options. Authored
  anchors can use the same manifest and are recorded in every capture. Different authored sites
  confound terrain with topology: do not call these isolated complexity comparisons.
- Milestones separate `preview_ready_ms` (exact readiness before the extra fence frame),
  `generation_ready_ms`, `render_ack_ms` (matching atomic road/terrain acknowledgement),
  `ghost_ready_ms` (guides uploaded for the committed generation), and `first_idle_ms`
  (also foreground water/border/residency work). `commit_ms` retains the five
  consecutive idle frames; `settle_tail_ms` isolates the final stable tail. These are CPU frame
  observations, **not GPU presentation timestamps**. Headless is uncapped unless
  `METRUM_GAMEPLAY_BENCHMARK_MAX_FPS` overrides it; cadence, viewport, engine, CPU/GPU names, worker
  counts, and source/binary/world fingerprints are recorded. Do not interpret headless/windowed
  differences as GPU execution cost. `state_after.command` exposes generation-matched core stages:
  command-queue wait, locking, add, finalization, surface/terrain, agents/lanes, buildings, routing,
  mesh, snapshot, and refined-state work, plus dirty edges and rebuilt chunks. Finalization includes
  its maintenance children; core work excludes queue wait, context publication and renderer work.
  Do not add inclusive parents and children. Cardinality scans occur outside segment clocks;
  `nodes` counts storage including aliases. Fixture totals include bookkeeping/verification and
  are not the primary response-time KPI.
- Authoring commands wake the simulation thread through its existing channel; they do not wait
  for the next 60 Hz movement tick (`ROAD-18`). Completed edits publish the road/terrain render
  snapshot even between ticks. A separate monotonic deadline gates the unchanged fixed simulation
  delta; camera/command wakeups neither advance ticks early nor postpone their deadline, and a
  slow edit does not trigger catch-up bursts. Coalesced speed/camera updates survive intervening
  wakes. Scheduling adds `O(1)` work without a new queue, thread, or spatial index; edit snapshots
  reuse the existing buffers and publication path.
- Summaries separate `initial_road`, `edit_1`, and later edits. p95 is null below 100 observations;
  p99 is null below 1,000. These are descriptive quantiles, not confidence intervals or independent
  process replicates. The wrapper rejects existing outputs and validates full fixture coverage,
  monotonic milestones, command generations, and visible generation-matched ghost output through
  `tools/road_benchmark_report.py`. Paired captures must also agree on guide vertex counts and
  pre/post-edit graph, lane, agent, and building cardinalities.
  Unavailable native guide data preserves the existing mesh and retries once next frame; it is
  not an empty world. A successful empty result explicitly carries its generation (`ROAD-15`).
  Compare separate unprofiled pairs with
  `python3 tools/road_benchmark_report.py --baseline a1.json a2.json --candidate b1.json b2.json`
  (default `first_idle_ms`; also `--metric render_ack_ms` or `--metric command.routing_ms`).
  Alternate A/B and B/A process order
  without concurrent load, using identical inputs/settings. The report rejects incompatible or
  profiled captures and reports paired process-median deltas/ranges without significance claims.
  Start with unchanged-build A/A runs. Early captures without ghost-generation checks could
  silently drop guide work on lock contention and are not valid baselines or noise controls.
  Fewer than three process pairs are explicitly exploratory; even more pairs do not establish a
  win without repeatability beyond the measured A/A variation. Do not use this suite's default
  single process to gate small gains.
  Geometry rejection, unexpected generations, missing fixtures, and degree mismatches remain
  failures, never faster samples or reasons to relocate a pinned fixture.
- The indexed visible-height sampler was validated with three matched unprofiled release/headless
  process pairs, alternating A/B and B/A order. Each process ran sides `0,8,16` with one warmup and
  two measured repetitions, verifying identical guide counts and pre/post-edit graph/lane totals.
  Both binaries included the guide-retry and deterministic-split fixes; only the sampling path
  differed. Local T click-to-first-idle process medians fell from `251.5` to `64.9 ms` at 112
  background edges (paired reduction `72.5–79.1%`), and from `1139.9` to `139.2 ms` at 480 edges
  (`87.1–88.4%`). Worst observed 480-edge edit frame intervals fell from `1050.3` to `108.1 ms`.
  The empty-background T changed from `38.1` to `41.3 ms`, within the observed unchanged-build
  variation: no small-network gain is claimed. These are road-tool CPU responsiveness results,
  not whole-game FPS or GPU results. Captures and A/A controls are under the ignored
  `benchmark-results/road-perf-pass-1/stable-*` paths; earlier captures in that folder are superseded.
  That pass left full-network guide generation and CCH ordering as the next substantial costs.
- The subsequent `ROAD-14`/guide/CCH pass uses three matched release/headless process triplets:
  A = footprint fix only, B = exact CCH rebuild improvements, C = B plus per-edge guide reuse.
  Each process runs sides `0,8,16,32` once, with no extra whole-fixture warmup; remote-grid setup
  exercises the road pipeline before local measurements. Order is A1 B1 C1 C2 B2 A2 A3 B3 C3,
  without competing compiler/test load. All guide counts and pre/post graph/lane cardinalities match.
  At 1,984 background edges, local T click-to-first-idle medians are `3731 → 202 ms`
  (paired reduction `93.6–94.6%`); the routing stage alone is `3437 → 21.4 ms`.
  With the improved router held fixed, guide reuse reduces that edit from `359 → 202 ms`
  (`40.2–43.9%`). The worst observed T frame interval falls from `3715 → 121 ms`.
  At 480 background edges the same edit falls from `142 → 80 ms` (`28.1–54.9%`).
  These are descriptive process medians and CPU frame observations, not GPU/FPS claims.
  Isolated topology-only Criterion rebuilds also improve: side 8 `0.675 → 0.309 ms`,
  side 16 `36.3 → 1.93 ms`, and side 32 `2419 → 14.4 ms`.
  The faster order exposed discarded shortcut alternatives and premature query termination;
  those correctness defects are fixed and independently oracle-tested (`ROAD-17`).
  CCH still rebuilds globally; scoring is incremental within a rebuild. Complete guide packing
  and upload still scale with total guide vertices.
  Small-network results are not uniformly better. Three additional process pairs at sides `0,8`,
  each with one warmup and five measured fixture repetitions, show the empty-world first stroke
  regressing from `36.2 → 41.2 ms` (`13.9–17.3%`), while its following T is `48.2 → 48.1 ms`.
  The 112-edge first stroke improves `63.1 → 58.4 ms`; its T result is inconclusive.
  That first-road regression was tracked separately as `ROAD-18`; its follow-up is below.
  Captures, staged comparisons, superseded experiments and verification are under ignored
  `benchmark-results/road-perf-pass-2/`.
- `ROAD-18` follow-up isolates an existing uninterruptible simulation-thread sleep: cheap road
  commands could spend up to one 60 Hz interval waiting to start. The interruptible channel wait
  and immediate edit snapshot remove that scheduling delay without changing geometry or routing.
  Three matched release/headless process pairs run sides `0,8`, one warmup and five measured
  repetitions, in A1 B1 B2 A2 A3 B3 order without compiler/test load. Empty-network first-road
  click-to-first-idle medians fall `43.7 → 30.8 ms` (`16.1–36.3%` lower across paired processes);
  command-queue wait falls `7.33 → 0.079 ms`. The following T improves `41.9 → 35.3 ms`.
  Unchanged-build process medians still vary (A1/A2 `−6.5%`, B1/B2 `+19.5%` for first-road latency),
  so these are descriptive measurements, not significance or whole-game FPS claims.
  At 112 background edges, first-road results are mixed across pairs; the following T improves
  `64.8 → 52.6 ms`. Three additional side-32 pairs (one fixture each, no extra warmup after grid
  construction) retain the large-grid gains: first road `194 → 193 ms`, local T `217 → 200 ms`.
  Both large-grid ranges cross zero, so no further large-grid speedup is claimed. All paired
  guide counts and pre/post graph/lane/agent/building cardinalities match. Captures, comparisons,
  binary fingerprints, exclusions and verification are under ignored `benchmark-results/road-18/`.
- `ROAD-19` initially added the compiled junction display without changing the moving ribbon or idle delay
  (that scheduling is superseded by `ROAD-21`).
  Three matched release/headless process pairs use the same current frontend/readiness-checking
  harness with baseline and candidate native libraries, sides `0,8`, one warmup and five measured
  repetitions, in A1 B1 B2 A2 A3 B3 order without competing build/test load. Readiness now includes
  successful junction-mesh staging. Empty-background T preview readiness is `67.4 → 67.5 ms`;
  with 112 background edges it is `61.3 → 67.3 ms`. Both paired ranges cross zero and unchanged-build
  readiness varies by about 10–12% on some operations; this is a visual improvement, not a speedup.
  Commit results are mixed: empty first stroke `32.8 → 34.2 ms`, T `40.6 → 35.6 ms`; 112-edge first
  stroke `42.9 → 47.2 ms`, T `53.6 → 49.9 ms`. The 112-edge first-stroke paired range is `+0.3–13.2%`;
  its mesh stage is only `0.403 → 0.445 ms`, so the total difference cannot be attributed to owner
  recording from these observations alone. Guide output and pre/post graph/lane/agent/building
  cardinalities match. Three additional side-32 process pairs (1,984 background edges, one fixture
  each after grid construction, no extra whole-fixture warmup) leave T preview readiness unchanged
  at `62.0 → 61.9 ms`, but T click-to-first-idle regresses `182.0 → 206.5 ms`
  (`+4.4–15.2%` per pair). Its mesh stage stays `0.459 → 0.459 ms`; snapshot/surface and
  post-command publication times account for the observed difference, without an isolated causal
  explanation in that pass. This regression was tracked as `ROAD-20`; its later recovery is below.
  The 48-fixture layout matrix, three interaction fixtures, saved-city delete/redraw/save/load replay,
  four Godot suites and single-worker junction replay pass. Real-renderer T/cross/unequal-width
  captures checked road appearance on a solid plane; they did not exercise committed terrain cutouts.
  `ROAD-23` supersedes that incomplete seam validation and the blanket preview hover offset.
  Captures, noise controls, fingerprints and exclusions are under ignored
  `benchmark-results/junction-preview/`. These remain CPU/headless observations, not GPU or FPS results.
- `ROAD-21` replaces idle-gated junction display with continuous latest-input updates. Three serial
  release/headless captures per implementation use 96 moving inputs per T, crossing and unequal-width
  T trace, with no midpoint pause. The old implementation shows zero junction updates during motion;
  final captures show `58–72` for T, `40–43` for crossing and `46` for unequal-width T. Final per-process
  median input-to-new-mesh latency is `19–37 ms`; the age of the currently displayed pose has medians
  `20–50 ms` and maximum `84 ms`. First display takes `20–52 ms`; all remaining motion frames retain
  a junction. Tool-process CPU medians are `0.36–2.07 ms`, maximum `15.40 ms`. These are separate
  processes, not paired speedup estimates or GPU/FPS measurements; actual input timestamps are
  recorded and the timers do not guarantee OS-pointer or presentation cadence. Each distribution
  has fewer than 100 samples, so p95/p99 are intentionally absent. The final harness adds continuity
  assertions and optional visual-only captures to the baseline trace; input geometry/cadence is unchanged.
  A contention regression caught pending validation incorrectly clearing visible geometry; it now
  retries while preserving the last display-only pose. All 1,566 Rust tests pass (one ignored), all
  Criterion targets check, and five Godot suites pass. The 48-fixture matrix, three continuous-input
  interaction fixtures, sides `0,8,32`, saved-city replay and single-worker stream also pass.
  Software-rendered T/cross/unequal-width poses during motion and matching preview/commit captures
  pass; visual-only captures deliberately pause for readback and are excluded from timing results.
  Timing captures, excluded diagnostic runs and binary/frontend fingerprints live under ignored
  `benchmark-results/junction-realtime/`. These runs did not establish recovery of the commit
  regression; the separate `ROAD-20` follow-up is below.
- `ROAD-22` extends the compiled scene to bends and straight continuations, including unequal
  widths and sloped terminal reprofiles. It removes unconditional hover-ribbon construction and
  consumes matching exact results before cheap validation. A red-before/green-after regression
  also prevents a pending parcel check from wasting a completed display pose. Three interleaved
  baseline/candidate release/headless pairs (A1 B1 B2 A2 A3 B3), with 100 warmups and 1,000 measured
  calls per lane count, reduce unused-ribbon hover validation p50 from `78 → 40 µs` for two lanes
  and `135 → 56 µs` for eight lanes. All calls validate, cost checksums match, and exported unused
  ribbons fall from 1,000 to zero per case. This saves `38–79 µs/call`, not half of total preview time.
  Three final 96-input motion captures show `67–90` normal-bend and `66–81` unequal-width-bend
  updates, versus zero before; new-mesh latency medians are `18–19 ms`. Existing T/crossing update
  counts remain within before/after variability; early higher T rates did not persist, so no stable
  whole-preview speedup is claimed. Displayed pose age still reaches `100 ms` in a crossing run;
  continuous updates are not a guaranteed frame-time bound. All 1,569 Rust tests pass (one ignored),
  including cold-commit bend/continuation comparisons, plus five Godot suites, the 48-fixture matrix,
  interaction/scaling/saved-city/single-worker replays and rendered bend/commit checks. Captures,
  exact fingerprints and diagnostic exclusions live under ignored `benchmark-results/bend-preview/`.
  That pass did not recover `ROAD-20`. These are CPU/headless measurements, not GPU or presentation latency.
- `ROAD-23` removes blanket preview lift over committed coverage and closes vacated cutouts with
  temporary ground. Nine actual-terrain rendered fixtures (T, crossing, unequal-width T, water-reset,
  bend, unequal-width bend, continuation, adjacent T junctions and slope) reduce preview-only sky
  pixels from `142–355` to zero; cancellation restores the exact source coverage. The slope references
  retain six source/eight cold-commit subpixel seams: those exact reference pixels are not attributed
  to preview or claimed fixed. Tests now use clipped CDT patches rather than a solid plane.
  All 1,573 Rust tests pass (one ignored), plus five Godot suites, single-worker junction replay,
  benchmark-target compilation and formatting. Three matched release/headless process pairs use
  the identical frontend and 96-input moving traces with no competing build/test/image-analysis load.
  Median-of-process-p50 new-mesh latency is T `35.9 → 36.5 ms`, crossing `36.7 → 36.7 ms`, unequal T
  `35.7 → 36.0 ms`, bend `18.6 → 18.5 ms`, and unequal bend `18.4 → 19.1 ms`. Wide-bend updates are
  `74/87/74 → 62/70/75` per 96 inputs; the added geometry has a measurable cost in two pairs.
  T results remain noisy (one paired p50 crosses a frame boundary by `16.9 ms`); these samples do
  not establish unchanged performance or a speedup. Maximum final displayed-pose age is `83.6 ms`.
  Captures, exact build/input fingerprints, paired distributions and exclusions are under ignored
  `benchmark-results/preview-seams/verification.txt`. The final release library is deployed; live
  terrain and commit semantics remain unchanged. This is not full future excavation/grading.
- `ROAD-20` follow-up removes redundant whole-network ghost-snap indexing from road-tool snapshot
  publication. A current side-32 Samply capture with existing stage timers measures `31.5 ms` building
  that index inside a `40.1 ms` T snapshot. This identifies avoidable
  commit work, not proof that mesh-owner bookkeeping caused the historical `182 → 207 ms` delta.
  The final implementation queries the existing immutable edge index and streams exact local guides;
  its internal visitor uses no heap-backed traversal/candidate buffers. Equal-distance snapping uses
  world-XZ order, and ordinary collecting graph queries retain their existing traversal order.
  Three matched final release/headless process pairs run sides `0,8,32`, one fixture each, no extra
  fixture warmup after grid construction, in A1 B1 B2 A2 A3 B3 order without competing build/test load.
  At 1,984 background edges, first-road click-to-first-idle medians fall `175.3 → 152.6 ms`
  (`9.2–19.0%` lower per pair), and T medians fall `199.2 → 140.8 ms` (`21.7–32.6%` lower).
  T snapshot work falls `40.6 → 6.1 ms` (`84.7–86.1%` lower per pair). Unchanged-build side-32 T
  comparisons vary up to `3.6%` for A and `13.0%` for B, below each paired T improvement.
  Three additional pairs at sides `0,8`, with one warmup and five measured repetitions per fixture,
  reduce 112-edge first-road medians `54.1 → 39.5 ms` and T `54.6 → 41.3 ms`; all pairs improve.
  Empty-network first-road/T paired ranges cross zero: no empty-network speedup is claimed.
  Preview readiness is mixed: scaling empty-network T rises `40.4 → 47.3 ms`, while
  warmed 112-edge T falls `47.3 → 40.9 ms`. Fixed scripted inputs bypass mouse snapping; the separate
  release cursor diagnostic measures dense queries at `98.7–98.9 µs`, versus `1.29–1.56 µs` for a
  prebuilt reference index, at both 112 and 1,984 edges. This accepts about `0.10 ms` local cursor work
  to eliminate whole-network edit work; it is not a query-speed or universal frame-time improvement.
  Guide counts and pre/post graph/lane/agent/building cardinalities match in all accepted pairs.
  All 1,575 Rust tests pass (two ignored), plus five native Godot suites, single-worker junction replay,
  benchmark-target compilation and formatting. Exact preview geometry, terrain, commit validation and
  visible guide generation remain intact. Final captures, unchanged-build controls, fingerprints and
  exclusions are under ignored `benchmark-results/road-20-33koNf/verification.txt`; the final release
  is deployed. These are descriptive CPU/headless process medians, not GPU/FPS or significance claims.
- Criterion now distinguishes `compile_dirty_unchanged_edge` / `compile_dirty_unchanged_terrain`
  from real `compile_dirty_lane_width_change` / `compile_dirty_changed_terrain` kernels. The grid
  is centered inside its terrain, sample/world coordinates agree, changed samples lie inside the
  invalidation footprint, and every compile must publish successfully. Mutation and initial cache
  construction and input destruction are outside the compile timer; only one large prepared
  input is retained per iteration. `cargo bench --bench surface_benchmark -- --test`
  smoke-executes all kernels. `./run.sh --test` checks all Criterion targets for API drift and runs
  benchmark statistics/report regressions alongside existing tests.
- Before `ROAD-21` removed the idle gate, exact previews started after `25 ms` of pointer
  idle instead of `100 ms`, with motion resetting the delay. A completed exact rejection still
  replaces the earlier cheap candidate verdict, so the
  tool turns invalid immediately rather than displaying a stale valid coarse preview. The targeted
  `METRUM_GAMEPLAY_BENCHMARK_MATRIX=road08` workload keeps the formerly 90-second apparent-hang
  site pinned. The old expected rejection is stale: current geometry accepts the exact preview and
  commits the curve, so schema 3 requires successful settlement and a degree-two junction. Two
  clean-world replays pass. Exact validation no
  longer invokes a cold full compile over every node copied into its bounded graph excerpt. It marks
  only the candidate-required edges/nodes and incident topology dirty, then runs the same incremental
  compiler used by authoritative edits; the existing required-piece checks and exact preview-to-commit
  certificate remain unchanged. Samply attributes `18.1%` fewer preview samples to node compilation,
  while direct compiler timing for the hardest double-T preview fell from about `41 ms` to `27 ms`.
  `METRUM_GAMEPLAY_BENCHMARK_MATRIX=double_t` isolates that clean-world fixture for repeated checks.
  Exact preview topology processing also records and profiles the same bulk split-edge dirty ledger
  consumed by simulation-thread commit finalization. The insertion-dirty-node and bulk-profile-node
  scope builders are shared by preview and commit, keeping the work proportional to the locally
  changed topology while preventing scope drift. On the close double-T's third commit this changes
  exact reuse from `2/5` to `5/5` spans and from `2/4` to `4/4` nodes; direct surface compilation
  falls from about `37.3 ms` to `0.69 ms`, and repeated end-to-end third-commit p50 falls from
  `92.0 ms` to `83.0 ms`. The targeted regression requires complete artifact reuse across all three
  bulk commits, and the clean 32-fixture controlled release profile passes.
  See
  [`reference.md`](reference.md) for artifact names, controls, and interpretation.
- centroid ownership makes these chunks deterministic update/upload batches. A triangle may extend
  beyond its home chunk, so this contract does not yet authorize independent chunk streaming or
  residency; Godot derives each instance AABB from its actual vertices for normal scene culling
- refined road-touched terrain uses fixed world-aligned bounded CDT core tiles rather than
  connected-footprint windows whose bounds grow with the road network
- an interior border-candidate query must reject against the immutable world snapshot without
  acquiring the simulation-core lock; only an endpoint actually near the map border may enter the
  authoritative node lookup
- independent span and chunk work should use Rayon when mutation boundaries allow it
- hot-path loops must avoid avoidable allocation
- road materials are prewarmed when the resident road tool enters the main scene, before the first
  committed road mesh needs them
- road debug output must split terrain, water, zoning, and total patch-debug timings
- road debug output must use cached zoning statistics instead of scanning parcel payloads

`ROAD-05` is the active refined-terrain performance contract:

- each tile has stable global-grid identity and parent-patch-clipped core bounds separate from its
  content fingerprint
- tile inputs collect exact road/site contributors and their full required grading influence; a
  guessed fixed-neighbor halo is not an acceptable substitute
- the full local fingerprint covers every clipped contour and provenance record, local terrain
  sample, grading guide/constraint, render step, core bound, and contract revision that can affect
  output, while unrelated world geometry cannot invalidate the tile
- an edit rebuilds `old_coverage union new_coverage` plus deterministic seam-dependency tiles, so
  moved or removed ownership cannot leave a stale clipped hole
- unchanged fingerprints reuse immutable compiled tile geometry and render buffers from the last
  accepted generation; cached buffers include vertices, normals, UVs, indices, local normal-sum
  magnitudes, and side-seam manifests, so only changed tiles enter conversion and the Rayon build
  set
- regular boundary-lattice samples stay local to directly adjacent seam filler; only non-lattice
  geometry breakpoints become patch-wide filler partitions, preventing the fixed tile lattice from
  producing a Cartesian filler-grid expansion
- only tiles with an exact road/site contributor enter Spade; contributor-free dirty neighbors use
  that side-manifest-aware regular filler, and each contributor's adaptive grading margin is probed
  once then shared by coverage and guide generation; on the focused double-T workload this halves
  the first affected patch from `16` CDT windows to `8` and lowers aggregate commit p50 by about
  `7.8%`
- independent contributor margin/guide jobs and independent tile clipping/sampling/fingerprint jobs
  use ordered Rayon collection; canonical manifest aggregation stays serial, preserving exact output
  while cutting dense double-T refined-input p50 from `10.677 ms` to `4.886 ms`; across the full
  controlled matrix this lowers measured commit CPU samples by `16.1%` and commit wall-time sum by
  `6.3%`, with all 32 fixtures passing
- pre-clipped source provenance first resolves exact component edges through a deterministic sorted
  edge index and reserves the metric `O(E * P)` overlap scan for nonexact partitions; canonical CDT
  input skips source-vertex recovery only when every source endpoint is already present at the same
  quantized position and height, reducing the normal path to `O((E + P) log P)`. In the controlled
  Samply capture this removed all sampled source-split work, reduced outer road-loop clipping from
  `567` to `386` samples (`31.9%`), canonicalization from `294` to `236` (`19.7%`), and complete
  refined-CDT construction from `795` to `700` samples (`11.9%`), with all 32 fixtures passing
- one refined patch is published atomically only for the exact requested generation; stale work
  cannot publish, while the immutable last accepted generation remains the visible and reusable
  source until a complete replacement is accepted
- one road-surface edit stages every affected span and required node piece before changing published
  compiler records or chunk indexes; a failed required `JunctionN` latches that invalidation
  generation, leaves its dirty work pending, and retains the last complete surface and mesh instead
  of repeatedly compiling or publishing a mixed old/new generation
- replacing the runtime world publishes the new final road-surface/mesh generation before terrain
  and water workers resume; water-only road-tool query revisions continue to use the unchanged
  published road generation for clipping instead of forcing or waiting for an unrelated mesh rebuild
- the current Godot contract still publishes one complete mesh per render patch, so final buffer
  concatenation, filler, duplicate-normal reconciliation, and upload remain
  `O(patch output vertices + indices)`; that bounded step does not re-query, re-triangulate,
  reconvert, or rescan triangle normals and window-side vertices for reused tiles

Complex `JunctionN` compilation now retains canonical contact and ownership-topology caches behind
shallow-cloned immutable handles. An exact compile-input match during terrain-only invalidation
keeps the final top surface and rebuilds only earthwork. An exact-XZ, height-only edit reprojects
cached inserted contact vertices onto fresh height carriers and reuses the rail/contact topology
when ordered node-local mouths, rail topology, and the carrier registry still match. Raw graph edge
IDs are publication metadata rather than topology identity; projected side joins still receive the
current generation's edge IDs. When a topology-changing edit prevents whole-rail reuse, exact
same-material contour-pair contributors retain immutable contact results and only pairs touching a
changed contributor rerun their overlay work. Raised-step compilation separately caches exact
target-group unions, source/group contributions, exact source/owner-group-pair overlap,
source/source contact points, and cross-kind contour-pair output including empty results. A fixed
world-aligned source/target tile index and exact semantic source registry bound candidate discovery
before cache lookup. Current-generation results are not replayed into the second pass: only newly
introduced source contributors, pairs touching them, or changed target groups do geometry work,
and contact-point incidence queries only the indexed local sources. Cross-kind pair fingerprints
visit the exact owner-pair authority bucket with cached bounds instead of rescanning all constraints.
Positional mouth or owner rebinding remains a safe cache miss rather than replaying stale output.

Contact noding retains both exact pair-local candidates and final ordered-XZ output for each
connected potential-contact component. An unchanged component replays its final keys through the
canonical contour setter, which reprojects fresh height carriers and updates current constraints;
any member, relevant role constraint, component merge, split, addition, or removal uses the full
deterministic fixed-point path. Contact retention keeps exact authority buckets behind immutable
handles, reverse-indexes source presence and owner/kind handoffs, and reuses collision-checked keep
decisions whenever their exact relevant buckets are unchanged. One current-generation authority is
shared by final retention and endpoint validation; building that authority after a source edit
still traverses current source geometry once, while each decision fingerprint is proportional to
its relevant buckets instead of all contact sources. Expensive raised-step geometry is therefore
bounded by changed source/group contributors and source pairs touching new sources; unchanged
noding components cost their output size, while changed components retain the existing fixed-point
bound. Whole boolean ownership is still reused for an exact uniform canonical-mm translation across
every paired contour and carrier height. When a topology or non-uniform-height edit requires fresh
ownership, canonical cleanup now reuses exact contributor-local clean/union and final self-touch
split results. Final-boundary construction reuses exact point-provenance decisions keyed by the
locally relevant footprint point and source-local carrier geometry and heights. Once those points
produce the same ordered owned shapes and rail constraints as the prior generation, one exact
assembly entry replays the final footprint, region seams, boundary arrangement, and diagnostics,
skipping footprint union, seam reconstruction, boundary-reference construction, and arrangement
construction. Promoting that assembly also preserves only the exact seam contributor keys recorded
while building it, so unrelated intermediate entries do not accumulate across generations.
Region-seam extraction and noded edge-seam materialization use the same immutable
previous-generation promotion rule: only exact contributors encountered directly or recorded as
constituents of a reused assembly enter its replacement cache; a changed build drops entries it no
longer encounters. Contributor caches are retained for `Terminal`, `Bend`, and
`JunctionN`, allowing a two-road `Bend` to seed the common third-road transition; non-junction
entries retain the contributor state without retaining complete rail and ownership payloads.
Caches are shallow-cloned behind immutable handles. Boundary-reference construction uses the
existing quantized point index instead of scanning every region and footprint point for every owned
edge.
On assembly misses, global footprint union, fixed-point convergence, arrangement validation, and
atomic publication remain live deterministic correctness barriers. Topology-changing final
assembly is therefore reduced by local cleanup/seam reuse but is not yet strictly proportional to
changed regions; canonical ownership is still only partially incremental.

Node export now promotes immutable semantic products from the last successful topology generation:
final explicit-step topology, candidate height conflicts per stable exact-XZ vertex cohort, raw
top-boundary contributions per region geometry, and raised-step spans plus unoriented face
geometry per exact step/support fingerprint. Current-generation explicit-step authorization,
global multi-XZ conflict aggregation, owner-wide top-edge cancellation, face deduplication,
orientation, and sorting remain live. Positional explicit-step and grade-authority indices are
bound only against the current generation, and a replacement export cache publishes atomically
with the rest of the successful node topology. The discarded arrangement-derived raised-step face
pass is test-only; production builds directly from final top support. Export diagnostics split
final explicit-step topology time from height-split validation and report product-specific reuse.
Final-step misses use compact edge keys, edge-relevant authority fingerprints, and the shared
world-aligned segment-tile index, so changed work compares only changed edges with spatially
overlapping compatible boundary candidates; negative and duplicate global pair keys are not
materialized.
Matching preview/commit junctions now also retain the final attached arrangement. Reuse first
rebuilds the bounded base rails and checks the complete node-local rail key and source-carrier
registry; boolean ownership is reusable only under an exact uniform canonical-mm translation, and
the attached arrangement is reusable only when every carrier height has identical floating-point
bits. This skips repeat height-field construction, arrangement noding/conflict validation,
triangulation, triangulation validation, and face attachment without making raw graph IDs semantic.
Complete cached top-face, boundary, and assembled node-export buffers remain later work.

A remote crossing may replace one half-edge incident to an older junction while creating another
junction elsewhere on that road. Both nodes belong to the same required publication generation.
Boolean vertices inside the numeric-dust envelope may select an exact generated side-join over
dust-near mouth carriers only when that unique side-join contour owns the point and every
alternative is a declared same-source mouth carrier; raised-step authority may transfer across
paired same-material owners only when an exact same-height, same-source-band bridge region covers
the whole edge; and longitudinal curb/sidewalk seam endpoint drift is accepted only inside the
deterministic overlay-dust envelope.

Topology-changing final `JunctionN` compilation is still synchronous on very large multi-mouth
nodes. Future final-node responsiveness work should retain these requirements:

- async final compile with versioned jobs
- immutable compile snapshots
- deterministic latest-result publication
- old-mesh / pending-mesh visual state
- incremental `JunctionN` compile or stronger contact/export indexing

Do not reopen shipped `ROAD-01` geometry hardcuts for editor responsiveness unless the fix changes
the roadbed ownership contract itself.

## Kuopio Terrain Regression Replay (`ROAD-24`)

`benchmarks/fixtures/kuopio-terrain/kuopio-terrain-map.sqlite` is the immutable, version-59
user capture with 70 saved road edges (about 27 MB). `placements.json` reconstructs 37 logged
two-point placement attempts as 12 connected-site sequences plus a separate rejected-attempt
case. The importer verifies the final accepted physical polylines against the save and every
source-terrain sample against `godot/bootstrap/worlds/kuopio_324km2_10m.sqlite`. The processed
Kuopio database is a different file and is not substituted. Runtime checks pin both asset hashes.

```bash
./run.sh --replay-road-terrain-headless
./run.sh --replay-road-terrain
# Select one location, including its preceding road edits:
METRUM_GAMEPLAY_TERRAIN_CASE=kuopio_04 ./run.sh --replay-road-terrain
```

These commands select the existing gameplay harness's `terrain` matrix. One invocation runs
each selected case once; use independent invocations for repeatability. Each case reloads clean
source terrain, retains its ordered road edits, and drives production RoadTool preview/commit
with the existing generation/render/guide settlement fences. Inputs use recovered post-snap
XZ endpoints and recorded lane counts; heights are resampled from current source/visible
surfaces, never copied from the broken committed profiles. This is **location reconstruction,
not an exact mouse/control-point/Shift-key replay**. Unsupported or incomplete log imports fail;
failing locations are neither relocated nor prefiltered. A failed step stops dependent steps,
but independent cases continue. Rolled-back staged geometry never seeds subsequent inputs.

The diagnostic helper exports prepared preview profiles and Rust-produced physical road/source
height pairs after each step, records expected edge counts, and writes checkpointed metrics.
The windowed path also captures paired `before`, `preview`, and `committed`/`failed` screenshots
using the actual gameplay viewport. Outputs stay under ignored `benchmark-results/`: metrics,
`.terrain-audit.json`, and a `*.metrics-artifacts/` image directory. Source saves are never written.
The profile export is O(edge slots + profile points), parallel across edges with Rayon, and
runs only at diagnostic boundaries, not in production per-frame paths.

The Python audit explicitly checks severe profile budgets: grade ratio 1.0 (45 degrees),
absolute source offset 5 m, and adjacent longitudinal pitch change 30 degrees. These are
fixture acceptance budgets, **not new gameplay grade limits or a complete road design spec**.
It reports offending positions, source grades, cut/fill, and pitch changes; nonfinite/vertical
profiles, incomplete cases, and failed placement/render work fail the run. Exit 2 indicates a
completed diagnostic with failed audit checks. Frozen reference-save profiles are reported
separately, not treated as desired heights or required to change before repaired replays pass.
Numeric coverage currently excludes watertightness, terrain caps, and complete rendered
preview/commit mesh parity: screenshots still require visual review. `RoadEditPlan` and terrain
geometry fixes remain outstanding, rather than being hidden by a green settlement-only test.

Initial release/headless and Forward+ validation completed all 13 cases with identical profile
audit results: 12 settled their full sequences and the recorded terrain-conflict case rejected
again. The rendered run saved 117 before/preview/commit-or-failure images. Replays reached grade 11.76 (about 85 degrees),
37.34 m source offset, and 117.78 degrees adjacent pitch change. These are reproduced defects,
not accepted baseline quality. Dumps/screenshots run outside operation timers but perturb caches
and preview dwell; these diagnostic timings are **not accepted performance measurements**.
Keep using matched unprofiled schema-3 workloads for performance claims. Run launchers serially:
`run.sh` redeploys the shared GDExtension library.

To import a fresh capture without overwriting an existing manifest:

```bash
python3 tools/road_terrain_replay.py --import-log road-terrain-locations.log \
  --output benchmark-results/kuopio-placements-check.json
python3 -m unittest discover -s tools -p test_road_terrain_replay.py
```

## Debug And Diagnostics

`--debug road` must report enough data to locate ownership failures before rendering hides them:

- dirty edge / node / chunk counts
- node kind and incident mouth count
- rail, ownership, arrangement, triangulation, export, and validation timings
- contact candidate counts; source/group, source-pair, noding pair/component, and retention-cache
  hits/misses; and emitted constraint counts
- terrain CDT input/output counters
- per-patch final terrain mesh face-delta, face-slope, tie-in widening, retaining-wall face, and
  longest-triangle-edge summaries after the regular filler mesh has been appended
- retained road seam constraint counters
- final span / node top-region polygons and triangles with owner, material, and provenance keys
- compact cut/fill summaries for touched roads, including max fill, max cut, max grade, and a
  `near-grade` / `fill-heavy` / `cut-heavy` / `mixed` mode label
- post-boolean node footprint / asphalt / non-road shapes, owned-region contours, side-join
  contour provenance, and corner-trim application state when geometry-dump debug capture is active
- opt-in `METRUM_DEBUG_ROAD_PROBE=1` hover probes that log every final road-surface triangle under
  or near the probed XZ point, including material, owner, node/span source, region id, and triangle
  coordinates
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
- terrain-CDT clipping retains every positive-overlap source segment when one output edge spans
  multiple source-owned boundary IDs
- terrain-CDT grading envelope behavior for convex and concave roadbed footprints
- rejection of terrain faces inside road-owned footprints
- authored and imported DEM terrain agreement
- deterministic rebuilds and equivalent edit-order identity
- local invalidation without unrelated chunk rebuilds
- refined-CDT unchanged-tile reuse, changed-tile rebuild, old-coverage removal, deterministic shared
  tile seams, and stale-generation rejection
- rendered mesh upload containing the same canonical raised-step intervals as the compiled surface
- a missing, empty, failed, conflicted, still-pathological, wrong-contract, or malformed engineered
  terrain payload prevents all sibling terrain uploads, road-chunk swaps, and network
  acknowledgement for that generation; production-shaped Godot coverage exercises the real
  prepare/stage/commit transaction, while `ok` contained output remains a baked clipped mesh
- road render chunk partitioning preserves the complete global triangle multiset across shifted,
  positive, and negative chunk boundaries, with no duplicate triangle ownership
- the terrain-aligned road grid keeps a representative central local road in one render chunk
- the deterministic road-chunk benchmark preserves identical local owner/output signatures at each
  Rust scale and identical changed payloads at each Godot resident-instance scale

Use the focused surface tests for narrow changes and the full `surface` suite when changing shared
ownership, terrain, query, node, or render contracts.

## Archive

The archived hardcut history remains useful when investigating why a repair path is forbidden or
why a particular provenance rule exists:

- [`archive/roads_hardcut_history_2026-05-31.md`](archive/roads_hardcut_history_2026-05-31.md)

Do not update the archive as live planning. Update this file, [`roadmap.md`](roadmap.md), and
[`project.md`](project.md) for current road status.
