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

Road tops now share world-space paving sampling and matte Lambert lighting with authored
plots and graded frontage (`RENDER-13`), including a consistent normal-map basis and sky reflectance.
Road/yard shader bodies and material setup are shared; the old inline stripe code is removed
because markings are emitted as their own mesh layer. See
[plot material consistency and GPU evidence](terrain.md#plot-grass-and-frontage-materials--render-13).

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

## Road marking lighting — RENDER-12

Committed crossings and lane lines use shared, non-emissive per-pixel lighting with
roughness 1.0. Their existing vertex colours, alpha and geometry remain unchanged.
The former unshaded material bypassed night illumination, leaving white and yellow paint
bright against dark asphalt. Paint now responds to ambient, directional and local lights;
editor placement overlays retain their separate preview materials.

This changes only the cached marking material: no simulation work, allocations, city scans,
extra draws or geometry uploads. GPU shading remains O(visible marking fragments), using
the engine's existing light evaluation.

Fresh validation: `road_marking_lighting_test.gd` passes darkness, daylight, low directional
light and local-light checks for white and yellow paint with the shared AgX/bloom/SSIL
environment. Display-space luminance was respectively 0/0, 0.608/0.519, 0.047/0.032 and
0.698/0.619. Matched unprofiled Godot 4.7.2 Forward+ runs on RX 7900 XTX, two alternating
old/new pairs with 60 warm-up and 120 measured frames, gave 0.064/0.064 → 0.067/0.067 ms
at 640×360 and 0.310/0.307 → 0.315/0.316 ms at 1920×1080. This isolates two paint
patches under one directional and one local light, not a whole-city frame.

Command: `XDG_DATA_HOME=/tmp/metrum-road-light RAYON_NUM_THREADS=4 timeout 90 godot --path godot --script res://tests/road_marking_lighting_test.gd -- --asset-editor --benchmark-markings`.
Vsync is disabled during measurement. Artifacts: `/tmp/metrum-road-light/render.log` and
`identities.txt`. Build base: `124be55d13d7eccf02681fc7e906ee43cf1f4a94` plus this material
change; unchanged release extension SHA-256:
`5368fe8e87315e028fad2437e87a26119a0b7b1b7a63ec7b7bf592b8415dbe89`.

## Ownership Boundaries

### Logical Graph

The graph is authoritative for:

- node and edge identity
- route connectivity
- lane counts and modal permissions
- authored road class: `Standard`, `Bridge`, or `Tunnel`
- authored plan polyline control points in world XZ
- finalized physical profiles retained on edges after the shared `RoadEditPlan` finalizer

Endpoint-to-road projection evaluates the XZ parameter and world interpolation in `f64`, then
narrows the final point once. Refinement projects the authored endpoint onto the selected full
geometry segment; it does not reconstruct position from the rounded `segment + t` address.
An endpoint already on a straight centreline therefore retains its representable coordinates.
The native orthogonal zoning-block fixture caught the earlier sideways drift at `(30, -150)`;
that drift changed the road's tangent and produced incompatible zoning frames. Verification is
recorded with the [zoning references](zoning.md#native-road-transactions-and-rendered-references).

Road insertion, node movement, rollback and undo also refresh the zoning system's recorded
straight-road grid choices from the affected edge set. Node movement invalidates both old and
new corridors. These choices belong to zoning, use one local adjacency ring, and survive save
remapping; see [persistent alignment](zoning.md#persistent-straight-road-alignment-checkpoint).

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

Span terrain-clip source endpoints retain the first matching canonical XZ/height coordinate through
a temporary sorted key table, O(P log P + E log P) for P loop points and E source edges. Earthwork
vertex normals reuse one winding computation per loop. The node-move reservation guard stages
exact span checks before expensive node solving and uses the existing exact artifact-reuse checks
for current live-node candidates. Geometry contracts and evidence are recorded in
[zoning validation costs](zoning.md#staged-node-validation-and-boundary-lookup-cost).
Node ownership cleanup also shares immutable source-point and rail-path preparation between its
passes, scoped to one borrowed rail set and keyed by exact source and policy. Contour work and
error predicates remain unchanged; see [source preparation reuse](zoning.md#reuse-of-immutable-ring-source-preparation).

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

Full and incremental rebuilds share physical-lane and node-connection construction. Incremental
updates assign new IDs in sorted edge/node order and gather surviving lane references only from
the affected nodes' existing adjacency. Only lanes arriving at a rebuilt node lose their old
connections; the opposite direction on a preserved road retains the untouched far-end junction.
Local discovery/map work scales with incident lanes, plus O(K log K) sorting of K affected owners
and their existing geometry/connector work. Ordered publication preserves deterministic IDs.

Reattachment chooses the smallest squared distance, then the smallest lane ID for an exact tie
(`AUDIT-01-A14`). Approximate distance ties are not transitive and can make the selected lane
depend on unordered edge iteration. The query remains allocation-free and scans only lanes in
the affected closure; its cost is O(P), where P is their total polyline point count. The existing
outer invalidation/reattachment passes still visit the agent store; this change does not add a
new agent spatial index or claim to remove that O(A) work.

### Lane Reattachment Audit Measurements (2026-09-12)

Five alternating unprofiled CPU-0 release pairs measure the local query against two connected
100 m roads with four vehicle lanes. Prebuilt positions span longitudinal coordinates 0–200 m
and lateral coordinates −5–5 m. Fixture construction and result hashing are outside timing;
each process performs three warmups and 21 query batches. Selected lane IDs, lane distances and
squared-distance checksums match across builds and all runs. The separate close-distance/order
regression intentionally changes the old incorrect result and also checks exact ties.

| Queries | Before → after median, ms |
| --- | --- |
| 1,024 | 0.078691 → 0.076529 |
| 16,384 | 1.256949 → 1.235671 |
| 131,072 | 10.059423 → 9.889772 |

Three further alternating pairs run the existing populated paved-site planning fixture on
eight physical cores (`taskset -c 0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`). Four local occupied
paved sites and the measured T-junction remain fixed while remote buildings, parcels, agents
and roads grow. Each build verifies identical local products across all background sizes.
After three warmups, each size measures 100 planning/readiness/worker calls; fixture preparation
is excluded and the one-time snapshot is reported separately.

| Remote buildings / roads | Total agents | Plan before → after, ms | Worker before → after, ms | Snapshot before → after, ms |
| --- | --- | --- | --- | --- |
| 0 / 0 | 24 | 19.713903 → 19.978799 | 20.671695 → 20.863355 | 0.006833 → 0.006239 |
| 1,000 / 4 | 6,024 | 19.934972 → 19.654014 | 20.728349 → 20.633692 | 0.054038 → 0.055384 |
| 10,000 / 40 | 60,024 | 19.813491 → 20.078570 | 20.793456 → 20.633240 | 0.352461 → 0.347382 |
| 100,000 / 391 | 600,024 | 20.382618 → 19.840608 | 20.969874 → 20.822044 | 3.471363 → 3.376629 |

These are medians of process medians. Local planning remains comparable and independent of
background size; snapshot construction still scales with the stored city. This planning fixture
does not time applying an edit or remapping the agent store. No concurrent builds or other
benchmark jobs ran during timing, and both source trees and executables remained fixed.

Reproduce with `python3 /tmp/metrum-full-audit/match_lane_reattachment.py`. The runner records
commands for `simulation::economy::agents::remap::tests::benchmark_lane_reattachment` and
`nodes::sim::core::tests::road_plan_scaling::populated_paved_road_plan_scaling`, using
`--exact --ignored --nocapture --test-threads=1` and `METRUM_DEBUG=0`. Query runs use one worker;
planning runs use eight. Results are in `lane-reattach-matched-{bench,summary}.json` under that
directory. `lane-reattach-{before,after}-identity.json` records all source hashes and Rust 1.98.1
(`48a229cea`, 2026-09-01). Binary SHA-256 values are
`d7490495dd5c66a0a9ddd3e86d013ac1e8e756e4ae6828096a805c0540b05bf4` before and
`14e2f21e2685cae09f4f0aed7ba7057b4fa40515f1d099de9c60c60fb93a4651` after.

### Lane Rebuild Audit Measurements (2026-09-12)

`AUDIT-01-A16` reproduces two incremental-update failures: an untouched far-end junction loses
its outgoing connections, and identical edits assign different physical/connector IDs through
unordered sets. The corrected tests preserve remote connections and compare 32 repeated edits.
The shared builders replace copied full/incremental code, three fixture closures reuse the existing
road helper, and two weaker tests are removed. One checked no vehicle routes because its loop only
encountered pedestrian table entries; the other called every connector a crosswalk. Existing tests
retain actual vehicle turns, crosswalk markings, lane geometry and full/incremental equivalence.

Three alternating CPU-0/one-worker release pairs run
`simulation::network::lanes::tests::rebuild_benchmark::incremental_lane_rebuild_scaling`.
The fixed frontier has two rebuilt roads and one preserved road. Each process adds isolated remote
roads, builds the graph/lanes outside timing, warms one update, then measures 21 updates. Local
physical and connector geometry, distances, markings and destinations are hashed after timing;
references use lane roles so background-dependent numeric IDs do not affect the comparison.
Products match across background sizes and repeats within each version. The corrected version
intentionally differs from the baseline because the far-end connections now survive.

| Remote roads | Local rebuild before / after, ms | One-time fixture setup before / after, ms |
| ---: | ---: | ---: |
| 0 | 0.013385 / 0.013207 | 0.132 / 0.128 |
| 1,000 | 0.089305 / 0.014780 | 1.876 / 1.880 |
| 10,000 | 0.891426 / 0.013518 | 19.597 / 19.573 |
| 100,000 | 24.387279 / 0.011468 | 206.802 / 206.027 |

The previous update populated a temporary map from every surviving road; the corrected map stays
local. These are lane-update timings, not complete road-commit or frame timings. As a separate
check, three matched pairs run the populated paved-site planning fixture above on eight physical
P-cores / Rayon workers. All local planning products remain identical as remote buildings,
parcels, agents and roads grow. The largest fixture has 100,000 remote buildings, 391 remote roads
and 600,024 total agents.

| Remote buildings | Plan before / after, ms | Worker before / after, ms | Snapshot before / after, ms |
| ---: | ---: | ---: | ---: |
| 0 | 20.071 / 19.939 | 20.934 / 20.803 | 0.007431 / 0.006799 |
| 1,000 | 19.969 / 19.683 | 20.608 / 20.749 | 0.052251 / 0.055508 |
| 10,000 | 20.176 / 19.932 | 20.872 / 20.815 | 0.349490 / 0.354304 |
| 100,000 | 20.150 / 20.396 | 20.857 / 20.878 | 3.349324 / 3.282118 |

Planning remains comparable; this does not claim removal of its one-time world snapshot or the
existing agent invalidation pass. Both workloads use Rust 1.98.1, offline release lib-test builds,
`METRUM_DEBUG=0` and no concurrent builds/tests. Exact commands, source identities, the three-file
diff and matched results are `/tmp/metrum-full-audit/lane-rebuild-*`; replay with
`python3 /tmp/metrum-full-audit/match_lane_rebuild.py`. Before/after executable SHA-256 values are
`e37149dd31660156f5cddd4976b2c2cc324536cc888a92c3a71184583f6c3ea1` and
`991bd166a3c5da6d8620fef8bce48fada818cba4107980a7a141ebe1364a8a5d`.

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

Node movement and merging update graph ownership, spatial entries and edge routing metrics in the
same mutation (`AUDIT-01-A17`). Merges resolve both canonical parents before selecting the lower
ID, remove old R-tree entries before changing geometry, and retire the removed node from the
lookup grid. Reindexing skips aliases. Local adjacency supplies the moved/merged edges; sorting
covers only the participating nodes' incidence. Self-loops retain two endpoint incidences but
receive one geometry update.

Endpoint deformation uses horizontal distance along the road. Independently sampled control and
physical profiles first share the union of their support positions, interpolating each profile's
own heights at added positions. Both receive the same smooth displacement, preserving their
common XZ alignment and distinct vertical profiles. Already aligned profiles allocate nothing;
merging unequal support sets takes O(C + P) time and temporary output storage. Edge length and
slope-aware routing cost refresh through the existing cost calculator. Clip, lane, terrain and
agent-plan publication remain the caller's coordinated edit work.

Editor node selection and border-connection lookup share the simulation's nearest-node search
(`AUDIT-01-A18`). Both use the existing 16 m node grid and require a live canonical node. Editor
hits use XZ distance; border lookup retains 3D distance. The radius is exclusive and equal-distance
nodes choose the lowest ID, including fresh cursor snaps and endpoint acquisition from a retained
edge. The bridge only converts an absent result to its existing `-1` UI value.

Node rectangles share one allocation-free visitor. It probes covered lookup cells, or scans the
hash table when its allocated capacity is smaller than the rectangle's covered-cell count
(`AUDIT-01-A20`). Counting populated cells alone missed table capacity retained after clear/rebuild.
Exact coordinate checks retain inclusive rectangle bounds. Cost is O(min(covered cells, table
capacity) + nodes in visited cells), plus incident-edge checks for candidates. Global rectangles
therefore follow stored index capacity rather than empty world space. Sorted collecting queries
retain their ordering and deduplication. No new index or per-query candidate buffer is introduced. Retired 3D network-point snapping, the unused
whole-network edge query, its segment projector and the unused projection-data API are removed.

Road-inspector hover selection uses the existing edge R-tree and physical-profile XZ geometry
(`AUDIT-01-A19`). Deleted road slots cannot win selection. The existing zoning-depth-derived
selection radius remains inclusive, repeated profile points remain valid, and exact distance ties
choose the lowest live edge ID regardless of tree update history. The search visits overlapping
bounds and their profile segments without allocating candidate lists; the bridge supplies the
configured radius and converts the result to its existing UI ID.

Thirteen unused geometry-query methods are removed with their complete caller chains: the old
obstacle-polygon export, polygon ray depth, road-boundary projection, curved frontage, 2D geometry
export, averaged network direction and unused connection-coordinate utility. The current 3D road
geometry API and active node/edge inspectors remain the rendering and selection paths.

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

Road guiding lines and guide snapping are removed; see
[road guide removal](#road-guide-removal-road-27).

Road surface, earthwork and fine-query chunk owner indices now share immutable map branches
across preview snapshots using `imbl`, with sorted owner sets shared by `Arc`. A local membership
change copies its trie path and touched owner set, preserving older snapshots. Spatial coverage,
query predicates and owner ordering are unchanged. Cached node inputs and reverse owner-to-chunk
lists also share map roots and immutable records, including inputs held by preview certificates
and bounded undo entries. Other graph/surface snapshot fields still copy, so this does not
establish whole-edit locality. The dependency rationale and matched evidence are owned by
[ZONE-04](zoning.md#shared-compiler-inputs-and-reverse-coverage).

Mouse motion performs no curve baking, validation, or mesh upload in the input-event handler.
The tool resolves the current pointer once per frame, then coalesces changed position/settings into
one lightweight preview update. Camera-only movement also refreshes the preview; motion that leaves
the resolved snap unchanged preserves the displayed exact result. Exact compilation starts immediately
and continues during motion: one running job completes while the current curve replaces pending
intent, then the newest input is dispatched. The native mailbox also holds at most one pending job;
there is no idle timer or unbounded request backlog. Cache/request matching uses exact points, so fine movements cannot
redisplay an old result within the former 5 cm tolerance. Clicks resolve their current pointer before
building the committed curve, including clicks arriving before the next frame. The road-options
checkbox controls zoning-grid snapping. The Shift shortcut and old 15-degree / fixed-10-metre
snap are removed. Road connections stay enabled. Zoning snap uses persistent curb alignment,
selected road width and configured cell size. It captures directions only within 5 degrees and
half a cell of lateral displacement. Other angles stay free. The placement
and complexity contract is owned by [ZONE-04](zoning.md#11-road-generated-cell-zoning--zone-04).

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

Committed farm fields also reserve land (`ECON-07`). Road creation rejects `field_overlap` from
both the stroke corridor and finalized local carriageway/curb/sidewalk/junction polygons. The
final guard queries live field reservations before adopting or consuming a road plan. Preview
caches include the agriculture revision; see [`economy.md`](economy.md#field-placement-and-editing-econ-07).

Placement validity also includes local road-surface compileability. A preview or commit replays the
candidate's local post-split topology before acceptance, including interior crossings against nearby
road edges. Any edit that would fail to compile the new span or its required endpoint `Terminal` /
`Bend` / `JunctionN` pieces is rejected before it reaches the live graph; tight switchbacks are
allowed only when the compiled surface topology can actually represent them.
The exact async preview carries a shared `RoadEditPlan` for its matching commit. The simulation
thread requires identical raw road points, lanes, snap mode, surface generation and source/visual
terrain revisions before borrowing the prepared profile and skipping its exact candidate replay. The plan
also retains the finalized local graph delta: resolved connections, splits and merges, terminal
extensions, junction-adjusted profiles, clip distances and affected edge/node scope. Preview and
unplanned commit use the same finalizer. Matching commits install the solved delta through
`TransitNetwork`, mapping preview-local identities to existing or appended live slots without
repeating intersections, junction solving, regrading or clipping. An incomplete mutation frontier
cannot be adopted. Stale/missing click plans are rebuilt through the same local compiler against
borrowed live inputs before opening the transaction; no whole-city snapshot is copied at click.

The completion plan also retains local road/site terrain CDT tiles and joined patch render
buffers. Default road-only hover retains just the solved road products and local query inputs;
optional full preview completes terrain during hover (`ROAD-29`). Planned
cutout queries remove replaced source owners **before** union and remap local provenance to the
same live IDs that topology adoption will allocate. Existing road contributors outside the graph
excerpt are retained through the existing chunk/query indices. The simulation command uses the
production grading, fixed-tile CDT and buffer builders after click; there is no second terrain
compiler. It captures nearby site footprints/support heights through the allocator's existing
512 m chunk index under the simulation lock, then compiles the affected patches through Rayon
before opening the transaction. An unprepared index leaves the terrain candidate provisional:
road-only pointer work does not capture terrain sites. Full preview captures bounded site
inputs without rebuilding or scanning the resident building store.
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
product changes authored terrain or skips rollback. Structural stamp materialization, cutout
patch discovery and terrain compilation happen after click in the default road-only mode
(`ROAD-28`); full preview opts into this work during hover (`ROAD-29`). Hover reads terrain
samples for the authoritative road profile and junction heights, then publishes the complete
canonical road scene: spans, terminals, junctions, sidewalks, curbs, crossings and lane markings.
Preview display replaces the new stroke, its junctions and bounded approach transitions only.
Existing connected roads outside those bounds retain their committed vertices and attributes;
neighboring junctions stay resident. Bounds derive from the junction crossing core, the 32 m
profile blend support and roadbed width. Boundary triangles are partitioned with interpolated
attributes instead of being discarded. Both sides use solved heights without a blanket display lift. The renderer performs no terrain-dependent clearance
solve, terrain-contact retessellation or vacated-cutout infill. Occasional clipping/z-fighting
and exposed old cutouts are accepted within the edited preview footprint, not on distant existing roads. In road-only mode terrain resources, samples, payloads and acknowledgments stay unchanged,
including while idle; terrain renderer residency is not a dependency. Full mode temporarily
substitutes resident draw meshes only, leaving authoritative samples/cache payloads untouched.

`plan_state` describes road geometry readiness in road-only mode and complete road/terrain
readiness in full mode: pending, provisional, invalid, stale, consumed or
ready. Ready road geometry is not full placement authorization. It requires a complete local
road solve, matching road/source/visual dependencies and current water/parcel/field/cell checks.
Godot stages canonical road meshes with retained source owners; the latest completed pose remains
visible while newer input compiles. An older rejected request cannot clear a newer provisionally
valid pose. Missing current road products remain visibly under checking.

A matching click shares the immutable road preparation, graph/profile delta and canonical surface
products, then owns its newly completed terrain plan. It does not mutate the published preview
or repeat the road solve. A stale or missing road candidate is compiled locally against current
inputs. Full commit readiness additionally requires current site inputs, valid terrain buffers
and all final ownership/coverage checks. Product claims remain single-use after preflight;
terrain rejection preserves the committed world and reports failure to the tool. The terrain
preview exporter, temporary terrain renderer and paired-display readiness gates are removed.
Repeated cursor work is bounded by the affected road neighborhood and emitted vertices; terrain
work follows affected patches/sites only after click. Shared context snapshot costs and full
placement costs are measured separately from repeated previews.

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
and builds no extra index. The same sampler serves snapping and other visible-height
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

### Player-selectable road preview modes (`ROAD-29`)

**Status: done (2026-09-28), release extension deployed.** Options → Gameplay → Road construction preview:

- **Road only (faster)** is the default. Draw the new road, junctions and bounded approach
  transitions over unchanged terrain; build terrain when placed.
- **Road and terrain** also prepares the final local grading/cutout terrain buffers during
  hover. It may respond more slowly. Stage both mesh batches before displaying either;
  use immediate road feedback while terrain resources are unavailable.

The preference is stored as `gameplay/road_preview_mode` in `user://settings.cfg`. Apply updates
active road tools immediately, retires old requests/displays and restores original terrain.
Cancel discards pending preference edits; Reset selects Road only until Apply. New tools load
the saved preference. Mode is a presentation/work request, not a different placement rule.

Both modes retain the bounded visible road footprint, exact neighboring/previous-cursor reuse
and solved heights without a blanket lift. Full mode reuses the production earthwork/CDT
planner: compute road/earthwork inputs off-lock, capture only indexed local site inputs under
the core lock, then compile local terrain through Rayon off-lock. No new spatial index or
whole-city scan is introduced. Terrain draw substitution leaves source samples, textures,
resident payload caches and acknowledgments unchanged. Cancel, tool teardown, mode changes
and patch replacement/recycling restore the exact resident meshes; patch invalidation clears
the paired road preview too. Road-only staging does not depend on terrain residency.

A matching full-mode click shares the completed immutable terrain plan after rechecking all
road, terrain and site dependencies. If terrain dependencies changed, placement recompiles
terrain before opening the transaction. The same atomic validation/adoption/rollback path
serves both modes; hover never mutates authoritative terrain or topology.

Fresh validation: 2,013 Rust tests passed (83 ignored), five headless Godot suites
(settings, junction previews, preview stream, chunk renderer, zoning road tool), and
14 rendered junction fixtures, including five full-terrain captures. Rustdoc and release
build pass. Terrain staging requires all affected patches to be resident; until then the
immediate road feedback remains available. Render diagnostics use a flat diagnostic material,
not the production terrain shader.

Matched, unprofiled release measurements use four Rayon workers and `METRUM_DEBUG=0`.
`benchmark_road_preview_modes --ignored --nocapture` runs 32 cursor positions on an 80 m
branch meeting a 192 m existing road, discarding eight warmups. Worker/readiness p50/p95:
road-only **16.612/19.721 ms**, road and terrain **22.137/25.963 ms**. Setup/snapshot and GPU
upload are excluded; this is a mode comparison, not a universal frame-time guarantee.
`populated_cell_road_plan_scaling --ignored --nocapture` holds the local edit fixed, with
100 samples per size. At 0 / 1k / 10k / 100k background buildings, road-only worker p50 is
0.933 / 0.946 / 0.910 / 0.929 ms; full worker p50 is 6.448 / 6.815 / 6.514 / 6.520 ms.
Local products match throughout, reaching 600,024 agents, 100,004 parcels and 391 background
roads. One-time snapshots are separate: 0.006 / 0.012 / 0.046 / 0.358 ms.

Artifacts: `/tmp/metrum-preview-modes/{modes,locality,full-tests,rendered,rustdoc}.log` and
`capture_*_full.png`. Run native checks with `cargo test --offline --release --lib` from
`rust/` and the filters above; run Godot scripts under `godot/tests/` against that release.
Use `TMPDIR` on a writable filesystem (these runs used `rust/target/preview-test-tmp`).
Native test binary SHA-256: `6a1bd37b08e0c2006394ad47b9aeb4fb1d86f767008fb86d67460e418029f41e`;
deployed extension: `6382e3a4c62cb6d5ba53532ccb3ab83caebe1ef2a1fc1723bc828934cbbf632f`.
The final extension also includes the generation-label export correction validated by the
Godot suites; native timings measure the same planner implementation.

User-requested repeat after deployment (2026-09-28): rebuilt the release test binary from
the final source, then ran the same benchmark three times sequentially with four Rayon workers,
debug disabled and no competing builds/tests. Each run has eight warmups and 24 measured poses
per mode. Road-only p50 values: **16.383 / 16.575 / 16.345 ms**; full-mode p50 values:
**22.060 / 21.531 / 21.986 ms**. Median of run medians: **16.383 vs 21.986 ms**, or
**5.603 ms / 34.2% more CPU planning time** for full mode. Per-run p95 ranges are
18.132–21.691 ms and 23.617–25.000 ms, respectively. These timings exclude retained source
mesh filtering, worker lock/queue waits, bridge serialization, GPU upload and display latency.

Fresh locality rerun also passes with identical local products at all four city sizes:
road-only worker p50 **0.898 / 0.956 / 0.998 / 0.998 ms**, full worker p50
**6.245 / 6.450 / 6.597 / 6.875 ms**. Snapshot setup is separate:
0.005 / 0.014 / 0.050 / 0.399 ms. This fixed-input reuse workload differs from moving-cursor
planning above. Commands are the same filters documented above; fresh artifacts are
`/tmp/metrum-preview-modes/modes-requested.log`, `modes-requested-{2,3}.log` and
`locality-requested.log` in that directory. Test binary SHA-256:
`db06828a61d03bc0eb2c34c6c98559bf474b26b9051245d7ff3b714528cd6250`.
Deployed extension identity remains unchanged.

### Preview display performance plan (`ROAD-30`–`ROAD-36`)

**Status: `ROAD-30`–`ROAD-34` done; `ROAD-35`–`ROAD-36` planned (2026-09-29).** Implement and
validate one task at a time. `ROAD-30` establishes the baseline below. Next: `ROAD-35`; use the
measurements to justify any reprioritization. Each task has a separate roadmap entry and acceptance evidence. These are
candidate improvements identified by source inspection, with no measured speedup yet.
The `ROAD-29` native comparison (16.383 vs 21.986 ms) excludes frontend presentation costs
and does not establish which frontend stage dominates. Road/junction solving remains a
separate cost even if display work becomes cheaper.

**Primary target: Road and terrain.** Prioritize full road/junction/terrain responsiveness
when selecting the next optimization. Shared improvements should benefit both modes;
full-mode terrain export, texture/resource reuse and paired staging take precedence over
road-only-specific polish. Keep Road only available as a comparison while this work proceeds.
The expectation that its eventual advantage may be small is a hypothesis to test with
rendered end-to-end measurements, not a conclusion from the native-only comparison.

After the measured improvements, review whether Road only provides a meaningful latency,
frame-time or memory advantage across the supported workloads/hardware. Record the resulting
recommendation: retain both modes, or simplify to Road and terrain. Removing a mode or changing
the default is a separate product decision after that review; this plan changes neither.

#### `ROAD-30` — Measure input-to-display latency

Extend the existing road preview/gameplay benchmark and render diagnostics to correlate an
input/request ID across input sampling, dispatch, worker start/end, retained-mesh filtering,
result polling, bridge export, Godot validation, mesh/texture staging, atomic installation
and the first rendered frame containing the result. Measure result age while moving and
time to the exact final pose after stopping. Separate queue/lock waits, one-time snapshots,
CPU command submission and rendering-server waits. A render fence or frame callback alone
does not establish physical GPU upload time or monitor presentation latency; label those
boundaries accurately and use GPU timing only where supported.

Use full mode as the primary baseline and run both modes on matched flat/sloped T and multi-junction fixtures, continuously moving
and stationary, including dense affected chunks and full-mode terrain residency retries.
Record p50/p95, frame hitches, exported bytes, changed/reused chunks and layers, resource
creations and worker utilization. Keep instrumentation behind the existing benchmark/debug
facilities; establish acceptance timings with matched unprofiled release runs.
**Exit:** reproducible baseline with stage attribution, workload/build/worker/render settings,
commands and artifact locations in this section. No implementation speedup claimed.

**Implementation (validated 2026-09-29):** the existing gameplay benchmark now
accepts `METRUM_GAMEPLAY_BENCHMARK_MATRIX=preview`. It drives the production RoadTool through
flat/sloped T, flat/sloped neighboring-cross, dense-chunk and controlled terrain-residency
retry fixtures, with Road and terrain first and Road only second. Each case/mode resets its
512 m world, waits for each setup commit and renderer to settle, warms the preview, then
measures stationary replacements and a moving pointer followed by an exact final pose.
Moving input uses absolute 60 Hz deadlines (400 scheduled samples at 100 repetitions);
late frames coalesce overdue inputs. The capture records scheduling lateness and validates
total trace duration, so a relative timer cannot silently halve one mode's input rate.
Latency starts at scripted world-space input sampling, excluding OS input/raycast latency.
Setup commits and snapshot construction are outside the measured drag. Existing
`METRUM_DEBUG_PERF=1` snapshot diagnostics attribute one-time copying separately.

The opt-in collector correlates sampled input IDs with native request IDs. Rust adds a
fixed-size optional timing record per request; disabled captures do not read clocks in
these new stages. Frontend capture is enabled by the benchmark or `METRUM_DEBUG_PERF=1`.
It holds at most eight live request records and 4,096 observations per stream. Packed byte
accounting visits payload containers and array lengths, never vertices. Complexity is
O(changed payload metadata) per result and O(1) per frame; no city-wide query or index is added.

- Native durations: mailbox queue, context read-lock/Arc capture, road solve/render,
  retained-cache lookup plus earthwork preparation, core-lock wait, local site/mesh capture,
  terrain compilation and retained-mesh filtering. Disjoint worker stages sum to worker
  elapsed time. Bridge diagnostics add result read-lock wait, readiness and packing.
- Frontend: total polling calls/time, road buffer validation, CPU mesh submission,
  terrain preflight, mesh and resource creation, road/terrain installation, and first
  `frame_post_draw` after installation while that request is still displayed. Headless
  runs use a separately labelled process-frame boundary and are not rendered acceptance.
- Totals and sub-stages overlap intentionally: queue timing starts inside request
  dispatch; `poll_ms` includes bridge operations;
  worker-end-to-poll includes publication and result read-lock wait; `stage_install_ms` includes the
  validation/resource/install sub-stages. Do not sum overlapping fields. Terrain resource
  time includes node/material/image/texture and retaining-wall creation, not GPU upload time.
- Moving observations include both age when a new result first appears and displayed
  result age on subsequent frames. Stationary observations require the exact sampled pose;
  the final stop-to-exact-frame time is recorded separately. Frame hitches use 33.333/50 ms
  thresholds. Packed bytes exclude dictionary/object overhead; resource counts describe
  Godot objects, not physical GPU allocations. Changed/reused road
  chunks/layers and created terrain meshes/nodes/materials/images/textures are counted.
- Worker service fraction is observed, polled worker wall time excluding core-lock wait,
  divided by the moving interval. Boundary jobs may remain incomplete; this is not CPU
  core utilization. Diagnostic captures also collect latest engine viewport CPU/GPU
  timings and an out-of-band post-workload CPU command-drain measurement. Those values
  do not establish a request's physical GPU upload duration or monitor presentation.
  See [Godot RenderingServer timing contracts](https://docs.godotengine.org/en/stable/classes/class_renderingserver.html#class-renderingserver-method-viewport-get-measured-render-time-gpu).

The existing `tools/road_benchmark_report.py --validate` accepts the preview matrix and
rejects missing modes, duplicate request observations, reversed milestones, stale final
poses, missing residency retries, truncated captures and mutations of authoritative road
state. Existing quantile policy is retained: p95 needs 100 observations, p99 needs 1,000;
otherwise the report uses null. Diagnostic stage attribution and unprofiled release latency
captures are separate artifacts. No implementation speedup is claimed by `ROAD-30`.

Run the complete matrix through the existing launcher (synthetic world, no authored map
required). Use a distinct run ID for each process; do not run builds or other benchmarks
concurrently with acceptance measurements:

```bash
RAYON_NUM_THREADS=4 METRUM_DEBUG=0 METRUM_DEBUG_PERF=0 \
METRUM_GAMEPLAY_BENCHMARK_MATRIX=preview \
METRUM_GAMEPLAY_BENCHMARK_REPETITIONS=100 \
METRUM_GAMEPLAY_BENCHMARK_WARMUP_REPETITIONS=8 \
METRUM_GAMEPLAY_BENCHMARK_MAX_FPS=60 \
METRUM_GAMEPLAY_BENCHMARK_RUN_ID=road30-release-a \
./run.sh --benchmark-gameplay-roads
```

Repeat with a new run ID for an independent process; use `METRUM_DEBUG_PERF=1` only for
separate stage/GPU diagnostics. `METRUM_GAMEPLAY_BENCHMARK_CASES` can select a comma-separated
subset of `flat_t,sloped_t,flat_multi,sloped_multi,dense,residency_retry`; both modes still run.
Optional `METRUM_GAMEPLAY_PREVIEW_CAPTURE_DIR` captures final rendered poses after timers
finish; keep it unset for acceptance. Headless runs are suitable for pipeline correctness
checks but cannot establish rendered latency. The renderer's original preview geometry,
terrain, scheduling and commit rules are unchanged by this measurement task.

**Acceptance evidence (2026-09-29):** two independent unprofiled release processes, each
with eight warmup poses, 100 measured stationary replacements and 400 absolute-deadline
60 Hz moving samples per fixture/mode. All 12 case/mode combinations passed in each run;
a separate diagnostic process passed the same matrix with native/GPU timing enabled.
Inputs, authoritative state, cadence and exact final-pose checks passed; no observations
were dropped. These are small controlled neighborhoods, not a city-scale or cross-hardware claim.

Hardware/settings: Core i9-12900K, Radeon RX 7900 XTX / RADV, Godot 4.7.2, X11,
Forward+ Vulkan, 1280×720, VSync enabled, 60 FPS cap, four Rayon workers, simulation paused.
Build: `cargo build --offline --release --manifest-path rust/Cargo.toml`,
Rust 1.98.1 (`48a229cea`), existing release profile (optimized, debug info level 1).
All captures use release library SHA-256
`f01783c0d56537a0e67fc060ba614c8eaa5e1183672d6bb109b5a9af67ed7ac7`.

The table reports ranges of the two process-level quantiles, in milliseconds; ranges are
not confidence intervals. Displayed age includes frames between new result installations.

| Fixture | Full p50 / p95 | Road-only p50 / p95 | Moving displayed-age p95: full / road-only |
| --- | --- | --- | --- |
| `flat_t` | 45.4–45.8 / 50.9–51.8 | 23.1–25.4 / 41.5–42.0 | 63.6–64.5 / 53.0–55.4 |
| `sloped_t` | 45.3 / 62.3–62.6 | 39.1–39.2 / 42.6–44.7 | 68.5–68.7 / 55.6–57.8 |
| `flat_multi` | 47.3–47.7 / 51.8–52.2 | 40.3 / 44.5–45.9 | 67.1–67.5 / 53.2 |
| `sloped_multi` | 64.4–64.5 / 68.1–69.0 | 41.4–41.8 / 57.3–57.8 | 86.7–102.1 / 69.5–69.8 |
| `dense` | 49.6–49.7 / 53.3–55.5 | 27.4–42.8 / 44.8–51.3 | 56.4–70.4 / 59.3–67.5 |
| `residency_retry` | 94.1–94.3 / 103.5–103.7 | 23.0–25.7 / 39.2–43.3 | 63.3–66.5 / 53.0–53.1 |

The 60 FPS cadence matters: dense road-only medians varied from 27.4 to 42.8 ms
between the two processes. Work crossing a frame boundary can change observed latency
by approximately one frame. These descriptive baselines do not establish a fixed mode
speedup ratio or an optimization gain over earlier development captures.

The retry fixture deliberately withholds resident patch lookup until three failed staging
attempts; its ~94 ms full-mode median includes that injected delay. It is not a natural
streaming-latency estimate. Ordinary final-stop observations ranged from 46.8–98.6 ms in
full mode and 39.0–75.1 ms in road-only mode (one continuous-drag stop per case/process).
Across all cases the two acceptance runs recorded 58/45 frame intervals over 33.333 ms
and 0/0 over 50 ms; per-case counts and distributions are retained in the raw captures.

Diagnostic stationary medians below locate costs; they are not acceptance latency or
additive totals. Worker time and polling gaps vary between runs/modes even where the
road algorithm is shared. All values are milliseconds.

| Fixture / mode | Worker | Worker end → poll | Bridge packing | Road validation | CPU mesh submission | Terrain resource creation | Stage + install total |
| --- | --- | --- | --- | --- | --- | --- | --- |
| flat_t / full | 19.69 | 13.52 | 0.39 | 1.91 | 0.29 | 3.50 | 7.70 |
| flat_t / road-only | 15.88 | 1.48 | 0.15 | 1.94 | 0.33 | 0.00 | 2.58 |
| sloped_multi / full | 36.62 | 13.30 | 0.48 | 3.37 | 0.43 | 3.90 | 10.10 |
| sloped_multi / road-only | 29.89 | 3.70 | 0.24 | 3.42 | 0.50 | 0.00 | 4.23 |
| dense / full | 23.27 | 8.67 | 0.57 | 5.06 | 0.59 | 4.08 | 12.54 |
| dense / road-only | 16.27 | 1.75 | 0.27 | 5.12 | 0.66 | 0.00 | 6.11 |

For those full-mode fixtures each new pose created a median eight road meshes/40 road
layers and 16 terrain patches (32 nodes, 16 materials/images/textures). Regular planes
already use the terrain mesh cache: the final identity-checked counters report 20 new
terrain meshes/12 reused planes for the T and multi-junction cases, or 22 new meshes/10
reused planes for the dense case. Retaining-wall ArrayMeshes count as creations even
when they have no drawable surfaces.
Retained GPU chunk/layer reuse was zero while clipping footprints moved. Median packed
payloads were 0.654 MB (flat T), 0.991 MB (sloped multi) and 1.455 MB (dense). Observed
worker service fractions ranged from 0.55–0.84 across the six rows above. Diagnostic
viewport GPU medians were 0.67–0.87 ms; the separate quiescent CPU command drains were
0.001–0.002 ms. Neither figure isolates GPU transfers or internal waits inside API calls.

These measurements support tackling `ROAD-31` validation and `ROAD-32` terrain-resource
reuse next. `ROAD-33` can reduce retained-buffer validation/upload volume; `ROAD-35` has
a measurable polling/scheduling interval to investigate. Packing and CPU mesh submission
are smaller costs in these fixtures. Road/junction solving remains separate, and the
choice to retain one or both preview modes remains deferred until after improvements.

Artifacts: `rust/target/road30-artifacts/` (also available as `/tmp/road30`) contains
`acceptance-v3-a.json`, `acceptance-v3-b.json`, `diagnostic-v3.json`, their matching logs
and `*-manifest.json` files. Manifests pin source hashes, binary, command, working directory
and environment. `run_baselines.py` reproduces the isolated-profile sequence. These runs
used the existing `/tmp/metrum-road28/candidate/godot` project with workspace scripts and
library, the workspace `rust/target/preview-test-tmp` TMPDIR/XDG profile, and the direct
command `godot --path /tmp/metrum-road28/candidate/godot --windowed --resolution 1280x720 -- --gameplay-road-benchmark`.
The launcher recipe above uses its default 1920×1080 window, so use the recorded command
and manifests when matching these exact numbers. Only v3 artifacts constitute the accepted
baseline; earlier development captures are superseded.
`captures3/` contains diagnostic final-pose screenshots taken outside operation timers.

**Correctness/build checks:** five release Rust mailbox/timing tests, nine Python report
validation/comparison tests, and the Godot metrics, junction-preview, stream-preview and
chunk-renderer suites pass. The junction suite verifies resource counters against distinct
mesh identities, including reused planes and terrain-free bridge previews. Rustdoc completes
without warnings; `bash -n run.sh` and `git diff --check` pass. The verified release library
was deployed atomically to `godot/bin/libmetrum_rise.so` with the capture hash above.
Logs live alongside the captures:
`rust-tests-final.log`, `rustdoc.log`, `metrics-final.log`, `mesh-count-final.log`, and
`final-road_preview_stream_test.log` / `final-network_tool_chunk_renderer_test.log`.

Commands used for targeted checks:

```bash
TMPDIR="$PWD/rust/target/preview-test-tmp" cargo test --offline --release --manifest-path rust/Cargo.toml --lib road_preview::
cargo doc --offline --no-deps --manifest-path rust/Cargo.toml
python3 -m unittest discover -s tools -p test_road_benchmark_report.py
```

Godot checks use `godot --headless --path /tmp/metrum-road28/candidate/godot --script
res://tests/<suite>.gd` with the isolated TMPDIR/XDG profile and four Rayon workers from the
capture manifests. No road-planning algorithm changed, so this task does not establish a
new populated-map planning speedup or repeat the unrelated city-scale acceptance suite.

#### `ROAD-31` — Validate immutable road buffers once in Rust

`godot/scripts/tools/network_tool.gd::_append_road_surface` scans all position, normal, UV
and colour components in GDScript before every upload. Extend the existing terrain pattern
(`terrain_mesh_payload_validated`) to road payloads: validate final immutable buffers in Rust
after clipping/export preparation, then retain cheap type/count/revision checks in Godot.
The validation certificate must apply to the exact published buffers; missing certificates
or malformed inputs must still be checked or rejected. Do not weaken failure-atomic staging.
**Bound:** O(produced vertices) native validation; O(changed layers) frontend shape checks,
plus unavoidable buffer upload. **Exit:** malformed/non-finite payload rejection tests,
identical valid geometry, and measured main-thread validation reduction in both modes.

**Implementation (validated 2026-09-29):** `NetworkMeshData::seal` is the only way to set the
certificate. It checks every layer, including empty layers, for whole triangles,
matching normal/UV/colour counts and finite components, then freezes the mesh in its `Arc`.
`NetworkMeshData` is not `Clone`, so shared buffers cannot change after certification.
All three publishers seal on the producing thread, never Godot's main thread:
committed chunks in `precompute_road_mesh_data` (simulation thread), and planned and
retained preview chunks on the preview worker. Retained-cache reuse shares the sealed `Arc`,
so reused chunks are not validated again. Each exported chunk dictionary carries
`road_mesh_payload_validated`. `network_tool.gd::_build_road_chunk_instance` reads it once
per chunk. A non-bool value rejects the chunk; a missing or `false` value keeps the
per-vertex GDScript scan. Certified chunks still pass the per-layer key, Variant type,
triangle-count, attribute-count and material checks. Staging remains failure-atomic.
Native validation is O(chunk vertices) per sealed chunk and runs off the main thread. Its
cost is included in worker timings, with no separate timer.

**Acceptance evidence (2026-09-29):** matched baseline (`HEAD` `358e8313`, clean worktree) and
candidate processes were interleaved: two unprofiled acceptance runs and one
`METRUM_DEBUG_PERF=1` diagnostic run each. The workload, hardware, settings, window and
command were the same as the `ROAD-30` v3 captures. All 72 case/mode captures passed
`--validate`; none dropped observations. The median of stationary replacements
in the diagnostic runs, in milliseconds:

| Fixture / mode | Road validation | Stage + install total | Worker |
| --- | --- | --- | --- |
| flat_t / full | 1.87 → 0.07 | 7.71 → 5.97 | 19.4 → 19.9 |
| flat_t / road-only | 1.94 → 0.07 | 2.56 → 0.68 | 15.9 → 15.7 |
| sloped_multi / full | 3.35 → 0.08 | 10.17 → 6.65 | 37.0 → 36.9 |
| sloped_multi / road-only | 3.43 → 0.09 | 4.28 → 0.88 | 30.0 → 29.6 |
| dense / full | 5.02 → 0.08 | 12.30 → 7.28 | 23.6 → 23.8 |
| dense / road-only | 5.05 → 0.09 | 5.98 → 1.08 | 16.1 → 16.5 |

Road validation fell to 0.07–0.11 ms in all 12 case/mode combinations. What remains is the
constant-per-layer type/count checks. CPU mesh submission and packing are unchanged
within noise. Worker medians moved by at most ±0.8 ms in either direction, which is within run-to-run
variation. The native seal therefore has no measurable worker cost in these fixtures.

The unprofiled acceptance ranges below span two processes per side, in milliseconds.
Ranges are not confidence intervals.

| Fixture / mode | Stationary p50 | Stationary p95 | Moving displayed-age p95 |
| --- | --- | --- | --- |
| `flat_t` / full | 45.2–45.4 → 43.1–43.7 | 48.2–48.8 → 46.1–46.4 | 53.3–60.2 → 53.0–58.3 |
| `sloped_t` / full | 45.3 → 43.1–43.3 | 50.1–51.9 → 47.8–48.3 | 68.7–68.8 → 68.4 |
| `flat_multi` / full | 46.7–46.9 → 43.8–44.0 | 50.4 → 47.6–48.6 | 66.2–66.6 → 53.1–53.7 |
| `sloped_multi` / full | 64.0–64.1 → 60.7 | 66.9–67.0 → 63.1 | 86.6–102.1 → 86.5–87.2 |
| `dense` / full | 49.5–49.7 → 44.8–45.2 | 51.3–52.6 → 48.0–49.0 | 69.4–70.7 → 53.1–68.5 |
| `residency_retry` / full | 93.5–93.6 → 91.9 | 97.6–97.7 → 95.3–95.6 | 65.3–65.5 → 53.1–53.2 |
| `flat_t` / road-only | 22.9–24.3 → 21.4–21.5 | 39.1–41.1 → 38.0 | 52.8–53.1 → 52.9–53.4 |
| `sloped_t` / road-only | 39.1–39.2 → 37.0–37.2 | 42.0–42.6 → 38.7–38.9 | 55.9–56.0 → 55.1–55.4 |
| `flat_multi` / road-only | 27.1–39.9 → 36.9–37.1 | 42.1–43.3 → 38.9 | 52.9–53.4 → 53.2–53.3 |
| `sloped_multi` / road-only | 41.5–41.7 → 37.8–38.0 | 45.9–46.5 → 40.3–53.7 | 69.8 → 69.3–69.8 |
| `dense` / road-only | 27.1–29.6 → 22.4–22.6 | 44.1–45.6 → 38.3–39.2 | 52.7–52.9 → 53.1–53.4 |
| `residency_retry` / road-only | 23.2–25.5 → 21.1–21.9 | 40.8–41.7 → 37.3–38.4 | 53.0–53.3 → 53.1–53.3 |

Full mode stationary medians improved by about 1.6–4.7 ms in every fixture in both processes. Road-only
improved except `flat_multi`: its baseline medians straddled a frame boundary (27.1 vs
39.9 ms), so that row is inconclusive. One candidate process's `sloped_multi` road-only p95 was
53.7 ms; the other was 40.3 ms. Frame intervals over 33.333 ms fell from 27/56 (baseline
processes) to 0/0 (candidate); none exceeded 50 ms on either side. Moving displayed age
is dominated by cadence and worker time. Some full-mode p95s dropped by about one frame. This is
a small controlled-neighbourhood result, not a city-scale or cross-hardware claim.

Artifacts: `rust/target/road31-artifacts/` holds `{baseline,candidate}-{a,b,diag}.json`,
their logs and manifests, `run_ab.py` (the reproducer), `summarize.py`/`diagnostic-compare.txt`
and `acceptance.py`/`acceptance-compare.txt`. The isolated projects are under
`rust/target/road31/{baseline,candidate}/godot`. They use the workspace or baseline-worktree
scripts and release libraries, with the `ROAD-30` override profile name. The `rust/target/preview-test-tmp`
TMPDIR/XDG profile was used, with four Rayon workers and 1280×720. The benchmarked candidate
library SHA-256 is `d3f1dea4…a2103495`; the baseline is `f3eb73fa…2f2264a`. After the runs, an
`is_multiple_of(3)` clippy fix produced the deployed `godot/bin` library
`837f5616…23fb88e4`. That fix does not change behaviour, and the Rust and Godot checks were rerun on it.
`tools/road_benchmark_report.py --baseline/--candidate` does not yet accept preview-matrix
captures; the two helper scripts compute the tables above.

**Correctness checks:** new Rust test `seal_certifies_only_well_formed_finite_buffers`
covers NaN/Inf in each attribute, a missing colour and a partial triangle. Preview parity
tests now also assert that planned and retained meshes are certified. All 71 release `network::render`
and `road_preview` tests pass; rustdoc reports no warnings; clippy reports nothing new.
The Godot `network_tool_chunk_renderer_test` covers:
- a certified payload uploads the same surface arrays as an uncertified one;
- a non-bool certificate is rejected;
- certified payloads with bad counts or wrong Variant types are rejected;
- an uncertified non-finite payload is rejected.

That suite plus the junction-preview, stream-preview, preview-metrics and benchmark-metrics
suites pass headless in the isolated profile. The full debug `cargo test --lib` shows 2014 passed and 2 failed. Both failures
are zoning tests (`partial_paint_erase_and_external_exclusion_match_full_chunk_rebuilds`,
`occupied_cells_survive_native_node_merge_regeneration_and_undo`), and both fail identically on
the clean baseline worktree. They are unrelated to this change.

#### `ROAD-32` — Reuse preview rendering resources

`road_junction_preview.gd` currently creates planned chunk nodes/meshes for each completed
request; `road_terrain_preview.gd` additionally duplicates materials and creates height
images/textures before discarding the previous batch. Reuse preview-owned nodes/materials
and compatible texture allocations, using `ImageTexture.update()` for matching dimensions,
format and mipmaps. Retain exact unchanged terrain patch products where dependency identity
permits it. Keep a bounded staging/display pair so resource updates cannot expose a partial
road/terrain replacement. Reuse of mesh buffers themselves is tracked in `ROAD-36`.

Never mutate committed terrain resources. Height textures remain necessary for terrain
shading even when vertex heights are baked. **Bound:** resources proportional to the active
preview/staging footprint, no retained cursor history; work proportional to changed products.
**Exit:** fewer resource creations in matched runs; exact restoration on cancel, mode change,
teardown and patch recycling; failed staging leaves the previous complete display intact.

**Implementation (validated 2026-09-29):** both preview halves keep one displayed set and one
detached staging set.
- `road_terrain_preview.gd` owns slots of a node, a retaining-wall child, a `ShaderMaterial`, an
  `Image` and an `ImageTexture`. Staging only takes spare slots, never displayed ones.
  - Each reused material copies the resident patch material's shader parameters, writing only
    changed values; `heightmap` and `height_is_baked` stay preview-owned. The result matches a fresh
    `duplicate()`. The committed material is never written.
  - Heights go into the slot's `Image`. A texture with matching dimensions is updated in place with
    `ImageTexture.update()`; otherwise a new texture is created and bound.
  - `commit` returns the previous display to the spares and trims them to the displayed patch count.
  - `reset` frees every slot. `road_tool.gd` calls it on cancel, mode change, main-mesh reset and
    teardown.
  - Slots are not tied to patch keys, so they survive footprint moves. A slot whose resident parent
    was freed is skipped.
- `road_junction_preview.gd` recycles replaced and failed chunk nodes and their `ArrayMesh`es as
  detached spares, trimmed to the displayed count. `network_tool.gd::_fill_road_chunk_mesh` fills a
  cleared mesh; committed chunk construction shares it.
- A road batch that fails after terrain staged returns both halves' staging slots untouched by the
  display.
- Exact terrain product retention is not implemented: every request builds fresh
  `CachedRefinedTerrainPatch` Arcs, so no cross-request product identity exists yet. `ROAD-34`
  adds display revisions for unchanged terrain products.
- Material sync is O(shader uniforms) per staged patch. Every other step is O(staged products).
  Memory is bounded by twice the displayed footprint.

A headless microbenchmark on the acceptance machine (CPU side only) measured 143 µs per material
`duplicate()` against 22 µs per parameter sync of the 123-uniform terrain shader, and 8.4 against
2.3 µs for image/texture creation against `set_data` plus `update`.

**Acceptance evidence (2026-09-29):** matched baseline (`HEAD` `f8db533c` scripts) and candidate
processes were interleaved: two unprofiled acceptance runs and one `METRUM_DEBUG_PERF=1`
diagnostic run each. Both sides loaded the same frozen release library (SHA-256 `837f5616…23fb88e4`),
because this task changes only GDScript. The workload, hardware, settings, window and command match
`ROAD-30` v3. All 72 case/mode captures passed `--validate` and none dropped observations.

Diagnostic stationary medians show every per-request terrain material, texture, image and node
creation eliminated (16/16/16/32 → 0). The eight road chunk meshes are reused instead of created,
in both modes. Values in milliseconds:

| Fixture (full mode) | Terrain resources | Stage + install total | Worker |
| --- | --- | --- | --- |
| `flat_t` | 3.60 → 0.81 | 6.04 → 2.38 | 20.0 → 19.7 |
| `sloped_t` | 3.69 → 0.87 | 6.22 → 2.55 | 27.9 → 27.5 |
| `flat_multi` | 3.86 → 1.01 | 6.72 → 2.89 | 23.2 → 23.0 |
| `sloped_multi` | 4.07 → 0.94 | 7.09 → 2.82 | 37.5 → 37.4 |
| `dense` | 4.02 → 1.03 | 7.41 → 3.20 | 23.9 → 23.9 |
| `residency_retry` | 4.53 → 0.93 | 6.42 → 2.64 | 20.3 → 20.3 |

Road-only stage-and-install totals stay at 0.72–1.15 ms on both sides. Detaching and reattaching
road nodes costs about what creating them did. During the continuous drag, full-mode median
stage-and-install fell from 7.7–10.9 to 2.5–3.2 ms per installed result. Retained reuse stayed at
0% of moving installs on both sides, because moving clip bounds invalidate the retained batch
(`ROAD-33`).

Unprofiled acceptance ranges (two processes per side, milliseconds; not confidence intervals):

| Fixture / mode | Stationary p50 | Stationary p95 | Moving displayed-age p95 |
| --- | --- | --- | --- |
| `flat_t` / full | 43.0–43.4 → 39.8 | 46.9–48.1 → 42.5 | 53.1–61.8 → 56.4–56.6 |
| `sloped_t` / full | 43.4–43.6 → 39.7–39.9 | 46.3–46.7 → 42.7–43.7 | 68.5–68.8 → 55.4–68.4 |
| `flat_multi` / full | 43.9 → 40.3–40.5 | 47.3–48.6 → 43.2 | 52.9–53.2 → 57.4–57.8 |
| `sloped_multi` / full | 60.5–60.7 → 57.2–57.4 | 63.2–64.5 → 59.9–60.0 | 86.5–86.7 → 86.5–102.0 |
| `dense` / full | 44.8–45.0 → 41.0–41.8 | 47.9–48.2 → 43.4–44.3 | 53.2–63.2 → 53.1–53.4 |
| `residency_retry` / full | 92.3–92.5 → 88.3–88.7 | 96.2–106.9 → 90.7–96.4 | 53.2–53.5 → 57.9–59.2 |
| `flat_t` / road-only | 21.3–21.6 → 20.9–21.5 | 37.1 → 36.8–38.1 | 53.2–53.3 → 52.8–55.1 |
| `sloped_t` / road-only | 37.3–37.4 → 37.1–37.3 | 38.7–38.9 → 38.5–38.7 | 53.2–53.8 → 53.1–53.2 |
| `flat_multi` / road-only | 36.7–37.1 → 36.8–37.0 | 38.0–39.2 → 38.6–38.8 | 53.2–53.5 → 55.7–56.0 |
| `sloped_multi` / road-only | 37.8–39.0 → 38.2 | 41.4–54.3 → 40.5–53.6 | 69.7–70.0 → 69.9 |
| `dense` / road-only | 22.6–23.2 → 22.6–22.9 | 38.5–39.2 → 38.4–38.9 | 53.1–56.5 → 53.1–68.6 |
| `residency_retry` / road-only | 21.5 → 22.0–22.1 | 37.6 → 38.3–38.7 | 53.0–53.6 → 53.0–53.3 |

Full-mode stationary medians improved by about 3.0–4.2 ms in every fixture and both processes. The p95s
improved by similar amounts, except `residency_retry`, whose ranges overlap. Road-only is unchanged within noise, as expected: its only reused resources are road nodes.
Frame intervals over 33.333 ms fell from 1/5 to 0/0; none exceeded 50 ms. Moving displayed age is
dominated by frame cadence and worker time. Individual p95s moved by about one frame in both
directions, so no moving-latency change is claimed. This is a small controlled-neighbourhood result,
not a city-scale or cross-hardware claim.

Artifacts: `rust/target/road32-artifacts/` holds `{baseline,candidate}-{a,b,diag}.json`, their logs
and manifests, `run_ab.py`, `summarize.py`/`diagnostic-compare.txt` and
`acceptance.py`/`acceptance-compare.txt`. Isolated projects live under
`rust/target/road32/{baseline,candidate}/godot`; the baseline scripts come from a detached `HEAD` worktree.

**Correctness checks:** `road_junction_preview_test` now also verifies:
- successive replacements alternate between two terrain slot sets and two road node sets, and the
  third replacement reuses the first set's nodes, materials and textures;
- spares stay within the displayed footprint;
- reused materials equal the resident material for every uniform except the preview heights, and the
  preview heights use a preview-owned texture;
- a malformed road half fails the paired batch without changing the displayed terrain or road
  resources;
- leaving full mode frees every terrain slot.

Existing checks still verify that committed meshes, materials, textures and payloads keep their
identities across mode change, cancel and invalidation. That suite plus the stream-preview,
chunk-renderer, preview-metrics and benchmark-metrics suites pass headless in the isolated profile.

#### `ROAD-33` — Keep unaffected road geometry resident across boundary changes

`RoadPreviewRetainedCache::reuse` already skips filtering/export/upload for matching source
generation, chunks, owners and bounds. Exact boundary changes still invalidate the retained
batch. Separate the unchanged portion of affected chunks from the short junction/approach
portions whose clipping changes. Extend existing ownership ranges, chunk caches and revision
tracking; do not introduce another spatial index or shrink global chunks as a substitute.
Use exact owner/dependency identity so native junction reuse can propagate into mesh reuse.
**Bound:** setup may partition affected source chunks once; repeated work should follow
changed local owner ranges/triangles, without rescanning unrelated resident city geometry.
**Exit:** dense-chunk cursor replays preserve unaffected mesh identities and attributes,
including markings, while reducing retained bytes and uploads; no holes, duplicated road
surfaces or existing-road height changes outside the allowed junction/approach footprint.

**Implementation (validated 2026-09-29):** `RoadJunctionPreview::retain_existing` makes two
`preview_partition` passes over each snapshotted source chunk. Both copy triangles without clipping,
so every attribute and owner range is preserved:
- the resident split: every owner except the replaced nodes and bounded source edges. This is
  sealed as `retained`.
- the approach sources: whole bounded-owner triangles, held unsealed behind an `Arc`.

`clip_approaches` then clips the approach sources to the current junction/profile bounds and seals
the result as `approach`. This is the same per-triangle operation the previous single pass used, so
resident plus approach equals the old retained output triangle for triangle.

`RoadPreviewRetainedCache` stores both halves and no longer depends on `bounds`. It still requires
the source mesh generation, chunk grid, replacement chunks, replaced nodes and bounded edges to
match. On a hit, the worker adopts the resident `Arc` without the core lock or a snapshot, and
re-clips only the approach sources inside its retained stage. The bridge sends `approach_chunks` with
every pose; `retained_chunks` is still sent only when the retained revision changes.
`road_junction_preview.gd` stages approach chunks into the per-request set, so staging stays
failure-atomic, and records `approach_chunks`.

**Bound:** the split is O(snapshot triangles), once per owner set. Repeated work is
O(bounded-owner triangles × junction bounds), and O(approach vertices) to seal and export. Unrelated
resident city geometry is never rescanned. No spatial index or chunk-size change was added.

**Acceptance evidence (2026-09-29):** matched baseline (the `ROAD-32` workspace scripts and library
`837f5616…23fb88e4`, snapshotted before this change) and candidate (library
`025e4cd3…a69da1ec`) processes were interleaved: two unprofiled acceptance runs and one
`METRUM_DEBUG_PERF=1` diagnostic run each. A second diagnostic pair ran in reversed order. The
workload, hardware, settings, window and command match `ROAD-30` v3. All 96 case/mode captures
passed `--validate` and none dropped observations.

The stationary phase repeats one pose, which already hit the old retained cache. The change shows
during the continuous drag, where every new pose moves the junction bounds. Moving-phase medians per
installed result from the first diagnostic pair:

| Fixture | Retained reuse (moves) | Packed payload, full / road-only (KB) | Retained stage, full / road-only (ms) |
| --- | --- | --- | --- |
| `flat_t` | 0% → 100% | 637 → 621 / 333 → 316 | 0.12 → 0.12 / 0.20 → 0.19 |
| `sloped_t` | 0% → 100% | 651 → 634 / 346 → 330 | 0.12 → 0.12 / 0.17 → 0.19 |
| `flat_multi` | 0% → 100% | 924 → 832 / 573 → 482 | 0.23 → 0.22 / 0.31 → 0.27 |
| `sloped_multi` | 0% → 100% | 968 → 868 / 607 → 508 | 0.25 → 0.22 / 0.30 → 0.29 |
| `dense` | 0% → 100% | 1421 → 815 / 920 → 316 | 0.25 → 0.12 / 0.35 → 0.17 |
| `residency_retry` | 0% → 100% | 638 → 620 / 333 → 316 | 0.14 → 0.12 / 0.20 → 0.18 |

- Each moving install now reuses four retained chunks (20 layers) instead of re-uploading them.
- The per-request set becomes four planned plus four approach chunks. The approach chunks replace
  the four re-uploaded retained chunks, so the uploaded chunk and layer count is unchanged (8 chunks,
  40 layers). Only the bytes shrink. Uploads fall only where an affected chunk has no bounded source
  owner.
- The dense fixture, which has the most unrelated geometry in the affected chunks, cuts moving
  payloads by 43% in full mode and 66% in road-only mode.
- Stage-and-install medians changed by less than 0.5 ms (dense road-only 1.49 → 0.96 ms).

Worker medians are unchanged within run-to-run variation. Moving-phase worker medians agree within
1.5 ms across all four diagnostic runs. Stationary `road_ms` medians vary by up to 9 ms between
processes on either side (for example baseline `flat_t` road-only: 17.0 ms and 26.5 ms), and the
reversed-order pair reproduced the spread on the baseline. The unchanged road solve dominates that
stage; the new clip is inside the retained stage, 0.09–0.30 ms.

Unprofiled acceptance ranges (two processes per side, milliseconds; not confidence intervals):

| Fixture / mode | Stationary p50 | Stationary p95 | Moving displayed-age p95 |
| --- | --- | --- | --- |
| `flat_t` / full | 40.0–40.1 → 40.0–40.1 | 42.7 → 42.6–42.7 | 53.4–54.3 → 53.3–53.4 |
| `sloped_t` / full | 40.8 → 40.1–40.3 | 55.4–58.6 → 55.8–56.0 | 68.5–68.7 → 68.9–69.8 |
| `flat_multi` / full | 40.5–40.6 → 40.2–40.4 | 43.8–59.4 → 43.2 | 53.4–68.6 → 53.4–58.1 |
| `sloped_multi` / full | 57.2–57.4 → 57.1 | 59.8–60.1 → 59.5–60.0 | 86.7–86.9 → 86.7–88.2 |
| `dense` / full | 41.3–41.9 → 40.8 | 44.4–44.7 → 44.0–57.0 | 53.3–57.7 → 57.2–57.4 |
| `residency_retry` / full | 88.7–88.8 → 88.8–89.7 | 97.8–98.1 → 101.3–105.2 | 53.4–60.2 → 53.4–69.0 |
| `flat_t` / road-only | 21.8–22.0 → 21.6–22.0 | 38.4–38.5 → 38.3 | 53.2–53.4 → 53.3 |
| `sloped_t` / road-only | 37.1 → 37.0 | 38.7 → 38.6 | 53.3–55.1 → 53.5–53.9 |
| `flat_multi` / road-only | 37.0–37.2 → 36.9–37.4 | 38.9 → 39.0–39.1 | 53.4–53.9 → 53.3 |
| `sloped_multi` / road-only | 37.7–38.5 → 38.2–38.9 | 53.7–54.0 → 53.7–54.0 | 69.9–71.6 → 69.9–70.2 |
| `dense` / road-only | 23.0–23.2 → 22.1–22.7 | 39.3–39.4 → 38.5–38.8 | 53.3–54.5 → 53.3–53.6 |
| `residency_retry` / road-only | 22.0–22.1 → 22.2–22.3 | 37.9–38.5 → 38.9–39.0 | 53.5–54.1 → 53.4–53.9 |

End-to-end latency is unchanged within frame-cadence noise. Removed retained filtering and upload
bytes cost well under a millisecond in these small neighbourhoods, so a latency gain was not
expected. The benefit is locality: repeated work and bytes now follow the edited approaches rather
than all existing geometry in the affected chunks. Frame intervals over 33.333 ms were 0/0 (baseline)
and 1/0 (candidate); none exceeded 50 ms. The residency-retry fixture's full-mode p95 is dominated by
its injected staging delay. This is a controlled-neighbourhood result, not a city-scale claim.

Artifacts: `rust/target/road33-artifacts/` holds `{baseline,candidate}-{a,b,diag,diag2}.json`, their
logs and manifests, `run_ab.py`/`run_diag2.py`, `summarize.py`/`diagnostic-compare.txt`,
`moving.py`/`moving-compare.txt` and `acceptance.py`/`acceptance-compare.txt`. Isolated projects and
the frozen baseline scripts and libraries are under `rust/target/road33/`.

**Correctness checks:** new Rust test `moving_junction_bounds_reuse_resident_split_and_clip_only_approaches`
slides a T 6 m along an existing road on flat and sloped terrain, beside an unrelated road in the same
chunks. It verifies that:
- the owners and chunks are equal while the bounds differ;
- the cache hit shares the resident `Arc` and keeps the revision;
- every mesh is certified;
- the unrelated markings stay resident;
- approach vertices are fewer than resident vertices;
- resident plus approach equals, bit for bit, a fresh single-pass partition at the new bounds in
  every chunk;
- the approach geometry actually changes.

The retained-cache test now requires reuse across moved bounds. The cold-commit parity and
neighbouring-cross tests compare planned + retained + approach. All 72 release `network::render` and
`road_preview` tests pass, rustdoc reports no warnings, and clippy reports nothing new.

The Godot junction test now:
- includes approach chunks in the existing-road height check;
- requires the T, wide T and sloped T shifted poses to keep the retained revision and send approach
  chunks.

That suite plus the stream-preview, chunk-renderer, preview-metrics and benchmark-metrics suites pass
headless in the isolated profile.

#### `ROAD-34` — Export only changed packed payloads

Extend the retained-revision protocol through
`rust/src/nodes/simulation_node/network_api/road_tool/preview.rs` and the mesh exporters to
reuse immutable Godot-ready payloads and send changed chunks/layers or terrain products only.
Current export already uses packed arrays, and GDScript caches each successfully received
result. Target real allocations/copies and unchanged products, rather than assuming repeated
idle-frame serialization or per-element Godot calls. Reuse existing owner/product revisions;
do not hash or scan the whole city to discover changes. Mutable readiness metadata remains
generation checked independently of cached geometry.
**Bound:** O(changed payload bytes + local metadata); bounded cache for active context only.
**Exit:** full and delta delivery produce identical displays, missing/outdated cache revisions
request a complete payload, and export allocations/bytes/time fall in matched replays.

**Implementation (validated 2026-09-29):** measurement first showed which products actually repeat.
In the benchmark fixtures, every pose moves the planned road and the junction bounds, so every
planned and approach chunk changes between poses. Retained chunks already travel only on a revision
change (`ROAD-33`). Of the 16 full-mode terrain patches per pose, 12 (10 in `dense`) are road-free
heightfields that the moving road never touches, yet each pose re-exported and restaged all 16.
This task revises terrain products; planned and approach chunks stay per-pose.

- `RoadPreviewTerrainRevisions` (`rust/src/nodes/sim/core/road_preview/terrain_revisions.rs`)
  lives in the preview worker. After each terrain compile it gives every product a display
  revision. A road-free patch (`input_road_loops == 0`) whose height snapshot equals the previous
  product for its key keeps that product's revision, because the bridge exports nothing else for
  it. Refined patches rebuild their buffers per request and always get a fresh revision. The map
  is replaced per compile, so it holds one footprint, not drag history. Cost: O(patches + compared
  road-free samples), on the worker. The comparison replaces the main-thread copy, upload and
  validation of the same bytes.
- `get_preview_road_surface_result(request_id, retained_revision, terrain_revisions)` takes the
  revisions Godot still holds. Each patch carries `terrain_revision`. A held revision is exported
  as `terrain_patch_metadata_dict` plus `unchanged: true`, with no height or mesh buffers. Every
  other patch is complete. Zero or unknown revisions always export complete.
- `road_terrain_preview.gd` records `key`, `revision` and resident level of detail on each filled
  slot. `revisions()` lists displayed slots and detached slots whose content survived.
  - An `unchanged` payload still passes the residency, generation and world-extent checks.
  - A displayed slot is carried into the new display untouched until commit, which resyncs its
    material and draw settings from the resident patch. A detached slot is resynced while detached.
  - Held slots are claimed before fresh payloads take spares, so no fill overwrites them.
  - Carried slots never return to the spares on failure, so a failed batch leaves the previous
    display intact.
- If no slot holds a named revision, or the resident node or level of detail changed, the slot
  forgets its revision and staging sets `missing_revision`. `road_tool.gd` then re-polls the same
  request with the current holdings, which exports those patches complete. If that result was
  superseded, the cached pose is dropped and requested again.

**Acceptance evidence (2026-09-29):** matched baseline (committed `HEAD` `ad96f3c9`: scripts and
library `6a402274…222822ec`) and candidate (library `34d83730…35666719`) processes were
interleaved: two unprofiled acceptance runs and one `METRUM_DEBUG_PERF=1` diagnostic run each. The
workload, hardware, settings, window and command match `ROAD-30` v3. All 72 captures passed
`--validate`; none dropped observations.

Diagnostic medians, full mode (stationary; moving medians agree within 0.2 ms). Times in ms:

| Fixture | Patches restaged | Terrain resources | Terrain install | Stage + install total | Packed KB |
| --- | --- | --- | --- | --- | --- |
| `flat_t` | 16 → 4 | 0.81 → 0.31 | 0.16 → 0.42 | 2.43 → 1.95 | 621 → 592 |
| `sloped_t` | 16 → 4 | 0.86 → 0.33 | 0.17 → 0.43 | 2.50 → 1.98 | 634 → 605 |
| `flat_multi` | 16 → 4 | 0.92 → 0.35 | 0.18 → 0.52 | 2.62 → 2.24 | 832 → 803 |
| `sloped_multi` | 16 → 4 | 0.93 → 0.35 | 0.17 → 0.52 | 2.68 → 2.27 | 868 → 839 |
| `dense` | 16 → 6 | 0.99 → 0.49 | 0.18 → 0.47 | 2.78 → 2.45 | 817 → 793 |
| `residency_retry` | 16 → 4 | 0.97 → 0.34 | 0.12 → 0.14 | 2.74 → 2.45 | 621 → 592 |

- Height images, texture uploads and preflight validation now follow the changed patches only. The
  texture updates per result fall from 16 to 4 (6 in `dense`).
- Commit now resyncs the carried slots' materials (O(shader uniforms) each), so terrain install
  rises by about 0.3 ms. The net main-thread stage-and-install gain is 0.3–0.5 ms.
- The skipped road-free patches are small (about 2.5 KB of heights each), so packed bytes fall by
  only 25–30 KB (3–5%). Bridge packing is unchanged within noise.
- Road-only payloads and staging are unchanged, as expected. Worker medians match except two
  candidate road-only diagnostic medians (`flat_multi`, `residency_retry` stationary, about 27 vs
  17 ms). Those are in `road_ms`, which this change does not touch; the same process-to-process
  spread was reproduced on the baseline during `ROAD-33`.

Unprofiled acceptance ranges (two processes per side, milliseconds; not confidence intervals):

| Fixture / mode | Stationary p50 | Stationary p95 | Moving displayed-age p95 |
| --- | --- | --- | --- |
| `flat_t` / full | 40.0–40.2 → 39.4–39.5 | 42.8–43.0 → 41.5–41.9 | 55.4–59.9 → 53.4–56.5 |
| `sloped_t` / full | 39.9 → 39.1–40.2 | 56.2 → 55.4–55.6 | 69.2–69.3 → 59.0–69.6 |
| `flat_multi` / full | 40.3–40.8 → 39.5–40.2 | 42.9–43.4 → 42.1–42.4 | 53.5–56.6 → 53.8–68.8 |
| `sloped_multi` / full | 56.9–57.4 → 56.4 | 59.9–60.3 → 58.9–59.0 | 86.7 → 86.7 |
| `dense` / full | 40.5–40.8 → 40.1–40.5 | 43.7–43.9 → 42.6–43.0 | 53.6–58.1 → 56.5–57.1 |
| `residency_retry` / full | 88.6–88.7 → 88.3–88.4 | 101.4–104.7 → 92.5–104.2 | 53.7–59.3 → 53.4–55.9 |
| `flat_t` / road-only | 21.3–21.9 → 21.7–22.1 | 37.8–38.5 → 38.3–38.4 | 53.4–54.0 → 53.3–53.6 |
| `sloped_t` / road-only | 37.1 → 37.1 | 38.9–53.4 → 39.7–53.4 | 53.3–53.9 → 53.4–53.8 |
| `flat_multi` / road-only | 37.0–37.1 → 37.0–37.2 | 38.9–39.0 → 38.8 | 53.5–56.0 → 53.4–54.6 |
| `sloped_multi` / road-only | 38.2–38.3 → 37.7–38.0 | 53.9–54.8 → 53.6–54.0 | 69.3–71.3 → 69.8–70.0 |
| `dense` / road-only | 22.3–22.7 → 22.0–22.4 | 38.4–38.5 → 38.3–42.0 | 53.4–53.6 → 53.3–54.5 |
| `residency_retry` / road-only | 21.5–22.0 → 21.3–21.9 | 38.7–39.3 → 37.9–38.4 | 53.3–53.4 → 53.0–53.3 |

Full-mode stationary medians fall by up to about 1 ms, consistent with the main-thread saving, but
the ranges touch or overlap in `sloped_t` (39.9 → 39.1–40.2) and `dense` (40.5–40.8 → 40.1–40.5). Full-mode p95s
fall by 0.5–1.5 ms, except `residency_retry`, whose ranges overlap and are dominated by its injected delay. Road-only is unchanged within noise. Moving displayed age is dominated by frame
cadence and worker time; individual p95s move by about one frame in both directions, so no
moving-latency change is claimed. No frame interval exceeded 33.333 ms in any run. This is a small
controlled-neighbourhood result, not a city-scale claim.

Not done here, by measurement: planned and approach chunks change with every pose in these
fixtures, and an approach clip depends on every bound whose x/z planes cross its triangles, not
only overlapping bounds. Per-chunk reuse would need content comparison for no measured gain.
Refined terrain patches could keep their revision only if the worker offered previous CDT windows
to the builder; the moving road changes every window it crosses, so that is left out.

Artifacts: `rust/target/road34-artifacts/` holds `{baseline,candidate}-{a,b,diag}.json`, their logs
and manifests, `run_ab.py`, `summarize.py`/`diagnostic-compare.txt` and
`acceptance.py`/`acceptance-compare.txt`. The isolated projects and the frozen baseline scripts,
tests and library are under `rust/target/road34/`.

**Correctness checks:**
- Rust `only_identical_plain_patches_keep_their_revision`: equal road-free snapshots in fresh Arcs keep
  their revision; changed heights, refined patches and patches dropped from the footprint get new ones;
  revisions are never reused.
- Rust `preview_terrain_revisions_keep_unchanged_plain_patches_across_poses`: over two real preview
  poses, revisions are kept exactly for the patches whose road-free snapshots are equal, at least one
  is kept, and the patch under the moving road changes.
- Godot `road_junction_preview_test` now verifies, in every terrain fixture:
  - a poll with all revisions held exports every patch as metadata with no packed buffers;
  - a mixed delta exports exactly the withheld patch complete, restages only that patch, keeps the
    other displayed slots, and displays exactly what complete delivery displays (heights, meshes,
    walls, baked flag);
  - after `reset`, a delta for a superseded result fails without a partial display and requests the
    pose again, and a delta for the current result refetches complete terrain and displays identically.

All 74 release `network::render`, `road_preview` and revision tests pass; rustdoc reports no warnings;
clippy reports nothing new. The junction, stream-preview, chunk-renderer, preview-metrics and
benchmark-metrics suites pass headless in the isolated profile. `zoning_reference_test` (78 errors)
and the windowed `plot_material_lighting_test` fail identically with the baseline and with the
`ROAD-32` library, so both failures predate this change. The verified library was deployed to
`godot/bin`.

#### `ROAD-35` — Keep the worker fed with the latest pending input

The native mailbox is already bounded, but `road_tool.gd` waits to consume the running result
before dispatching again. Integrate one replaceable latest pending input with the running
request and independently consume completed results. Preserve usable context-valid older
poses as display-only feedback; continuous input must not starve all completed previews.
Exact input/dependency checks still govern readiness and click adoption. Reuse the existing
mailbox and worker; add no request backlog, per-input thread or simulation work in GDScript.
**Bound:** O(1) pending requests and bookkeeping, independent of input-event rate.
**Exit:** measured reduction in worker idle gaps/result age, prompt exact-pose convergence
after stopping, and regressions for rapid motion, context/mode changes, stale results and
click/cancel ordering. A scheduling gain must not silently increase geometry work unboundedly.

#### `ROAD-36` — Update compatible mesh buffers in place

After the earlier measurements/reuse work, evaluate ArrayMesh vertex/attribute region updates
for exact matches in vertex layout, count and connectivity. Changed topology rebuilds only
its affected surfaces. Preserve material partitions, normals, colours, indices, bounds and
culling; use staging buffers to maintain atomic display replacement. Check the supported
Godot rendering backend/API before selecting dynamic-buffer flags or lower-level server APIs.
**Bound:** O(changed buffer bytes), bounded staging memory; no city-wide repacking.
**Exit:** stable-topology and topology-changing replays match rebuilt meshes and cancellation
behavior, with a measured upload/staging improvement. Defer this larger refactor if the
baseline shows too little compatible work to justify its complexity.

**Shared acceptance and references.** Report full-mode responsiveness first and both-mode
comparisons for every step; shared-path changes must not regress the comparison mode.
Preserve both selectable modes during this work, the new-road/junction
visibility boundary, unchanged committed geometry, generation fences and atomic commit/
rollback. Reuse the existing native preview/locality fixtures and Godot settings, stream,
junction and chunk-renderer tests, adding only task-specific regressions. For planning/cache
changes, repeat populated-map locality with the affected neighborhood fixed as background
roads, buildings, parcels and agents grow; separate setup from repeated work. For every
optimization, record matched release before/after results and correctness checks under its
stable ID before marking it done. Headless chunk tests measure CPU staging/command drain;
rendered checks are required for GPU/display claims. No engine fork is required.

API references: [ImageTexture update](https://docs.godotengine.org/en/4.7/classes/class_imagetexture.html#class-imagetexture-method-update),
[ArrayMesh buffer updates](https://docs.godotengine.org/en/4.7/classes/class_arraymesh.html),
and [Godot thread-safety constraints](https://docs.godotengine.org/en/4.7/tutorials/performance/thread_safe_apis.html).
Moving GPU resource creation to workers can introduce synchronization stalls; treat it as
a measured design choice, not an automatic consequence of moving validation into Rust.

### Junction preview without terrain reconstruction (`ROAD-28`)

**Status: done (2026-09-28).** Tracked as `refactor`, `done`, `P1` in
[`roadmap.md`](roadmap.md). The release extension is deployed. The current preview/readiness
contract uses the split below; earlier ROAD-23/24/25 measurements describe their original builds.

**Goal and agreed behavior**

Make junction previews respond faster during continuous cursor movement by removing full
terrain reconstruction from the path to displaying the next road result. Preserve the complete
planned road/junction scene: connecting spans, bends, terminals, asphalt, sidewalks, curbs,
crosswalks and lane markings. An outline or stroke ribbon alone does not satisfy this goal.

- While moving, sample the existing terrain/support and retain the authoritative road profile,
  grade and junction-height rules. Terrain-aware road geometry is still required; a flat or
  arbitrarily floating preview is not the requested result.
- Leave terrain meshes, samples, ownership and payload acknowledgments unchanged throughout
  preview, including when the pointer stops. Do not build future terrain CDT tiles, grading
  meshes, structural terrain stamps or paired terrain display payloads before a click. Retain
  only the road-side footprint/support work actually required to define correct road geometry.
- Occasional preview clipping or z-fighting against unchanged terrain is explicitly accepted.
  Do not deform the road to clear every terrain sample or retain expensive display-only
  clearance/contact/infill solves solely to make the preview perfectly meet terrain. Preserve
  correct replacement of affected road owners and restoration on cancel; review exposed old
  cutouts explicitly without reintroducing full terrain reconstruction as a visual repair.
- On click, complete terrain integration and final validation asynchronously, then publish the
  matching road and terrain together. The existing committed cutout/grading contract that
  prevents terrain overlapping the road remains unchanged. A queued click is not acceptance.

**Baseline coupling addressed**

`run_road_preview_worker` in `rust/src/nodes/sim/core/road_preview.rs` previously prepared the local
road scene, captured terrain-site inputs and called `compile_preview_terrain` before publishing
the result. The bridge in `rust/src/nodes/simulation_node/network_api/road_tool/preview.rs`
exported readiness with paired products; `road_tool.gd` staged those through the now-removed
`road_terrain_preview.gd`. `RoadEditPlan::status` required completed terrain, and the former
`AddRoad` filter in `core/thread.rs` discarded a supplied plan unless it was already complete.
Merely hiding terrain or skipping the worker call would leave unnecessary work or force a
second road solve on click. The former road-only renderer also added clearance and infill;
selecting that path unchanged would retain work the new contract no longer requires.

**Refactor sequence**

1. Capture a current release baseline for continuous junction movement and click settlement
   using the existing road benchmark/preview-stream fixtures. Attribute road/profile compilation,
   terrain preparation, bridge export, mesh construction/upload and scheduling separately.
   Historical stationary-preview timings are not a moving-preview baseline or a promised gain.
2. Separate road-product reuse eligibility from full commit readiness within the existing
   `RoadEditPlan` / `RoadTerrainPlan` ownership. A valid road candidate may lack terrain products;
   that must not make it stale, require terrain-render resources, or authorize a commit. Keep
   exact request points, lane/snap settings, source/visual/road generations and affected local
   topology dependencies. Preserve immutable published plans and single-use product adoption.
3. Publish the complete canonical road/junction scene without waiting for terrain construction.
   Reuse the existing compiled road renderer and retained-owner CPU/GPU caches. Remove paired
   terrain export/staging and unnecessary terrain-only capture from this cursor path, and avoid
   cloning both adjusted and unadjusted road buffers when only one is needed. Keep bounded
   latest-input scheduling, current snapping, generation checks, last-completed-pose display,
   invalid-result feedback and cancellation/world-reset cleanup. GDScript stays a thin bridge.
4. Resolve the actual click input and revalidate road reuse on the simulation thread. Complete
   a matching road plan with terrain products using current indexed site/terrain dependencies;
   rebuild the local road solve only when absent, stale or genuinely invalidated. Recheck live
   water, zoning/cells/parcels, fields and buildings. A changed terrain dependency must not
   blindly adopt obsolete road heights. Complete required products before opening the live
   transaction, then retain existing atomic validation, rollback, charging and acceptance
   acknowledgment. Terrain rejection leaves the committed world/render pair intact and reports
   rejection without finishing the gesture as though placement succeeded.
5. Remove superseded preview-only plumbing once callers/tests have migrated; preserve shared
   terrain compilation used by commit and other tools. Update the current preview/readiness
   contract, relevant `terrain.md` / `earthworks.md` references and dashboard on implementation.
   Do not leave parallel compatibility modes or a hidden pointer-idle terrain compile.

**Scope and performance bounds**

Extend the existing systems and spatial indices. This is not a chunk-size change, routing
rewrite, engine modification, prefab-road replacement or separate approximate junction solver.
Broader preview-to-preview topology caching and render subdivision are follow-ups only if
measurements justify them; they are not prerequisites for deferring terrain work.

Cursor work must follow affected road owners, their incident dependency closure and emitted
road vertices, with existing indexed queries and bounded retained caches. Exact input matching
remains O(input points); retained mesh reuse uses the existing owner/chunk memberships. No
full-city scan/copy, unbounded request queue or new spatial index is permitted. Terrain work
after click must follow affected patches/tiles and indexed local site candidates. Reuse buffers
and existing Rayon paths; do not add allocations to per-tick/per-agent loops. Report one-time
context snapshot costs separately from repeated preview work and from full commit costs.

**Acceptance and validation for implementation**

- Compare canonical road/junction vertices, heights, materials and markings with an independent
  cold commit for straight/curved roads, bends, T/four-way/mixed-width/close junctions, slopes,
  extensions and bridges. Both preview and final placement must use the same road authority.
- Assert that moving and stationary previews perform no deferred terrain construction or
  terrain mesh upload/replacement. Existing committed terrain resources remain identical through
  preview/cancel. Unrelated roads stay visible, and terrain/render resources restore correctly
  after invalidation, tool disposal, world replacement or a rejected click.
- Test matching-plan click reuse without another road solve; click-before-next-frame, stale
  cursor/source/topology/site revisions, missing renderer resources and terrain failure. Preserve
  atomic road/terrain publication, rejection feedback, undo and save/load behavior. Adapt the
  existing road-plan/terrain-plan Rust tests, road-tool bridge tests, `road_junction_preview_test`,
  `road_preview_stream_test` and network-renderer regressions; do not weaken commit safety tests.
- Run matched, unprofiled release motion workloads covering flats, hills, chunk boundaries and
  dense junction neighborhoods. Record input-to-new-mesh latency, displayed-pose age/update
  count, frame times and stage costs; report click-to-visible/first-idle latency separately so
  moving work to the click is explicit. Use a one-frame (~16.7 ms at 60 Hz) ordinary-preview
  target as an engineering aim, not a measured result or arbitrary-junction guarantee. Compare
  medians/tails across repeated processes with enough observations for each reported percentile.
- Repeat the existing populated-map locality check with the same neighborhood and growing remote
  roads/buildings/parcels/agents; compare identical local road and committed terrain products.
  Record workload/build hashes, worker settings, commands and artifacts here. Headless CPU
  measurements do not establish GPU upload or presentation latency; include rendered verification
  of the full moving junction and final no-overlap result. Acceptance requires demonstrated
  moving-preview improvement, preserved correctness and a documented click-latency tradeoff.

**Implementation progress**

- [x] Capture current release motion/commit baselines and build identities.
- [x] Separate reusable road geometry from complete terrain/commit readiness.
- [x] Publish canonical junction meshes without terrain preparation or display repair.
- [x] Complete terrain on click with reuse, dependency checks and atomic rejection/commit.
- [x] Migrate targeted regressions and remove superseded preview plumbing.
- [x] Verify correctness, rendered behavior, matched performance and populated-map locality.
- [x] Update shipped contracts and record acceptance evidence.

**Preview locality correction — verified, 2026-09-28**

The initial renderer treated the compiler dependency neighborhood as the visible edit.
It replaced full connected owners and lifted their meshes while terrain cutouts stayed fixed.
The display now retains existing owners outside the bounded junction/profile footprint and
preserves neighboring junctions. Retained-cache keys include the footprint, so moving a
connection cannot reuse an obsolete cut boundary. No uniform preview lift is applied.

Local compilation offers complete committed span/node artifacts through existing exact-input
replay, plus one previous successful cursor certificate. Cache entries pin road invalidation,
authored terrain and visual terrain revisions; fresh sections, mouth geometry and visibility
checks still gate reuse. The cache replaces its last bounded entry rather than retaining drag
history. Capture visits indexed local IDs; candidate matching is bounded by the local compile
neighborhood. Display partitioning costs O(local triangles × local junction bounds), with
reused scratch buffers and no allocation per triangle. Terrain reconstruction remains click-only.

Fresh correction verification: 2,012 release Rust tests pass (82 ignored), including pointer
identity reuse of the neighboring crossroads, seven-layer retained geometry, clip-union area
conservation and cold-commit geometry/provenance parity. The 14-case headless and rendered
junction suites pass, including flat/sloped neighboring-cross fixtures and exact cancel
restoration. Preview instances have zero presentation lift. Reviewed captures preserve the
existing crossroads; the new stroke can still intersect unchanged terrain. The sloped fixture
has 13 preview-only background pixels within the local edit, so this is not terrain integration.

Matched unprofiled native release runs on an i9-12900K, four Rayon workers, `METRUM_DEBUG=0`:
`benchmark_neighboring_cross_preview` performs 32 cursor positions per flat/sloped fixture,
excluding the first eight warm-up updates and all fixture construction. It measures local
planning plus planned mesh export, not frontend presentation or retained GPU upload. Median /
p95 milliseconds before → after: flat **20.154 / 22.897 → 17.112 / 18.324**; sloped
**34.390 / 36.965 → 24.559 / 28.147**. This is one matched pair, not a replay of the user's exact
scene or a universal frame-budget guarantee. Changing the new junction still requires solving it.
The supplied diagnostic trace identified repeated neighboring-junction work (roughly 110 ms
in one slow compile); it was used for diagnosis, not acceptance timing.

The existing populated-map benchmark reports matching road/terrain products at 0 / 1,000 /
10,000 / 100,000 background buildings, up to 600,024 agents and 391 remote roads. Repeated
identical-input worker medians are **0.971 / 0.943 / 1.032 / 1.031 ms**; separate one-time context
snapshot costs are **0.006 / 0.012 / 0.046 / 0.381 ms**. These warm-cache results do not represent
new-junction motion. Click terrain medians remain **5.65 / 5.46 / 5.90 / 6.01 ms**.

Commands: `RAYON_NUM_THREADS=4 METRUM_DEBUG=0 cargo test --offline --release --manifest-path
rust/Cargo.toml --lib benchmark_neighboring_cross_preview -- --ignored --nocapture
--test-threads=1`, and the same command with `populated_road_plan_scaling`. The full suite uses
`-- --test-threads=4` and workspace-local `TMPDIR`; an initial `/tmp` run hit SQLite I/O errors.
Artifacts: `/tmp/metrum-preview-correction/{before,after,locality,full-tests-final,godot-junction-final,rendered}.log`
and `capture_{neighbor_cross,sloped_neighbor_cross}*.png`. Final test executable SHA-256:
`1370ee7cf1b5cceeb8da0174b19f4dfcc348a7a094a218982eb640e7449bd452`.
Baseline production was the preceding ROAD-28 implementation; the same benchmark body ran
before and after the correction. Terrain clearance is still deferred to placement.

The continuous-motion, chunk-renderer and zoning-road-tool headless suites also pass. Release
build and Rustdoc are warning-free. The deployed extension SHA-256 is
`7861fe38a4bf8d2725583d8346a4586274ad23a0c27d2ce0a32ce29920db6a14`.

**Initial acceptance evidence — 2026-09-28**

`RoadEditPlan` shares immutable road products through an Arc; a matching click adds its own
terrain products and retains single-use adoption. Structural stamp/cutout preparation is also
removed from the topology preview path. The worker publishes one canonical mesh set, and the
initial Godot helper applied a fixed 2 cm presentation offset (removed by the locality correction above). No terrain clearance queries or
vertex repair were performed for that offset. Complete commit readiness still requires terrain.

All 2,010 active release Rust library tests pass (81 ignored), including shared-road reuse,
road-only commit rejection, visual-only staleness, curved cold-commit mesh parity and existing
rollback/undo/save regressions. All four headless suites pass: `road_junction_preview_test`,
`road_preview_stream_test`, `network_tool_chunk_renderer_test` and `zoning_road_tool_test`.
Rustdoc reports no warnings; all eight benchmark-report tests pass. A preceding full native
run encountered a SQLite `disk I/O error` in a zoning save test; the final full rerun passes.

The 12-case junction suite and eight-case moving suite also pass with Wayland, Vulkan Forward+
and the RX 7900 XTX. Reviewed captures show complete flat junctions, accepted slope clipping
and exposed old cutouts during bends. Cancel restores exact source coverage. The final flat
connected fixtures have zero sky pixels inside their committed cutouts; the isolated/sloped
fixtures retain 1–8 diagnostic edge pixels, so this is not a universal watertightness claim.
The final explicit-Wayland junction run has no script/runtime errors or ObjectDB leak warning;
verbose output includes hardware RGB8-to-RGBA8 image conversion warnings. Moving captures are
visual checks only, excluded from acceptance timing; the offset chunk fixture is partly outside
the fixed capture camera and is covered numerically by the full motion run.

Matched acceptance used three separate unprofiled processes per build, ordered A/B, B/A, A/B,
with no concurrent builds or profiling. Each motion process sends 360 inputs per fixture at
60 Hz. Both builds use the identical extended eight-case harness. Values below are medians of
the three process p50s/p95s and update counts; every reported latency p95 has at least 100
observations per process. Latency measures input-to-new-mesh submission, not GPU presentation.

| Moving fixture | Latency p50, before → after (ms) | Latency p95 (ms) | Updates / 360 inputs | Displayed-pose age p50 (ms) |
|---|---:|---:|---:|---:|
| T | 36.03 → 19.65 | 37.89 → 37.23 | 178 → 271 | 50.02 → 21.08 |
| Four-way | 53.54 → 36.42 | 55.68 → 52.78 | 121 → 174 | 66.91 → 50.18 |
| Wide T | 36.29 → 35.68 | 38.34 → 37.08 | 175 → 179 | 50.10 → 37.75 |
| Bend | 35.26 → 18.14 | 36.37 → 19.21 | 225 → 347 | 35.32 → 18.15 |
| Wide bend | 35.54 → 18.32 | 36.78 → 35.49 | 190 → 313 | 35.92 → 18.41 |
| Sloped T | 36.22 → 35.61 | 38.18 → 37.12 | 177 → 179 | 50.04 → 37.26 |
| Chunk-boundary T | 36.12 → 19.75 | 38.38 → 37.00 | 178 → 289 | 50.05 → 19.97 |
| Dense T neighborhood | 37.34 → 36.68 | 40.19 → 39.13 | 176 → 179 | 50.10 → 39.83 |

The improvement is strongest for ordinary T/four-way junctions and bends. Wide, sloped and
dense cases do not demonstrate a comparable median latency gain. Most tails remain near the
baseline, and the 16.7 ms engineering aim is not reached. More frequent completed poses also
increase frontend work in some cases: mean tool processing per frame rises 1.61 → 2.15 ms
for T and 1.63 → 2.32 ms at the shifted boundary; it falls 1.68 → 1.37 ms for wide T.
There is no claim of a universal frame-time reduction.

Each interaction process uses one warmup and three measured repetitions per mode (nine
measured clicks per mode/build across processes). The following are medians of process
medians; this sample count does not support tail estimates.

| T click mode                    | Generation ready, before → after (ms) | Atomic render acknowledgment (ms) | First idle (ms) | Five-idle-frame settlement (ms) |
| ---------------------------------| --------------------------------------:| ----------------------------------:| ----------------:| --------------------------------:|
| Stationary completed preview    | 6.73 → 10.21                          | 27.56 → 32.54                     | 34.34 → 36.36   | 62.10 → 62.13                   |
| After pointer trace             | 6.80 → 9.58                           | 28.71 → 27.46                     | 34.42 → 34.38   | 62.01 → 62.00                   |
| Immediate, no completed preview | 40.09 → 38.79                         | 52.14 → 52.56                     | 59.09 → 59.51   | 86.75 → 87.07                   |

Every measured candidate stationary/trace click reused its road plan; all immediate clicks
used the cold local road compiler. Reuse eligibility takes 0.016–0.017 ms, followed by
5.34–5.41 ms of deferred terrain preparation; an immediate click spends 13.58 ms on road
planning and 5.04 ms on terrain. Stationary core work rises 3.39 → 8.05 ms, and its render
acknowledgment is about 5 ms later. That is the explicit cost moved from hover to click.
First-idle also includes foreground water/border/residency work; acknowledgment is CPU-side,
not a presentation timestamp. Inclusive command phases must not be summed.

Three separate release locality runs hold the same four-site neighborhood fixed and grow
remote buildings/parcels/agents/roads. Each size uses three warmups and 100 observations.
All seven local road mesh layers, including vertex/normal/UV/color attributes, and all terrain
products remain identical. Medians of process medians:

| Remote buildings | Road-only worker p50 (ms) | Deferred terrain p50 (ms) | Cold complete plan p50 (ms) | Context snapshot once (ms) |
|---|---:|---:|---:|---:|
| 0 | 14.08 | 5.60 | 19.41 | 0.004 |
| 1,000 | 13.95 | 5.51 | 19.32 | 0.013 |
| 10,000 | 14.00 | 5.64 | 19.57 | 0.048 |
| 100,000 | 14.08 | 5.92 | 19.83 | 0.404 |

The largest fixture has 600,024 agents, 100,004 parcels and 391 remote roads. Repeated planning
follows the affected neighborhood; the separately reported context snapshot grows with resident
state. This does not establish locality for unchanged full-commit routing or city simulation.

Separate diagnostic `perf` captures (`cpu-clock:u`, 99 Hz, DWARF call stacks, 96 motion inputs)
show terrain-CDT/contact splitting samples in the baseline. Optimized/inlined symbols and
unresolved Godot frames limit attribution, and the capture includes fixture setup. These profiles
are not acceptance timing or evidence for precise per-stage percentages. The click phase clocks
above directly quantify the deferred terrain and reused/cold road work.

Build/workload identity: i9-12900K, CPU affinity 0–23, `RAYON_NUM_THREADS=4`, `METRUM_DEBUG=0`,
Rust 1.98.1 and Godot `4.7.2.stable.arch_linux.ed1daf0bf`. Baseline runtime is git
`7369654f6e7cc04d8b5b1c09663ecbc776966cfb`; candidate native diff SHA-256 is
`0d29b6a7de18d5b5d4388c56236d151aecfc41272d1d15e8d04ce88e7faa8541`.
Debug-stripped baseline/candidate library hashes begin `d29aa6117e00202e` /
`e2d654c2fc5629d6f`; identical motion harness hash begins `1973f82ac2d06eeb`.
Full hashes and script identities are in `/tmp/metrum-road28/matched-identity.json`.
The deployed unstripped release has SHA-256
`236527ffcf605aa402c38dd0c2e2a867780568e5458074c90d26ec86462a234e` and matches the
measured candidate's ELF build ID `cd1d8cdc538a57e857e1a02ccb343e874b2e8282`.

Reproduction commands (run performance work without competing jobs):

```bash
env RAYON_NUM_THREADS=4 METRUM_DEBUG=0 cargo test --offline --release --manifest-path rust/Cargo.toml --lib -- --test-threads=4
bash /tmp/metrum-road28/run-matched.sh
env RAYON_NUM_THREADS=4 METRUM_DEBUG=0 cargo test --offline --release --manifest-path rust/Cargo.toml --lib nodes::sim::core::tests::road_plan_scaling::populated_road_plan_scaling -- --exact --ignored --nocapture --test-threads=1
env XDG_DATA_HOME=/tmp/metrum-road28/profile-data XDG_CONFIG_HOME=/tmp/metrum-road28/profile-config RAYON_NUM_THREADS=4 METRUM_DEBUG=0 METRUM_JUNCTION_PREVIEW_CAPTURE=/tmp/metrum-road28/final godot --display-driver wayland --path /tmp/metrum-road28/candidate/godot --script res://tests/road_junction_preview_test.gd
```

Artifacts under `/tmp/metrum-road28/`: `matched-{baseline,candidate}-{1,2,3}-{motion,interaction}.json`
and corresponding logs, `locality-{1,2,3}.log`, `comparison.json`/`comparison.txt`,
`full-rust-tests.log`, `rustdoc.log`, `report-tests.log`, `rendered-junction-final.log`,
`rendered-motion.log`, `final_*.png`, `motion_*_motion_*.png` and `profile-*.data`/reports.
All six interaction JSON captures pass the existing benchmark reporter's validation. Projects
and user-data profiles are isolated under that directory; user saves and installed assets are
not benchmark inputs. The earlier five-case baseline captures are diagnostic history and are
not mixed into the eight-case comparison above.

### Road guide removal (`ROAD-27`)

All road guiding lines, their guide-specific snapping, the G shortcut, global guide cache,
render meshes and bridge APIs are removed. This includes endpoint continuations and offset
guide grids. There is no replacement guide system.

The zoning-grid checkbox still controls cell-aligned snapping, including free angles outside
its capture range. Existing road/node connection snapping and placement distance/angle
measurements remain. Cursor queries use the existing spatial indices; removal adds no new
hot-path work or spatial structures. Benchmark output schema 4 omits guide readiness/count
fields while retaining road output and generation checks.

Fresh validation: all 2,010 active release Rust tests pass (81 ignored), along with Godot's
`network_tool_chunk_renderer_test` and `road_preview_stream_test`, and eight benchmark-report
tests. The cursor regression checks that former continuation guides no longer attract the
cursor while zoning-grid, free-angle and real-road connection behavior remain intact.
Changed Rust files pass formatting checks; the repository-wide formatting check still reports
unrelated formatting in existing vegetation/zoning files. The release extension is atomically
deployed; logs and executable/library hashes are in `rust/target/no-road-guides/`.

Matched unprofiled release runs compare the preceding contextual-guide executable with guide
removal on Rust 1.98.1, i9-12900K CPU 0, `RAYON_NUM_THREADS=1 METRUM_DEBUG=0`.
Run `bash rust/target/no-road-guides/benchmarks.sh` for the existing
`benchmark_zoning_snap_locality --ignored --nocapture --test-threads=1` fixture.
At 0 / 1,000 / 10,000 background roads, median query times are
0.069 → 0.061 / 0.077 → 0.071 / 0.080 → 0.083 µs; all checksums match
`(266240, 532480)`. Setup is excluded. These single trials support retained query locality,
not a speedup claim; guide generation and upload no longer execute.

### Empty generated-carrier height lookup (`ROAD-26`)

The 2026-09-28 crash dump `logs/metrum-crash-20260928-191757.369-pid1460594.log`
identifies an empty height candidate list in generated-carrier materialization. The expression
`(heights.len() == 1).then_some(heights[0])` evaluates its indexed argument even when the
condition is false. A point outside the generated height triangles therefore panics instead
of returning the intended `None`, affecting both preview compilation and commit validation.

The lookup now returns a height only when exactly one distinct millimetre height remains.
Empty or ambiguous support stays unresolved for the existing height validation pipeline;
materialization must not invent a height or widen geometric support. The final selection is
O(1), with no additional allocations, spatial queries, or change to triangulation/deduplication.
The targeted regression reproduces the original panic before the fix, then verifies missing
support, shared-triangle-edge deduplication and subsequent valid lookups. Fresh release
verification passes all 2,014 Rust tests (82 ignored), `network_tool_chunk_renderer_test`
and `road_preview_stream_test`, including continuous movement through five junction/bend
fixtures. The extension is deployed. This reproduces the failing lookup, not the exact road
layout from the crash dump.

Fresh matched, unprofiled release locality runs on the i9-12900K (Rust 1.98.1,
CPU 0, `RAYON_NUM_THREADS=1`, `METRUM_DEBUG=0`) use
`taskset -c 0 <release-test-executable> populated_cell_road_plan_scaling --ignored --nocapture --test-threads=1`.
Each run measures 100 samples per size with fixed affected geometry, increasing background
buildings/painted cells from 0 to 100,000, roads from 0 to 391, parcels from 4 to 100,004 and
agents from 24 to 600,024. Local products match at every size. Worker p50 before → after is
37.510 → 35.081, 35.919 → 35.678, 35.491 → 35.880 and 35.957 → 36.173 ms at
0 / 1,000 / 10,000 / 100,000 background buildings. Largest-case compile p50 is
35.061 → 35.198 ms; readiness is 0.150 → 0.145 ms. One-time snapshot setup is measured
separately (0.369 → 0.366 ms at the largest size). These runs retain local planning cost;
they do not establish a speedup. Logs, reproduction, build/deployment SHA-256 identities and
continuous-preview metrics are in `rust/target/road-preview-empty-height/` (`before.sha256`,
`after.sha256`, `deployed.sha256`, `populated-before.log`, `populated-after.log`).

### Third-road boundary and placement feedback (`ROAD-25`)

The 2026-09-13 `road.log` repeatedly rejected the third road at the existing sidewalk boundary
`(2834.492, -8658.672) → (2835.014, -8662.223)`. Node earthwork splitting discarded distinct
submillimetre segments using a metric length threshold. The resulting source loop had gaps even
though its polygon still connected the vertices; later terrain union could not recover ownership.
Splitting now discards only identical canonical XZ endpoints. Every distinct arrangement edge
keeps its solved height and source; no tolerance expansion, invented owner or clipping bypass is used.
The predicate is O(1) per existing local boundary subsegment; geometry/source work remains bounded
by the affected node and terrain patches.

An exact scene can carry an empty `surface_vertices` field because its geometry lives in chunks.
On rejection, Godot now builds the fallback ribbon when that array is empty and preserves the error
label if mesh upload fails. A queued click keeps its gesture until the simulation acknowledges
acceptance or rollback. The bridge retains one bounded completion receiver; polling is O(1),
nonblocking and does not acquire `SimCore`. Duplicate clicks wait, rejected strokes remain editable,
and completions cannot clear another gesture. Logs distinguish `queued` from the authoritative
`road_commit_result`; the replay importer understands both current and historical captures.
`--debug road` enables logging; the existing `METRUM_DEBUG_SURFACE=1` separately enables the cyan
surface overlay.

Regression coverage includes the exact logged boundary location, a nearby third-junction hole,
micrometre-to-submillimetre owned edges, empty exact-preview fallback rendering, and queued,
accepted, rejected and superseded placement feedback. Fresh verification and release locality
measurements on 2026-09-13:

- `RAYON_NUM_THREADS=8 METRUM_DEBUG=0 cargo test --offline --lib`: 1,767 passed, 60 ignored.
- Headless `network_tool_chunk_renderer_test`, `road_junction_preview_test` and
  `road_preview_stream_test`: all pass against the rebuilt release library. The gameplay benchmark
  script passes Godot's parser check, the replay importer passes eight Python tests, and `run.sh`
  passes `bash -n`.
- A deterministic 144-layout sweep reconstructs the logged junction on the checked-in Kuopio
  source world, varying native f32 trunk coordinates and branch length. All 144 three-road layouts
  now preview and commit; the baseline rejected 17. Ten baseline failures identify the exact
  boundary coordinates from `road.log`. This is a reconstruction, not a complete recorded mouse replay.
- Three alternating unprofiled release process pairs use eight Rayon workers pinned to CPUs
  `0,2,4,6,8,10,12,14`, 100 observations per size and the existing
  `nodes::sim::core::tests::road_plan_scaling::populated_paved_road_plan_scaling` fixture. The local
  roads/sites stay fixed while background buildings, parcels, agents and roads grow. All six runs
  retain identical local products at every size. No other builds, tests or replays overlapped these
  acceptance timings.

| Background buildings | Worker p50 before → after (ms) |
| --- | --- |
| 0 | 20.894 → 20.694 |
| 1,000 | 20.609 → 20.553 |
| 10,000 | 20.659 → 20.630 |
| 100,000 | 20.658 → 20.862 |

Values are medians of the three process medians. The largest fixture contains 600,024 agents,
100,004 parcels and 391 background roads. Compile p50 is 19.975 → 20.212 ms; readiness remains
below 0.018 ms. Separately measured one-time snapshot cost is 0.006 → 0.007 ms with no background
buildings and 3.403 → 3.439 ms at 100,000. The approximately 1% largest-case worker difference
does not change locality; these measurements do not establish whole-city frame rate.

Command: `RAYON_NUM_THREADS=8 METRUM_DEBUG=0 taskset -c 0,2,4,6,8,10,12,14 BINARY --exact nodes::sim::core::tests::road_plan_scaling::populated_paved_road_plan_scaling --ignored --nocapture --test-threads=1`.
Release test executable SHA-256 before: `088c53a52ba53b229113388d75f8a1b5a5b83fec0e4687d58d1233bf0bd100f6`;
after: `42f30b3027712d14291a762f6e8410bfddce53430e91efce86b61e2c037443b6`.
Deployed library SHA-256: `bb5e27f6da196ecf4613b492da28f8281063ca4f10d65c7983217a34557e0151`.
Artifacts: `/tmp/metrum-road-fix/` contains `locality-{before,after}-{1,2,3}.log`,
`locality-summary.json`, correctness/build logs, `sweep-all.log` (baseline), `sweep-after.log`,
and the original failing regression trace `rust-repro.log`.

### Node Edit Audit Measurements (2026-09-12)

All four initial regressions fail on the baseline: duplicate road bounds after a merge, an
unchanged physical endpoint after movement, a double-translated self-loop and the wrong canonical
parent after merging through an alias. Additional assertions cover retired-node lookup, retained
independent height samples and refreshed grade-dependent routing cost. The unused
`remove_node_and_merge_edges` API and its sole rejection test are removed, together with a second
caller-side length pass, discarded split length, duplicate end-node binding and unnecessary
geometry clone.

Three alternating unprofiled CPU-0 / one-worker release pairs hold one incident road fixed while
adding disconnected background roads. Each process takes nine samples after one warmup; fresh
whole-graph clones, graph setup, result destruction and verification are outside mutation timing.
The merge benchmark uses coincident endpoints so both builds have valid matching geometry/index
products; the noncoincident correctness regression independently catches the stale-entry bug.
Exact local geometry fingerprints and live-edge query counts match across versions/backgrounds.

| Background roads | Merge before / after (µs) | Move before / after (µs) |
| --- | --- | --- |
| 0 | 0.124 / 0.167 | 0.150 / 0.162 |
| 1,000 | 1.488 / 0.543 | 0.453 / 0.509 |
| 10,000 | 19.341 / 1.852 | 1.326 / 1.957 |
| 100,000 | 420.238 / 4.956 | 4.940 / 5.044 |

The global O(E) merge scan is removed. Remaining index cost is logarithmic in city size; ordering
is O(K log K) in the two local incident sets, and profile/cost work follows their point counts.
The additional local correctness work is visible in the tiny cases, including a 0.631 µs move
increase at 10,000 background roads. These figures exclude graph cloning and the later coordinated
edit publication; no whole-frame speedup is inferred.

The same final processes separately measure the corrected unequal-profile path. These are
**after-only** measurements, because the baseline does not support those inputs correctly. Each
case uses 21 samples after one warmup, with edge cloning, result destruction and height/position
checks outside timing. Source profiles alternate even/odd horizontal support positions; every
original height sample remains represented.

| Control / physical supports | Shared supports | Alignment + deformation (µs) |
| --- | --- | --- |
| 16 / 17 | 31 | 0.435 |
| 256 / 257 | 511 | 5.556 |
| 4,096 / 4,097 | 8,191 | 83.470 |

Three separate eight-worker pairs run the populated paved-road locality fixture. Four local
occupied sites remain fixed while buildings, parcels, agents and roads grow. Each version retains
identical local products across all background sizes; fixture setup is excluded and the one-time
planning snapshot remains separately reported.

| Remote buildings | Plan before / after (ms) | Worker before / after (ms) | Snapshot before / after (ms) |
| --- | --- | --- | --- |
| 0 | 19.930 / 19.806 | 21.021 / 20.658 | 0.006 / 0.007 |
| 1,000 | 20.189 / 19.838 | 20.809 / 20.750 | 0.059 / 0.060 |
| 10,000 | 20.172 / 19.711 | 20.936 / 20.694 | 0.357 / 0.349 |
| 100,000 | 20.198 / 19.766 | 21.024 / 20.608 | 3.431 / 3.372 |

The largest planning fixture contains 391 background roads and 600,024 total agents. Planning
cost remains comparable and local; one-time snapshots still scale with the stored city. This is
not acceptance of the remaining global agent invalidation or full pathing-rebuild costs.

Reproduce with `python3 /tmp/metrum-full-audit/match_node_edits.py`. The exact ignored entries are
`simulation::network::topology::node_edit_tests::benchmark_node_edit_locality` and
`nodes::sim::core::tests::road_plan_scaling::populated_paved_road_plan_scaling`, using
`--exact --ignored --nocapture --test-threads=1`, `METRUM_DEBUG=0`, and offline release builds.
Mutations/profiles use `taskset -c 0`, `RAYON_NUM_THREADS=1`; planning uses
`taskset -c 0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`. Results, setup/clone timings, source snapshots
and the seven-file diff are under `/tmp/metrum-full-audit/node-edits-*`. Rust is 1.98.1; executable
SHA-256 values are `cc95a605c79771c82391601ef4a7b33db3bc0bf3853cb4d4cd6b8e15c3e49130` before and
`b4746664fd713fea768dc93d62097c04e8f002df08fc63938d9097850ffb58fc` after. The initial capture hit
quota; its incomplete executable/empty archive were removed, old artifacts were losslessly
archived and verified, and capture was retried atomically. No measurements ran from that failed
capture or concurrently with archival, compilation, tests or source changes.

### Node Query Audit Measurements (2026-09-12)

`AUDIT-01-A18` replaces the editor's complete node scan with the shared indexed search and removes
unused query/projection code. The baseline selects node 1 instead of equally close node 0 in the
spatial query (`node-query-before-regression.log`). Regressions retain exclusive radii, XZ versus
3D distance, live/alias filtering, lowest-ID ties across cells and reversed retained edges, and
rectangle results against an independent point filter, including global bounds.

Five alternating matched unprofiled release process pairs use CPU 0 and one Rayon worker. A fixed
24 m local road is surrounded by 0/1,000/10,000/100,000 disconnected remote roads (2 through
200,002 total nodes). Each operation runs one warmup and nine samples of 64 queries, cycling three
local hits and an empty location at radius 5 m. Setup is timed separately; result checking is
outside the query timer. All outputs match the expected `[0, 1, 0, -1]` targets at every size.
These are warm isolated query measurements, excluding bridge locks, rendering and a whole frame.

| Background roads | Editor query µs, before → after | 3D query µs, before → after | Cursor acquisition µs, before → after |
| ---: | ---: | ---: | ---: |
| 0 | 0.006750 → 0.017953 | 0.035969 → 0.018266 | 0.045250 → 0.020484 |
| 1,000 | 4.846547 → 0.042734 | 0.035109 → 0.042687 | 0.044422 → 0.060313 |
| 10,000 | 99.944250 → 0.043125 | 0.034250 → 0.042875 | 0.043578 → 0.060750 |
| 100,000 | 2318.796000 → 0.043391 | 0.034625 → 0.043281 | 0.043672 → 0.061203 |

The editor no longer scales with unrelated nodes. With only two nodes its fixed index overhead is
about 11 ns; on larger fixtures the already-local 3D and cursor searches add about 9 and 18 ns.
The latter include exact bounds checks and stable node ties; no universal speedup is claimed.
Node candidates allocate nothing. The existing edge-candidate buffer in cursor acquisition is
outside this cleanup and remains an open audit item.

Three separate populated-road planning pairs use eight physical P-cores and eight Rayon workers.
They hold four local building sites fixed while increasing background buildings, roads and agents;
the largest case contains 391 remote roads and 600,024 agents. Every run preserves its local
products. Snapshot creation remains separate from repeated planning:

| Background buildings | Compile ms, before → after | Worker ms, before → after | Snapshot ms, before → after |
| ---: | ---: | ---: | ---: |
| 0 | 19.821 → 20.108 | 20.989 → 21.034 | 0.007 → 0.007 |
| 1,000 | 19.835 → 20.051 | 20.616 → 20.749 | 0.058 → 0.058 |
| 10,000 | 19.913 → 19.851 | 20.789 → 20.805 | 0.339 → 0.350 |
| 100,000 | 20.255 → 20.055 | 20.972 → 21.012 | 3.358 → 3.369 |

Commands and identities:

- Query: `RAYON_NUM_THREADS=1 METRUM_DEBUG=0 taskset -c 0 BINARY --exact nodes::sim::query::tests::benchmark_editor_node_query_locality --ignored --nocapture --test-threads=1`.
- Planning: `RAYON_NUM_THREADS=8 METRUM_DEBUG=0 taskset -c 0,2,4,6,8,10,12,14 BINARY --exact nodes::sim::core::tests::road_plan_scaling::populated_paved_road_plan_scaling --ignored --nocapture --test-threads=1`.
- Before release test executable SHA-256: `14f7b87aaf06f9e046843fb731164e6c3d3ac63d610d7ba6cf88d537ce250c96`.
- After: `b4cc03a9f9290b26156280c1157a66705c6c6038110994184dcb5230c2688f9a`.
- Rust: `rustc 1.98.1 (48a229cea 2026-09-01) (Arch Linux rust 1:1.98.1-1)`.
- Artifacts: `/tmp/metrum-full-audit/node-query-{before,after}-identity.json`, `node-query-bench.json`, `node-query-summary.json`, `node-query-planning-bench.json`, `node-query-planning-summary.json`, raw per-pair logs and `node-query.diff`.

Final source/test compilation is warning-free. All 1,763 release tests pass (58 ignored),
`node-query-final-tests.log`. No builds, tests or source changes overlapped the accepted benchmark
runs. Initial test compilation caught a missed UI-sentinel assertion during the Option-return
conversion; it was corrected before the final suite and measurements.

### Edge Hover Audit Measurements (2026-09-12)

The baseline regression for `AUDIT-01-A19` selects deleted edge 0 instead of live edge 1
(`edge-query-before-tests.log`). The second test preserves inclusive radius, lowest-ID ties,
height-independent selection, repeated profile points, endpoint projection and empty-space results.
Both pass with the corrected query, including reversed R-tree insertion history and deletion of
every local road. All 1,765 release tests pass (59 ignored), `edge-query-after-tests.log`.

To test the query without constructing a whole `SimCore`, the baseline moves its original full
scan and projection arithmetic into the simulation interaction module. Both measured versions
use that same isolated interface, with unchanged graph fixtures and the bridge's radius policy.
Five alternating unprofiled release process pairs use CPU 0 and one Rayon worker. One fixed 28 m
local road and 0/1,000/10,000/100,000 remote roads each contain eight profile points. One warmup
and nine samples of 64 queries cycle three local hits and empty space at radius 5 m. Setup is
reported separately and assertions run after each timed sample. Every output matches
`[0, 0, 0, -1]` at every size.

| Background roads | Query µs, before → after |
| ---: | ---: |
| 0 | 0.019000 → 0.019484 |
| 1,000 | 18.299766 → 0.028547 |
| 10,000 | 191.978563 → 0.034500 |
| 100,000 | 3459.187406 → 0.038906 |

These warm kernel measurements establish removal of the unrelated-road scan, not a whole-frame
speedup. Bridge locks, input/rendering, graph construction and retired API registration are not
part of the query timer. Cost follows visited R-tree entries plus candidate profile segments;
dense overlapping bounds still require examining those candidates. No new spatial index or
per-query heap storage is introduced. This batch changes selection and removes unused APIs;
it does not change local road-edit planning, whose separate A18 measurements remain historical
for their recorded executable.

Command: `RAYON_NUM_THREADS=1 METRUM_DEBUG=0 taskset -c 0 BINARY --exact simulation::network::interaction::tests::benchmark_editor_edge_query_locality --ignored --nocapture --test-threads=1`.

- Before release test executable SHA-256: `a8d62cd8c19c3d6af01de5c71b327c80a08ef8ddbac92baf0ce02d749142a489`.
- After: `d02c0c5fbe9354eb7a8e7d86ee40822d808aa83e89e607fdcf0667f261351b52`.
- Rust: `rustc 1.98.1 (48a229cea 2026-09-01) (Arch Linux rust 1:1.98.1-1)`.
- Artifacts: `/tmp/metrum-full-audit/edge-query-{before,after}-identity.json`, `edge-query-bench.json`, `edge-query-summary.json`, raw per-pair logs and `edge-query.diff`. Identities include the corrected GDScript API header alongside Rust sources.

No builds, tests or source changes overlapped the accepted measurements. Test/benchmark compilation
is warning-free, `edge-query-after-check.log`.

### Retained Node-Grid Capacity Measurements (2026-09-12)

`AUDIT-01-A20` corrects the visitor's choice between local grid probes and table traversal.
`HashMap` traversal follows allocated capacity, which clear/rebuild retains, rather than the
number of populated cells. Comparing capacity preserves results and bounds local work after undo
or index rebuilding without shrinking storage or adding an index/buffer.

Five alternating matched unprofiled release process pairs use CPU 0 and one Rayon worker. One
20 m road has two populated node cells. The fixture reserves increasing capacity, invokes the real
`rebuild_all_indices()`, and verifies that capacity is retained. One warmup and nine samples of 64
queries cycle three hits and empty space at radius 5 m. Setup and output assertions are outside the
query timer; every sample returns `[0, 1, 0, -1]`. Median process medians, in microseconds:

| Retained table capacity | Editor, before → after | Cursor acquisition, before → after |
| ---: | ---: | ---: |
| 3 | 0.015234 → 0.015547 | 0.017547 → 0.017656 |
| 1,792 | 0.059141 → 0.051281 | 0.043922 → 0.058469 |
| 114,688 | 1.127375 → 0.050484 | 1.175156 → 0.058625 |
| 1,835,008 | 22.150859 → 0.050688 | 22.372094 → 0.058484 |

Local queries stop scaling with historical storage. The 1,792-capacity cursor case adds about
15 ns; this is not a universal speedup. Fresh processes retain standard randomized hash placement,
so baseline scan timing varies even with matched capacity and queries. These warm kernel timings
exclude bridge locks, rendering and setup, and do not establish whole-frame speedups.

Three separate matched planning pairs use eight physical P-cores and eight Rayon workers. The
fixed local neighborhood produces identical products through 100,000 background buildings and
600,024 agents. Compile medians remain 19.6–19.9 ms and worker medians 20.6–20.8 ms across sizes;
at the largest size compile is 19.827 → 19.747 ms and worker is 20.778 → 20.755 ms. One-time
snapshot creation is measured separately (3.329 → 3.413 ms at the largest size).

- Query: `RAYON_NUM_THREADS=1 METRUM_DEBUG=0 taskset -c 0 BINARY --exact simulation::network::interaction::tests::benchmark_node_grid_retained_capacity --ignored --nocapture --test-threads=1`.
- Planning: `RAYON_NUM_THREADS=8 METRUM_DEBUG=0 taskset -c 0,2,4,6,8,10,12,14 BINARY --exact nodes::sim::core::tests::road_plan_scaling::populated_paved_road_plan_scaling --ignored --nocapture --test-threads=1`.
- Before release test executable SHA-256: `3fbda58cf0889ebe799363b0bab112efb91ee8607abee492c0bed982220c75b0`.
- After: `088c53a52ba53b229113388d75f8a1b5a5b83fec0e4687d58d1233bf0bd100f6`.
- Rust: `rustc 1.98.1 (48a229cea 2026-09-01) (Arch Linux rust 1:1.98.1-1)`.
- Artifacts: `/tmp/metrum-full-audit/node-grid-capacity-{before,after}-identity.json`, query and planning `*-bench.json`/`*-summary.json`, raw matched-pair logs and `node-grid-capacity.diff`.

All 1,765 release tests pass (60 ignored), `node-grid-capacity-after-tests.log`. Test/benchmark
compilation is warning-free. No builds, tests or source changes overlapped accepted measurements.

## Performance Contract

Correctness without acceptable performance is not done.

`AUDIT-01-G1` corrects the common integer-key rounding helper: ties round away from zero,
but representable values immediately below a half must remain below the boundary. Large
already-integral values and saturating conversions retain Rust's standard semantics.
The former offset-plus-cast shortcut violated that contract. On the audited build, three
alternating unprofiled release process pairs of
`nodes::sim::core::tests::road_plan_scaling::populated_road_plan_scaling --exact --ignored --nocapture`
used 24 Rayon workers, three warmups and 100 observations per background size. With
0/1,000/10,000/100,000 remote buildings, worker p50 medians were
`20.920/20.908/20.756/20.892 → 21.453/21.600/21.712/21.676 ms`.
At 100,000 buildings (600,024 agents, 391 remote roads), direct planning was
`20.355 → 20.692 ms`; readiness stayed below 0.020 ms, and the separately measured
one-time snapshot was `3.317 → 3.371 ms`. Every run retained identical local terrain products.
The small bounded cost of correct rounding is accepted; repeated planning still follows
the fixed neighborhood. Exact binaries, source hashes and raw process logs are under
`/tmp/metrum-full-audit/quantization-*`, with merged results in `quantization-locality-matched.json`.
The separate scalar experiment is diagnostic evidence, not a whole-city timing claim.

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
  and `first_idle_ms`
  (also foreground water/border/residency work). `commit_ms` retains the five
  consecutive idle frames; `settle_tail_ms` isolates the final stable tail. These are CPU frame
  observations, **not GPU presentation timestamps**. Headless is uncapped unless
  `METRUM_GAMEPLAY_BENCHMARK_MAX_FPS` overrides it; cadence, viewport, engine, CPU/GPU names, worker
  counts, and source/binary/world fingerprints are recorded. Do not interpret headless/windowed
  differences as GPU execution cost. `state_after.command` exposes generation-matched core stages:
  command-queue wait, locking, add, finalization, surface/terrain, agents/lanes, buildings, routing,
  mesh, snapshot, and refined-state work, plus dirty edges and rebuilt chunks. `ROAD-28` adds
  `preview_plan_reused`, `road_plan_ms` (reuse validation or cold road solve) and `terrain_plan_ms`
  (deferred click completion); both timings are included in `add_ms`. Finalization includes
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
  monotonic milestones and command generations through `tools/road_benchmark_report.py`.
  Schema 4 removes the retired global-guide readiness/count fields. Paired captures still require matched
  pre/post-edit graph, lane, agent and building cardinalities; reset fixtures must repeat road
  output cardinalities. Schema-3 historical artifacts require their original verifier.
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
- shared road/site materials load only shader inputs that contribute to their output. Displacement
  textures are not bound by these shaders. Road concrete uses geometric normals; the concrete
  normal texture is loaded when a concrete site material actually needs it.
- road debug output must split terrain, water, zoning, and total patch-debug timings
- road debug output must use cached zoning statistics instead of scanning parcel payloads

`AUDIT-01-G10` removes the unused displacement loads, the road-concrete shader's discarded normal
sample and the texture loader's forwarding wrapper. Material/texture sharing and the texture assets
remain intact. Before/after rendering produces byte-identical images for six material variants,
three shapes (two marked road planes and a box) and two camera angles: 36 references total.

Five alternating fresh-process comparisons use Godot 4.7.2, OpenGL Compatibility and Mesa 26.2.2
llvmpipe under Xvfb, with `RAYON_NUM_THREADS=8`, `LP_NUM_THREADS=8` and CPU affinity
`0,2,4,6,8,10,12,14`. Median road prewarm calls fall from 1.739 to 1.178 seconds. Concrete-site
creation now loads its required normal map, so the combined road/site call time is the meaningful
total: 1.739 to 1.439 seconds (17.3% lower). RenderingServer texture-memory deltas are constant
within each version: 425,022,797 to 285,212,666 bytes after road prewarming, and 425,022,797 to
374,691,150 after all six materials. That removes about 133.3 MiB at road prewarm and 48 MiB after
the site materials are present. These are texture accounting and material-factory call timings,
not RSS, peak memory, first-draw shader compilation or gameplay frame-rate measurements. Each
process starts with empty factory caches; OS file caches are not flushed.

Artifacts are `/tmp/metrum-full-audit/material-inputs-*`: source hashes/snapshots, the actual render
harness and images, image comparison, all process commands/logs and matched measurement summaries.
`python3 /tmp/metrum-full-audit/match_material_inputs.py` replays the five pairs and restores the
accepted five source files between exited engine processes. The unchanged release extension is
`2dcc2885453d135cef74f03272df8b50e437f168bd28f298f01c5a489d35ef20`.

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
Use matched unprofiled schema-4 workloads for new performance claims. Run launchers serially:
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
