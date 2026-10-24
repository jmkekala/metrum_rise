# Roads

## Purpose

This document is the live road-surface contract for Metrum Rise. It owns the road cross-section
profile, junction layout, the visible roadbed mesh, road queries, the road side of terrain
integration, the road preview / commit / undo pipeline, and the guardrails that keep roads
deterministic and performant.

It does not own lane-routing policy, building frontage semantics, terrain storage internals, or the
shared engineered-ground model. Those are owned by [`entrance_and_exit.md`](entrance_and_exit.md),
[`terrain.md`](terrain.md), and [`earthworks.md`](earthworks.md). Pedestrian crossings and lanes are
described here only where they consume road geometry.

## Status

**`ROAD-68` — in progress on branch `road` (2026-10-23).** The legacy road-surface compiler is
retired and is being replaced by the [constructive roadbed](#constructive-roadbed-road-68) below.
Gate status is in the [implementation plan](#implementation-plan-road-68).

The legacy compiler (`rust/src/simulation/network/surface/`, about 145k lines in 437 files, of which
about 67k production lines built junctions) emitted an overlapping corridor polygon for every band
of every arm, then discovered ownership and heights with polygon booleans, winding and claim
priorities, three noding passes, generated contacts and height-authority rules. Its fresh rendered
baseline ([`ROAD-67-A`](#baseline-road-67-a-legacy-compiler)) missed the 16.7 ms deadline on all
750 sampled moving poses, and the isolated five-way junction compiled in 101 ms. The `ROAD-67-B`
prototype that ported the same semantics onto a shared subdivision was stopped: its graph stage
alone consumed the solve budget before heights, export or terrain, and it was still reconciling
millimetre height conflicts that only exist because legacy contours interpolate heights
independently. `ROAD-67` is closed as superseded. Its design notes, measurements and the full
`ROAD-01`–`ROAD-67` history are archived in
[`archive/roads_legacy_surface_2026-10-23.md`](archive/roads_legacy_surface_2026-10-23.md).

On the `road` branch:

- the game may be unplayable between `ROAD-68` gates; every gate commit still compiles, adds no
  Clippy warnings and passes its own tests
- save compatibility is not maintained; no road-surface geometry is saved, and load rebuilds every
  piece from the graph
- the legacy compiler stays available for visual comparison on `main` and at tag `road-legacy`
  (`d6c80f91`, created in `ROAD-68-A`)

### Why junctions can be constructed

The product geometry is far simpler than the legacy compiler assumed:

- only four band kinds are ever emitted: `Carriageway`, `CurbOrShoulder`, `Sidewalk` and
  `Footpath`; `Median`, `Parking`, `CycleTrack` and `TramReservation` are reserved
- a car road's cross-section is fixed: `sidewalk | curb | carriageway | carriageway | curb |
  sidewalk`, or shoulders in place of curb and sidewalk
- there are exactly two height levels: ground (carriageway, footpath) and raised, `+0.12 m` (curb,
  sidewalk, shoulder)
- every `Bend` and `JunctionN` already has one solved plane (`JunctionEndpointProfilePlane`), and
  span cross-sections reach it at full blend weight at the mouth

A junction surface is therefore `P(x, z)` for ground bands and `P(x, z) + 0.12` for raised bands,
bounded by curves that follow directly from the arms. Nothing overlaps, so nothing needs to be
arbitrated.

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

Splitting an edge at a junction merges any control knot within 5 cm of the junction, the
intersection node capture distance, into the junction on either side (`ROAD-51`, fixed
2026-10-04). Before, only the knot before the junction merged, and only within 1 mm. An endpoint
snapped onto a road it already lies on keeps its exact projection. Before, it moved onto a knot
within 5 cm. An endpoint off the road still snaps to a knot that close, as before.

Long roads are resampled at about 6 m in f32, so knots land within millimetres of crossings, or
centimetres of a projection, at whole multiples of the spacing. That caused two failures:

- **1M grid.** A knot 1.1 mm past the junction at (-4200, 600) on the 20.4 km row left the second
  half a 1.1 mm first segment. The four-way failed to compile and the row was rejected with
  `surface_geometry_invalid`.
- **300k grid.** The row at z = -75 ended on the outer cross streets 27 mm from their knots.
  Moving its ends onto those knots shifted the whole row 27 mm sideways, after its crossing
  junctions had been placed on the original line. Every row edge then ended in a 27 mm jog, and
  eight four-ways failed.

Five centimetres is far above f32 spacing anywhere in a 20 km world, and far below any authored
segment. Regressions: `split_next_to_a_knot_leaves_no_sub_millimetre_segment` and
`endpoint_on_a_road_beside_a_knot_keeps_the_new_road_on_its_line` (`simulation/network/topology.rs`).

Road insertion, node movement, rollback and undo also refresh the zoning system's recorded
straight-road grid choices from the affected edge set. Node movement invalidates both old and
new corridors. These choices belong to zoning, use one local adjacency ring, and survive save
remapping; see [persistent alignment](zoning.md#persistent-straight-road-alignment-checkpoint).

The graph is not the final visible road surface, final terrain clip carrier, final earthwork
boundary, or final node polygon carrier.

### RoadbedSystem

`RoadbedSystem` (`rust/src/simulation/network/roadbed.rs`, replacing the legacy
`RoadSurfaceSystem`) owns:

- cross-section profiles and junction layouts derived from the graph
- immutable span and node pieces: meshes, outer chains and query grids
- the road chunk index and the source / compiled generation gate
- visible-surface height, band and ray queries
- the road side of terrain integration: patch loops, grading envelope and patch selection
- preview artifacts, commit adoption and undo records
- road debug output

It evaluates heights only as `plane + level` at nodes and as the graph's finalized physical profile
along spans. It does not solve longitudinal profiles; the graph's shared `RoadEditPlan` finalizer
does.

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

## Constructive roadbed (`ROAD-68`)

### Invariants

1. **Construct, never discover.** Every boundary is generated by a rule from the arms. The road
   compiler uses no polygon boolean, winding or claim-priority ownership, noding, generated
   contacts, or provenance / height-authority rules.
2. **One value, one function.** Mouth cross-sections (shared by span and node), corner curves
   (shared by the mesh, crosswalks and pedestrian lanes) and heights (`plane + level`) are each
   computed once and copied, never recomputed independently.
3. **WYSIWYG.** Preview, commit, undo and load produce bitwise-identical pieces from the same graph
   inputs. Road queries answer from the triangles that render.
4. **Watertight by construction.** Adjacent bands share vertex sequences, curb faces share their
   edges with both levels, spans and nodes share mouth vertices, and terrain seams reuse the road's
   outer-edge vertices.
5. **No repair.** A construction that cannot satisfy its rules returns a typed error, and the edit
   is rejected with a reason. There is no tolerance snapping, nearest-height, averaging, fallback
   geometry or render masking.
6. **Deterministic.** Arms, dirty sets and chunks are processed in sorted order and internal
   geometry uses `f64`. Output does not depend on the Rayon worker count, hash order, or an edit
   history that produces the same graph.
7. **Local.** An edit touches only its [dirty closure](#invalidation-and-chunks) and the chunks
   and terrain patches covered by its old and new pieces.

### Dataflow

```text
graph XZ (authored alignment, widths, lanes, modes, class)
  -> cross-section profile                   per edge
  -> junction layout                         per node: arm order, sectors, corners, mouth stations
  -> profile solve (existing finalizer)      hard zone = mouth station; junction and terminal planes
  -> mouth sections                          per edge end, shared by span and node
  -> node pieces + span pieces               meshes, outer chains, query grids
  -> render chunks | terrain patches (existing CDT) | queries | lanes and crossings
```

Layout depends only on XZ geometry and profiles, so the profile solve can pin each arm to its
plane over exactly the node piece's extent. Heights never feed back into layout.

### Cross-section profile

`RoadProfile` is a pure function of the edge (`primary_type`, `allowed_types`, `width`, `class`),
ported unchanged from the legacy band rule. Bands are listed left to right in the edge's forward
frame:

| Edge | Bands | Levels |
| --- | --- | --- |
| Car road with sidewalks | sidewalk 1.35 m, curb 0.15 m, carriageway `w/2`, carriageway `w/2`, curb, sidewalk | raised, raised, ground, ground, raised, raised |
| Car road without sidewalks | shoulder 0.75 m, carriageway `w/2`, carriageway `w/2`, shoulder | raised, ground, ground, raised |
| Footpath | footpath `max(edge.width, 2 m)` | ground |

`w` is `max(edge.width, LANE_WIDTH)`. The profile, not the `Edge::width` doc comment, defines what
width means. The raised offset is `CURB_STEP_HEIGHT_M` (0.12 m). New band kinds, such as medians,
parking or cycle tracks, are added as explicit ordered bands with a level; nothing else in the
compiler may special-case them.

### Junction layout

A layout is built for every node except degree-two pass-through nodes (the existing
`directions_are_pass_through` rule). Those own no node piece; their spans meet at one bisected
cross-section, as today.

**Arms.** Each incident live edge end becomes an arm with:

- its centerline polyline in control-geometry XZ, oriented away from the node
- its profile, oriented outward: left and right as seen walking away from the node
- its direction, taken from the first metre of the centerline as `validate.rs` measures it

Arms are sorted counter-clockwise by direction; ties break by edge index, then edge end.

**Offset curves.** For an arm, `o(s, l)` is the point at station `s` along the centerline,
offset by signed lateral distance `l` along the section lateral axis spans already use. Every band
boundary is an offset curve at its lateral offset.

**Sectors.** Each pair of consecutive arms `a`, `b` (counter-clockwise) bounds one sector between
`a`'s left side and `b`'s right side, with angle `phi`. A fixed angular constant `eps` owned in code
classifies it:

- concave, `phi < pi - eps`: a curb-return corner
- straight, `|phi - pi| <= eps`, or convex, `phi > pi + eps`: a round join around the node

A `Terminal` has one sector of `2 pi`, which is convex.

**Concave corner.** Let `la` be `a`'s left carriageway-edge offset, `lb` be `b`'s right one, and
`R` the curb-return radius ([D1](#product-decisions)).

1. The corner centre `C` is the first intersection, by distance from the node, of
   `o_a(., la + R)` and `o_b(., -(lb + R))`. When they do not cross at stations `>= 0`, as in an
   obtuse sector between a narrower and a wider carriageway, the search also uses each arm's
   first offset segment extended behind the node (station `< 0`), and the tangent station on
   the wider arm may be negative ([open question 6](#road-68-open-questions)).
2. The tangent stations `sa*` and `sb*` are the stations of `C` on those two curves. Every band
   boundary on that side reaches the corner at the same station, because each lies on the arm's
   lateral line through `C`.
3. Band boundary `j` runs from `o_a(sa*, la_j)` to `o_b(sb*, lb_j)`. In polar form about `C`, its
   angle and radius each interpolate linearly from those exact endpoint values. For straight arms
   with equal profiles this is an exact family of concentric arcs. Endpoints are copied from the
   offset curves, never re-derived.
4. All boundaries of one corner use one sample count `N`, from the longest boundary and fixed
   maximum segment length and angle constants. Every band is therefore a quad strip.

Unequal profiles on the two sides interpolate per boundary. A band missing on one side has zero
width there and tapers along the corner.

**Round join.** On straight and convex sectors, band boundary `j` is an arc about the node from
`o_a(0, la_j)` to `o_b(0, lb_j)`, with the radius interpolated linearly by angle. A straight
sector between unequal widths gives a radial step at the node; its taper is
[D4](#product-decisions). In a straight sector narrower than `pi`, the arms overlap, so each
boundary starts where its offset curve crosses the sector bisector instead of at station zero
([open question 5](#road-68-open-questions)).

**Mouth stations.** For each arm, the mouth station is the larger of `s_min` and the tangent
stations of its concave corners, plus the crosswalk depth on crossable arms
([D8](#product-decisions)). Between a corner's tangent station and the mouth, the arm's band
boundaries follow its offset curves. On a crossable arm that depth is the zebra strip, right
outside the curb returns: its inner edge is the larger tangent station, where the curbs stop
curving, its outer edge is the mouth, and its crossing line is the middle, with kerb points at
the two carriageway edges. The mouth is the arm's full cross-section at that station:
the node piece ends and the span begins there. When both ends of an edge reach the same station,
the span is empty and the two node pieces share that mouth section.

**Fitting.** Each end may use at most half of its edge's length.

- If a corner does not fit, `R` is reduced towards `R_min`. If it still does not fit, layout
  returns `LayoutError::CornerDoesNotFit { node, arm }`.
- A self-intersecting carriageway core returns `LayoutError::CoreNotSimple { node }`.

Both are placement rejections with player-facing reasons, alongside the existing
[`ROAD-54`](#road-geometry-validation-road-54) limits: illegal arm angles below 25° and degree
above 6.

**Products.** `JunctionLayout` stores:

- per arm: the mouth station, tangent stations and crosswalk strip (inner and crossing
  stations, kerb points)
- per sector: its kind, `R`, `C` and sampled boundary curves

Graph clips (`start_clip` / `end_clip`), the profile solve's hard zone, crosswalk placement and
pedestrian corner routes all read the layout. `graph/rebuild/clips.rs` and
`junction_profile_crossing_core_m` no longer compute setbacks of their own, and pedestrian lanes
compute no crossing or corner geometry of their own
([Pedestrian Junction Crossings](#pedestrian-junction-crossings)).

### Heights

- `Bend` and `JunctionN` use their solved plane `P`. The profile finalizer pins each incident
  arm's physical profile to `P` on `[0, mouth station]` and blends back to the source grade outside
  it, as today. A knot at the mouth station makes the pin exact there: an adapted arm takes it in
  its control polyline, a stable one in its physical profile only. A `Terminal` gets a plane
  through its road end and its mouth centreline point, with the arm's end grade along that run and
  no crossfall; it is read from the solved profile, so the finalizer leaves terminals alone.
- Every node-piece vertex is at `P(x, z) + level`. A curb face is a vertical quad between the
  ground and raised vertices at the same XZ.
- Mouth sections are evaluated directly as `P(x, z) + level`. Span sections at full blend weight
  use the same function, so the shared seam is bit-identical. A finalized centerline more than
  1 mm from `P` at the mouth is a profile-solve defect: it is reported as an error, not repaired.
- Spans keep the existing section model: centerline height from `physical_geometry`, band heights
  from their level, and the existing plane blend inside the transition zone.

### Pieces and meshes

**Node piece.**

- **Carriageway core.** One closed loop crosses each arm's mouth, follows its carriageway edges
  back to the corners, and runs along each sector's carriageway-edge curve. It is fanned from the
  node when the node sees every loop edge, and Spade CDT triangulates it otherwise (a `ROAD-68-C`
  correction). The core is planar, so no other interior vertices are needed.
- **Raised strips.** Quad strips between consecutive band boundaries, for each sector and arm
  side.
- **Curb faces.** Along every carriageway boundary except the mouths.
- **Mouth vertices.** Copied from the shared mouth section.

**Span piece.** Sections are placed at the physical knots, at the refinement stations, and at the
two mouth sections (copied). Each band is one quad strip, with curb faces along both carriageway
edges.

**Other node kinds.**

- `Bend` is a two-arm junction with one concave corner and one round join.
- `Terminal` is a one-arm junction with a round cap ([D2](#product-decisions)).
- Bridges and tunnels use the same pieces, with class-specific terrain handling in
  [`ROAD-68-F`](#road-68-f--remaining-families).

**`PieceMesh`.** A piece's mesh holds:

- positions
- per-layer triangle index lists (carriageway, curb or shoulder, sidewalk, footpath, curb face),
  with each triangle's band kind kept for queries
- outer chains: footprint boundary polylines oriented with the road on their left, each running
  from one mouth corner to the next
- bounds
- a per-piece triangle query grid, reusing the legacy `SurfaceTriangleGrid` design

Pieces are immutable and shared through `Arc`.

**Watertightness.** Within a piece, every edge borders exactly two triangles, except outer-chain
edges (which meet terrain) and mouth edges. Every mouth edge appears in exactly one span and one
node piece, with bit-identical endpoints. Tests check both conditions on every fixture.

### Footprint and terrain

The terrain CDT core in `rust/src/simulation/terrain/cdt/` stays. It is generic: patch-clipped
loops, source samples and tie-in guides in; stitched patch mesh out. Its contract in
[`terrain.md`](terrain.md) and [`earthworks.md`](earthworks.md) is unchanged:

- loops are hard constraints
- faces inside road footprints are omitted
- seam vertices take road heights
- source samples are inserted only outside footprints
- building-site boundaries share the CDT, and the road height wins at shared vertices

The road side is rewritten:

- **Patch loops.** The roadbed builds each patch's loops directly from outer chains: collect the
  chains of pieces overlapping the patch from the chunk index, clip them to the patch rectangle,
  and connect exits to entries along the rectangle boundary. Orientation is known, so holes such as
  closed city blocks need no classification. Cost is `O(S log S)` for `S` local chain segments.
  The legacy `terrain_clip/` union and `query/terrain_cdt/` adapters are deleted.
- **Source tags.** `TerrainCdtRoadBoundarySource` keeps `BuildingSiteBoundary`. Its span, node and
  handoff variants are replaced by one `RoadBoundary { piece, chain, segment }`.
- **Grading envelope.** Tie-in guide samples and constraints are reimplemented against the outer
  chains, following the existing rule in [`earthworks.md`](earthworks.md#geometry-decision): 2 m
  or finer sampling, at most `EARTHWORK_MAX_MARGIN_M`.
- **Patch selection.** Patches overlapping the old and new pieces' bounds plus grading margin.
- **Bridges and tunnels.** They do not clip terrain mid-span. Abutment and portal cutouts keep the
  existing rules ([`ROAD-68-F`](#road-68-f--remaining-families)).
- **Source terrain.** The derived mesh never rewrites it. Placed roads stay fixed under later
  terrain edits.

### Queries

`RoadbedSystem` answers:

- visible-surface height at an XZ point, optionally carriageway-only
- band kind at a point, and band kinds near a disc (tree clearance)
- ray hits against the visible surface (cursor and picking)
- prepared owner-local lane queries (lane height snapping)

Each query maps XZ to pieces through the chunk index, then to triangles through the piece's grid,
so the answer comes from the rendered triangle. Visible-world precedence is unchanged: client
surface, then structural earthwork, then visual terrain ([`terrain.md`](terrain.md)).

### Invalidation and chunks

An edit's dirty closure is:

- the changed edges
- every node incident to them
- every edge incident to those nodes, because their mouths may move

Node pieces are rebuilt for those nodes and spans for those edges. The far end of an incident edge
is unaffected unless the edge itself changed, because a layout reads only its own arms.

Render chunks keep the existing terrain-aligned grid. A changed piece rebuilds the sorted union of
its old and new chunks, and every triangle has one deterministic centroid home chunk. The Rust to
Godot chunk payload stays byte-compatible, so Godot's committed-chunk renderer is unchanged:
`NetworkMeshData` layers, owner ranges, the generation and acknowledgement protocol, and removal
tombstones.

### Preview, commit, undo and load

**Context.** The road tool works on an immutable, versioned local context: the graph
neighbourhood, terrain samples, sites, and zoning and field reservations. It is captured when the
tool activates and refreshed when the simulation publishes a new generation. No per-pose work takes
the core lock, which also resolves the hover core-lock bug listed under Validated Bugs in
[`roadmap.md`](roadmap.md) (`get_agriculture_field_overlay_revision`).

**Preview.** `preview_road(context, stroke)` runs synchronously in RoadTool's early process
callback:

1. Finalize the local edit plan.
2. Compile the dirty closure.
3. Build the affected terrain patches in parallel on the owned Rayon pool.
4. Pack the road and terrain payloads.

It returns the payloads together with a dependency fingerprint: the generations of touched edges
and nodes, terrain patch revisions, and site, zoning and field revisions. Godot stages and shows
the result in the same frame. There is no mailbox, worker queue, request-ID retirement, late
publisher, coarse preview ribbon or stale-result path. Reintroducing asynchrony requires a measured
reason.

**Commit.** A click submits the preview's plan as an ordered simulation operation.

- If the fingerprint is still current, the simulation installs the preview's pieces and terrain
  patches without recompiling, then applies the topology mutation (graph, lanes, zoning, parcels).
- If it is stale, the simulation recompiles locally and synchronously.

Godot keeps showing the preview until the committed generation is acknowledged, then swaps road and
terrain atomically, so no frame shows neither.

**Undo.** The undo record holds the replaced pieces (`Arc`) and terrain patch revisions; undo
restores them by swapping.

**Load.** Load compiles every piece from the graph in parallel. A junction that fails layout or
the profile solve fails the load with its typed error.

### Commit consequences

Any topology change currently rebuilds the CCH from scratch and marks every flow field dirty
(`TransitNetwork::rebuild_pathing`). At city scale that alone exceeds a frame.
[`ROAD-68-G`](#road-68-g--commit-consequences-inside-one-frame) takes it off the commit's critical
path ([D7](#product-decisions)). Lane invalidation and agent reattachment remain local, as today.

### Complexity bounds

| Stage | Bound |
| --- | --- |
| Profile | `O(B)` for `B` bands |
| Layout, per node | `O(k log k + sum over corners of (Pa + Pb) + V)`: degree `k <= 6`, arm centerline points `P` inside the setback, emitted boundary vertices `V` |
| Node mesh | `O(V + n log n)`, with `n` carriageway-core vertices passed to the CDT |
| Span mesh | `O(sections * B)` |
| Dirty closure | `O(changed edges + their incident node degrees)` |
| Terrain patch | `O((S + G) log (S + G))` for `S` local chain segments and `G` patch samples plus guides |
| Height, band or ray query | `O(1)` chunk lookup plus `O(triangles in one grid cell)`, or the cells the ray crosses |
| Chunk rebuild | `O(pieces in the changed chunks)` |

No stage scans the city.

### Stage budgets

Provisional budgets per preview pose, on the [reference machine](#baseline-road-67-a-legacy-compiler)
and the declared fixtures:

| Stage | Budget |
| --- | --- |
| Context refresh | measured separately; never per pose |
| Edit plan (topology, profile solve) | 1.0 ms |
| Roadbed (layout and pieces for the dirty closure) | 0.5 ms |
| Terrain patches, four workers | 3.0 ms |
| Payload packing | 1.0 ms |
| Godot stage and install | 2.0 ms |

Together, about 7.5 ms, which leaves the rest of the 16.7 ms frame for rendering. The isolated
targets are a four-way node piece in 0.1 ms, a six-way in 0.3 ms, and a 100 m span in 0.05 ms.
Budgets are revised openly from measurements; the end-to-end target is not.

### Guardrails

- **Size.** `roadbed/` production code stays within 8,000 lines, tests excluded, through
  `ROAD-68-H`. Each gate records the count. Going over requires a written reason here.
- **No stage caches.** Only final immutable pieces are kept. A new cache needs a measurement that
  shows the need.
- **No hot-path allocation.** Per-pose scratch buffers live in the preview context and are reused.
- **No second implementation of shared geometry.** Pedestrian corner routes, crosswalk stations
  and street-furniture offsets read the layout.
- **Benchmarks gate performance.** Every gate that touches a hot path adds or reruns a Criterion
  benchmark; passing tests are not performance acceptance.

### Product decisions

These defaults apply unless the user overrides them:

| ID | Decision | Default |
| --- | --- | --- |
| D1 | Curb-return radius | 3.5 m at the carriageway edge (user-confirmed 2026-10-24; was 6 m). The minimum is the raised width plus 0.5 m. The radius is reduced to fit; if the minimum does not fit, the edit is rejected. |
| D2 | Terminal shape | Round cap; the curb and sidewalk wrap around the end. |
| D3 | Preview modes | One mode, road and terrain together. The `ROAD-29` "Road only" mode is removed in `ROAD-68-E`. |
| D4 | Width change on a straight side | A radial step at the node in `ROAD-68-C`; a linear taper along both approaches in `ROAD-68-F`. |
| D5 | Footpath meeting a road junction | It joins the sidewalk ring at sidewalk height, with its end pinned there by the profile solve. Its ribbon ends at the sidewalk outer edge on shared vertices. |
| D6 | Roads without sidewalks | Keep the current raised 0.75 m shoulder. |
| D7 | Routing at commit | Confirmed by the user, 2026-10-23. Rebuild the CCH on an immutable graph snapshot as a scheduled job and adopt it at a fixed tick offset. Route queries issued meanwhile wait for adoption. If the job is late, the simulation blocks at the adoption tick, so results do not depend on machine speed. Revisit if the query backlog at adoption is too large at 100k buildings. |
| D8 | Crosswalks | Right outside the curve (user-confirmed 2026-10-24). On crossable arms the mouth moves 3 m beyond the corner tangent station, and the 3 m zebra fills that straight approach: its inner edge is where the curb stops curving. The crossing is square to the road at the strip's middle, its kerb points at the two carriageway edges; sidewalk lanes end on that line, as `ROAD-64` requires. No zebra sits on a span. |

## Implementation plan (`ROAD-68`)

Gates are sequential unless noted. Every gate ends with:

- a commit that compiles and adds no Clippy warnings
- its own tests passing, plus `cargo test --lib` in debug
- the `roadbed/` production line count recorded
- updates to this section, [`roadmap.md`](roadmap.md) and [`project.md`](project.md)

A test that cannot pass until a later gate is marked
`#[ignore = "ROAD-68-<gate>: <reason>"]` and listed under the gate that re-enables it. At
`ROAD-68-H`, none may remain.

| Gate | Status | Scope |
| --- | --- | --- |
| `ROAD-68-A` | done | Remove the legacy compiler; add the roadbed façade; the crate compiles |
| `ROAD-68-B` | done | Profiles, spans, meshes, queries and render chunks |
| `ROAD-68-C` | done | Junction layout and node pieces for car roads |
| `ROAD-68-D` | open | Footprint chains and terrain patches |
| `ROAD-68-E` | open | Synchronous preview, commit adoption, undo and load |
| `ROAD-68-F` | open | Footpaths, mixed profiles, bridges, tunnels, crossings, street furniture and sites |
| `ROAD-68-G` | open | Commit consequences inside one frame (routing, flow fields) |
| `ROAD-68-H` | open | Rendered acceptance and final cleanup |

### `ROAD-68-A` — remove legacy, stand up the façade

1. Commit the stopped `ROAD-67-B` prototype (its untracked `height/shared.rs`, `height/grade/graph*`
   and `ownership/subdivision*` modules, and its tracked edits) to a side branch for reference.
   Restore the tracked files on `road`, then tag `d6c80f91` as `road-legacy`.
2. Delete:
   - `rust/src/simulation/network/surface/`
   - `rust/benches/surface_benchmark.rs` and its `Cargo.toml` entry
   - legacy-only tests elsewhere, such as `render/test_road_mesh.rs`
3. Add `rust/src/simulation/network/roadbed.rs` as the API router, with modules under `roadbed/`.
   It provides:
   - `RoadbedSystem`, owned by `TransitNetwork` in place of `road_surface`: dirty marking,
     `compile_dirty`, the source / compiled generation gate, the chunk index and piece storage
   - the public types `RoadProfile`, `BandKind`, `Level`, `JunctionLayout`, `SpanPiece`,
     `NodePiece`, `PieceMesh`, `OuterChain` and `MouthSection`
   - the query signatures

   Pieces are empty at this gate.
4. Port every consumer outside `surface/` to the façade:
   - simulation-core preview, commit and undo
   - `render/road`
   - lanes, the building allocator, zoning, agriculture and vegetation
   - save/load and validation
   - the junction-profile solve
   - the CDT and async-terrain adapters
   - the Godot API

   Queries return `None`, consumers take their existing "surface missing" path, and roads render
   nothing. Delete the asynchronous preview worker, mailbox, reclaim thread and topology-reuse
   plumbing. The preview API keeps its Godot signature and returns empty payloads.

Acceptance:

- the crate builds, and Clippy reports no new warnings
- library tests pass, with the ignores listed under their gates
- every public façade item is documented

Result (done 2026-10-23): `surface/` (432 files) is deleted, together with the asynchronous preview
worker, mailbox, reclaim thread, terrain plan, topology reuse and the planned visual-terrain
writes. Every consumer uses the façade, the queries answer `None`, and roads render nothing.
`roadbed/` holds 829 production lines.

- Plan readiness now repeats the commit's field corridor check, as it already did for cells and
  parcels, so a plan over a reserved field is `invalid` (`field_overlap`) instead of `ready`.
- Debug library suite (`RAYON_NUM_THREADS=4`): 1,595 passed, 0 failed, 348 ignored. Of the
  2,679 legacy passes, 1,592 still pass, 3 moved with their code into `road_edit/input.rs` and
  `roadbed/geometry.rs`, 889 were deleted with the code they tested (786 of them in `surface/`),
  and 195 are ignored below. Three `tests/growth_scenario.rs` integration tests are ignored too.
- Clippy (`--all-targets`) reports the same 59 warnings as the legacy tree. `cargo doc` reports
  no missing docs.
- `./run.sh --test` sequence, isolated profile: `cargo check --benches` and `cargo test` pass, and
  39 of 45 Godot script runs pass. The 6 that need road geometry are skipped by `run.sh` until
  their gate (below). The two Forward+ vegetation tests did not run (no display). The script itself
  stops earlier, at the unrelated fixture check in [open question 4](#road-68-open-questions).

#### `ROAD-68-A` disabled tests

Each test below is marked `#[ignore = "ROAD-68-<gate>: <reason>"]` and is re-enabled by the gate
that heads its table. Tests of deleted mechanisms were deleted, not listed.

`ROAD-68-B` (154): re-enabled by `ROAD-68-B`. 140 pass; the 13 frontage-seam tests in
`nodes::sim::core::tests::building_site_terrain` moved to `ROAD-68-D` and
`bridge_entry_points_never_use_the_global_pool` to `ROAD-68-F` (tables below, and the
[`ROAD-68-B` result](#road-68-b--profiles-and-spans)).

`ROAD-68-C` (7): re-enabled by `ROAD-68-C`. Four pass:
`failed_junction_precompute_retains_matching_render_and_road_tool_generation` (its failure probe now
reads `failed_nodes`, since a failed node no longer leaves the roadbed dirty),
`branch_from_existing_t_compiles_four_way_junction_on_flat_and_slope`,
`kuopio_third_road_preserves_existing_junction_boundary_owners` and
`road_dependencies_use_query_grid_and_invalidate_local_pieces`. Three moved:
`kuopio_saved_reference_repairs_rejected_junction_before_publication` to `ROAD-68-E`, and
`a_residential_street_gets_evenly_spaced_lights_on_one_side_behind_the_kerb` and
`headlight_house_junction_full_load_compiles_with_terrain_earthwork` to `ROAD-68-F` (tables
below). `road_edit::tests::assert_cold_geometry_matches` compares node-piece meshes and errors
again, and `node_merge_adoption_clears_survivor_restrictions_and_rejects_open_frontier` again
requires the merged node's piece before the merge.

`ROAD-68-D` (25):

| Module | Tests | Reason |
| --- | --- | --- |
| `nodes::sim::core::tests::building_site_terrain` | `access_snapshot_pad_ownership_overrides_pending_visual_terrain`, `cars_and_walkers_enter_and_exit_on_compiled_yards`, `cars_pitch_and_roll_on_compiled_yards_in_both_access_directions`, `engineered_queries_reject_render_rejected_ground`, `interpolated_vehicle_batch_keeps_lane_poses_and_supports_changed_xz`, `kuopio_reference_supports_zoned_graded_yards_without_changing_fixture`, `large_paved_yards_remain_flat_over_hills_and_depressions`, `new_world_clears_populated_city_and_engineered_terrain`, `site_changes_far_apart_refresh_only_the_patches_they_touch`, `small_commercial_pads_leave_a_watertight_sloping_frontage`, `yard_hedges_stand_on_the_graded_lawn_of_a_sloping_yard`, `yard_material_changes_reuse_cdt_but_not_final_paving_buffers`, `zoned_yards_cover_uphill_and_downhill_frontages` | the frontage paving seam takes road heights only where the road's outer chains enter the terrain CDT (moved from `ROAD-68-B`) |
| `nodes::sim::core::tests` | `planned_topology_terrain_rejection_restores_split_dependents` | road terrain patches |
| `nodes::sim::core::tests::road_terrain_plan` | `kuopio_recorded_profiles_and_terrain_preserve_quality_limits` | road terrain patches |
| `nodes::sim::editing::tests` | `accepted_road_has_current_terrain_payloads_before_publication`, `terrain_rejection_restores_split_dependents_and_preserves_undo_history`, `undoing_road_restore_invalidates_derived_render_cache`, `undoing_road_restore_invalidates_road_locked_terrain_state` | road terrain patches |
| `nodes::simulation_node::tests::async_payload` | `conservative_road_grading_pad_filters_only_margin_only_engineered_patches`, `unmatched_surface_preserves_road_ownership_while_site_ownership_refreshes` | road terrain patches |
| `nodes::simulation_node::tests::cdt` | `terrain_cdt_adjacent_tiles_emit_identical_shared_side_vertices`, `terrain_cdt_guides_use_shared_source_height_and_final_grade_limits`, `terrain_cdt_input_keeps_multi_loop_tie_in_guides_unconstrained`, `terrain_cdt_local_bounds_expand_for_required_grading_distance` | road terrain patches |

`ROAD-68-E` (2):

| Module | Tests | Reason |
| --- | --- | --- |
| `nodes::sim::editing::tests` | `undoing_fourth_junction_mouth_restores_exact_pre_edit_surface_topology` | undo restores the exact pre-edit junction pieces and mesh |
| `nodes::sim::core::tests::road_terrain_plan` | `kuopio_saved_reference_repairs_rejected_junction_before_publication` | a save from before `ROAD-68-C` re-pins every junction whose profile reaches only the legacy hard zone, not just node 34; the load rebuild owns save migration (moved from `ROAD-68-C`, [open question 8](#road-68-open-questions)) |

`ROAD-68-F` (5; 7 at the gate, two re-enabled by the
[C follow-up](#tighter-corners-and-junction-crosswalks-follow-up)):

| Module | Tests | Reason |
| --- | --- | --- |
| `simulation::network::render::road::street_lights::tests` | `a_residential_street_gets_evenly_spaced_lights_on_one_side_behind_the_kerb` | lights keep a span-end clearance; the terminal cap needs layout furniture stations (moved from `ROAD-68-C`) |
| `simulation::save::tests` | `headlight_house_junction_full_load_compiles_with_terrain_earthwork` | node 1 joins a footpath to two roads (D5), so it has no node piece yet (moved from `ROAD-68-C`) |
| `nodes::sim::core::tests::zoning_buildability::cell_zoning::movement` | `node_move_into_paint_or_erased_occupied_claim_is_atomic`, `node_move_rejects_terminal_cap_overlap_outside_the_straight_corridor` | node-move reservation check against moved pieces |
| `nodes::simulation_node::tests::global_pool` | `bridge_entry_points_never_use_the_global_pool` | unchecked node moves leave a saved parcel over the moved road, so the load entry fails; every other entry point keeps to the shared pool (moved from `ROAD-68-B`) |

The 22 other `ROAD-68-F` tests (street lights, road signs, vegetation clearance and the site
support target) pass on span sections and were re-enabled in `ROAD-68-B`.

Godot scripts skipped by `run.sh --test`:

| Gate | Script | Reason |
| --- | --- | --- |
| `ROAD-68-E` | `network_tool_chunk_renderer_test` | the moving preview uploads a road mesh |
| `ROAD-68-E` | `road_junction_preview_test` | the preview exports the junction scene and a ready plan |
| `ROAD-68-E` | `road_preview_stream_test` | junction previews update while the pointer moves |
| `ROAD-68-F` | `zoning_cells_tool_test` | node moves check painted cells; parcels beside cells need frontage |

`zoning_road_tool_test` (`ROAD-68-B`: parcel lots need road frontage heights) passes again and
runs since `ROAD-68-B`. `zoning_reference_test` (`ROAD-68-C`: lots exclude the junction sidewalk
footprint) passes again and runs since `ROAD-68-C`.

#### `ROAD-68` open questions

These need a user decision; work that does not depend on them continues.

1. **Planned overlap checks before commit.** The legacy commit tested the planned junction and
   span footprints against painted cells and fields (`PlannedRoadSurfaceQuery`). `ROAD-68-A` keeps
   only the corridor checks (`cells_overlap_road_corridor`, field corridor), in both the commit and
   plan readiness, so a junction corner may overlap painted cells until `ROAD-68-E`/`ROAD-68-F`
   restore the check from preview pieces. Options: accept on the branch (current), or block commits
   that touch reserved cells near nodes.
2. **Node moves over painted cells.** The node-move reservation check compiled a local surface.
   It is removed until `ROAD-68-F`; node moves are currently unchecked against cell reservations.
3. **Site CDT margin.** Building-site loops used the legacy seam-vertex grading margin for their
   sample bounds. They now use the base local sample margin. Steep site tie-ins may sample less
   terrain until `ROAD-68-D` defines the grading envelope for both roads and sites.
4. **Pinned Kuopio fixture hash (`ROAD-69`) — resolved 2026-10-23, re-pinned.**
   - **The fault.** `./run.sh --test` stopped at its first step: `tools/test_road_terrain_replay.py`
     (`test_pinned_fixture_integrity`) failed, on `road-legacy` too.
   - **The cause.** Each parent of the merge `209d3c9c` had a save matching its own manifest
     (`6bda216e…` and `12d1f538…`). The merge committed a regenerated third save, `5ac4cb0f…`,
     written in the merged save format, but kept one parent's manifest.
   - **The fix.** `placements.json` now pins `5ac4cb0f…`. Restoring `6bda216e…` would have
     brought back a save from before the merge's format changes.
   - **Verification.** The manifest's other integrity checks (attempts 1–37, attempt 36
     rejected, 70 live edges, source terrain) pass against the committed save, and all 8
     `test_road_terrain_replay.py` tests pass.
5. **Straight sectors narrower than a half turn (`ROAD-68-C`).** The [round join](#junction-layout)
   from station zero makes the carriageway core cross itself whenever `phi < pi` and the two
   widths are equal: the arms overlap in a sliver of angle `pi - phi`, and their carriageway edges
   cross at station `la * tan((pi - phi) / 2)`. A straight road split by a side road lands on
   either side of `pi` by float noise, so about half of all such T junctions would fail with
   `CoreNotSimple` (the layout test `nearly_straight_sectors_keep_the_core_simple` fails that way
   with the literal rule). A concave corner is no substitute there: its centre is ill-conditioned
   near `pi`, and it does not exist for unequal widths. Current default: below `pi`, each boundary
   starts where its offset curve crosses the sector bisector, so the arms meet in a miter. Equal
   widths share one miter point, and a width change steps radially along the bisector (D4). At
   `pi` and above, the round join is unchanged. Options: keep the miter, or join every straight
   sector at station zero and reject the overlapping half.
6. **Obtuse corners between unequal widths (`ROAD-68-C`) — defaulted: backward extension
   (2026-10-24); the user may override it.**
   - **The problem.** A concave corner needs `o_a(., la + R)` and `o_b(., -(lb + R))` to cross at
     positive stations. With a narrower arm `a`, that requires `la + R >= (lb + R) * cos(pi - phi)`,
     and shrinking `R` only makes it harder. A two-lane arm (3.5 m) next to a four-lane arm (7 m)
     fits only up to about 137°, so a side road meeting a four-lane through road at 40° failed with
     `CornerDoesNotFit` on its 140° side.
   - **The default.** When no crossing exists at stations `>= 0`, the search also uses each arm's
     first offset segment extended behind the node. The corner centre then lies on the wider arm's
     curb-offset line about 0.7 m behind its station zero, and its tangent station there is
     negative. The fillet reaches into the neighbouring straight sector's span of that line, where
     the carriageway edge continues anyway; `CoreNotSimple` still guards the result, and
     `CornerDoesNotFit` remains only when no crossing exists even then.
   - **Why.** It keeps the curb-return construction, its exact endpoints and one radius per
     corner, and it rejects nothing a player would expect to build. Accepting the rejection until
     `ROAD-68-F` tapers would refuse every oblique side road on a wider road; widening `eps` only
     moves the boundary.
   - **Evidence.** Layout tests `obtuse_corners_between_unequal_widths_extend_the_wider_arm_backwards`
     (120°–160°, both orders) and `a_two_lane_side_road_meets_a_four_lane_through_road` (40°, 60°,
     90°), and the node fixtures `mixed_40`, `mixed_60` and `mixed_90`, flat and sloped.
7. **Stable arms pinned at both ends (`ROAD-68-C`).** The profile solve pins an arm to `P` up to its
   mouth with a knot there. An adapted arm takes the knot in its control polyline, so it survives
   every later solve. A stable arm's control polyline must not change, so it takes the knot in its
   physical profile only, and keeps its physical knots on later solves unless that solve adds new
   supports. When the other end's solve does add supports, it resamples the physical profile on
   the control polyline and drops this end's knot; the node then fails with `MouthOffPlane`
   (`junction_profile_mismatch`). No fixture hits it. Options: accept the rejection (current); keep
   pinned physical knots through the resample; or let the solve add mouth knots to stable control.
8. **Saves from before `ROAD-68-C` (`ROAD-68-C`).** Their clips come from the legacy setback rule
   (about 5.5 m at a right-angle T) and their profiles are pinned only that far, while layout mouths
   sat at 9.5–16 m at the gate, and closer with the follow-up's 3.5 m radius (2.5 m closer at a right
   angle). On load most of their junctions fail the 1 mm mouth check, and the existing
   load repair re-solves them after recomputing their clips: the Kuopio reference fixture loads,
   but about ten of its junction profiles change. Options: accept the load repair (current); re-pin the
   reference fixtures; or a save migration with `ROAD-68-E`'s load rebuild.
9. **Width steps on straight sectors (`ROAD-68-C`).** D4's radial step has no face of its own. A
   straight sector between a narrower and a wider carriageway gets the generic strips, which are
   zero-area slivers along the step, and the carriageway-edge curb face runs the whole step, up to
   the wider carriageway edge. The piece is watertight (fixture `width_step`), but beyond the
   narrower arm's sidewalk a 12 cm face stands at the wider carriageway's edge and the outer chain
   runs along the step at raised level, which `ROAD-68-D` will see. Options: accept until
   `ROAD-68-F`'s taper (current); build explicit step faces and chain segments now.
10. **Crossings before `ROAD-68-F` — resolved 2026-10-24: keep the layout rule and move the
    lanes, including the bend rule (user decision).** The layout's crossable rule (car-road arms
    with sidewalks at junctions of three or more arms; bend arms only when the node forces a
    crossing; none where a node override removes it) now decides both the 3 m mouth extension and
    the pedestrian lanes' crossings, which take the layout's zebra strip
    ([C follow-up](#tighter-corners-and-junction-crosswalks-follow-up)). Nodes without a layout
    have no crossing. The two disabled lane and sign tests pass again.
11. **Publication while a node fails (`ROAD-68-C`, guardrail) — resolved 2026-10-24, option (b).**
    - **Before.** While any node failed to compile, `SimCore::precompute_road_mesh_data` and
      `road_tool_snapshot_from_core` held the last fully valid generation. The screen could keep a
      failed node's previous mesh while queries saw no piece there. That was stale geometry,
      against invariants 3 and 5.
    - **Now.** A failed node publishes as a gap: it has no piece, its chunks are republished
      without it, and a `road_node_failed` line on the road debug channel names it and its typed
      error. The road-tool context refreshes as usual, and `failed_nodes`/`node_error` still
      report the failure.
    - **Placement, commit and load.** Unchanged: placement validates layouts and rolls back a
      commit that still fails, and load fails on a failed node.
    - **Test.** `failed_junction_publishes_a_gap_not_its_previous_mesh` replaces the old
      hold test. It fails if the hold is restored.
    - **Follow-up for `ROAD-68-E`.** Option (c): validate affected layouts in every operation
      that dirties nodes (undo, terrain sync, node moves), so a published gap can only come from
      a bug.

12. **Walkers switch sides only on crossings (`ROAD-68-C` follow-up).**
    - **The problem.** Under the D8 rule (open question 10), walkers cross only on junction arms
      and forced bend arms, and wrap a road end round its cap. Unforced bends, pass-through
      nodes and other nodes without a layout have no crossing. The legacy lane rule crossed one
      arm at every degree-two node and road end, so a walker could change sides at every node,
      and the walking planner relies on that. It routes on the road-node CCH, which has no
      sidewalk sides, and its cost forecasts use a whole-network proof that every node lets every
      arriving sidewalk reach every leaving one (`ConnectorTargets::complete` in
      `agents/tick/planning/candidate/query.rs`).
    - **Consequences.** One node without a crossing makes that proof false for the whole network.
      Walk forecasts then quote per lane pair rather than per node context. A walk that must
      change sides and whose node path reaches no crossing fails to plan. The street-grid
      benchmark city has four corner bends, so its proof is now false. The economy benchmarks
      were not re-run.
    - **Evidence.** `test_pedestrian_crosses_junction` (a straight road, home and shop on opposite
      sides) found no walk until its road got a junction.
      `prepared_commute_queries_match_connector_paths_without_growing_storage` keeps its network
      complete only with forced crossings at its two bends. Forced crossings at pass-through
      nodes do nothing now, because those nodes have no layout.
    - **Options.**
      - (a) Accept (current), and later make the walking planner side-aware: sidewalk contexts in
        the CCH, or a per-node proof.
      - (b) Cross one arm of every bend by default, on its node piece right outside the curve;
        lay pass-through nodes out where a crossing is forced.
      - (c) Unmarked crossings at those nodes. This breaks `ROAD-64` (walkers cross only on drawn
        zebras).
    - **Before merging to `main`:** the user should decide, and the economy performance contract
      should be re-run.

Spec corrections in `ROAD-68-A`:

- The façade defines only what consumers use (`RoadbedSystem`, `BandKind`, `RoadSection`,
  `RoadBand`, `PieceId`, `LaneSurfaceQuery`, `SurfaceChunkKey`, `CURB_STEP_HEIGHT_M`); `RoadProfile`,
  `Level`, `JunctionLayout`, `SpanPiece`, `NodePiece`, `PieceMesh`, `OuterChain` and
  `MouthSection` arrive with the gates that build them.
- Placement profile validation measures grade and clearance over the prepared profile points; the
  legacy check sampled compiled span sections of the same profile.

### `ROAD-68-B` — profiles and spans

1. `roadbed/profile.rs`: the [profile table](#cross-section-profile).
2. `roadbed/span.rs`: section sampling between mouth stations, band quad strips, curb faces and
   outer chains. Mouth stations come from the existing graph clips until `ROAD-68-C`.
3. `roadbed/mesh.rs` and `roadbed/query.rs`: `PieceMesh`, the triangle grid, height, band and ray
   queries, and prepared lane queries. Re-enable lane height snapping and building frontage height.
4. Render: `render/road/standard_surface/*` consumes `PieceMesh`, and lane markings come from
   sections. The chunk payload format is unchanged.

Acceptance:

- spans are watertight
- span heights equal the section evaluation
- query heights equal the rendered triangles
- output is bitwise deterministic across runs and worker counts
- the chunk partition preserves the triangle multiset
- a new `roadbed_span` Criterion benchmark is recorded

Result (done 2026-10-23): every road, footpath, bridge and tunnel edge has a span piece between
the mouth stations its graph clips give, and roads render again: carriageway, curbs or shoulders,
sidewalks, footpaths and curb faces, with lane markings, street lights and signs on the span
sections. Height, band, ray, overlap and lane queries answer from the rendered triangles, so lanes
snap to the roadbed and buildings place on their frontage again. Junction, bend and terminal areas
stay empty until `ROAD-68-C`. `roadbed/` holds 3,048 production lines.

- New roadbed tests (`roadbed::span::tests`, `standard_surface::top_surface::tests`) check the
  acceptance list on flat and 2.5% fixtures with every profile: watertight spans (each edge borders
  two triangles except outer-chain and mouth edges; curb faces pair raised and ground vertices at
  equal XZ), vertices bit-equal to the re-evaluated sections, query heights equal to the highest
  rendered triangle at every top-triangle centroid, bitwise-identical pieces across runs and 1, 4
  and 8 workers and after an incremental rebuild, the chunk payload holding exactly the pieces'
  triangles in their centroid home chunks, and rebuilds and changed chunks limited to the closure.
  The legacy width-taper, pass-through axis and folded-interval regressions are ported.
- Debug library suite (`RAYON_NUM_THREADS=4`): 1,767 passed, 0 failed, 189 ignored, 36 of them
  `ROAD-68` gate ignores (C 7, D 25, E 1, F 3). All 154 `ROAD-68-B` ignores were removed:
  140 pass, 13 moved to `ROAD-68-D` and 1 to `ROAD-68-F`. 22 `ROAD-68-F` furniture, vegetation and
  site tests pass on span sections and were re-enabled; 1 moved to `ROAD-68-C`. See the
  [disabled tests](#road-68-a-disabled-tests).
- Clippy (`--all-targets`) reports the same 59 warnings as the legacy tree. `cargo doc` reports no
  missing docs.
- `./run.sh --test` (isolated profile, no display) exits 0: the Kuopio replay checks,
  `cargo check --benches`, `cargo test` (the library as above, and `tests/growth_scenario.rs` with
  its 3 re-enabled tests: 4 passed) and the Godot bridge suites, including the re-enabled
  `zoning_road_tool_test`, with no script errors. Five scripts stay skipped until their gates; the
  two Forward+ vegetation tests need a display and did not run.

Benchmark (`roadbed_benchmark`, group `roadbed_span`, 2026-10-23): release build, i9-12900K,
pinned to the P-cores, `RAYON_NUM_THREADS=4`, no other build or benchmark running. `build/*` is
one isolated `RoadbedSystem::build_span_piece` on an edge between two terminal nodes with 12 m
knots on a 2.5% slope (sections, mesh, chains, grid and chunk footprint). `publish/span_100m_t_arm`
marks one 100 m arm of a T junction dirty and runs `compile_dirty` (closure, shared-pool build,
index and chunk update).

```bash
cargo bench --bench roadbed_benchmark --no-run
RAYON_NUM_THREADS=4 METRUM_DEBUG=0 taskset -c 0,2,4,6,8,10,12,14 $BENCH_BINARY --bench --noplot
```

| Case | Median |
| --- | --- |
| `build/span_25m` | 5.82 µs |
| `build/span_100m` | 16.74 µs |
| `build/span_400m` | 60.60 µs |
| `build/span_100m_curved` | 16.77 µs |
| `build/span_100m_four_lane` | 17.08 µs |
| `build/span_100m_rural` | 12.21 µs |
| `build/span_100m_footpath` | 4.44 µs |
| `build/span_100m_bridge` | 8.97 µs |
| `publish/span_100m_t_arm` | 37.56 µs |

A 100 m span builds in 0.017 ms against the 0.05 ms target.

Spec corrections in `ROAD-68-B`:

- **Query cells.** Point, ray and overlap queries map XZ to pieces through a fixed 32 m query-cell
  index (the legacy query-chunk span), kept beside the render-chunk index. Render chunks are 512 m
  in the game, so a lookup through them alone would test every piece of a 512 m chunk. A piece is
  listed in every render chunk and query cell that its section-pair bounds touch, padded 1 cm for
  the renderer's f32 rounding: a superset of its triangles' centroid home chunks, so a chunk
  rebuild always sees every triangle homed there.
- **Mesh groups.** `PieceMesh` groups triangles by band kind (all eight kinds), then curb faces,
  instead of five fixed layers. The renderer maps every kind other than carriageway and curb to the
  sidewalk layer, as the legacy renderer did.
- **Changed chunks.** Compiles accumulate their changed chunks until the mesh publisher takes them
  (`changed_chunks`, `clear_changed_chunks`); two compiles before one mesh publish no longer lose
  chunks. `compile_dirty` also returns the chunks it changed, for vegetation invalidation.
- **Mouth stations.** Until `ROAD-68-C` they are the graph clips, which are metres along
  `geometry` (the `Edge` doc comment said fractions and is corrected), used as stations along the
  physical profile's 3-D length as the legacy section model did.
- **Dirty closure.** Until node pieces exist it is the dirty edges plus the edges incident to dirty
  nodes.
- **Lane markings** start and end relative to the span's first and last sections, which now sit at
  the mouths instead of the edge ends.
- **Bridges and tunnels** get full span decks. The legacy tunnel visible-range clipping returns
  with portals in `ROAD-68-F`.

### `ROAD-68-C` — junction layout and node pieces

1. `roadbed/arms.rs` and `roadbed/layout.rs`: arms, sectors, corners, round joins, mouth stations,
   fitting and `LayoutError`.
2. Graph integration:
   - clips and the profile hard zone come from the layout
   - add terminal planes
   - remove `junction_profile_crossing_core_m` and the graph's dependency on roadbed half-width
     helpers
3. `roadbed/node.rs`: the carriageway core through Spade, raised strips, curb faces and shared
   mouth sections, for `JunctionN`, `Bend` and `Terminal`.
4. Layout errors become placement rejections with reasons in the road tool.

Acceptance:

- fixtures, flat and at a 2.5% slope: T, four-way, five-way and six-way; Y junctions at 25°, 40°
  and 60°; bends from 10° to 170°; a terminal; mixed two- and four-lane arms
- node and span pieces are watertight across their shared mouths
- node heights are bitwise `P + level`
- output does not change with edge insertion order
- neighbouring junctions on a short edge either fit or fail with `CornerDoesNotFit`
- rebuilds stay local
- a new `roadbed_junction` Criterion benchmark meets the isolated targets, with its worker count
  recorded

#### Layout module (part 1)

`roadbed/layout.rs` builds a `JunctionLayout` from abstract arms. Each `ArmInput` carries:

- a key (edge index and end)
- centreline knots oriented away from the node, with left lateral axes and stations
- left and right bands outward from the centreline, with the core bands marked
- `max_station_m`
- a crossable flag

The module does not read the graph or spans. Part 2 feeds it from the section frame and wires
its products into clips, the profile solve and node pieces. Construction follows the
[junction layout](#junction-layout) rules, with these choices:

- The corner centre is the crossing of the two offset curves with the smallest `max(sa, sb)`,
  over stations up to each arm's `max_station_m`; when there is none, the first crossing that
  extends an arm's first offset segment behind the node (part 2, open question 6).
- Straight sectors narrower than `pi` meet in a miter on the sector bisector
  ([open question 5](#road-68-open-questions)).
- Bands beyond the carriageway edge match by position outward. A band on one side only tapers to
  zero width. Matched bands must agree in kind and level, or layout fails with `BandMismatch`;
  footpaths meeting roads (D5) arrive in `ROAD-68-F`.
- Fitting bisects `R` in 20 steps between `R_min` and `R` (D1: 3.5 m since the follow-up, 6 m at
  the gate) and keeps the largest radius that fits.
- The core loop check uses exact orientation predicates (`robust`, already built for Spade), so
  it agrees with the triangulation that consumes the loop.
- Typed errors: `NoArms`, `NonFiniteNode`, `NonFinite`, `DuplicateArm`, `DegenerateArm`,
  `InvalidBands`, `DuplicateDirection`, `BandMismatch`, `ArmTooShort`, `CornerDoesNotFit` and
  `CoreNotSimple`.

Layout constants:

| Constant | Value | Reason |
| --- | --- | --- |
| `CURB_RETURN_RADIUS_M` | 3.5 m (6 m at the gate) | D1, at the carriageway edge |
| `CURB_RETURN_CLEARANCE_M` | 0.5 m | D1: `R_min` is the corner's raised width plus 0.5 m |
| `CROSSWALK_DEPTH_M` | 3 m | D8; the zebra strip, whose crossing line is the middle of the depth |
| `MIN_MOUTH_STATION_M` (`s_min`) | 1 m | keeps every mouth off the node's lateral line, where round joins start |
| `STRAIGHT_SECTOR_EPS_RAD` (`eps`) | 1° | covers the direction noise of a split straight road (f32 knots, below about 1e-3 rad anywhere in a 20 km world); a visually straight width change is a radial step (D4) |
| `MIN_SECTOR_ANGLE_RAD` | 1° | consecutive arms closer than this fail with `DuplicateDirection` |
| `MAX_SAMPLE_SEGMENT_M` | 0.5 m | longest sampled boundary segment |
| `MAX_SAMPLE_ANGLE_RAD` | 10° | largest turn per boundary sample |
| `RADIUS_BISECTION_STEPS` | 20 | resolves any range below 4 m to under 4 µm |
| arm direction run | 1 m, at least 5 cm | as `validate.rs` measures it |

Tests (`roadbed/layout/tests.rs`, 20):

- invariance to arm input order (bitwise) and to rotating and moving the junction
- mirrored layouts for a symmetric cross and Y
- corner endpoints bitwise equal to the offset-curve points
- concentric arcs for straight equal arms
- the 3 m crosswalk extension and `s_min`
- `R` shrinking to fit, and `CornerDoesNotFit` on short arms
- `CoreNotSimple`
- degrees 1 to 6; arm angles of 25°, 40°, 60°, 90° and 180°; bends from 10° to 170°
- the terminal round cap
- two-lane meeting four-lane arms, with a tapered sidewalk
- the straight width-change step
- nearly straight sectors from 1e-9 rad to 0.9°
- curved arms
- obtuse unequal-width corners, extended backwards (part 2)
- the typed errors

Every fixture also checks exact endpoint copies, sample counts, mouth limits, a counter-clockwise
core and the provenance of every core vertex.

Layout timings, measured fresh on 2026-10-23 with `roadbed_layout_benchmark`:

- **Build:** release, sources of `a7b98f04`
- **Machine:** i9-12900K, pinned to P-cores
- **Workers:** one thread; the layout uses no Rayon
- **Method:** Criterion defaults, 100 samples, and no overlapping build or benchmark (exclusive
  timing lock)
- **Fixtures:** straight two-lane arms with sidewalks and crosswalks, 6 m knots and 50 m of reach

```bash
cargo bench --bench roadbed_layout_benchmark --no-run
taskset -c 0,2,4,6,8,10,12,14 $BENCH_BINARY --bench --noplot
```

| Case | Samples | Layout time | Share of the isolated node-piece target |
| --- | --- | --- | --- |
| `layout/t` | 126 | 7.06 µs | — |
| `layout/four_way` | 240 | 12.39 µs | 12% of 0.1 ms |
| `layout/six_way` | 486 | 28.24 µs | 9% of 0.3 ms |

The node-piece mesh and its CDT come in part 2, inside the same targets.

#### Graph integration and node pieces (part 2)

Result (done 2026-10-24; the rendered screenshots are added separately): every bend, junction
and road terminal gets a node piece, so junction areas render again, and spans run between the
mouth sections their node pieces evaluate. `roadbed/` holds 5,784 production lines (tests,
fixtures and test-only helpers excluded).

- **Arms** (`roadbed/arms.rs`): one `ArmInput` per live incident edge end. Knots are the span's
  centreline points (physical profile, else control polyline) walked away from the node, exact
  duplicates dropped; stations are horizontal metres; laterals are the span frame's (centred
  secant at interior knots, segment direction at the ends) turned left of the outward direction.
  Bands come from `RoadProfile`: at an edge's start the layout's left is the positive lateral
  side, at its end both swap. A node has no layout when it is merged into another, has no surface
  arm, is a pass-through pair, has a footpath arm (D5, `ROAD-68-F`) or is a bridge or tunnel
  terminal (`ROAD-68-F`).
- **Mouth sections**: one per arm, evaluated by the node on the arm's offset curves at its mouth
  station (`ArmInput::offset_point(s, ±l)`), with every height `P(x, z) + level`. The span and
  the node piece copy the same `Arc<MouthSection>`; spans at pass-through nodes evaluate their
  own, as in `ROAD-68-B`. When an edge's two mouths are within 1 mm, there is no span and the end
  node copies the start node's section. The mouth's span station is the span model's `f32` 3-D
  station of the same centreline point.
- **Heights**: `P` is the solved junction plane (`solved_junction_endpoint_profile_plane`) at bends
  and junctions and the terminal plane at terminals. A finalized centreline more than 1 mm from
  `P` at a mouth fails the node with `NodeError::MouthOffPlane`.
- **Node piece** (`roadbed/node.rs`): the carriageway core is the layout's core loop with each
  mouth row's interior carriageway vertices inserted, fanned from the node when the node sees
  every loop edge (exact orientation) and triangulated by Spade's CDT otherwise; one strip per band
  per sector (sample quads) and per arm side (zipped by station from the sector to the mouth);
  curb faces along every level change except mouths; one outer chain per sector, from one mouth
  corner to the next with the road on the left. Vertices are shared by exact position.
- **Graph**: `graph/rebuild/clips.rs` sets each clip to the arm's layout mouth station at bends and
  junctions (zero at terminals, pass-through nodes and failing nodes); the width padding and acute
  heuristics are gone. The edit pipeline runs clips before the profile solve, whose hard zone is
  the clip (`junction_profile_crossing_core_m` is removed). Load repair recomputes the clips of
  failing nodes before re-solving them.
- **Rejections**: placement validation checks every affected node on the post-edit excerpt
  (`roadbed::check_node`) and rejects with `junction_corner_does_not_fit`,
  `junction_core_not_simple`, `junction_band_mismatch`, `junction_arm_too_short`,
  `junction_profile_mismatch` or `junction_layout_invalid`, for preview and commit alike; a node
  that still fails at compile rolls the commit back as before.
- **System**: the dirty closure is the dirty edges, their nodes (and a reconnected edge's previous
  nodes), and those nodes' incident edges. Nodes plan first (arms, layout, plane, mouths), then
  node pieces and spans build on the shared pool and publish in piece order. `failed_nodes` and
  `node_error` report failures, and a failed node has no piece. It publishes as a gap with a
  `road_node_failed` diagnostic, never as its previous mesh (open question 11). Queries, raycasts,
  `pieces_in_bounds`, chunks and generations cover node pieces; `lane_query` answers junction
  lanes from their node piece; the renderer emits
  node pieces under their node owner with world UVs (junction asphalt keeps `road_color`).

Tests (`roadbed::node::tests`, fixtures flat and at a 2.5% slope: T, four-, five- and six-way;
Y at 25°, 40° and 60°; bends of 10° to 160°; mixed four-lane through roads with a two-lane side
road at 40°, 60° and 90°; a straight width step; every arm ending in a terminal):

- every node builds its piece (`Bend`, `Junction`, `Terminal`) and every edge its span
- node pieces are watertight (every edge borders two triangles except outer-chain and mouth
  edges), every open mouth edge is an open mouth edge of the span there bitwise, spans hold the
  node's mouth section, and every curb face looks at ground level
- every node vertex is bitwise `P(x, z) + level`
- every mouth vertex is bitwise `offset_point(mouth station, ±l)` on the layout's arm
- node pieces do not change with edge insertion order or direction
- two junctions on a 10–40 m link either fit within half of the link each or fail with
  `CornerDoesNotFit`
- an edit rebuilds exactly its closure
- layout: obtuse corners between two- and four-lane arms at 120–160°, and a four-lane through
  road with a two-lane side road at 40°, 60° and 90°

Checks (fresh, on the gate sources):

- Debug library suite (`RAYON_NUM_THREADS=4`): 1,797 passed, 0 failed, 187 ignored
  (`ROAD-68-B`: 1,767, 0, 189). `ROAD-68` ignores: D 25, E 2, F 7 (tables above).
- Clippy (`--all-targets`) reports the same 59 warnings as the legacy tree. `cargo doc` reports no
  missing docs.
- `./run.sh --test` (isolated profile, no display) exits 0: the Kuopio replay checks,
  `cargo check --benches`, `cargo test` (the library as above and `tests/growth_scenario.rs`: 4
  passed) and the Godot bridge suites with no script errors, including the re-enabled
  `zoning_reference_test` (8 layouts). Four scripts stay skipped until `ROAD-68-E` and
  `ROAD-68-F`; the two Forward+ vegetation tests need a display and did not run.

Benchmark (`roadbed_benchmark`, 2026-10-24): release build, i9-12900K, pinned to the P-cores,
`RAYON_NUM_THREADS=4`, Criterion defaults with 200 samples, no other build or benchmark running.
`roadbed_junction/build/*` plans and builds one node piece from the graph (arms, layout, plane,
mouth sections, core, strips, curb faces, chains, grid, footprint); arms are 100 m two-lane roads
with sidewalks and crosswalks, 12 m knots, on a 2.5% slope. `publish/t_closure` marks a T's node
dirty and compiles its closure: the node piece and its three spans.

```bash
cargo bench --bench roadbed_benchmark --no-run
RAYON_NUM_THREADS=4 METRUM_DEBUG=0 taskset -c 0,2,4,6,8,10,12,14 $BENCH_BINARY --bench --noplot
```

| Case | Median | Target |
| --- | --- | --- |
| `roadbed_junction/build/t` | 41.35 µs | — |
| `roadbed_junction/build/four_way` | 70.80 µs | 100 µs |
| `roadbed_junction/build/six_way` | 127.22 µs | 300 µs |
| `roadbed_junction/publish/t_closure` | 131.74 µs | — |
| `roadbed_span/build/span_100m` | 16.38 µs | 50 µs |
| `roadbed_span/publish/span_100m_t_arm` | 134.81 µs | — |
| `layout/four_way` (`roadbed_layout_benchmark`) | 12.26 µs | — |

The first build measured 223 µs (four-way) and 473 µs (six-way). A profile showed 43% in Spade's
exact incircle fallback: the corner samples are cocircular, so every Delaunay flip near a corner
took the exact path. Fanning the core from the node when it sees every core edge, a shared
`f64`-bits hasher for vertex deduplication and one id lookup per sector sample brought them under
target. `publish/span_100m_t_arm` grew from 37.6 µs because one dirty arm now rebuilds its two
nodes and the T node's three spans.

Spec corrections in `ROAD-68-C`:

- **Obtuse unequal corners** search backwards along each arm's first offset segment when the
  curves do not cross ahead of the node ([open question 6](#road-68-open-questions)).
- **Core triangulation**: the core is fanned from the node when the node sees every core edge,
  and only otherwise goes through Spade's CDT. Both are exact triangulations of the same loop;
  the fan avoids the CDT's exact incircle path on the cocircular corner samples.
- **Mouth sections** are evaluated by node pieces on the arms' offset curves rather than by the
  span's section function, so `offset_point(mouth, l)` and the stored row are the same values.
- **Clips** are horizontal metres along the roadbed centreline, the layout's station measure;
  terminals keep a zero clip, so lanes still run onto the terminal cap.
- **Terminal planes** run through the road end and the mouth centreline point and are read from
  the solved profile; the finalizer does not re-solve terminals.
- **Mouth knots**: the finalizer inserts a knot at each pinned mouth (control polyline of an
  adapted arm, physical profile of a stable one), and a stable arm without new supports keeps its
  physical knots, so the other end's pin survives.
- **Width tapers** apply only at pass-through nodes; node pieces own width changes at bends and
  junctions.
- **Crossable arms** (D8) in this gate: car-road arms with sidewalks at junctions of three or more
  arms, bend arms only where the node forces a crossing, none where a node override removes it.
  At the gate, pedestrian lanes still placed crossings by the `ROAD-64` lane rule; the
  [follow-up](#tighter-corners-and-junction-crosswalks-follow-up) moved them onto the layout.
- **Nodes without pieces**: footpath nodes and footpaths meeting roads (D5), and bridge and tunnel
  terminals, keep the `ROAD-68-B` span ends until `ROAD-68-F`.

#### Rendered reference screenshots

**Status:** done 2026-10-24, release build `ae9c8d4b`.

**Test and fixtures.** `godot/tests/road_junction_reference_test.gd` builds scripted fixtures through the real
`SimulationNode`, the production road chunk renderer and the terrain mesh. Every road is 1+1 lanes with
sidewalks and 80 m arms, in a 512 m synthetic world, flat and on a 2.5% slope rising toward +x:

- T
- four-way
- Y at 25°, 40° and 60° (the branches leave the stem's end at ±angle/2)
- a 45° bend
- a terminal
- a T whose through road is 2+2 lanes

**Views.** Each fixture is captured at a fixed 1600×1000 in three views:

- oblique, with terrain
- the same oblique camera with the terrain node hidden, so the ground-level carriageway is not
  hidden by the coplanar unclipped terrain until `ROAD-68-D`
- a top-down orthographic plan, 60 × 37.5 m, with terrain hidden

**Results.** The 48 images and the manifest report no fixture failures.

- Corners are rounded and sidewalks wrap them continuously.
- Straight sides stay straight.
- Mouths are seamless on both slopes.
- Curb faces run along every carriageway boundary.
- Terminals get the round cap (D2).
- The 25° Y's inner corner lies about 20 m from the node, as its angle requires.

**Run command.** Use an isolated profile, and render on gamescope's headless GPU backend so the
run does not depend on the physical display:

```bash
env -u WAYLAND_DISPLAY -u DISPLAY XDG_DATA_HOME="$ARTIFACTS/profile/data" \
  XDG_CONFIG_HOME="$ARTIFACTS/profile/config" \
  METRUM_ROAD_JUNCTION_REFERENCE_DIR="$ARTIFACTS/screenshots" \
  gamescope --backend headless -W 1600 -H 1000 -- \
  godot --path godot --script res://tests/road_junction_reference_test.gd
```

The profile also needs a unique `config/name` override, as `./run.sh --test` builds one. Do not
use `xvfb-run`: without DRI3, Godot falls back to the session's real Wayland display and stalls
while the monitor sleeps.

**Known limits (cosmetic).** Junction asphalt carries none of the span asphalt's crack and patch
detail, so the mouth line is faintly visible. Lane lines end at the mouths. Terrain shows through
or over roads in the terrain views until `ROAD-68-D`, and the preview stays empty until
`ROAD-68-E`.

#### Tighter corners and junction crosswalks (follow-up)

**Status:** done 2026-10-24, after the user reviewed the gate's screenshots.

The user made two decisions:

- D1: the curb-return radius is 3.5 m at the carriageway edge.
- D8: crossings sit right outside the curve. Open question 10 is resolved as "keep the layout
  rule and move the lanes", including the bend rule.

This pulls the crossings item of `ROAD-68-F` forward.

**Layout.**

- `CURB_RETURN_RADIUS_M` is 3.5 m. A right-angle corner of two-lane roads now has its tangent at
  7 m and its mouth at 10 m; at the gate they were at 9.5 m and 12.5 m.
- Each crossable arm carries a `Crosswalk`:
  - `inner_station_m`: the larger tangent station, where the curbs stop curving
  - `station_m`: the crossing line, the strip's middle
  - `kerbs`: the two carriageway-edge points on that line, from `ArmInput::offset_point`
- The strip's outer edge is the mouth.

**Node pieces.**

- `NodePiece::crosswalk_kerbs(key)` publishes the kerb points on the node plane through
  `node::crossing_kerbs`. Lanes use the same function.
- `NodePiece::crosswalk_approach(key)` is the arm's section at the strip's inner edge. Road signs
  at a zebra stand on the span sections extended by it, so the sign for traffic leaving the
  junction stands between the corner and the zebra.

**Lanes (`roadbed/walkways.rs`, `lanes/pedestrian_junctions.rs`).**

- The lane rebuild lays out the nodes it touches with `NodeWalkways::collect`. This is the same
  graph-to-layout function node pieces use (`NodeArms::collect`, `NodeArms::layout`), so lanes
  and pieces agree bitwise whichever of the lane rebuild and the roadbed compile runs first.
  Nothing else changes order.
- A full rebuild lays out every node on the shared pool. An incremental rebuild lays out only the
  nodes of its closure.
- Sidewalk lanes run on the profile's walkway centre: midway between the carriageway edge and the
  sidewalk's outer edge, which is the old line for consistent edges.
- At a layout node a sidewalk lane ends exactly on the layout point:
  - the crossing line on a crossable arm
  - otherwise the mouth
- Crossings are the layout's strips. Corner routes follow the walkway centre around each sector,
  and a road end's route wraps its cap.
- Nodes without a layout have no crossing (no span zebras). There, neighbouring sidewalks join
  straight, or round the node where the straight walk would cut into the asphalt.
- `rounded_sidewalk_corner_path_xz` and the `cavalier_contours` dependency are removed.
  `config::CROSSWALK_INSET` became `SIDEWALK_NODE_INSET`, used only at nodes without a layout.
- Lane markings on spans no longer leave a crosswalk gap: the zebra is on the node piece.

**Other changes.**

- **Crossing toggles.** A crossing moves its arm's mouth, so `set_crosswalk_override_internal`
  now re-lays the node out like an edit:
  - recompute the clips
  - re-pin the arms in the profile solve (every arm stable)
  - mark the node's roadbed closure dirty
  - rebuild its lanes
  - record undo for every incident edge
  A toggle whose layout fails is refused.
- **Loading older saves.** A save from before this change holds pedestrian connectors the layout
  no longer routes (crossings at bends, pass-through nodes and road ends). Loading omits them, and
  walkers on them replan. Vehicle connectors must still match.

**Consequence.** Walkers now change sides only on crossings ([open question
12](#road-68-open-questions)).

**Tests.** `two_way_zebra_has_two_opposite_right_hand_signs_at_the_kerb` and
`sidewalks_through_bends_between_curved_roads_stay_on_the_sidewalk` pass again. The second now
checks every walk point against the compiled sidewalk band, because its centreline-distance proxy
fails inside an acute curb return. New tests:

- `lanes::tests::crossings`, on T, four-way, six-way, 40° Y and forced 45° bend fixtures, flat and
  at 2.5%:
  - zebras fill the straight approach right outside the corners (inner edge at the tangent,
    mouth 3 m out, strip on the carriageway), with no other markings
  - lane kerb points are the node piece's, bitwise
  - sidewalk lanes end on the crossing line, and both directions walk the marking's kerb segment
  - corner walks stay on the sidewalk band
  - full and local rebuilds repeat the hub's connectors bitwise
- `pedestrian_route_the_layout_no_longer_has_is_dropped_and_its_walker_replans` (save load)
- `toggling_a_crossing_moves_its_mouth_and_lanes_follow_the_node_piece` (`SimCore`): the clip
  moves 3 m, and lanes publish exactly the node piece's crossings after each toggle

Updated for the new rule:

- `test_crosswalk_counts`: no crossing at a road end or a pass-through node
- the clip fixture: mouth 10 m
- the layout's shrink fixture: 9.5 m limit, `R` = 3 m
- `incremental_rebuild_repairs_connectors_at_expanded_edge_far_end` (a forced bend crossing)
- `test_pedestrian_crosses_junction` (now a T)
- `prepared_commute_queries_match_connector_paths_without_growing_storage` (forced bend
  crossings)
- the height check in `incremental_lane_rebuild_is_lifted_without_the_renderer`
- the sign-shift test (approach sections)

`ROAD-68` ignores: D 25, E 2, F 5.

**Checks** (fresh, on the follow-up sources):

- Debug library suite (`RAYON_NUM_THREADS=4 METRUM_DEBUG=0`): 1,806 passed, 0 failed, 185 ignored
  (gate: 1,797, 0, 187).
- Clippy (`--all-targets`) reports only the 59 legacy-tree warnings, and `cargo doc` reports no
  missing docs.
- `./run.sh --test` (isolated profile, no display) exits 0:
  - the library suite as above, and `tests/growth_scenario.rs` with 4 passed
  - the Godot bridge suites, with no script errors and `zoning_reference_test` passing (8 layouts)
  - the same four scripts skipped (`ROAD-68-E` 3, `ROAD-68-F` 1); the Forward+ vegetation tests
    need a display and did not run
- `roadbed/` holds 6,136 production lines (gate: 5,784), under the 8,000 cap.

**Benchmarks** (2026-10-24):

- Build: release, with the gate's `roadbed_benchmark` binary as a control in the same session.
- Machine: i9-12900K, pinned to the P-cores, `RAYON_NUM_THREADS=4`.
- Method: Criterion defaults with 200 samples, under the exclusive timing lock with no build
  running; the new binary ran twice.
- Workload: the gate's fixtures, so the node pieces now carry 3.5 m corners.

| Case | Gate (control) | Follow-up | Target |
| --- | --- | --- | --- |
| `roadbed_junction/build/t` | 41.22 µs | 28.11 / 28.35 µs | — |
| `roadbed_junction/build/four_way` | 70.92 µs | 46.39 / 46.91 µs | 100 µs |
| `roadbed_junction/build/six_way` | 126.54 µs | 81.37 / 81.51 µs | 300 µs |
| `roadbed_junction/publish/t_closure` | 114.27 µs | 98.57 / 92.77 µs | — |

The smaller radius cuts the corner samples. The crosswalk approach sections, one per crossable
arm, cost less than that saves.

**Lane rebuild locality.** `lanes::tests::rebuild_benchmark::incremental_lane_rebuild_scaling` is
a release test, run with `--ignored`. It holds a three-edge closure fixed and adds 0 to 100,000
unrelated roads. The gate and follow-up binaries ran alternately, two rounds each, in the same
session; the gate's first, cold round is discarded. The fixture's bend now has a forced crossing;
at the gate it had the legacy degree-two crossing.

| Background roads | Incremental median, gate | Incremental median, follow-up | Full-rebuild setup, gate | Full-rebuild setup, follow-up |
| --- | --- | --- | --- | --- |
| 0 | 0.0127 ms | 0.0214 ms | 0.30 ms | 0.29 ms |
| 1,000 | 0.0130 ms | 0.0213 ms | 2.6 ms | 4.0 ms |
| 10,000 | 0.0129 ms | 0.0203 ms | 26 ms | 39 ms |
| 100,000 | 0.0134 ms | 0.0201 ms | 270 ms | 425 ms |

- The local rebuild stays flat as the city grows. It costs about 8 µs more, because it now lays
  out the closure's nodes; a closure is laid out sequentially, since dispatching a handful of
  nodes to the pool costs more than laying them out.
- A full rebuild lays out every node on the shared pool. That is about 0.8 µs per node: the
  background roads add 200,000 terminals.

```bash
cargo bench --bench roadbed_benchmark --no-run
RAYON_NUM_THREADS=4 METRUM_DEBUG=0 taskset -c 0,2,4,6,8,10,12,14 $BENCH_BINARY --bench --noplot roadbed_junction
cargo test --release --lib --no-run
RAYON_NUM_THREADS=4 METRUM_DEBUG=0 taskset -c 0,2,4,6,8,10,12,14 $TEST_BINARY --ignored --nocapture \
  --exact simulation::network::lanes::tests::rebuild_benchmark::incremental_lane_rebuild_scaling
```

**Screenshots.** The [reference harness](#rendered-reference-screenshots) re-ran on the follow-up
debug build (48 images, no fixture failures). The T, four-way and Y plan views show:

- tighter corners
- each zebra on the node piece's asphalt right outside the curb returns, ending at the mouth where
  the span begins
- no zebra on any span

The 45° bend has no crossing, as the bend rule says.

### `ROAD-68-D` — footprint and terrain

1. `roadbed/terrain.rs`: patch loops from outer chains, the grading envelope and patch selection.
2. `terrain/cdt/model.rs`: the `RoadBoundary` source variant, with source ordering simplified to
   match.
3. Rewire `nodes/simulation_node/variant_export/cdt/*` and `async_terrain/*`. Simplify the CDT
   source labels in Godot's `terrain.gd` debug display.

Acceptance:

- no terrain face centroid lies inside the footprint, checked with the roadbed query
- seam vertices are bitwise equal to the road's outer edge
- closed blocks produce holes
- seams across patch boundaries are continuous
- the road–site shared-vertex rule holds
- the existing CDT suite passes
- per-patch timing is recorded, and the T fixture's patches total at most 2 ms on four workers

### `ROAD-68-E` — preview equals commit equals undo, in one frame

1. The versioned tool context and `preview_road`. The context also fixes the hover core-lock bug.
2. Fingerprinted commit adoption, undo records and the load rebuild.
3. Godot:
   - `road_tool.gd` calls the synchronous preview from its early callback
   - delete `road_preview_publisher.gd`, the request-ID plumbing and the coarse preview ribbon
   - merge `road_junction_preview.gd` into the road tool's preview display or delete it
   - keep the reversible terrain staging in `road_terrain_preview.gd`
   - apply [D3](#product-decisions)
4. Update `network_tool_chunk_renderer_test.gd` and `road_preview_stream_test.gd` for the
   synchronous path.

Acceptance:

- preview pieces, terrain buffers and payloads are bitwise equal to the committed ones
- a commit with stale dependencies recompiles and equals a fresh compile
- undo restores bitwise
- save → load reproduces bitwise-identical pieces
- the Godot bridge suites pass
- native per-stage timings are within budget on `sloped_t` and `sloped_branch`

### `ROAD-68-F` — remaining families

- footpaths: footpath-only nodes, and footpaths meeting roads ([D5](#product-decisions))
- roads without sidewalks, and mixed widths and profiles, including the D4 taper
- bridges: decks through the existing bridge renderer, and abutment cutouts
- tunnels: portals; this covers `ROAD-61`
- crossings: done early, in the [`ROAD-68-C` follow-up](#tighter-corners-and-junction-crosswalks-follow-up)
  (crossings and sidewalk corners from the layout; `rounded_sidewalk_corner_path_xz` removed).
  Footpath nodes (D5) still need layouts, and with them crossings and corner walks of their own
- lane markings at junctions, street lights and road signs
- building sites, driveways and paving
- zoning, agriculture and vegetation overlap queries

Acceptance:

- focused fixtures for each family
- the [crossing contract](#pedestrian-junction-crossings): a zebra marking and its pedestrian
  connectors share one kerb segment
- deterministic street-light and sign placement
- the building frontage tests are re-enabled

### `ROAD-68-G` — commit consequences inside one frame

This gate is independent of `ROAD-68-C` to `ROAD-68-F` and may start once `ROAD-68-A` lands.

1. Measure the commit breakdown on the populated fixture at 0, 1k, 10k and 100k buildings: graph
   mutation, lanes, zoning and parcels, roadbed install, terrain install, render publication, CCH
   and flow fields.
2. Implement [D7](#product-decisions) in the scheduler ([`simulation.md`](simulation.md)) and in
   the routing contract ([`entrance_and_exit.md`](entrance_and_exit.md)). Flow fields refresh after
   adoption.

Acceptance:

- click → committed generation installed and acknowledged in under 16.7 ms at 100k buildings
- routing-adoption latency reported separately
- agent records bitwise equal across worker counts and with an injected rebuild delay

### `ROAD-68-H` — acceptance and cleanup

1. The [rendered runs](#targets), with the simulation paused and active, and the populated
   locality check.
2. Remove the remaining dead paths and every `ROAD-68` ignore. Update references to legacy types
   in [`terrain.md`](terrain.md) and [`earthworks.md`](earthworks.md). Record final line counts.

## Measurement and acceptance

### Baseline (`ROAD-67-A`, legacy compiler)

Fresh rendered baseline, 2026-10-23:

- **Build:** release, source `b75e03c9`
- **Machine:** i9-12900K, RX 7900 XTX / RADV, Godot 4.7.2 Forward+, X11
- **Display:** 1920×1080, VSync, 60 FPS cap
- **Workers:** four Rayon workers
- **Simulation:** paused
- **World:** synthetic 512 m, 2.5% slope in the sloped cases

| Fixture | Stationary input-to-frame p50 / p95 / max, ms | Moving displayed-age p50 / p95 / max, ms | Exact deadline met (schema 2) |
| --- | --- | --- | --- |
| `sloped_t` | 57.02 / 63.91 / 75.92 | 53.10 / 69.69 / 87.05 | 0 / 380 |
| `sloped_branch` | 57.24 / 77.10 / 96.32 | 74.01 / 103.64 / 185.54 | 0 / 370 |
| `flat_cross` | 74.50 / 77.19 / 96.06 | 102.89 / 136.82 / 170.52 | not run |
| `sloped_cross` | 91.62 / 109.15 / 126.31 | 120.18 / 186.92 / 286.64 | not run |

Isolated legacy junction compile medians: flat / sloped T 14.07 / 19.79 ms, flat / sloped cross
21.54 / 22.91 ms, sloped oblique 36.36 ms, five-way 101.02 ms.

The full method, binary hashes, collector schema and diagnostic stage split are in the
[archive](archive/roads_legacy_surface_2026-10-23.md#first-baseline-and-compiler-decision-2026-10-23).

### Targets

These apply in road-and-terrain mode on the reference machine and settings, to `sloped_t`,
`sloped_branch`, `flat_cross`, `sloped_cross` and the dense preview fixture:

- **Moving.** With the simulation paused, every sampled pose's exact result renders within 16.7 ms
  of sampling. With the simulation active on the populated fixture, at least 99% do, and the report
  attributes every miss. A stale or superseded result counts as a miss.
- **Stationary.** Input-to-frame p99 under 16.7 ms, and stop to exact frame under 16.7 ms.
- **Commit.** Click → committed generation acknowledged in under 16.7 ms, with no frame that shows
  neither the preview nor the committed road.
- **Locality.** With the edited neighbourhood held fixed, grow remote buildings, parcels, agents
  and roads from 0 to 100k. Local products stay bitwise identical and per-pose timings stay flat.
  Context refresh is reported separately.

### Method

Reuse the schema-2 deadline collector, the gameplay road benchmark and the native fixtures.

- Run matched, unprofiled release processes with no overlapping builds or benchmarks, pinned to
  P-cores.
- Record hardware, settings, worker counts, extension hash and exact commands, and keep captures
  under `$ARTIFACTS`.
- `frame_post_draw` is a rendered boundary, not proof of physical presentation. Headless timings
  cannot establish rendered acceptance.

```bash
RUSTC_WRAPPER= cargo build --release --manifest-path rust/Cargo.toml
RAYON_NUM_THREADS=4 METRUM_DEBUG=0 METRUM_DEBUG_PERF=0 \
METRUM_GAMEPLAY_BENCHMARK_MATRIX=preview \
METRUM_GAMEPLAY_BENCHMARK_CASES=sloped_t,sloped_branch \
METRUM_GAMEPLAY_BENCHMARK_REPETITIONS=100 \
METRUM_GAMEPLAY_BENCHMARK_WARMUP_REPETITIONS=8 \
METRUM_GAMEPLAY_BENCHMARK_MAX_FPS=60 \
METRUM_GAMEPLAY_BENCHMARK_OUTPUT_DIR="$ARTIFACTS" \
METRUM_GAMEPLAY_BENCHMARK_RUN_ID=<fresh-run-id> \
./run.sh --benchmark-gameplay-roads
```

For the cross fixtures, set the cases to `flat_cross,sloped_cross` and use a fresh run ID.

## Pedestrian Junction Crossings

> **`ROAD-68`:** since the [`ROAD-68-C` follow-up](#tighter-corners-and-junction-crosswalks-follow-up),
> the crossings and sidewalk corner curves below come from the [junction layout](#junction-layout)
> (`roadbed::NodeWalkways`), and `rounded_sidewalk_corner_path_xz` is removed. Otherwise this
> contract is unchanged; the legacy lane audit measurements are archived.

One authoritative crossing record drives both pedestrian routing and zebra-marking rendering:

- an arm is crossable if and only if its lane rebuild emits a `CrosswalkMarking`. Lanes take
  the layout's crossable rule ([D8](#product-decisions)): junction arms, and bend arms only where
  the node forces a crossing. A node without a layout has none ([open question
  12](#road-68-open-questions))
- the marking is the layout's zebra strip: its kerb points are the two carriageway-edge points on
  the strip's crossing line, at the node plane's height, bitwise the node piece's
  (`NodePiece::crosswalk_kerbs`)
- the two directional pedestrian connections for that arm traverse the exact same asphalt-edge
  segment stored by the marking
- routes between adjacent arms stay on the sidewalk and must not create direct mouth-to-mouth
  chords through the junction carriageway. They run on the centre of the raised walkway, midway
  between its carriageway-edge and outer boundary curves. From a sidewalk end they follow the
  arm's offset curve back to the sector, then the midpoints of the sector's boundary samples,
  then the next arm's offset curve out to its sidewalk end. A road end's route wraps its cap
- every incoming sidewalk mouth has one precomputed route to each reachable outbound sidewalk
  mouth; ordinary edge routing selects the shortest route to an arm, while exact building access
  can select either sidewalk without falling back to a side-to-side lane reselection
- physical road-sidewalk lanes run on the profile's walkway centre. At a node with a layout they
  end exactly on the layout point: the crossing line of a crossable arm, and the mouth of any
  other arm. Those are the first and last points of their junction connectors, so walkers cannot
  pass a zebra and backtrack. At a node without a layout they stop
  `config::SIDEWALK_NODE_INSET` beyond the clip
- at a node without a layout, neighbouring sidewalks join by a straight walk, or by the
  node-centred arc where that walk would cut into the asphalt of either arm, judged against the
  arm centrelines (`TRAFFIC-04`). `ROAD-68-F` gives footpath nodes layouts
- a same-side reversal uses an explicit stationary connector at the mouth instead of crossing the
  road or reattaching farther along the reverse sidewalk
- a crossing is square to the road where it is drawn (`ROAD-64`). Both kerb points, and the two
  sidewalk ends they join, lie on one section line of the arm, at the strip's crossing station.
  Before `ROAD-64` the sidewalk lanes were trimmed separately and the graph endpoint's normal was
  used, so crossings on curved arms came out skewed

Crossing availability and geometry must not be independently reconstructed by the renderer,
selection queries, or agent movement.

Footpaths (`TransitType::Foot`, `ROAD-55`) join the same junction graph as one-mouth arms. Until
`ROAD-55` the builder skipped them, so their lanes had no junction connectors and walkers could
never plan a route onto one; footpaths had been dead ends since pedestrians moved onto lanes.

- A footpath's lanes stop where the path leaves the paved area of the roads at that node, the
  outer edge of their sidewalks: a ray at angle `d` to a road arm of paved half-width `w` leaves
  it after `w / |sin d|` along the arm (capped at 10°) and after `w` away from it. The trim is a
  pure function of the node's geometry, applied when lanes are built, so full and partial
  rebuilds agree. Where only footpaths meet, each stops 1 m short of the node.
- Between two road arms, the footpath joins the sidewalk corner of that sector at its nearest
  point and follows the corner to the sidewalk on either side. At a single road arm it joins both
  sidewalks directly. Footpaths meeting only footpaths connect through the node.
- A node with a footpath arm has no junction layout until `ROAD-68-F` (D5), so it has no crossing
  and its corners are the straight or node-centred walks of a node without a layout.
- The node records these pedestrian routes as turn rules keyed by the footpath edge, as sidewalk
  routes are keyed by lane index ±100. `RegionGraph::is_vehicle_turn_rule` tells both apart from
  player-set car turn rules, so neither puts a junction into car whitelist mode. Until this was
  fixed, a rebuild that saw a footpath's rule (any full rebuild, and every load) dropped every car
  connector at that junction, and a save with a car on one failed to load ("saved connector …
  has no matching junction route"); pathfinding refused car routes through it too.

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

## Road Geometry Validation (`ROAD-54`)

Placement accepts geometry no road could have. The profile solver treats its 16% grade as a
target and relaxes each span's limit to the terrain slope under it
(`solve_standard_road_profile`), and the preview measures `max_grade` without comparing it to
anything. Nothing measures horizontal radius, kinks where two edges meet at a degree-2 node, or
the angles between junction arms. Six player saves from 2026-10-08 (`road-*.sqlite`,
screenshots in `screenshots/2026-10-08-roads/`) showed, within 250 m of their cameras:

| Save | Worst measured |
|---|---|
| `road-too-steep` | 24-42% grades straight up the fall line |
| `road-too-tilted` | 15-39% grades; 37° and 41° kinks at degree-2 nodes |
| `road-house-yard-broken` | 38-39% grades on a fronted street; 29-30° kinks |
| `road-snowflake` | a degree-10 junction with 20° between arms; a 6.7 m radius on a 7 m road; 77% grade |
| `road-broken-tunnel` | an 82% tunnel opening into a degree-6 junction; portal pieces left floating |
| `road-vertical` | a 108 m two-point bridge at 307% |

`network/validate.rs` measures these read-only, at two tiers: `Illegal` (geometry to reject) and
`Design` (legal, worth a warning). Defaults:

| Check | Design | Illegal |
|---|---|---|
| Grade, `Standard` | > 10% | > 16% |
| Grade, `Bridge` / `Tunnel` | > 6% | > 16% |
| Grade change per horizontal metre | > 0.018 | > 0.06 |
| Horizontal radius | < v² / (127 · 0.20), v in km/h | < 1.25 × visual roadbed half-width |
| Kink at a degree-2 node | > 3° | > 10° |
| Smallest angle between junction arms | < 60° | < 25° |
| Junction degree | > 4 | > 6 |

The illegal tier allows about 20% leeway over common Western design limits, so a city may be a
little more playful than an engineer would draw it, but not mangled: grade change 0.05 became
0.06 and the arm angle 30° became 25°.

The radius is measured over a window of three roadbed half-widths from each vertex, as the
window's horizontal length over its net signed turn. A polyline corner has no radius of its own,
and the profile solver densifies centrelines to 20 cm near junctions, so a per-vertex estimate
read a gentle 17° corner as a 3 m curve. Over the window, a stretch is illegal once it turns more
than about 137°: a hairpin, not a street corner.

Edges are judged on `physical_geometry`. Arm directions at a node use the first metre of each
centreline, without clips, so sub-5 cm end segments do not decide them and a smooth curve split at
a node does not read as a kink. `validate_edge` is `O(points)`, `validate_node` is
`O(degree log degree)` plus the one-metre prefix, and `audit_graph` runs both over the graph with
Rayon in a deterministic order.

Placement refuses what an edit adds (`validate::edit_violation`, called from the preview and
commit validator once the edit's local graph is finalized and before its surface compiles):
illegal horizontal radius anywhere on the drawn road's pieces (the inserted edge and every piece
split off it), and an illegal junction arm angle or degree at their endpoints unless that node was
already as bad before the edit, so a bad junction can still be extended or repaired. The road tool
shows the measured value and the limit. Not refused yet:

- grade and grade change, which wait for the hill policy below: the profile solver still raises
  each span's grade limit to the terrain under it, so refusing them would refuse every steep
  hillside and, on the Kuopio DEM fixture, ordinary roads over bumpy ground (0.067 per metre)
- direction kinks where two roads meet end to end: a street corner the bend piece rounds

An audit of the 58 local saves found no illegal radius; illegal arm angles at 20° Y junctions in
the `simple-city` saves and the `road-snowflake` star; the degree-10 snowflake; and illegal grade
change only on the steep Kuopio hillside roads (0.06 to 0.12 per metre).

Remaining: the hill policy (grade), a debug overlay and preview colouring of design warnings.

## Drawing and placement (`ROAD-63`)

The road tool (`godot/scripts/tools/road_tool.gd`) draws continuously: an accepted road starts the
next stroke at its end, leaving in its end direction, until right-click or Esc, until the road
ends on an existing road or junction, or until it reaches the map edge: a road that raises the
external-connection prompt ends the chain, so the dialog never sits over a stroke still following
the pointer off the map. Each stroke has a start direction from
`get_road_tool_start_direction`, which reads only the edges at the cursor's snap target: a dead end
is continued (signed), a road or degree-two node gives its direction (either sense), free ground
and junctions give none. It replaced the tool's `get_road_tangent_at`, which scanned every edge in
the city on each click.

- Straight mode snaps the stroke every 45° from the start direction (straight on, square, half
  square) when it lies within 4° of one. Snapping runs in Rust inside `get_road_tool_cursor_pos` (`interaction::snap_stroke_angle`),
  after network snapping, which wins, and off while the zoning grid snaps. Shift draws free; the
  label shows `[snapped]`.
- Curve mode keeps the three-click quadratic curve (start, free control point, end) on every
  stroke, continuation included, and does not snap. A curve aimed only by its end point, as a
  constant-radius arc from the inherited direction, was tried and dropped: continuing a chain then
  forced each curve to leave along the previous one, so players could no longer choose the bend.

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

## Road marking lighting — RENDER-12

Lane lines use shared, non-emissive per-pixel lighting with roughness 1.0. Their existing
vertex colours, alpha and geometry remain unchanged. Zebra crossings are asphalt paint (below).
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

Command: `XDG_DATA_HOME=$ARTIFACTS/metrum-road-light RAYON_NUM_THREADS=4 timeout 90 godot --path
godot --script res://tests/road_marking_lighting_test.gd -- --asset-editor --benchmark-markings`.
Vsync is disabled during measurement. Build base: `124be55d13d7eccf02681fc7e906ee43cf1f4a94` plus
this material change; unchanged release extension SHA-256:
`5368fe8e87315e028fad2437e87a26119a0b7b1b7a63ec7b7bf592b8415dbe89`.

## Finnish road surface and markings (`ROAD-56`)

The carriageway draws as worn Finnish stone-mastic asphalt instead of one tiling texture
(`godot/assets/materials/road_asphalt.gdshader`, functions in
`godot/scripts/shaders/road_surface_finnish.gdshaderinc`): speckled granite aggregate in binder
that fades from near black (linear albedo 0.07) to light grey (0.17) with age, coarse ravelled
grain and blotches at about 2 and 6 m, sealed lengthwise and transverse cracks, long trench
patches (bands across the road or one lane, strips along it) each with its own age, small cut
and hand patches, manhole covers, drain grates in the gutters, grime along the kerb, and a
crumbling edge where no kerb hides it. Studded-tyre ruts and polished wheel paths appear only on
busy roads: their strength is 0 on a 1 + 1 road at 50 km/h, rising with lanes beyond two and
speed beyond 55 km/h. Stripes along every lane of a residential street looked wrong against the
Kuopio reference photographs. The material's `light()` adds a GGX sun specular to the paving
diffuse, so the asphalt has a sheen toward a low sun; it had none before. Kerbstones draw as light
granite (linear albedo about 0.4, speckled from the same aggregate texture) instead of a dark
asphalt texture. A kerb face parallel to the sun still reads darker than the kerb top: it gets
only sky light, and the engine has no bounce light from the sunlit road. A one-bounce term
(ground albedo 0.22, view factor (1 - n.up) / 2) added only about a tenth of the sun and was
dropped; most of the old near-black band was the dark asphalt texture. The functions and the 512 px aggregate
texture (`assets/textures/road/finnish/aggregate_packed.png`: R aggregate, GB slopes, A relief;
procedural, no photographs; original, CC BY 4.0) came from Codex look-dev against Wikimedia Commons photographs of
Finnish roads, which stay outside the repository.

Road stretches carry road space; junctions do not:

- Span asphalt quads get UV = (metres along the edge, signed metres from its centreline), exact at
  each region corner because span regions are built from sections (`road_space_corners`), and an
  8-bit COLOR = (forward lanes, backward lanes, speed limit in km/h, 128 + 127 × age).
- Age is a cosmetic hash of the edge's endpoints, so a road keeps its wear across saves and
  edge-id compaction. Nothing in the simulation reads it.
- Junction asphalt keeps world UVs and a zero COLOR, so the shader drops ruts, edges and lengthwise
  cracks there. The aggregate is sampled in world XZ everywhere, so stretches meet junctions
  without a seam.
- Cost: two texture reads and a few hundred ALU per asphalt fragment, no loops; the Rust side adds
  O(1) work per emitted vertex; the crossing attribute and partition costs are described below.

Lane lines follow Traficom's dimensioning table (Liite 14, 2023, K0) and the 2020 Road Traffic
Act, under which every longitudinal line is white:

| Line | Speed limit | Dash / gap | Width |
|---|---|---|---|
| Centre K1, lane line K2_1 | ≤ 50 km/h | 1 m / 3 m | 0.10 m |
| Centre K1_2 / K1_3, lane line K2_1_2 / K2_1_3 | > 50 km/h | 3 m / 9 m | 0.15 m |
| (none) | two-way 1 + 1 street at ≤ 50 km/h | unmarked | |

Zebra crossings use the Finnish L3_2 dimensions (Traficom, Liite 15, 2020, at 50 km/h or less): 0.5 m stripes with 0.5 m gaps, 2.5 m long along the road (formerly 0.4 m gaps and 2.0 m).

Stripes keep these sizes measured in plan (XZ), so crossfall no longer narrows them, and the
leftover width is split evenly between the two kerbs (`ROAD-64`).

Crossings are paint in the asphalt itself (`road_asphalt.gdshader`), not quads laid on top: a
quad floated over the road with its own flat colour and read as a printed decal. When a chunk's
mesh is built, the asphalt triangles under a crossing's rectangle are split along it with the
preview polygon clipper, and their vertices carry `CUSTOM0`: metres across and along the
crossing, paint age plus one from the edge hash (zero off a crossing), and the distance from the
kerb. The surface keeps its position, normal and UVs, so the paint shares the asphalt's
aggregate, relief, shadows, sun sheen, cloud shading and night lamps.

`fr_surface_painted` in `road_surface_finnish.gdshaderinc` paints off-white over the same two
aggregate samples: studded-tyre wear in two bands per lane thins the paint in streaks along the
traffic down to the stones (worn bars fade, they are not cut away), bar corners are rounded by
about 4 cm with a centimetre of soft overspray, older paint fades towards the asphalt, about a
third of crossings keep faint, well-worn traces of the old bars, each a few centimetres off to
either side or along, where a repaint missed them (one constant offset read as an embossed drop
shadow), and paint is a little smoother so it catches the sun. Paint adds no height to the relief
normal: a 2 mm step over its soft edge shaded one side of every bar like a bevel. The paint goes on after patches,
seals, trenches, drains and manhole covers, as a crossing is repainted over them; only cracks
break through it. Look-dev targets were local Commons and street photos (Jyväskylä, Okkerinkatu,
Kilonpuistonkatu, Nummikatu Oulu, Mustinlammenkatu and Neulaniementie Kuopio), not a photometric
calibration; paving-block crossings are not drawn.

Cost: one branch on asphalt off a crossing, bounded arithmetic on it, no extra texture reads.
The mesh gains 16 bytes per asphalt vertex and at most eight triangles per triangle a crossing
cuts; clipping is O(crossings × triangles) per owner, with no per-triangle allocations. Frame
p50 on the M2 Pro (release, `light-road` at 13:00, 120 frames, old and new alternated twice):
saved view 10.47-10.50 → 10.50-10.54 ms, eye level at a crossing 13.54 → 13.53-13.55 ms, 22 m
overhead 12.85-12.86 → 12.86-12.87 ms, i.e. unchanged. The rounded, streak-worn revision
(2026-10-15, same method plus 10 m over a crossing): saved 10.55-10.56 → 10.56-10.61 ms, eye
level 11.72-11.74 → 11.74-11.78 ms, 22 m 12.87 → 12.84-12.88 ms, 10 m 10.98-11.01 → 10.97-11.01 ms.

Limitations: wheel paths assume 3.5 m lanes; overlapping crossings take the first owner's
paint; the crossing does not yet follow a raised table.

Leaving 1 + 1 streets at urban speed unmarked is an assumption from common Finnish practice
(the residential streets in the Kuopio references have no centre line), not from the table. The
road tool has no street hierarchy and gives every road 50 km/h, so every two-lane road counts as
residential. Lane-line quads stay continuous geometry; `road_marking.gdshader` cuts the dashes
from UV.x (metres) and the pattern in UV.y (2 solid, 3 short, 4 long). Paint opacity rose from
0.35 to 0.8. Edge lines (K6), no-crossing lines (K3), warning lines (K4), wetness from weather, gravel roads and footpath surface variants are not wired yet.

Footpaths draw as Finnish light-traffic asphalt (`godot/assets/materials/road_path.gdshader`,
functions in `godot/scripts/shaders/path_surface_finnish.gdshaderinc`, also from Codex look-dev):
finer and darker than the carriageway, no wheel ruts, frost cracks across, root heave and
patches with age, and a crumbling margin. The sidewalk layer's material switched to this shader,
but roadside sidewalks still take its first branch and look exactly as before; only footpath
geometry carries the marker:

- Footpath span quads get road-space UVs and COLOR = (half-width in decimetres, 255 marker,
  surface kind 0 = asphalt, 128 + 127 × age).
- Node pieces where only footpaths meet get the marker with a zero half-width: the path surface
  in world space, age 0, so no cracks line up with the world axes.
- The stone-dust and forest-trail variants exist in the include (surface kinds 1 and 2) but no
  edge can select them yet: that needs a per-path surface setting in the tool and the save.
- The path mesh ends at the path edge, so the grass blend has nowhere to go and the edge reads
  as a hard line. A narrow apron strip beyond the edge would let it blend.

Frame cost, measured 2026-10-09 on the M2 Pro (release, 1080p, `kuopio-city.sqlite` at 12:00,
`--gameplay-road-benchmark` idle matrix, 210 warm-up and 120 measured frames, before = `c848c7f3`
and after = `a39fc786` alternated twice). Frame p50, before → after:

| View | Before | After | Draw calls |
|---|---|---|---|
| `city` pose, 60 m, centred on the save's camera | 20.2 / 22.3 ms | 22.4 / 21.3 ms | 2494, unchanged |
| street view, 20 m (temporary harness pose, not committed) | 20.4 / 20.0 ms | 16.7 / 16.9 ms | 2631, unchanged |

The street view, where asphalt fills much of the screen, got faster: the former material sampled
three 4K textures at two scales with anisotropic filtering, the new one samples one 512 px texture
twice. In the city view the change is inside the run-to-run noise.

The residential look (2026-10-09: speed-scaled ruts, ageing, blotches, trench patches, fittings,
sun specular, granite kerbs) was timed the same way on `simple-city-3.sqlite` (a residential
town with one 2 + 2 road), in a local build that merges this branch with the weather branch so
the save loads. Same library both sides; only the four material files were swapped, alternated
twice. Frame p50, before → after:

| View | Before | After | Draw calls |
|---|---|---|---|
| eye level on the 2 + 2 road, asphalt fills half the screen | 12.78 / 12.69 ms | 13.04 / 13.03 ms | 759, unchanged |
| 12 m above the same road | 15.04 / 14.99 ms | 15.09 / 15.26 ms | 1199 / 1211 |
| the save's own camera | 17.40 / 17.35 ms | 17.43 / 17.43 ms | 1692, unchanged |

About 0.3 ms where asphalt fills the screen and under 0.1 ms in a normal view. A first version
with a third, finer blotch octave cost 0.4 ms at eye level; the coarse grain already carries that
scale. Large-scale blotches read from the aggregate texture magnified instead of noise came out
nearly flat.

## Street lights (`ROAD-59`)

Every standard road and footpath gets street lights, placed in Rust with the road mesh
(`rust/src/simulation/network/render/road/street_lights.rs`) and drawn by
`godot/scripts/renderers/street_lights.gd`.

- Placement is a pure function of the compiled sections, along whole streets: chains of edges
  through degree-two nodes that take the same family of light (residential, main road, path).
  Lights spread evenly over the chain's visible length with both ends lit and every gap between
  the family's minimum and its spacing; a street shorter than the minimum gets one light in its
  middle. Only real junctions (degree three or more) keep 6 m clear; dead ends keep 2 m. Each edge
  emits the chain's lights that fall on it, so chunk rebuilds agree; the chain order is canonical
  (from the end with the lower node id), so a street lights the same whichever way it was drawn.
  O(chain length + lights) per covered edge. Per-edge placement, the first version, left chains of
  short edges and every edge shorter than its two margins dark.
- Where streets meet (a junction, or a degree-two node where a two-lane road becomes a four-lane
  one) every arm used to put its end pole at its own margin, so poles of neighbouring arms stood a
  few metres apart. Now the arms are taken in rank order (main road, residential, path, then lower
  edge id): an end pole keeps its margin unless it would stand within 12 m of an end pole already
  accepted at that node, and then walks away from the node in 0.5 m steps to the first clear spot;
  the chain's even spacing follows from the new margin. A candidate depends only on arms ranked
  above it at that node and on the opposite end's normal margin, so every edge computes the same
  result. O(degree · (chain sections + steps · degree)) per covered edge, degree at most six.
  Before/after: `screenshots/2026-10-09-roads/lights-width-change-*.png`.
- One-sided streets (1 + 1) keep one side of the world along the whole chain; wider roads get
  both sides, paired. The pole stands 0.35 m inside the back of the sidewalk on its surface, arm
  over the road; without a sidewalk, 0.75 m beyond the kerb or shoulder on the terrain; beside a
  footpath, 0.5 m beyond its edge.
- Kinds: 8 m galvanised steel (residential), wooden pole with an older sodium-coloured luminaire
  (residential edges whose cosmetic age is above 0.65), 10 m main-road pole (more than two lanes),
  post-top path light (footpaths). Spacing belongs to the family: 30 m residential (minimum 12 m),
  36 m main (14 m), 20 m path (8 m). A twin-arm median pole exists for divided roads. Spacings and
  offsets are look-dev defaults, not a lighting calculation.
- Each light is routed to its render chunk like a triangle and travels in the chunk payload as
  five floats (chunk-local x, y, z, yaw, kind). Godot keeps them outside the chunk meshes, which
  junction previews hide, and batches them per 64 m cell, model and level of detail into
  MultiMeshes with one merged surface each (parts in vertex colour): LOD0 to 140 m, LOD1 to
  420 m, LOD2 to 1400 m from the cell centre.
- Models: Codex look-dev (`gpt-6-astra`) against Wikimedia Commons photographs of Finnish street
  lights, Blender-built from code primitives, three LODs of at most 700 / 160 / 30 triangles, no
  textures; original, CC BY 4.0 (`credits.txt` beside the models).

At night (sun below 2°) a lamp is drawn three ways:

- the lens glows (`street_light.gdshader`), one batch per chunk;
- a camera-facing glow point (`street_light_glow.gdshader`): a 0.3 m halo near, held at no less
  than 2.2 pixels far, so a lit town reads from across the map after the poles' last LOD;
- its light: a real unshadowed SpotLight3D for the 32 lamps nearest the camera within 80 m (80°
  cone, range the lamp height plus 20 m), and for every other lamp the same light computed inside
  the lit materials (`street_lamp_light.gdshaderinc`).

The material lamps reproduce Godot 4.7.2's spot light term for term (`light_storage.cpp`
1004-1080 and `scene_forward_lights_inc.glsl` 457-463, 787-799 and 239-244): linear colour times
energy, the range window `(1 - (d/r)^4)^2 / d`, the 80° cone with Godot's stored
1 / spot_angle_attenuation (0.5 gives exponent 2), and Lambert, whose 1/π cancels the energy's
π. Each receiving material adds that times its own albedo and `1 - metallic`, with its mapped
normal, so asphalt, paint, kerbs, grass and walls keep their contrast and a wall facing a lamp is
lit like the ground. Specular is left to the real lights. Receivers: terrain (with field grain and
border faces), grass, roads, paving, crossings, site ground, walkers, lamp poles, water and its
border faces, and everything built by `WindowMaterials.create()` (buildings, fences, hedges,
rocks, signs). Trees keep their own shaders and get only real light.

The handoff is per lamp: when more than 32 lamps are within 80 m it moves in to the farthest one
that got a real light. Godot fades each real light by its head's distance to the camera over the
last 30 m before the handoff, and the material lamp takes the same smoothstep's complement, so
real plus material light stays constant and no lamp pops. Real lights reaching 160 m instead of 80 m
cost another 0.4 ms (2026-10-14), so the reach stays 80 m.

The lamps reach the GPU through a camera-centred grid (`street_lamp_grid.gd`): 128 × 128 cells of
16 m (2,048 m across), each listing up to 16 lamp heads whose range reaches it, one RGBAF texel per
head (position and kind; 4 MiB in all), plus a texel per lamp kind for colour and range. A street
puts about six to eight heads in a cell; 16 leaves room for junctions and median pairs, and a
fuller cell keeps the first 16 in stable world order. Per pixel: one fetch to find an empty cell,
otherwise at most 16 heads, one fetch each. The grid is rebuilt when chunks change or the camera
moves 256 m from where it was last centred, O(lamps in the window). It took 0.9 ms in the
`light-road` save; following every 16 m cell would have cost that per few metres of panning.

Past 500 to 700 m from the camera the per-lamp loop hands over to a far light map
(`street_lamp_far`): 512 × 512 cells of 16 m (8 km across), each holding the mean light its lamps
put on level ground, one bilinear fetch per pixel times the surface's albedo and upward normal.
Each lamp adds its kind's ground flux (the exact term above integrated over the ground on a 1 m
grid, so energy matches) to the four texels around it with bilinear weights; filtered sampling
spreads that as a tent of 32 m. Before, the 2 km grid was centred on the camera, so a street
1 km out under a tilted camera fell off it and went totally dark at a hard edge, where real
cities read as lit areas from a plane. The map is centred on where the camera looks (its
centre ray to the ground, at most 3 km out) and re-centred after 1,024 m: a full rebuild
splats every loaded lamp, 33 ms for 30,000 lamps plus 0.3 ms to upload; a chunk added or removed
splats only its own lamps in or out. Walls get no far light (upward normal only); at that
distance they are a few pixels.

Check (2026-10-15, `light-road` at 01:00, far against exact in the fade band, mean absolute
difference per channel of 255): 450 m 0.07, 650 m 0.13, 900 m 0.08. The `lights-distance-1`
save viewed from 976, 2,000 and 3,000 m is lit where it was black. Frame p50 on the M2 Pro
(release, 120 frames, old and new alternated twice): saved view 12.90-12.95 → 12.97-12.98 ms,
140 m out 8.12-8.13 → 8.17-8.18 ms, 320 m up 4.79-4.85 → 4.81-4.83 ms, a car 85 m away
17.50-17.52 → 17.48-17.54 ms, 650 m out 3.37 → 3.28-3.30 ms, 900 m out 3.28-3.29 → 3.18-3.19 ms.

Before, every lamp without a real light had an additive ground quad fitted to the real light's
mean brightness. It ignored surface colour, so a lit street read as an orange haze with flat
kerbs and crossings, and as a quad on the ground it never lit walls: a house past the handoff
stayed dark until the camera came close.

Check (2026-10-15, `light-road` save at 01:00, mean absolute difference per channel against
every lamp given a real light): 85 m above a junction 5.5 → 0.8 (of 255), 140 m out 1.2 → 0.13.
Frame cost on the M2 Pro (release, 120 frames, two alternated runs, frame p50, pool → material
lamps): saved view 12.56 → 12.86-12.96 ms, 140 m out 7.87 → 8.11-8.18 ms, 320 m up 4.57 →
4.75-4.83 ms, a car 85 m away 16.89-16.94 → 17.45-17.62 ms; at noon unchanged (10.53 → 10.48,
15.64 → 15.63). Most of it is the per-lamp evaluation on lit pixels: cell size (8 m), fetch
layout and real-light count (8, 16, 32) each moved it by under 0.1 ms. Lighting every lamp in
view with real lights instead cost +0.6 to +1.2 ms and still needed a far fallback.

The terrain's `light()` used its sun model for every light: no Lambert falloff, and a 2 % visibility
floor that added a share of each local light to every clustered-lighting cell the light reached,
which drew the cluster grid as blocks on the grass at night. Local lights now take the paving
Lambert diffuse; the sun and moon keep the ground-shadow model.

Not yet done: house windows do not light their yards; no sky glow over towns.

Frame cost, measured 2026-10-09 on the M2 Pro (release, `simple-city-3.sqlite` in the local
roads + weather build, 120 frames, same library with lights suppressed by a temporary switch,
alternated twice). Frame p50, off → on:

| View | Day (12:00), first version | Night (01:00), 12 real lights within 80 m |
|---|---|---|
| eye level on the 2 + 2 road | 13.01 / 13.02 → 13.04 / 13.02 ms | 16.13 / 16.15 → 17.22 / 17.43 ms |
| 12 m above it | 15.27 / 15.18 → 15.27 / 15.27 ms | 17.82 / 17.88 → 19.29 / 19.54 ms |
| the save's own camera | 17.45 / 17.42 → 17.43 / 17.47 ms | 19.79 / 19.81 → 21.87 / 21.76 ms |
| 420 m over the town, no real light in range | | 7.05 / 7.05 → 7.10 / 7.09 ms |

In that build, the poles, glow points and ground pools cost under 0.1 ms. The night cost is the real
lights, and it is nearly fixed once any is in view: 40 lights within 180 m, 24 within 120 m, 12
within 80 m and 4 within 50 m all cost 1.1 to 1.6 ms at eye level and 1.4 to 2.5 ms in the save's
view, and turning vegetation off removed little of it. Whether to keep real lights near the camera,
gate them by camera height, or make them a graphics setting is open.

## Road signs (`ROAD-60`)

Finnish road signs stand beside the roads, placed in Rust with the road chunks and drawn by
`godot/scripts/renderers/road_signs.gd` as one MultiMesh per sign model and chunk. They are
cosmetic: traffic ignores them.

- **Pedestrian crossings (E1-1, E1-2).** One sign per approach at every zebra, taken from its
  `CrosswalkMarking` (the crossing authority), on the right-hand side facing approaching traffic,
  1.75 m before the crossing centre so it stands at the near edge of the stripes, out of the
  walking line (it stood in the middle of the crossing before).
  A one-way road gets E1-1 on the right and E1-2 on the left, both facing the legal approach.
  Zebras lie on node pieces (`ROAD-68-C` follow-up), so signs stand on the span sections
  extended by the node's section at the zebra's inner edge, where the curb stops curving; the
  sign for traffic leaving the junction is held there, 1.5 m before the crossing centre.
- **Speed limits (C32-30/40/50/60/80).** As in Finnish towns, where the limit is mostly set for an
  area, a sign stands only where it tells the driver something: for traffic turning into a street
  at a junction of three or more arms whose limit differs from another road arm there, or, as a
  repeat after the junction, into a street of at least 300 m (120 m for roads of more than two
  lanes). Dead ends and degree-two nodes get none, nor streets under 40 m. The sign stands at least
  25 m past the crossing sign. The value is the nearest C32 to the edge's speed limit, within
  5 km/h. The first version put a sign at every street start beside the crossing sign, which made
  both unreadable and crowded every corner of a busy junction
  (`screenshots/2026-10-09-roads/signs-zebra-before.png`).
- **Warnings for awkward roads.** Measured from the edge's physical centreline with the road
  validator's own grade, grade-change and radius measurements, per travel direction, on the right,
  about 20 m before the hazard; one of each family per street direction, the nearest winning:
  - steep (A3.1 ascent, A3.2 descent, 16 or 18 %): where the road averages 16 %, the validator's
    illegal grade, over 100 m, labelled with the steepest such average rounded down to an even
    percentage. The first rule (any 15 m at 8 %, labelled by its steepest segment) signed the short
    ramp where a road meets a junction and put 18 % signs on tame streets: 1,245 steep signs over
    the 1,642 roads of the local saves, 62 on Kuopio's 106. Now 122 and 9;
  - curve (A1.1/A1.2, or A2.1/A2.2 when two opposite bends that each qualify lie within 100 m): a
    bend on the travelled road, junction fillets and mouths excluded, that turns at least 60°
    below a quarter of the design radius for the road's speed, measured over a complete radius
    window, and does not lie within 30 m of a junction mouth, where drivers slow anyway. The first
    rule (30° below half the design radius) signed ordinary street elbows and junction approaches:
    296 curve signs over the local saves, now 4;
  - uneven (A9): grade change above 0.07 per metre. At 0.035 the save survey flagged 13 % of
    Kuopio's roads; 0.07 flags about 6 %.

  They make the game honest about the roads placement still accepts: the hill in
  `traffic-signs.sqlite` gets 16 % and 18 % signs (`warning-sign-steep-after.png`).
- Signs facing the same way on the same side keep 12 m apart along the street, in priority
  crossing, speed, warning; a lower sign moves on, away from the junction, or is left out.
- Poles stand 0.5 m behind the kerb face on the sidewalk (measured from the face; it was
  measured from the sidewalk's inner edge, which put poles in the walking line), or 0.75 m beyond the kerb or shoulder on the
  terrain where there is no sidewalk, and keep 2 m from street-light poles by sliding along the
  road away from the junction; a sign with no room on its edge is left out.
- Priority, yield and stop signs (B1, B5, B6) are modelled and shipped but not placed: junction
  priority does not exist yet.
- Models: `godot/assets/models/road_signs/`, faces rasterized from public-domain Finnish sign
  SVGs (`credits.txt`), geometry CC BY 4.0; metres, pole base at the origin, printed face +Z. Commons has
  steep-hill faces only up to 13 %, so 14–18 % are composed from numeral paths of the real signs.

Placement is O(touched street edges + their points · radius window + crosswalks + lamps + signs)
per chunk rebuild, ordered by a total pose key, so the same network gives identical bytes. The
`road_chunk_benchmark` `chunk_emit_only` fixture (release, 4 Rayon workers) went from 41.2–42.7 to
42.7–43.5 µs a rebuild with the warning and spacing passes, flat from 1 to 1024 occupied chunks;
it ran beside other build jobs on the M2 Pro, so treat it as a scaling check, not an exact cost.
In game (`traffic-signs.sqlite`) the warning signs face the traffic they warn, on its right.

## Forbidden Regressions

Do not reintroduce:

- polygon booleans, winding or claim-priority ownership, arrangement noding, generated contacts, or
  provenance / height-authority rules in the road compiler
- caches of intermediate compiler stages
- a second implementation of corner, mouth or height geometry in lanes, render, terrain or Godot
- centerline-only road lifting or terrain flattening
- terrain fallback sampling for road height
- nearest-height, nearest-owner, min/max, averaging, owner-priority, or old-road-wins repair
- tolerance snapping or vertex deduplication to hide mismatched geometry
- render z-bias as geometry
- shader, water, zoning, material-order, cull-mode, or background masking for missing topology
- seam strips, closure carpets, guard strips, or connector patches
- Godot-side road topology, terrain clipping, or road-height decisions
- fallback from Spade CDT failure into other terrain clipping
- full-network road or terrain rebuilds for one local edit
- an asynchronous preview queue, stale-result path or second preview mode without a measured need

## Test Contract

Maintained coverage must prove:

- the profile band table for every edge type
- layout:
  - invariance to arm order
  - corner endpoints bitwise equal to their offset-curve points
  - mouth stations
  - fitting and every `LayoutError`
  - degrees 1–6 and arm angles from 25° to 180°
- watertightness within each piece and across mouths, curb faces on every carriageway boundary
  except mouths, and no overlapping material triangles
- node vertices at `P + level`, and span heights equal to the section evaluation
- query heights and ray hits equal to the rendered triangles
- bitwise determinism across runs, Rayon worker counts, and edit orders that produce the same graph
- dirty-closure locality, with no unrelated chunk or terrain patch rebuilt
- terrain:
  - no faces inside footprints
  - seam vertex identity
  - holes
  - seams across patch boundaries
  - road–site shared vertices
  - bridge and tunnel terrain support
- bitwise preview = commit = undo = load
- the crossing contract: a zebra marking and its pedestrian connectors share one kerb segment
- render:
  - chunk partitioning preserves the global triangle multiset
  - stale generations are rejected
  - road and terrain swap atomically in production-shaped Godot tests
- the deterministic road-chunk benchmark keeps identical local signatures at every scale

## Debug And Diagnostics

`--debug road` reports, per edit:

- dirty-closure counts
- layout, piece, terrain, packing and install timings
- per-node layout summaries: arms, sector kinds and angles, the radius `R` used, and mouth stations
- typed layout and profile errors, with node and arm
- per-patch chain and loop counts, and CDT counters

The road debug overlay draws mouths, corner centres, tangent stations and outer chains from the same
layout and pieces the mesh uses. Errors are diagnostics and placement rejections, never visual
fallbacks.

## Archive

- [`archive/roads_legacy_surface_2026-10-23.md`](archive/roads_legacy_surface_2026-10-23.md): the
  legacy surface compiler; `ROAD-01`–`ROAD-67` history and measurements; the full `ROAD-67-A`
  baseline method; the stopped `ROAD-67-B` prototype notes; and the legacy preview, terrain and
  commit pipeline details
- [`archive/roads_hardcut_history_2026-05-31.md`](archive/roads_hardcut_history_2026-05-31.md):
  the earlier roadbed hardcut spec

Do not update the archives as live planning. Update this file, [`roadmap.md`](roadmap.md) and
[`project.md`](project.md) for current road status.
