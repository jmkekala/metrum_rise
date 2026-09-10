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
- finalized physical profiles retained on edges after the shared `RoadEditPlan` finalizer

The graph is not the final visible road surface, final terrain clip carrier, final earthwork
boundary, or final node polygon carrier.

### RoadSurfaceSystem

`RoadSurfaceSystem` owns:

- source-profile preparation and compilation of the graph's finalized longitudinal profiles
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
half-width so ordinary acute mouths do not become long node-owned approach regions; mixed-width conflicts may
expand farther when needed for canonical side-join ownership. That expansion is geometry ownership,
not visual padding.
Edge span sampling, visible-surface section queries, and grounded `Standard` road earthwork ranges
must consume the same node-mouth ownership policy. Ownership answers which piece renders a surface,
not where elevation may change. Grounded approach profiles may rise or fall inside node ownership;
the shared crossing core and all material/terrain seams still require compatible heights and grades.
This rule is independent of source sampling density.

Degree-two `PassThrough` nodes own no node platform and therefore never run the `Bend` /
`JunctionN` endpoint-profile rewrite. Their incident spans preserve the already validated authored
vertical profiles. At a shallow alignment change, both spans use one deterministic bisected
cross-section axis at the shared endpoint, so `Bridge` / `Standard` and same-class handoffs meet
without a transverse cap, terrain slit, or overlapping roadbed.

Incident `Bend` / `JunctionN` rails use a shared graph endpoint profile plane before section, span,
and node compilation. Profile distances are measured in horizontal XZ metres because the cap is a
road grade rule, not a 3-D polyline-length rule. The source-fit and final plan solve may grade-limit only when
limiting stays close to the incident road samples; otherwise the original source-supported plane is
preserved.
Clip rebuilds update ownership distances only: they must not copy hard-pinned control geometry
over the independently eased physical profile. Junction support materialization and edge splits
sample the existing physical profile at control stations in a linear monotonic walk. Explicit
terrain authoring remains responsible for synchronizing both profiles after a terrain edit.
On a link to another connected endpoint, endpoint blending is bounded to its half of the link;
it must not overwrite the opposite endpoint's profile supports. An open terminal keeps the full
transition length because it has no competing endpoint profile.
For a true two-mouth `Bend`, fit the node-local plane to the incident source grades rather than
requiring a horizontal platform. The compatible crossing core is determined by incident roadbed
overlap, independently of the more distant visual ownership handoff. The 32 m transition outside
that core is retained: reducing its length is not the remedy for an unnecessarily constrained area.
Grounded road edits resolve grade limits and the final physical profile once in the shared
`RoadEditPlan` topology finalizer. Road and terrain compilation consume that physical solution;
surface compilation may construct lateral crossfall and material offsets, but must not apply
another longitudinal flattening blend. Structural approach integration remains tracked separately.

For any `JunctionN`, the endpoint profile solve first looks for deterministic opposite-mouth
authority corridors. Existing stable corridor mouths score above edited branch mouths, and the
highest-scored corridor owns the node base plane. Secondary opposite branch pairs must not rotate
that base plane when another road is added to the same `JunctionN`. When such a corridor exists,
the authority mouths retain their original control-grade authority. All affected physical crossing
profiles must nevertheless agree with the selected solution. When no compatible authority corridor
exists, the node falls back to the all-mouth least-squares plane. Finalization must not exclude a
prepared steep through-road from authority selection merely because its grade exceeds the
conservative raw-input fit limit; a later flat branch must not take over that hillside. The final
16% mouth-grade target applies only when it moves the fitted 12 m samples by at most 0.5 m.
Otherwise the supported plane is retained, including its crossfall during surface reconstruction.
Any changed mouth preserves the compatible crossing core and blends back to source grade
over a bounded transition. The 12 m profile sample is only a stable solve/control sample, not a
visible hard platform extent; it may anchor the source profile plane, but physical road geometry
must receive the same vertical-curve blend as other post-mouth support points. Support vertices are
materialized at the solve sample and sparsely through the outer transition so later section sampling
reads a gradual source-grade transition instead of one large planar ramp back to raw terrain.
Visible road-surface sections inside an active profile fade use a denser transition cadence than
ordinary road spans, so the rendered asphalt approximates the vertical curve instead of exposing
long planar facets.
The edit's adaptable edge set controls authored control-profile changes; any stable physical profile
changed to maintain crossing agreement becomes an explicit plan output and dependency. Section
sampling must also suppress sub-decimetre protected-handoff slivers so a support point cannot
create a visible near-vertical roadbed face immediately outside node ownership.

Topology dirtiness is not profile authorship. A local edit ledger records newly authored or
explicitly reprofiled roads; splitting an existing road does not make either retained half a new
grade authority. Split children inherit profile authorship only from an authored parent. Preview
and cold commit drain the same ledger, while adoption/rollback/load reset it. At an existing
junction dirtied only by a remote split, preserve control geometry and apply only the change from
the previously solved plane to its physical transition. Reapplying the entire blend accumulates
cut/fill with each nearby edit. New junctions still receive one complete final-profile solve.
This adds O(local authored edges) bookkeeping and O(local incident profile points) work, with
no resident-city scan.

Straight side joins retain both incident approach profiles, including their interior stations;
an extrapolated collinear miter must not send the path past its endpoint and back. After canonical
boundary-loop cleanup, discarded spur vertices must not reenter CDT as unconstrained guides.
Compact the local vertex/lookup/constraint set before adding intentional interior guides;
repair contours use the same loop cleanup. This stays bounded to one owned node region
(O(vertices + lookup entries + constraints log constraints)); height and coverage checks are unchanged.

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
- road-edit junction profile solving resolves its final grade target before applying the physical
  transition; true two-mouth `Bend` nodes fit the incident grades, while affected `JunctionN` mouths
  solve through deterministic authority-corridor selection; stable opposite-mouth corridors are
  scored deterministically, the best corridor keeps the node base grade for the whole `JunctionN`,
  physical crossing profiles adapt into that plane, and no-compatible-corridor cases fall back
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

