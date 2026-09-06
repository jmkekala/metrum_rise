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

Mouse motion performs no curve baking, validation, or mesh upload in the input-event handler.
The tool resolves the current pointer once per frame, then coalesces changed position/settings into
one lightweight preview update. Camera-only movement also refreshes the preview; motion that leaves
the resolved snap unchanged preserves the displayed exact result. Exact compilation remains async
after the existing idle delay. Cache/request matching uses exact points, so fine movements cannot
redisplay an old result within the former 5 cm tolerance. Clicks resolve their current pointer before
building the committed curve, including clicks arriving before the next frame. Shift angle/length rules and capture/release
distances are unchanged by this scheduling/snap-target change.

The moving and settled previews render asphalt, lane dividers, curbs, and sidewalks rather than a
uniform blue ribbon. They share committed-road texture resources and Rust's lateral band widths;
zero vehicle lanes render the actual 2 m walkway. Valid placement uses untinted materials with no
outline; pending validation is amber, and rejection is red. Lane coordinates remain in metres through curves.

Preview display positions are separate from the authoritative prepared points. Moving feedback
reuses the already prepared terrain-aware vertical profile; settled feedback uses compiled sections.
Both display meshes sample current visual terrain along and across the road, at 0.25–2 m spacing
tied to terrain resolution, and lift vertices 15 cm above
the higher of the planned roadbed and terrain, preserving raised band offsets. This keeps cut areas
visible before commit excavates them, without lowering elevated roads or changing tunnel/bridge
classification, validation, cost, or committed heights. Normal depth testing stays enabled; the
preview is not an always-visible overlay through buildings. It is a sampled placement display, not
an exact visualization of the future cut/fill terrain or junction ownership.

Display generation is `O(longitudinal samples × lateral strips)` for the current stroke, with
constant-time terrain-grid samples and no city-wide query or junction compilation. Buffer capacity
is computed before triangle emission. Packed material/UV data travels with the existing validation
or async result; GDScript performs no per-vertex terrain calls. Each update chooses the matching
exact mesh or moving mesh, avoiding a redundant coarse upload before an available exact result.

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
the transient compiler retains each successful artifact, and the gate independently requires every
candidate-owned span and changed endpoint node piece to exist. Authoritative simulation and render
compiles remain transactional, so placement cannot publish a road whose required surface is missing.
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
- `./run.sh --profile-gameplay-roads` is the primary end-to-end CPU profile. It loads the authored
  Kuopio world in a release window, moves the gameplay camera between deterministic sites, and
  drives the production RoadTool through delayed compiled preview and commit for a controlled
  matrix. Orthogonal two-lane bend, T, and four-way cases are the baselines; oblique T, mixed-width
  four-way, curved bend, close double-T, and chunk-corner four-way cases each vary one named
  complexity axis. A measured commit ends only after its newer network
  generation is acknowledged and foreground terrain, road, water, border-check, ghost-guide, and
  residency queues have settled; speculative prewarming and ready payload caches are not commit
  fences, and node degree is then a correctness gate. The accompanying JSON records phase
  wall-clock timestamps and p50/p95 preview, commit, and complete-fixture timings next to the Samply
  capture. The harness disables gameplay input for the entire run and requires each scripted road
  commit to advance the network generation exactly once; an unexpected external mutation aborts
  the capture instead of contaminating later fixtures. Fixture centers use a `640 m` cadence so the
  independent `180 m` workloads do not repeatedly align with the `510 m` terrain-patch grid; the
  former `520 m` cadence exposed the tracked `ROAD-06` terrain-CDT correctness bug rather than a
  stable profiling sample. The fixed controlled default contains 32 fixtures: one warmup and three
  measured repetitions of eight cases. It reloads the authored world between cycles so each case
  repeats at the same site against a clean network instead of mixing layout cost with accumulated
  network size or new terrain. The `baseline` matrix retains the former growing site sweep; increasing
  its repetition count can expose additional correctness sites, which are failures rather than
  profiling samples. A rejected refined-terrain generation is reported immediately with its
  patch, generation, CDT status, and error instead of waiting for the general timeout. Failure
  cleanup cancels the active road preview before the process exits, and the wrapper validates the
  metrics document because Samply may not propagate the recorded Godot process's nonzero status.
  `./run.sh --profile-gameplay-roads-headless` replays exactly the same workload through Godot's
  headless renderer as the CPU-only comparator. Exact previews now start after `25 ms` of pointer
  idle instead of `100 ms`; pointer motion still resets the delay and the worker still coalesces
  queued requests. A completed exact rejection replaces the earlier cheap candidate verdict, so the
  tool turns invalid immediately rather than displaying a stale valid coarse preview. The targeted
  `METRUM_GAMEPLAY_BENCHMARK_MATRIX=road08` workload verifies that the formerly 90-second apparent
  hang reports `surface_geometry_invalid` in the same bounded preview interval. Exact validation no
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