Save/load first compiles saved node positions, road grades, physical lengths, and junction clips
unchanged. Only rejected grounded junctions receive one local pass of the shared profile finalizer;
load fails if the detached graph still cannot compile. Valid nonincident profiles remain unchanged.
Derived road surfaces, terrain earthworks, and lanes rebuild without city-wide terrain-authoring
resynchronization. See [saved-reference validation](#coverage-and-saved-reference-completion).

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

The moving and fallback stroke previews render asphalt, lane dividers, curbs, and sidewalks rather than a
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

When the exact worker compiles a standard road, including an isolated stroke or an affected
`Bend`, `JunctionN` or `PassThrough`, feedback shows the local terminals, connections and spans
through the committed road renderer: asphalt, sidewalks, curb/step
faces, crosswalks and lane dividers. It exports the already-compiled validation neighborhood,
reconstructing lanes only inside that excerpt for the normal marking rules. It does not compile
the junction again, rebuild city routing or modify the live graph. Before display-only lifting,
all vertex attributes must match an independent cold commit, including unequal widths, slopes,
four-way crossings and an old terminal that becomes a pass-through without a nearby junction.
Isolated strokes move the same already-compiled neighborhood into this path, with no removed
source owners. The retained chunk batch may be empty on a blank map; unrelated roads sharing
those chunks retain their exact geometry. Empty old cutout sets bypass vacated-ground boolean
work entirely. Terminal-extension previews include the reprofiled existing edge
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
The exact async preview carries a shared `RoadEditPlan` for its matching commit. The simulation
thread requires identical raw road points, lanes, snap mode, surface generation and source-terrain
revision before borrowing the prepared profile and skipping its exact candidate replay. The plan
also retains the finalized local graph delta: resolved connections, splits and merges, terminal
extensions, junction-adjusted profiles, clip distances and affected edge/node scope. Preview and
unplanned commit use the same finalizer. Matching commits install the solved delta through
`TransitNetwork`, mapping preview-local identities to existing or appended live slots without
repeating intersections, junction solving, regrading or clipping. An incomplete mutation frontier
cannot be adopted. Stale/missing click plans are rebuilt through the same local compiler against
borrowed live inputs before opening the transaction; no whole-city snapshot is copied at click.

The plan also retains local road/site terrain CDT tiles and joined patch render buffers. Planned
cutout queries remove replaced source owners **before** union and remap local provenance to the
same live IDs that topology adoption will allocate. Existing road contributors outside the graph
excerpt are retained through the existing chunk/query indices. The worker uses the production
grading, fixed-tile CDT and buffer builders; there is no second terrain compiler. Nearby site
footprints/support heights are captured through the allocator's existing 512 m chunk index under
a short core lock, then compiled after releasing it. An unprepared index leaves the terrain
candidate provisional: pointer work never rebuilds or scans the resident building store.
Site grading queries the finalized local roads over the immutable resident world, excluding
replaced owners and preserving unmapped neighbors, global road-top precedence and prospective
live-ID nearest-edge ties. This adds only a bounded validation-excerpt copy, not a city snapshot.
Road splits and attachment repair change reference coordinates, not the authored world pose or
support height from which site geometry is derived. Captured site inputs therefore remain the
post-topology inputs; their grading already uses final planned roads. Commit verifies this exact
site set again, remapped road products, source and structural visual samples, affected-patch
coverage, bordered textures, ownership and exact query contributors. Different broad-phase margins
must select the identical road/site sets; no floating-point tolerance is used. It then adopts the complete planned
terrain batch, rebinding only payload generation metadata and sharing tile/seam/mesh buffers by
Arc. It does not assemble new CDT inputs or choose geometry from the previous live tile cache.
A mismatch rolls back the edit instead of compiling a different result after readiness.

Terrain validation separates pre-composition contributor/window checks from final publication.
Successful triangulation alone is insufficient: every clipped patch must retain valid final render
buffers with zero discarded faces. Missing or invalid buffers report a concrete failure, not an
indefinitely provisional plan. Cache insertion, cold validation and planned adoption share this
final gate. Explicit loop-free ordinary patches need no clipped render buffers.

Structural visual-height stamps use the same pattern: retain ordered support inputs and grid
writes, compare source/grid dependencies and actual triangles, then consume the offer once.
The plan materializes their final visual result in a bounded copy-on-write overlay using the
existing terrain storage chunks. Source resets and writes stay interleaved in live sorted chunk
order, including inclusive shared borders. Patch textures (with clamped border rings), CDT source
and boundary samples, road grading guides and site grading all consume this same visual view.
Untouched chunks borrow resident samples; source heights never change. Unchanged edits retain
the live sampling path. Overlay construction costs O(touched storage chunks + copied samples +
stamp writes), with O(1) indexed sampling and no full-map copy. Dense road/terrain sampling is
statically dispatched for resident versus planned inputs. Exact dependencies govern direct
tile/buffer adoption. Ownership and clip discovery now run after the ordered resets/writes,
using the same final visual view as compilation. Resident grading caches check both source and
visual-only revisions; changed overlays use a batch-local cache and cannot poison resident entries.
This keeps discovery bounded to indexed local owners and grades each boundary once per batch.
Ordinary grounded roads remain CDT-only; empty stamp reuse is not terrain-mesh reuse. Neither
product changes authored terrain or skips rollback. Exported `terrain_plan_state` distinguishes
compiled, invalid, stale, pending and provisional candidates, retaining deterministic failure
reasons and comparing exact local site inputs. Eligible road previews now export the
complete local terrain batch together with the canonical, unlifted road meshes. Godot stages all
required terrain patches before selecting those roads; no road-only clearance lift or vacated-cap
infill is included in this paired display. Rust exports planned ownership explicitly, including
regular terrain where the last cutout disappears. Display eligibility requires complete buffers,
current source/visual/road/site inputs, including post-stamp ownership and clip queries.
All required patches must already have renderer resources; ineligible or incomplete batches
retain the road-only preview. Isolated strokes use the same paired path as connected roads.
A missing-resource failure is retried on a new
preview request, not by rebuilding meshes every idle frame.

The temporary terrain meshes inherit each resident patch's transform and visibility. Original
mesh resources are saved and restored exactly on cancellation, invalidation or tool disposal;
patch update, LOD replacement, recycling and reset invalidate both halves before changing those
resources. This presentation never changes terrain samples, payload caches or renderer
acknowledgments. Work is O(local patch samples + exported road/terrain vertices + P log P) for P
affected patches, plus existing local terrain mesh construction/upload; no resident-world scan is added. Canonical road attributes
are copied once from the local production output before road-only display adjustments, not
compiled again. `plan_state` distinguishes pending, provisional, invalid, stale, consumed and
ready. Ready requires a complete local topology/product set, exact raw inputs/lanes/snap mode,
road/source/visual dependencies, a prepared site index and live water/parcel clearance. Product
claims are single-use and happen only after preflight. Godot still labels missing-resource or
road-only displays provisional; only a successfully staged pair exposes full readiness. An explicit
empty terrain batch (no affected terrain) is complete; missing/failed nonempty batches are not.
Known-invalid plans override the earlier road-only valid verdict rather than appearing pending.
An older pointer's rejected result cannot clear a retained pose for a newer provisionally valid
input. Retained older poses and coarse ribbons remain explicitly provisional even when their
underlying geometry is complete; only an exact current paired display claims readiness.
The existing simulation mutex encloses topology adoption, product validation, lane/agent/entrance
and routing maintenance, charging and matching render-snapshot preparation; publication retains
the existing generation fences. Failure before acceptance
restores the bounded graph/split-reference journal and exact visual storage chunks; successful
undo retains the same visual checkpoint. No new full-city snapshot or per-pointer index rebuild
is introduced. Plan-less subsystem diagnostics retain the cold compiler, but interactive commits
always require a complete plan.

Immutable surface candidates still feed commit compilation, which matches them by world position, piece kind,
and mouth count, then independently requires exact canonical node-local rail topology. Exact rail
height values additionally permit boolean ownership and the already validated, triangulated
arrangement to be rebound from preview-local node IDs to authoritative node IDs, including terrain
cutout boundary ownership. Any metadata,
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
- Add `--gpu-profile` to a windowed gameplay road command, for example
  `./run.sh --benchmark-gameplay-roads --gpu-profile`, to enable Godot's built-in GPU stage
  diagnostics. Output is retained in the run's existing `*.godot.log` under `benchmark-results/`
  (or the configured output directory). The modifier also works with `--profile-gameplay-roads`
  to collect CPU and GPU diagnostics together; headless workloads are rejected before building.
  JSON runtime metadata records `gpu_profiled: true` and `profiled: true`, so the existing paired
  report rejects these diagnostic captures as acceptance timings. GPU stage output is a periodic
  rendering breakdown, not per-operation GPU timing or presentation latency; existing JSON frame
  observations keep their CPU timing contract. This reuses the road workload and does not measure
  whole-city rendering or running-simulation contention.
  The 2026-09-10 diagnostic before runtime grass mipmaps on the RX 7900 XTX / i9-12900K,
  Godot 4.7.2 Forward+, 1920x1080,
  60 FPS cap, V-Sync enabled and 24 Rayon workers passed all 48 paired fixtures with GPU profiling
  and all 48 in the matching unprofiled release run. Post-warm-up periodic GPU averages had a
  `2.978 ms` median (`3.071 ms` largest average), with opaque rendering accounting for `95.2%`
  of reported time. The unprofiled operations still recorded frame intervals up to `39.820 ms`.
  An eight-fixture CPU/GPU diagnostic recorded network/terrain renderer calls up to
  `19.669 / 17.603 ms`, including setup/reloads; this does not identify the exact cause of the
  unprofiled spike. The user later reported moving windows between desktops during benchmarking;
  affected runs/time ranges are unspecified. Frame-interval outliers do not establish a game-side
  hitch, and precise timing comparisons need an uninterrupted repeat with the window stationary.
  These are diagnostic baselines, not speedup, individual-frame GPU-tail, or
  whole-city claims: the fixtures contain no buildings/agents and simulation is paused.
  Commands, build/workload identity, summaries and raw captures are retained under
  `benchmark-results/gpu-road-analysis-Jecrzy/`; see its `analysis.txt` and `analysis.json`.
  A subsequent repeat on the same hardware/settings, with grass mipmaps enabled and the user
  leaving the window stationary, passed all 48 fixtures in each of two sequential release runs
  (GPU-profiled, then unprofiled). The 78 post-warm-up GPU reports had a `0.630 ms` median and
  `0.788 ms` largest periodic average; opaque-pass mean was `0.502 ms`. The unprofiled run's
  996 observed frame intervals across 85 measured road operations averaged `16.360 ms` and
  reached `40.742 ms`; 14 operations contained an interval above `33.333 ms`. This counts
  affected operations, not individual spikes. Longer CPU callback intervals therefore recur
  without reported desktop switching, but their cause remains unresolved; periodic GPU
  averages cannot identify individual-frame stalls. Workload signatures and library hashes
  match between runs. No new zero-mip baseline was captured. Commands, build identity,
  texture hash checks and raw captures are in `benchmark-results/stationary-mipmaps-LZ6tB3/`;
  see its `results.txt` and `analysis.json`.
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

Interactive `RoadEditPlan` previews perform final local `JunctionN` compilation on the preview
worker; ready adoption consumes those products. The compiler itself remains synchronous, so
missing/stale click plans and cold subsystem callers can still block while rebuilding a large
multi-mouth node. Further responsiveness work must retain immutable versioned inputs,
deterministic latest-result publication and the last complete visible generation. More incremental
`JunctionN` compilation or stronger contact/export indexing remains separate from plan readiness.

Do not reopen shipped `ROAD-01` geometry hardcuts for editor responsiveness unless the fix changes
the roadbed ownership contract itself.

## Kuopio Terrain Regression Replay (`ROAD-24`)

Current status: the planning/adoption contract and captured geometry/performance acceptance are
complete. See [coverage and saved-reference completion](#coverage-and-saved-reference-completion)
and [controlled performance acceptance](#controlled-performance-acceptance) for the latest evidence.
The historical milestone results below are retained for comparison, not as pending work or current
failure counts. The capture is a regression fixture, not proof for all possible road geometry.

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
The Python audit alone does not prove surface coverage. The Rust replay also exercises actual
atomic adoption, exact retained/cold patch agreement, zero omitted terrain faces, strict tile
containment, matching shared-side heights and retained road constraints. The saved reference's
27 terrain patches are compiled as well. Screenshots still require visual review; settlement
alone is not a universal watertightness proof.

### Current replay and plan contract

`RoadEditPlan` replaces the preview validation certificate with an immutable,
shared prepared input: authored points, source/surface revisions, lane counts, snap mode, road
class, prepared longitudinal profile, validation result and terminal-extension changes. Exact
matching costs O(raw points) and lets commit borrow the worker's solve instead of preparing it
again. It retains the already finalized validation graph as a bounded delta, copying
changed profiles only. Explicit local-to-live maps preserve the planned split order and final
topology; split-dependent building/occupancy migration uses the existing journal with captured
pre-finalization split lengths. Live congestion, speed and frontage metadata survive adoption;
changed speed costs are recalculated without altering profiles. Merge survivors clear obsolete
turn restrictions. Solved junctions and moved/merged source nodes require complete live incidence
in the excerpt, even when the source node's position is unchanged.

Capture is O(local records + profile points). Adoption is O(local profile points + summed local
degree * log(degree) + changed edges * log(resident edges)), plus existing split-dependent
migration costs. There is no new city-wide scan or clone. The bounded source scope feeds the
existing undo checkpoint. Live water/parcel checks, exact road-product matching and terrain adoption,
rollback, lane/agent updates, routing, building references and charging still run in their existing
order. Optional compiled surface products transfer only after matching. The terrain-product slice
now precompiles road cutouts, local site grading, CDT tiles, joined patch buffers and structural stamps against the local old/new
ownership footprint. Work is bounded by local owner coverage, indexed patch contributors, source
samples and the existing CDT/raster/patch-composition cost; independent patch/tile builds use Rayon. Commit's exact
adoption checks are linear in local samples/products and borrow immutable terrain buffers.
Local site capture/validation costs O(queried index cells + local candidates log(local candidates) +
captured footprint vertices), summed over affected patches. Visible-height probes retain the
existing allocation-free owner-local triangle grids; nearest-road probes use the existing R-tree.
Exact site changes invalidate readiness; commit checks the post-topology site set independently.
Structural writes/resets now feed ownership/clip discovery, planned patch samples and all
CDT/grading samplers. Site snapshots use each patch's final production query margin. Visual-only
edits stale the terrain candidate even without a source revision change. Post-topology site
changes and full commit readiness/dependencies are now covered by the contract above. Complete paired
previews share the accepted geometry; incomplete displays remain explicitly provisional.

The source-profile solve uses a per-interval grade envelope: the greater of the 16% design target
and the sampled source/support grade at that interval. Net endpoint slope does not describe an
interior hill or valley; pin-to-pin feasibility alone must not flatten that relief. Local bounds
also prevent a steep interval from relaxing an entire otherwise gentle road. Envelope construction
is O(profile points); the existing fixed-count smoothing/curvature solve stays O(points), with
unchanged pinned heights. Source terrain and the captured save remain unchanged.

### Historical implementation milestones

The intermediate failures, missing features and test counts in this section describe their named
runs only. Later completion results supersede them; no historical failure budget is accepted today.

The initial profile-plan fixes make all 13 sequences place, including the formerly rejected
crossing. `kuopio_04`'s first stroke now stays within 0.23 m of source terrain instead of 37.34 m;
both `kuopio_04` and `kuopio_07` pass the diagnostic profile budgets. Across all 158 audited edge
profiles, maximum grade falls from 11.76 (about 85 degrees) to 0.80 (about 39 degrees). The full
audit deliberately remains failing: 32 edge/checkpoint entries exceed the source-offset budget
(up to 16.13 m), and one also exceeds the pitch-change budget (36.03 degrees). These entries are
not 32 independent roads. Terrain gaps still require the next coverage/terrain-plan stage.
Targeted Rust regressions check source preparation, junction solve/regrade and clip rebuild
separately; repeated support materialization and edge splits must preserve physical heights.
`--debug road` reports `road_edit_plan reused=...` and `road_edit_topology adopted=...` to stdout.
Targeted regressions assert exact planned/cold-commit profiles, clips, costs and compiled node/span
products for crossings, close double-Ts and terminal extensions. Further tests cover ID remapping,
remote geometry/index preservation, live metadata, merge restrictions, incomplete frontiers,
stale-plan fallback and terrain-rejection rollback of split-dependent buildings/occupancy.
The Godot log file alone excludes Rust stdout.

Verification: 1,599 Rust tests pass (two ignored), all five headless Godot bridge suites and 15
Python checks pass, and benchmark targets compile. New tests cover exact CDT mesh/buffer equality
and Arc reuse, changed source/input/frame/site-manifest rejection, nonempty structural stamps,
unmapped neighboring ownership and affected visual-terrain restoration after rejection. The patch
follow-up adds multi-tile joined-buffer identity/parity, missing-tile and fresh-manifest/error gates,
and a real newly derived building site: its affected patch recompiles while unaffected neighbors
may still reuse. Fresh patch metadata and payload revisions match cold compilation.
The site follow-up proves exact joined-buffer reuse for an actually site-influenced patch,
planned/live height and candidate-ID equality including unmapped neighbors, source/site staleness,
and pending status without rebuilding a dirty allocator index. Godot checks the visible provisional
label and stale terrain status after a road revision. The display follow-up checks canonical
unlifted road/cold-commit mesh parity, deterministic terrain batches, exact terrain-buffer parity
and deferral of structural display. Structural sample regressions additionally compare ordered
overlapping resets/writes, sparse-default resets, border/interpolation samples, nonempty tunnel
stamps, road/site grading guides, composed CDT buffers and exact post-reset commit reuse. A huge
sparse-world test verifies that the overlay only captures touched storage chunks.
Flat/sloped Godot fixtures verify paired publication, malformed
batch atomicity, exact cancellation restoration, invalidation before patch recycling and
provisional fallback for missing resident patches. Isolated-road regressions now cover flat/sloped
cold-commit mesh parity across chunk boundaries, preservation of an unrelated same-chunk road,
empty retained batches on a blank map, curved native previews and unchanged-preview upload reuse.
The moving-ribbon diagnostic explicitly requests fallback geometry after exact chunk display.
Fresh-world reference patch loading honors native retry requests; ignoring these caused an
intermittent first-road test timeout, now fixed by sharing the existing capture retry protocol.
Rustdoc builds without missing-doc warnings
(11 pre-existing broken/private intra-doc links remain).
The `terrain-plan-products-01` headless and `terrain-plan-products-render-01` Forward+ runs complete
all 13 cases / 39 strokes. The headless debug log confirms topology adoption on every stroke and
719 planned CDT tiles reused across 30 strokes; nine use fresh compilation. Their 158-profile
audits match each other and the previous `terrain-plan-topology-02` audit exactly. The rendered run saved 117 screenshots;
spot checks still show blue terrain gaps and junction bumps. Both diagnostics intentionally exit
2 for the same 32 quality failures, not placement failures. The earlier `terrain-plan-topology-01`
debug run hit a settlement timeout with water work pending after a successful commit; it is retained
as a failed run, not counted as acceptance. The final headless and Forward+ runs did not repeat it.

The patch-composition follow-up `terrain-plan-patches-01` completes 13 cases / 39 strokes and
reuses 815 tiles across 33 strokes; 26 strokes reuse 42 complete patch buffers. Its profile audit is byte-identical
to `terrain-plan-products-01`, including the 32 quality failures and diagnostic exit 2. The pinned
save checksum remains unchanged. `--debug road` now reports actual `reused_patch_buffers` identity
reuse separately from tile reuse. This follow-up has no new rendered capture or watertightness
certification.

The local-site follow-up `terrain-plan-sites-01` also completes 13 cases / 39 strokes, with the
same 815 reused tiles and 42 complete patch buffers. Its 158-profile audit is byte-identical to
`terrain-plan-patches-01`: 32 failures and diagnostic exit 2, with the pinned save unchanged.
No new rendered capture or terrain-gap fix is claimed. The matched unprofiled
`terrain-plan-sites-after-01` / `terrain-plan-patches-after-01` pair validates all 32 fixtures.
Per-operation preview-ready median deltas span -6.75 to +6.81 ms (candidate 13.57–47.92 ms);
first-idle deltas span -6.97 to +6.93 ms. This is one exploratory pair, not a speedup/regression
claim. These fixtures have no buildings: they measure the empty-site path, not populated-site
scaling. Populated-site geometry is covered by targeted parity tests; scaling measurements remain
necessary before full-plan performance sign-off.

The paired-display follow-up `terrain-plan-display-render-01` completes all 13 cases / 39 strokes
and saves 117 Forward+ screenshots. Debug output confirms actual paired terrain publication for
23 strokes; the other previews remain provisional road-only displays. The 158-profile audit is
byte-identical to `terrain-plan-sites-01`: the same 32 failures and diagnostic exit 2. Spot checks
of `kuopio_04` and `kuopio_07` still show junction bumps/blue gaps, not a geometry-quality fix.
The pinned save checksum is unchanged. The matched unprofiled `terrain-plan-display-after-02` /
`terrain-plan-sites-after-01` pair validates all 32 fixtures. Per-operation preview-ready medians
are 20.20–48.44 ms, with deltas -0.17 to +6.85 ms; first-idle medians are 20.49–34.38 ms, with deltas
-6.93 to +13.89 ms. This is one exploratory pair, not a performance sign-off. The preliminary
`terrain-plan-display-after-01` instrumentation run recorded 32 paired displays across 68 placements
(warmups included), but changed the benchmark harness and is excluded from timing comparisons;
the final timing run restores the original harness and keeps display counts behind `--debug road`.

The isolated-road follow-up `terrain-plan-isolated-render-01` completes 13 cases / 39 strokes and
saves 117 Forward+ captures. Paired terrain is actually displayed on 35 strokes, versus 23 in the
connected-only run. The 158-profile audit remains byte-identical, with the same 32 quality failures
and diagnostic exit 2; the pinned save is unchanged. `kuopio_04`'s first stroke now displays the
canonical terrain-following road/terrain pair. Four previews remained road-only because their planned
terrain reports `missing_terrain_clip_loops`: `kuopio_02` attempt 4 in patches `(17,18)` / `(18,18)`,
and `kuopio_11` attempts 31–33 in `(17,20)`. These were candidate ownership failures despite
successful fresh commit compilation, not placement failures.

The ownership follow-up `terrain-plan-ownership-render-01` resolves all four: all 39 strokes now
display paired terrain, with 13 completed cases and 117 Forward+ captures. Planning now uses the
live cached per-loop grading ownership calculation, excluding superseded owners before merging
the planned contribution, and the live site-overlap rule. Clip queries use actual per-patch grading
margins. Padded query hits no longer claim ordinary patches, independently of CDT output; genuinely
owned patches still fail on missing sources, loops or windows. Targeted regressions reproduce the
original failure, verify exact planned/cold patch ownership, margins and buffers through a branch,
and prove vacated patches lose replaced-road ownership. The coverage validator is unchanged.
The 158-profile audit is byte-identical to the isolated run (32 quality failures, diagnostic exit 2),
and the pinned save checksum is unchanged. Spot checks of `kuopio_02` and `kuopio_11` show matching
road/terrain poses, but junction bumps and blue gaps remain. This is not watertightness certification.
Structural stamp presentation, post-topology site changes and full readiness remained at this milestone;
the later structural/readiness passes below complete those contract items.

The approach-profile follow-up separates ownership handoffs from the geometric crossing core.
`RoadEditPlan` and direct insertion finalize the physical profile through one shared solve;
grounded section compilation no longer adds a second longitudinal flattening blend. Bends fit
incident grades. Node-owned straight approach boundaries retain physical-profile stations, and
rounded corners carry their endpoint heights and grades. Material offsets and source-height
precision survive contour cleanup, reuse and triangulation; topology identity rounding is not a
replacement height source. Adjacent-face normal checks exclude only triangles whose XZ altitude
is unresolved at source-coordinate precision; coverage and direct face validation still inspect
those triangles. There is no new global scan or spatial index.

The first rendered run, `approach-profile-render-01`, exposed two terrain constraint rejections
(`kuopio_11` attempt 33 and `kuopio_12` attempt 37). Both are now targeted production-pipeline
Rust regressions. Terrain noding had interpreted distinct near-parallel approach segments as
collinear overlap using its millimetre identity tolerance. Incidence now uses the micrometre
contour resolution; actual overlap and conflicting-height regressions remain enforced. The
approach contract and its seam follow-up pass 1,606 Rust tests (2 ignored), 15 Python tests and
all five headless Godot bridge suites. The full quality/performance follow-up is separate from
these correctness checks; structural writes/resets and full ready/atomic adoption were still open
at this milestone and are completed by the later passes below.

`approach-profile-render-02` places all 39 strokes across 13 cases and displays all 39 paired
previews, retaining 117 captures. Commit reuses 1,027 CDT tiles on 34 strokes and 58 joined patch
buffers on 31 strokes. All 158 physical profiles are audited: worst pitch change falls from
36.03 to 6.78 degrees, and maximum grade from 0.80 to 0.7568. However, 34 profile entries exceed
the 5 m source-offset budget (previously 32); two `kuopio_04` crossing profiles now reach 5.35 m
and 5.61 m. Maximum source offset remains 16.13 m. The saved reference fails node 34 compilation
and its render settlement fence, adding a separate audit failure (35 total, diagnostic exit 2).
The SQLite checksum is unchanged. Spot checks show successful previously rejected placements,
but blue terrain gaps remain. This is a validated profile-contract slice, not geometry-quality
or saved-map migration sign-off.

The unprofiled `approach-profile-after-01` run passes all 32 fixtures with 3 measured repetitions
and 1 warmup. Per-operation preview-ready medians are 20.29–47.94 ms; first-idle medians are
27.31–34.26 ms. The strict report refuses comparison with `terrain-plan-ownership-after-01`:
the harness, inputs and state cardinalities match, but additional profile supports change
ghost-guide vertex counts. That guard remains intact; no matched A/B improvement is claimed.

The source-relief follow-up, `relief-profile-render-01`, clears all 34 cut/fill failures without
changing locations, the SQLite reference, source terrain or quality budgets. All 39 strokes place,
all 39 paired road/terrain previews display, and 117 captures are retained. Across 158 physical
profiles, maximum absolute source offset falls 16.1289→3.9450 m and maximum grade
0.7568→0.6296. The two `kuopio_04` attempt-10 failures (edges 0/2) fall 5.3471/5.6130→0.4581/0.2234 m;
every profile in that case is below 1.82 m. Worst pitch change rises 6.7805→11.9874 degrees,
remaining below the unchanged 30-degree diagnostic limit. The audit still exits 2 solely because
the unchanged saved reference fails node 34 compilation/settlement. Spot checks still show small
blue terrain gaps; longitudinal acceptance does not certify watertight terrain or saved-map repair.

The complete Rust replay regression now covers every recorded stroke and all 158 checkpoints,
including cold terrain constraints. Additional tests cover interior hills/valleys with feasible
endpoints, steep corridor authority through splits, exact planned/cold geometry, non-accumulating
topology-only profile updates, both straight-join approach domains and discarded CDT spurs.
The three flat-node polygon goldens reflect CDT retessellation after unused vertex removal;
polygon/carrier counts and source-authority identities are unchanged. All 1,612 Rust tests
(2 ignored), 15 Python tests and five Godot bridge suites pass with `./run.sh --test --release`.

`relief-profile-after-01` passes the unprofiled 32-fixture matrix (3 repetitions, 1 warmup) and
strict matched comparison with `approach-profile-after-01`. Per-operation preview-ready medians
are 20.19–48.07 ms (deltas -0.53 to +6.64 ms); first-idle medians are 27.22–34.34 ms
(deltas -6.84 to +6.96 ms). The harness, inputs and work-cardinality checks remain unchanged.
This single process pair is exploratory evidence, not a speedup claim or completed-plan
performance certification.

The structural-sample follow-up passes 1,616 Rust tests (2 ignored), including the unchanged
158-profile Kuopio regression. Ordered resets/nonempty stamps, sparse-default coverage, clamped
texture borders, road/site grading, composed CDT buffers and exact post-reset reuse are tested.
The final `structural-samples-static-01` unprofiled capture validates all 32 fixtures with the same
harness, inputs and cardinalities as `relief-profile-after-01`. Preview-ready medians are
13.51–47.85 ms (deltas -6.84 to +0.54 ms); first-idle medians are 20.53–34.48 ms
(deltas -6.97 to +7.06 ms). Initial dynamically dispatched captures `structural-samples-after-01`
and `after-02` were slower and are not the final implementation. The final comparison remains
one exploratory process pair, not a speedup claim or full-plan/populated-site performance sign-off.
The subsequent post-stamp ownership slice removes the structural paired-display gate by moving
ownership/clip discovery onto the same final samples. Regressions cover visual-only cache
invalidation, preview cache isolation, nonempty tunnel stamps and post-reset patch reuse.
All 1,618 Rust tests (2 ignored), including the 158-profile Kuopio replay, and five Godot suites pass.
Three unchanged-build unprofiled captures (`poststamp-ownership-01/02/03`) validate 32 fixtures each
and strict matching against `structural-samples-static-01`. Preview-ready medians are respectively
27.22–68.19 / 20.26–49.61 / 13.50–48.03 ms; per-operation deltas against that baseline span
+6.55 to +22.12 / -6.89 to +6.90 / -6.85 to +13.01 ms. First-idle medians are respectively
27.17–34.34 / 27.39–34.32 / 20.52–34.38 ms. The repeated candidate runs expose substantial
run-to-run variation; these are not three independent baseline/candidate pairs or performance
sign-off. That milestone did not claim full readiness, atomic commit or watertight terrain coverage.

The readiness/adoption pass completes post-topology site dependencies and the transaction contract.
Building splits/attachment repair preserve authored site pose and support; grading uses final planned
roads. Ready plans pin exact inputs and road/source/visual/site dependencies and recheck live water
and parcel clearance under the simulation lock. Stale/missing click plans rebuild locally before
mutation. Commit checks remapped road products, final visual samples, patch coverage/ownership and
exact query contributor sets, then installs the existing terrain buffers with new payload metadata.
It never runs another CDT/input assembly to replace a ready plan's result. A mismatch restores local
graph/split references and exact visual storage chunks; undo retains that same visual checkpoint.
The bounded compiler halo now includes complete incidence at span endpoints: a remote four-way
approach can no longer compile as a preview terminal. This adds two fixed adjacency layers, not a
recursive graph expansion. Surface-valid bridge/tunnel candidates use canonical scene export too;
an explicit empty terrain batch is ready, while failed/nonresident batches remain provisional.
All 1,625 Rust tests (2 ignored) and 15 Python checks pass, including actual terrain adoption for
all 39 Kuopio strokes / 158 profile checkpoints, crossing/extension/close-double-T, site repair,
water/parcel invalidation and exact rollback/undo. Benchmark targets compile; rustdoc has no
missing-doc warnings (11 pre-existing link warnings). All five Godot suites pass, including bridge
readiness with unchanged terrain, empty batches, paired display and stale-plan invalidation.
The 14 terrain-plan tests also pass with one Rayon worker, including the full Kuopio replay.
This completes the planning contract, not
the separately tracked blue-gap/saved-reference geometry repair.

The initial unprofiled `ready-adoption-01/02/03` runs validate 32 fixtures each and strict matching against
`poststamp-ownership-01/02/03` (same harness, inputs, cadence and output cardinalities). Preview-ready
medians are 13.54–48.08 / 13.42–47.90 / 20.30–48.02 ms; first-idle medians are respectively
20.56–34.33 / 27.34–34.36 / 27.05–41.29 ms. Across process-median aggregates, preview deltas are
-19.85 to +0.06 ms and first-idle deltas -6.87 to +6.91 ms. First-idle is not uniformly faster;
the earlier unchanged-build variance prevents a general speedup claim. These runs precede the
final invalid-verdict propagation and retained-provisional-display fixes.

The final-behavior `ready-adoption-04/05/06` captures also validate all 32 fixtures each and strict
matching, but their aggregate preview medians are higher: 32.81–67.09 ms (deltas -0.19 to +25.71 ms
against the three poststamp baselines). First-idle remains 27.17–34.28 ms (deltas -6.86 to +6.84 ms).
Unchanged-build repeat `ready-adoption-07` retains the increase (preview medians 29.91–65.91 ms).
The formatting-only final rebuild, `ready-adoption-08`, measures 26.28–66.99 ms preview medians
and 27.18–41.19 ms first-idle medians; all 32 fixtures pass.
A performance-core-only diagnostic (`ready-adoption-pcore-diagnostic-01`, 16 Rayon workers) does
not consistently recover earlier latency; it is not a matched comparison or a game-default change.
`ready-adoption-diagnostic-01` retains a CPU profile for investigation, not headline latency evidence.
Two temporary valid-road-only controls also remain slower: omitting rejection propagation recreates
the exact `ready-adoption-01` binary hash but measures 32.63–68.33 ms preview medians;
restoring the old display-clearing/label path measures 25.89–61.50 ms. Neither control isolates
the increase to the final presentation fixes. Both complete behaviors are restored afterward;
these controls are diagnostic captures, not alternate supported implementations.
At that milestone, the slower captures were not dismissed as noise: correctness/readiness/adoption
was implemented, but preview-latency acceptance remained open. The completion investigation below
supersedes that status; the historical captures remain available for comparison.

### Coverage and saved-reference completion

The coverage investigation identified input and ownership defects, not a need for another
RoadEditPlan abstraction. Terrain CDT now keeps the uncut grading halo separate from the clipped
ownership polygon. Shared tile sides use the same grade-limited height authority; road guides
use the common visual/source terrain field instead of independently conflicting parent-edge
heights. Strict tile bounds exclude outside sliver vertices, and retained guides touching road
seams participate in constraint noding. Constraint intersections use metric incidence tolerance,
not a fraction of segment length that falsely extended long edges. Neither Rust nor Godot may
publish a terrain patch after discarding pathological faces: an incomplete patch invalidates the
whole paired publication. The original slope and cut/fill budgets are unchanged.

The Kuopio test now runs the actual atomic-adoption validator at every stroke. This exposed a
missing dirty approach in `kuopio_06`: the plan compiler now consumes the final profile solver's
entire dirty ledger, matching commit's products and terrain coverage exactly. The regression
also checks cold/retained patch agreement, zero omitted faces, strict tile containment, shared
tile-side heights and retained constraints. The rendered `completion-coverage-final` replay passes
all 39 placements and 158 profile checks with zero audit failures, including saved-reference
settlement, and retains 117 before/preview/committed captures. Reviewed hill junctions no longer
show the previously observed blue gaps or vertical end ramps. This is coverage of the captured
regressions, not a claim that arbitrary future inputs are mathematically watertight.

The immutable saved reference failed at node 34 because an old approach contained a 17 cm height
jump over roughly 3.5 cm. Load now first compiles saved geometry unchanged; only rejected grounded
junctions enter one bounded pass of the shared junction-profile finalizer. The detached loaded
graph must then compile successfully before publication, or load returns an error. Valid saved
roads retain authority; this is not a city-wide source-terrain regrade. The regression proves
every nonincident road profile remains exactly unchanged, and all 27 saved terrain patches pass.
The checked-in SQLite checksum remains
`e1d7e0baf4eefd23293f0a770345f50e455815124c0a0920aefe10cc7a1033c0`.

Source provenance recovery uses the existing R-tree dependency for bounded loop-local queries,
with deterministic semantic source selection. Halo grading excludes only segments beyond the
maximum input height range divided by the existing slope limit; exhaustive/bounded regressions
compare identical canonical results. For V input vertices, S local source edges, P queried
segments and H candidates, cost is O(V + S log S + P log S + H), with no new resident city-wide
spatial structure.
Guide/DEM constraint noding likewise queries a tile-local R-tree and reuses its hit buffer;
sorting hits by original edge index preserves the exhaustive path's height authority exactly.
The existing edge/edge incidence pass remains bounded to that tile, not the city graph.

The populated-neighborhood measurement is an ignored, release-only Rust benchmark:

```bash
cd rust
cargo test --release --lib populated_road_plan_scaling -- --ignored --nocapture --test-threads=1
```

It retains four occupied local sites and grows remote background state through 0 / 1,000 /
10,000 / 100,000 buildings, with real indexed parcels, six housed agents per building and
4 / 40 / 391 remote streets. Each level measures 100 observations after three warmups. It
separately reports borrowed-core plan compilation, readiness and the preview worker's local
road-mesh/site/terrain preparation, while proving identical local terrain products. Fixture
construction, global index preparation and the once-per-edit immutable context snapshot are
outside cursor-work timing; snapshot cost is reported explicitly. This is a locality test,
not Godot upload timing or a claim about actively ticking 600,024 full-FSM agents. Run it
alone, repeat in separate processes, and use the matched Godot matrix for end-to-end latency.

`road-plan-populated-completion-01/02/03.log` all pass with 24 Rayon workers. The table reports
the median of the three process-specific statistics, not a pooled latency distribution:

| Remote buildings | Plan p50 (ms) | Worker p50 (ms) | Worker p95 (ms) |
| ---: | ---: | ---: | ---: |
| 0 | 19.08 | 20.19 | 25.31 |
| 1,000 | 19.10 | 20.13 | 25.60 |
| 10,000 | 18.97 | 20.05 | 25.57 |
| 100,000 | 19.45 | 20.52 | 26.45 |

At 100,000 remote buildings, plan and worker medians grow about 2.0% and 1.6%, respectively;
readiness medians remain below 0.019 ms in every process. The one-time context snapshot grows
from roughly 0.01 ms to 3.37–3.47 ms as the road background grows, and is not hidden inside a
claim of constant snapshot cost. Local terrain products are identical at every level.

For larger timing samples of specific paired fixtures, set
`METRUM_GAMEPLAY_BENCHMARK_CASES=double_t_close_2l,four_way_mixed_8l_2l` with
`METRUM_GAMEPLAY_BENCHMARK_MATRIX=paired`. Selection preserves the original fixture order,
geometry and synthetic-world anchors; unknown or duplicate case IDs fail the workload. The
recorded matrix descriptors and harness hash keep baseline/candidate workload matching strict.
Other matrix modes do not accept this selector. The default remains the complete eight-case matrix.

### Controlled performance acceptance

The preserved pre-completion `ready-adoption-08` binary is the baseline (SHA-256
`e7db00089ff4f671ed567d45eff2f2aec0fda6a37e959280818b20e1cf17ac94`). Both sides use the same
Godot runtime, hardware, fixture definitions, cadence and guide cardinalities, with diagnostics
disabled. Each comparison uses three independent process pairs in AB / BA / AB order. CPU
profiles are separate diagnostics: they identified extra canonicalization work in full-halo source
recovery and grading queries. Those queries are now indexed/bounded; no thresholds or required
terrain products were removed to improve timings. Unchanged baseline runs also exhibit large
variation, so the historical 20–25 ms jumps cannot be assigned to RoadEditPlan from those samples.

The final complete-matrix `completion-local-{baseline,candidate}-01/02/03` runs validate 32
fixtures each (three measured repetitions, one warmup). Per-operation candidate preview medians
range 32.53–77.89 ms, and first-idle medians 27.06–34.35 ms. This small-sample screen retains a
10.52 ms wide-junction delta, so it was not used alone to dismiss the earlier regression.

The larger `completion-targeted-{baseline,candidate}-01/02/03` comparison keeps the two variable
fixtures at their original locations, with 20 measured repetitions and three warmups per process;
all 46 fixtures pass in every process. Median-of-process-median preview readiness is:

| Operation | Baseline (ms) | Completed plan (ms) |
| --- | ---: | ---: |
| Close double-T: initial road | 33.50 | 35.58 |
| Close double-T: first branch | 46.94 | 47.26 |
| Close double-T: second branch | 50.89 | 53.98 |
| Mixed-width crossing: initial road | 38.74 | 39.64 |
| Mixed-width crossing: crossing | 67.37 | 69.70 |

The remaining cost is real, not a speedup: approximately 3.09 ms for the second T and 2.33 ms
for the wide crossing. First-idle changes across these operations are only -0.05 to +0.09 ms
in the process-median aggregates. Reports use the pattern
`benchmark-results/road-plan-completion-{local,targeted}-{preview,idle}-comparison.json`.
There are insufficient observations for
end-to-end p95/p99 claims; the populated benchmark above has its own 100-observation p95s.

For this completion pass, that small measured increase is accepted for the
correct shared terrain solution: the expensive work is asynchronous and bounded to the edit,
ready-plan adoption does not introduce another terrain solve, and the 100,000-building worker
measurement stays near 20 ms with roughly 2% background-growth cost. This is not a promise of
60 FPS, a general speedup, or zero cost for a more complete geometric solution. The former
unexplained large-regression blocker is replaced by reproducible controlled measurements and an
explicitly recorded tradeoff. Reference fixtures and quality limits are unchanged; historical
failures remain recorded.

All 1,633 active Rust tests, 15 Python checks and five Godot bridge suites pass; the 15 terrain-plan
tests pass with one Rayon worker too. Release benchmark targets compile. The final optimized
headless `completion-local-validation` replay again passes 39 placements / 158 profiles and saved
reference settlement with zero audit failures. Together with the rendered capture and exhaustive
query-parity tests, this closes the four RoadEditPlan completion items for the captured workload.

### September 9 audit follow-up

The combined staged/unstaged audit checked topology/profile ownership, local invalidation,
source/visual/site dependencies, atomic adoption/rollback, terrain coverage and Godot presentation.
It found a common-validation gap: successful CDT windows could pass without present, valid final
render buffers. Pre-composition checks now remain separate from final acceptance; clipped patches
cannot enter the cache or publication through missing/invalid buffers. This adds constant-time
checks per patch, with no extra spatial query, geometry solve or city scan. The regression also
proves ordinary loop-free patches remain valid without clipped buffers.

Obsolete two-pass regrade APIs, the zero-valued `regrade` timing field and the old bulk-finalizer
alias are removed. The remaining conservative source-fit helper is test-only; production profile
finalization is shared. Cold subsystem compilation and test-only cold/reuse parity oracles remain
intentional, not alternate interactive commit paths. Rustdoc's 11 broken/private links and stale
load, site, undo, lock-duration and final-worker descriptions are corrected. Historical measurements
above remain evidence for their original binaries, not new performance claims for this audit.

Verification: 1,634 Rust tests pass (3 ignored), all 15 terrain-plan tests pass with one Rayon
worker, 15 Python checks and five Godot bridge suites pass. Release build, benchmark-target check
and rustdoc complete without warnings; formatting, source headers and diff whitespace checks pass.
The pinned SQLite checksum is unchanged. The unprofiled audit locality run, retained in
`benchmark-results/road-plan-audit-scaling-20260909.log`, keeps identical local products across
0 / 1,000 / 10,000 / 100,000 remote buildings (100 observations per level, 24 Rayon workers).
Worker p50 at the endpoints is 20.41 / 20.27 ms; readiness p50 is 0.0157 / 0.0169 ms and the
one-time context snapshot is 0.007 / 3.427 ms. This single-process recheck supports locality,
not a fresh matched A/B speedup claim. The separate historical `ROAD-06`/`ROAD-07` reports are
[parked](roadmap.md#parked-historical-reports), not current blockers or pending acceptance work.
Their geometry is not declared repaired by this capture; reopening requires a current reproduction.

### Historical exploratory timing runs

These earlier measurements do not supersede the completed matched acceptance above.

The matched unprofiled `terrain-plan-ownership-after-01` / `terrain-plan-isolated-after-03`
comparison validates all 32 fixtures. Per-operation preview-ready medians are 25.34–78.05 ms,
with deltas -16.41 to +8.53 ms; first-idle medians are 20.89–34.78 ms, with deltas -6.82 to +7.25 ms.
The harness and input matrix are unchanged. This is one exploratory pair, not a speedup or
full-plan performance sign-off; populated-site scaling remains unmeasured by this matrix.

The isolated-road timing runs `terrain-plan-isolated-after-01` / `after-02` validate all 32
fixtures but show preview-ready medians 8–30 ms above the older `terrain-plan-display-after-02`
baseline. A same-session `terrain-plan-isolated-control-01` temporarily restores only the old
connected-only export gate and also rises to 26.09–74.95 ms, so the earlier increase cannot be
attributed to isolated export alone. The gate is restored for `terrain-plan-isolated-after-03`,
which validates all 32 fixtures with the same binary hash as `after-02`: preview-ready medians
25.67–69.53 ms, deltas -15.59 to +15.65 ms versus the control; first-idle medians 23.27–34.44 ms,
deltas -7.07 to +7.19 ms. These unprofiled runs keep the same harness and input matrix, but provide
only one disabled-gate control, not repeated process pairs or full-plan performance sign-off.

The patch-composition slice's unprofiled `terrain-plan-patches-after-01`, matched against
`terrain-plan-products-after-01`, validates all 32 fixtures. Per-operation preview-ready medians are
5.5–26.2 ms lower; first-idle deltas range from 11.4 ms lower to 7.6 ms higher. This is one exploratory
pair, not a demonstrated speedup or a performance sign-off for the still-incomplete full preview.

The earlier terrain-product slice's matched unprofiled `terrain-plan-stamps-before-01` /
`terrain-plan-products-after-01` captures both validate all 32 fixtures. Per-operation preview-ready
medians range from 5.5 ms lower to 20.6 ms higher (most rise by 6–21 ms as CDT work moves before
click); first-idle medians range from 19.1 ms lower to 9.0 ms higher. This single pair exposes the
tradeoff, not a speedup or city-scale performance certification. Further preview-cache/locality
work must preserve exact dependency checks.

The topology-only milestone's matched, unprofiled schema-3 `terrain-plan-topology-before-01` / `after-01` captures both
validate all 32 fixtures (eight cases, three measured repetitions plus one warmup). In this single
process pair, per-operation backend core-work medians are 14–78% lower, while first-idle medians
range from 39% lower to 25% higher. These are exploratory observations only: frame cadence and
noise matter, and three or more process pairs plus unchanged-build A/A measurements are still
required for a performance claim. No city-scale performance certification is implied.

Before these fixes, release/headless and Forward+ validation completed all 13 cases with identical
profile audit results: 12 settled their full sequences and the recorded terrain-conflict case rejected
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
  prepare/stage/commit transaction; only complete `ok` output with zero omitted faces remains a
  baked clipped mesh
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
