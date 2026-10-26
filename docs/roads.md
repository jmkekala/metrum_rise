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
- the road side of terrain integration: patch loops, grading envelope and patch selection (the
  exact cut's; under [`TERRAIN-07`](terrain.md#graded-terrain-surface-terrain-07) the roadbed gives terrain only its outer chains, top
  triangles and terrain changes)
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
   outer-edge vertices. The terrain half is superseded by [`TERRAIN-07`](terrain.md#graded-terrain-surface-terrain-07): terrain meets the
   outer chains through an exact mask and road skirts, within a tolerance, not through shared
   vertices.
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

A layout is built for every node except degree-two pass-through nodes: two arms whose directions
are close enough to opposite (the existing `directions_pass_through` rule) and whose edges have
one profile (`arms::edges_pass_through`, the one rule of layouts, span ends and the profile
solve). Those own no node piece; their spans meet at one bisected cross-section, as today, and
their crossing lies on a span ([D8](#product-decisions)). A straight pair of two profiles (a width
change, or a road with sidewalks continued by one without) has a layout since `ROAD-68-F` part 2:
spans of different profiles share no mouth row, so its straight sectors taper (D4) and it crosses
once, as a bend does. The span model's own width-change taper, which only such pairs reached, is
removed.

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

**Round join.** On convex sectors, band boundary `j` is an arc about the node from
`o_a(0, la_j)` to `o_b(0, lb_j)`, with the radius interpolated linearly by angle. In a straight
sector each boundary starts where its offset curve crosses the sector bisector instead: ahead of
the node when the sector is narrower than `pi` and the arms overlap
([open question 5](#road-68-open-questions)), on the first offset segment extended behind the
node otherwise, so the arms meet in a miter point rather than a rounding-sized arc.

**Width-change taper ([D4](#product-decisions)).** A straight sector between unequal boundary
offsets tapers. Each such boundary meets the sector at the mean of its two arms' offsets, so the
join is that of equal widths, and runs linearly in station to each arm's own offset at the taper
station on both approaches. The taper takes half the legacy span taper on each arm
(`clamp(3 * width change, 12 m, 28 m)`, the removed span taper's `WIDTH_CHANGE_TAPER_*` rule;
a change of the bands beyond the carriageway alone, such as sidewalks against shoulders, has no
width change and takes the 12 m minimum), shortened
to what both arms hold beside their crosswalks; shorter than the 12 m minimum, layout fails with
`ArmTooShort`. Mouths lie at or beyond the taper station, so spans start at full width and copy
their mouth sections as before. `SectorBoundary::run_offset_m` is the one function node pieces,
the core loop and walkways place a run with.

**Mouth stations.** For each arm, the mouth station is the larger of `s_min` and the tangent
stations of its concave corners, plus the crosswalk depth on crossable arms
([D8](#product-decisions)). Between a corner's tangent station and the mouth, the arm's band
boundaries follow its offset curves. On a crossable arm that depth is the zebra strip, right
outside the curb returns: its inner edge is the larger tangent station, where the curbs stop
curving, its outer edge is the mouth, and its crossing line is the middle, with kerb points at
the two carriageway edges. The mouth is the arm's full cross-section at that station:
the node piece ends and the span begins there. An attached footpath that meets an arm side moves
only that arm's mouth, to `s_min` beyond the meeting point where that is farther; the zebra stays
right outside the curb returns ([open question 17](#road-68-open-questions)). Every run from
where a boundary reaches its arm's own offset (its corner's tangent station, a taper station, or
where it leaves a round join) out to the mouth is empty or at least `s_min` long: where one would
be shorter, the mouth moves `s_min` beyond the farthest reach (`ROAD-68-F` part 2). Without it, an
arm without a crossing whose two corners end a hair apart, as on a T of roads without sidewalks,
left a sliver between the nearer corner and the mouth. When both ends of an edge reach the same station,
the span is empty and the two node pieces share that mouth section.

**Footpath arms (D5).** At a node with a road arm, footpath arms are *attached*: they bound no
sector, and the road arms keep their sectors, corners and crossings as if the footpaths were
absent (the crossing rule counts road arms only). Each attached arm lies in one sector. Its two
edge curves (`o_f(., ±w)`) meet that sector's outermost boundary:

- on an arm side's run at or beyond its reach (the taper station of a tapered run, else where it
  leaves the sector), at the run's offset-curve point `o_a(s, l_outer)`; the arm's mouth then
  lies at least `s_min` beyond `s`, while a crossable arm's zebra stays at the corner and the node
  piece covers the run between them (open question 17)
- otherwise on the sector's sampled curve; the point is inserted into the outermost samples, and
  the same fraction of the same segment into every other boundary of the sector, so its strips
  stay quads

The attached arm's mouth is `s_min` beyond its farther edge point. Its ribbon, one band at
`P + raised`, runs from the boundary (on the boundary's own vertices) out to the mouth, so it
meets the sidewalk without a step; the profile solve pins the footpath to `P + raised` up to its
mouth. The outer chain is cut at each attached arm: one chain ends along its right edge at its
mouth corner, the next starts along its left edge. Attached arms are ordered by sector and along
its outer path, which in a sector through angle zero (a road end's cap) is not angle order. An
edge that meets no boundary inside the node, a ribbon outline that is not simple, or two ribbons
that touch or overlap fail with `LayoutError::FootpathDoesNotJoin { arm }`. Where only footpaths meet, they are ordinary arms
with one ground-level band per side, laid out by the corner and round-join rules above; a
straight pair of footpaths is a pass-through pair, but a footpath continuing a road end
straight on attaches to its cap.

**Fitting.** Each end may use at most half of its edge's length.

- If a corner does not fit, `R` is reduced towards `R_min`. If it still does not fit, layout
  returns `LayoutError::CornerDoesNotFit { arm }`.
- A self-intersecting carriageway core returns `LayoutError::CoreNotSimple`.

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
  it. A knot at the mouth station makes the pin exact there. Every physical profile is derived
  afresh from its edge's control polyline and the pins at both of its ends; the finalizer never
  rewrites control polylines, and its plane solve has no edit roles, so profiles depend only on
  the final graph ([pre-D edit-order independence](#edit-order-independence-pre-d)).
- **Plane solve.** A node's plane incidents (its roads, or its footpaths where only footpaths
  meet) are taken in incident order: the `f64` angle from world +x of each arm's direction
  towards its first control point beyond 1 mm (`roadbed::incident_direction`, the function node
  layouts and span ends classify with), with edge ids only for exactly equal directions, which
  layouts reject. Every incident gives the sample `max(12 m, its mouth station)`, at most half its
  edge, along its control profile (open question 31: never inside the stretch the pin holds);
  an arm too short for a sample at least 1 m from the node leaves the node without a
  plane (`NodeError::NoPlane`, `junction_profile_mismatch`), never a plane fitted without it. A
  bend takes the plane through its two samples. A junction, or a pass-through pair with attached
  footpaths, takes an authority corridor, a pair of arms at least as opposite as `dot <= -0.86`,
  else the least-squares plane of every sample. Corridors rank lexicographically in `f64`, with
  no blended score: (1) a corridor whose plane every other arm's sample meets within the 5 cm
  support tolerance first; (2) the straighter pair, by the lower dot of the two arm directions;
  (3) the flatter, by grade; (4) incident order. The arms outside the corridor take
  vertical-curve supports, attached footpaths included; a road end has no corridor, so its
  footpaths take supports only where their grade leaves its plane.
- **Scope and cost.** The solve derives every edge touching the affected nodes and pins its far
  end from the far node's own solve, so it reads the 2-ring of the affected nodes:
  `O(sum over touched edges of (points * inserted knots) + sum over their far nodes of degree * points)`,
  local but larger beside a high-degree hub. Every caller passes both ends of every edge whose
  control polyline it changed: road edits (both ends of each dirty edge), the crossing toggle, the
  load repair, and, since the pre-D review, node moves (the moved node and its arms' far nodes)
  and road removals (the removed edge's two ends), which rebuilt clips without re-solving
  before. A road edit plan solves on a local excerpt of the graph, which must hold that 2-ring:
  every edge near the stroke and two rings of incidence around them, and a plan whose affected
  nodes or the far nodes of the edges they re-derive lack an arm in the excerpt is refused
  (`ROAD-68-E` part 2 round 3; with one ring a far node lost its pin, open question 8). The solve,
  and the single-node solves of `check_node` and the plane readers, run in each thread's buffers
  (`ProfileSolveScratch`: the derived edges, the incidents by node, the samples, the pins, the
  profile being derived, its end distances and a `grade::GradeScratch`), taken for a solve, so
  once they have grown a solve allocates only where a changed profile outgrows its edge's
  buffer, and in the R-tree's re-index (`ROAD-68-K17`, [open question 37](#road-68-open-questions)).
- **Grade (`ROAD-68-K14`, [open questions 37 and 39](#road-68-open-questions)).** The stroke
  solve relaxes each 6 m span's 16% design grade towards the ground's slope only up to 33%, and
  builds the profile inside the bounds of its pins (its ends and the carriageways it meets) and
  the source-delta band (every other sample within 5 m of the ground, read exactly in `f32`; a
  pin further off the ground, such as a road end on an embankment, widens its neighbours' band by
  its excess, shrinking at the 16% design grade with the distance from it, `grade::source_bounds`):
  a forward and a backward pass give each sample's exact interval of feasible heights, and the
  profile takes the mean of two nearest-height passes, first to last and last to first, so it
  does not depend on the stroke's direction (`grade::grade_envelope`,
  `grade::construct_within_grade`). Where the bounds admit no profile within 33% it builds up to
  1:3; where none within 1:3, the stroke is refused (`road_too_steep`) with the least grade they
  allow. The widening shrinks at the design grade, not at the grade the profile is built to: at
  that grade a profile leaving an off-ground pin on flat ground runs exactly along the band's
  edge, where `f32` rounding decides feasibility (`ROAD-68-K15`). Junction blends are built the
  same way (`ROAD-68-K15`, question 39): in a derived physical profile of a standard road steeper
  than 33% or with a point more than 5 m from the source ground, the points a pinned end blends
  (past its mouth knot, up to its blend end) are rebuilt inside the band, every other point held
  (the plane stretches, the control polyline beyond the blends and the two ends; a held point off
  the ground widens the band as a pin does). A blend that eases an arm onto a flatter plane can
  climb faster than either, and one that follows a plane fitted to the arms' heights a mouth
  station out can leave the ground the stroke solve cut or filled the arm to. Where the held
  points and the band leave no blend within 1:3, it is built inside the band at the least grade
  they allow (`grade::construct_within_bounds`), which is steeper than the maximum, so placement
  refuses the edit as `road_too_steep` with that grade. Bridges and tunnels have no band, and a
  graph without a world (a unit fixture) passes no terrain. A profile within 33% and 5 m of the
  ground is left untouched (checked without allocating), so every flat and gently sloped product
  is unchanged. The whole-graph terrain sync builds each re-graded control the same way, its node
  ends held; where the ground leaves no profile within 1:3 it builds the least grade the band
  allows, and the terrain edit is not refused yet (open question 37, `ROAD-68-K14`'s open items). 33%
  lies a third of a percentage point under the maximum so
  that `f32` rounding of the derived points (world coordinates in the thousands of metres, knots
  5 cm apart at the closest) never reads above it.
- A `Terminal` gets a plane through its road end and its mouth centreline point, with the arm's
  end grade along that run and no crossfall; it is read from the solved profile, so the finalizer
  leaves terminals alone.
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
edges. One station rule places them (`ROAD-68-D`): the fixed stations are the protected ones
(span ends, mouths, crossings, the profile range and the ends of its fades) and the knots
outside their 0.125 m clearance; each interval between consecutive fixed stations is divided
evenly at its step, 2 m inside a profile fade and the class step (8, 12 or 10 m) elsewhere
(the width tapers left with the span taper, `ROAD-68-F` part 2). Cadence rows and class rows no longer start from different origins, so no two
rows fall a few millimetres apart before a knot.

**Other node kinds.**

- `Bend` is a two-arm junction with one concave corner and one round join.
- `Terminal` is a one-arm junction with a round cap ([D2](#product-decisions)). The cap's apex
  sample is its start turned a quarter exactly, not a polar sample, so an axis-aligned arm's
  apex lies on its axis.
- Bridges and tunnels use the same pieces: their spans are decks and road pieces like any span,
  and where they meet a road at a junction or bend they are arms of its node piece. A bridge or
  tunnel end without a continuation has no node piece; its span ends on its mouth row, which the
  abutment or portal closes. Their terrain side (abutment and portal cutouts, wall seams) is
  `TERRAIN-07`'s ([T14](terrain.md#terrain-07-open-questions);
  [`ROAD-68-F` part 2](#remaining-families-road-68-f-part-2-2026-10-25)).

**`PieceMesh`.** A piece's mesh holds:

- positions
- per-layer triangle index lists (carriageway, curb or shoulder, sidewalk, footpath, curb face),
  with each triangle's band kind kept for queries
- outer chains: footprint boundary polylines oriented with the road on their left (the
  counter-clockwise side in XZ, from +x towards +z), each running from one mouth corner to the
  next; `ROAD-68-D` reversed the span chains, which ran the other way, and tests the orientation
  numerically
- the strips hanging from the outer chains (`TERRAIN-07-B`), a layer of their own: road skirts
  0.5 m deep on grounded pieces (node pieces and standard spans), fascias 0.35 m deep on bridge
  spans down to the concrete slab, none on tunnels. Each strip is two triangles per chain segment
  whose top points are the chain's positions bitwise and whose bottoms lie straight below them,
  facing away from the road; the mesh keeps only the strip kind and constructs the triangles
  from the chains (`PieceMesh::for_each_strip_triangle`). Queries never see them
- bounds
- a per-piece triangle query grid, reusing the legacy `SurfaceTriangleGrid` design

Pieces are immutable and shared through `Arc`.

**Watertightness.** Within a piece, every edge borders exactly two triangles, except outer-chain
edges (which meet terrain) and mouth edges. Every mouth edge appears in exactly one span and one
node piece, with bit-identical endpoints. Tests check both conditions on every fixture. In the
published chunks (`TERRAIN-07-B`) the strips close the outer-chain edges: each is a strip's top
edge, the strips of two pieces share the vertical edge at their mouth corner, and only the skirts'
bottoms, and a structure's ends and a tunnel's sides, stay open.

### Footprint and terrain

**Terrain side superseded by [`TERRAIN-07`](terrain.md#graded-terrain-surface-terrain-07) (approach B, user 2026-10-25; draft spec for
review).** Terrain will be graded per 64 m tile from the outer chains, with an exact footprint mask
in the terrain shader and road skirts, and the exact cut below is deleted at `TERRAIN-07-G`. The
roadbed side stays as the contract terrain consumes:

- grounded outer chains, road on the left, joined at bit-identical mouth corners, `f64` positions;
  structure spans publish none
- the pieces' top triangles and the point-in-triangle predicate `height_at` uses, which `TERRAIN-07`
  takes as the footprint's coverage (no loops, so an open end flips no region; question 19)
- `take_terrain_changes`, which since `TERRAIN-07-A` also returns the changed grounded chain
  segments themselves (`TerrainChanges::segments`, endpoints with heights)
- the accessors `TERRAIN-07-A` reads them through, in `roadbed/query.rs`, no new geometry:
  `grounded_seams_in_bounds` (sorted by the bits of the start then the end) and its unsorted
  visitor, `grounded_pieces_in_bounds`, `for_each_grounded_top_triangle_in_bounds` and
  `top_triangle_height_at` (the predicate itself); bridge and tunnel spans are left out of both
  seams and coverage
- road skirts, one strip per grounded outer chain in the piece mesh (`TERRAIN-07-B`, built; see
  [pieces and meshes](#pieces-and-meshes))

Everything else in this section (patch loops, holes, open ends, source tags, the grading envelope,
seams on tile sides, watertight patches, pads in the patch frame, patch selection, tile clip,
ownership refresh) is the road side of the exact cut. It describes the code on `road` until
`TERRAIN-07-G` deletes it; new terrain work follows `TERRAIN-07`.

The terrain CDT core in `rust/src/simulation/terrain/cdt/` stays. It is generic: patch-clipped
loops, source samples and tie-in guides in; stitched patch mesh out. Its contract in
[`terrain.md`](terrain.md) and [`earthworks.md`](earthworks.md) is unchanged:

- loops are hard constraints
- faces inside road footprints are omitted
- seam vertices take road heights
- source samples are inserted only outside footprints
- building-site boundaries share the CDT, and the road height wins at shared vertices

The road side is rewritten:

- **Patch loops.** The CDT unit is the fixed world-aligned 64 m tile inside a heightmap render
  patch, which stays the dirty and ownership unit. Each tile takes its road loops from the
  rectangle of the tile plus its 64 m halo (`roadbed/terrain.rs`, `terrain_loops`): the pieces
  that rectangle touches come from `pieces_in_bounds` on the 32 m query grid (there is no
  chunk-to-pieces list), their chains join end to start at bit-identical mouth corners, and the
  generic rectangle clip in `terrain/rect_loops.rs` cuts the joined paths to the rectangle and
  joins each run's exit to the next entry counter-clockwise along its boundary, so the road stays
  on the left. Cost is `O(S log S)` for `S` chain segments of the touched pieces. The rectangle
  depends only on the tile, so every build, scope and ownership refresh sees the same loops. The
  legacy `terrain_clip/` union and `query/terrain_cdt/` adapters are deleted.
- **Holes.** Orientation is known: an outer boundary runs counter-clockwise in XZ, a hole (a
  closed block) clockwise, so `is_hole` comes from the loop's signed area. The CDT still needs a
  footprint group per loop, and a hole may hold an island (a separate road inside the block): all
  road loops of one rectangle share one group, and the CDT classifies a point by containment
  parity within a group (inside when an odd number of the group's loops contain it), which equals
  the innermost-containing-loop rule (`ROAD-68-D` correction; the old rule paired each hole with
  one outer loop).
- **Open ends.** A chain end inside the rectangle with no published grounded chain to continue it
  fails the rectangle with `TerrainLoopError::OpenEnd` (`roadbed_chain_open_end`): a failed
  node's gap (which also rejects the edit). There is no repair. Where a grounded piece meets a
  bridge or tunnel (a node piece's mouth, or a pass-through's shared mouth row) it publishes a
  wall run across the mouth row (`PieceMesh::wall_runs`, `TERRAIN-07-C` round 3,
  [T22](terrain.md#terrain-07-open-questions) 3 (a)): the row's vertices from the end corner of
  one chain to the start corner of the next, a curb pair once at its lower vertex. The loops join
  through it like a chain, and its segments are structure sources with the retaining-wall policy
  (`ChainSegment::wall`), so a structure built from a road commits (open question 19).
- **Source tags.** `TerrainCdtRoadBoundarySource` keeps `BuildingSiteBoundary`. Its span, node and
  handoff variants are replaced by one `RoadBoundary { piece, chain, segment, support }`, where
  `support` is grounded or structure (from the edge class) and carries the retaining-wall policy:
  only structures may tie in with walls. Sources order by their derived field order, adjacent
  segments of one chain merge into the lower segment, and tile fingerprints hash the sources. The
  CDT model stays free of network types (`TerrainCdtRoadPiece`). Correction (`ROAD-68-E` round
  4): the sources no longer name graph pieces. A split renumbers what a tile far from the edit
  sees although its vertices stay bit-identical (the far half of a split trunk becomes another
  edge's span, and its chain segments shift), so every tile along the trunk changed its
  fingerprint and re-triangulated. `road_terrain_loops` now makes a rectangle's loops canonical
  in its footprint alone: each loop starts at its least vertex by coordinate bits, loops are
  ordered by that vertex, and a source names the run of consecutive segments of one piece chain
  it lies on (`piece` is the run's number, ordered by the bits of its first two vertices, with
  the piece's kind; `chain` is 0; `segment` is the place in the run). Equality, order and merges
  between a tile's sources are as before; debug face sources show run numbers
  ([open question 28](#road-68-open-questions)).
- **Grading envelope.** One envelope for roads and sites (`terrain/grading.rs`, open question 3):
  rays leave every sourced boundary edge at the render step (2 m or finer) and every boundary
  vertex on its outward bisector; a ray reaches as far as the 1:2 tie-in from the seam height
  needs to meet the probed terrain, at least the base local sample margin and at most
  `GRADING_ENVELOPE_MAX_MARGIN_M`, the 64 m tile halo (open question 18). A loop's margin, the
  largest of its rays, bounds its influence; sites use their vertex heights, as non-road loops
  always did. Road guides are grade-limited samples on rings at one, two, four and eight steps
  and the ray's own reach, so a guide depends only on the boundary near it; rails join
  consecutive rings only for a single clean convex outer loop. One pass over a loop's rays
  (`grading::road_loop_grading`) probes each reach once for both the margin and the ring, and a
  tile emits only the rings of rays whose reach meets it (all of them when rails join them).
  Sites keep their sample-only apron guides.
- **Seams along a tile side.** A street on a 64 m tile line puts a road seam exactly on a tile
  side, with the road inside one tile and beside the other. Exact comparisons, no tolerance: in
  both tiles the seam's vertices on the side and the tile corners it passes are boundary
  vertices at the seam's heights (the clip's own interpolation), and a side sample on the seam
  lies on the footprint and is dropped, so both tiles emit the same side and no sliver. Arm knot
  laterals divide by the length rather than multiply by its reciprocal, so an axis-aligned arm's
  runs, terminal cap ends included, lie exactly on the span's grid line.
- **Watertight patches.** A render patch's terrain is its CDT tiles plus regular filler over its
  contributor-free tiles, and every terrain edge borders two faces with bitwise equal vertices
  unless it lies on a road seam, a site loop or a refined patch's outer side (`ROAD-68-D` part 4,
  test `reference_junction_terrain_is_watertight` over all 20 reference junctions). The filler is
  conforming: filler cells are bounded by patch and tile lines only; a cell side shared with a
  tile takes that tile's exported side vertices bitwise, every other cell side the render-step
  boundary lattice the tiles also put on their sides, and one strip per side joins it to the
  cell's regular grid, so no vertex of one side runs on as a grid line into other cells. Every
  face the CDT emits is exported unless it is exactly degenerate, and a face's slope is its own
  (no absolute cut-offs): thin and small faces are part of the triangulation. Open: the filler's
  lattice heights come from the patch snapshot and the tiles' from the terrain system, two
  bilinear evaluations that agree on sample points but may differ in the last bit between them,
  so a filler cell side whose neighbour is a tile of another patch, or a filler corner shared
  only through a tile corner, can differ by an ulp of height in worlds whose lattice misses the
  sample points (open question 23). A refined patch's outer side carries the boundary lattice,
  and an unrefined neighbour renders its regular heightmap grid there (a T-junction on a
  straight edge, as tiles on patch sides always had).
- **Pads and road seams share the patch frame** (`ROAD-68-D` part 5, open question 23). Every
  site's pad is part of the patch buffers: its footprint triangulated in f64, clipped to the
  patch canonically, converted once by `render_frame_local` and partitioned with the terrain
  faces by `SitePavingPartition`; there is no world-space foundation mesh. The road chunk over a
  patch has the patch's frame (its centre), so a seam vertex both meshes hold has the same bits
  in both (`reference_junction_road_and_terrain_share_seam_vertices`). Open: seam edges split
  by the CDT at a tile line inside a patch, and one straight `y_60` seam edge that passes a road
  vertex, are T-vertices on straight edges (open question 23).
- **Patch selection.** Every compile records the XZ bounds of the old and new pieces it replaced
  and the net multiset of their grounded outer-chain segments, keyed by endpoint bits, piece kind
  and chain start and end (`RoadbedSystem::take_terrain_changes`, `ROAD-68-E` round 4): a piece
  rebuilt unchanged, or a split trunk's far half, cancels out. A tile's loops come from the
  chains inside its halo, so they change only within the envelope cap (the halo) of a changed
  segment; its building sites grade against road surfaces and edges up to
  `site_feasibility_dependency_margin_m` beyond them, less than the cap, and a split
  re-parametrises a whole edge. So the edit marks the render patches and CDT tiles within the cap
  of a changed segment, and those within the cap of a changed piece that have a site within the
  local sample margin of their window (`road_terrain_tiles`), and stamps the tiles in the
  assembly ledger (`road_tile_dirty_at`, keyed by tile), so a later request rebuilds only those
  tiles and their neighbours. A patch is road-locked when any of its tiles has a road loop whose envelope reaches
  it, from the same per-tile loops; while the roadbed is dirty it keeps its last road ownership.
  Undo and rollback go through the same compile, so they follow. A world-scale road change (a
  load, or a bulk road build such as the populated-city fixture's or the in-game benchmark
  city's) refreshes every patch once with a new global payload generation
  (`refresh_all_engineered_terrain_patch_state`), which takes and drops the terrain changes its
  compiles recorded, so the next edit stamps and builds only its own terrain (`ROAD-68-D`
  part 6; before, those changes waited for the first road commit, which then graded and built
  every road patch of the city).
- **Tile clip.** The CDT core clips each loop to its tile with its own clip (an integer overlay
  for the components, the axis-aligned clip for exact intersection heights). Where a loop leaves
  the tile and comes back elsewhere (a row street beside a cross street outside the tile), the
  axis-aligned clip joins the two points with a run along the tile side, and the later side
  clips cut such runs into zero-width rails out to the tile corners, at heights no road has.
  Rail tips (a side vertex whose neighbours lie on the same side, with the boundary turning back
  there) are removed exactly, so a clipped loop holds only footprint vertices and its
  intersections with the tile sides; a loop without rails is unchanged (`ROAD-68-D` part 6,
  `a_loop_reaching_past_a_tile_side_adds_no_vertex_off_its_footprint`).
- **Ownership refresh.** A patch's road ownership is recomputed only while it is pending: a
  compile stamped it, or a terrain stroke changed heights within twice the envelope cap of it
  (its loops reach the cap beyond the tile, their rays the cap beyond the loops), or an undo that
  restores the terrain cleared it (every formerly engineered patch). An edit's
  validation refreshes its patches once, and the commit's later refresh finds them done. A
  building-site change refreshes only site ownership: it moves no road and no height. Patch
  margins grade in parallel; the largest margin is order-free, so it is the same on any worker
  count.
- **Bridges and tunnels.** They do not clip terrain mid-span. Abutment and portal cutouts belong to
  the terrain surface rewrite `TERRAIN-07` (user, 2026-10-25; its wall seams,
  [T14](terrain.md#terrain-07-open-questions)). The grounded chains that end at a structure close
  across its mouth with a wall run (open ends above), so the legacy terrain builds those patches
  and the edit commits (`TERRAIN-07-C` round 3, open question 19).
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
its old and new chunks. A triangle inside one chunk goes there whole; one crossing chunk lines is
clipped at them by the canonical rectangle clip (`terrain/rect_clip.rs`: each split point a
function of its edge alone), so both sides of a chunk line take the same vertices. A chunk's
payload (mesh layers, lights and signs) is local to the chunk's centre, the frame of the terrain
patch over it, converted from the pieces' f64 positions by the terrain's own
`render_frame_local`; Godot places each chunk node at that centre (`ROAD-68-D` part 5, open
questions 23 and 24). Zebras cut no triangle: the asphalt under a crossing's bars carries its
paint frame. Since
`ROAD-68-G` part 2 a rebuilt chunk keeps its unchanged owners' triangles from the published chunk
and emits only the pieces built since the last publication, whose lanes alone are lifted onto the
new surface ([local commit publication](#local-commit-publication-road-68-g-part-2-2026-10-24)). The Rust to
Godot chunk payload stays byte-compatible, so Godot's committed-chunk renderer is unchanged apart
from the chunk frame: `NetworkMeshData` layers, owner ranges, the generation and acknowledgement
protocol, and removal tombstones. Layer 0, once reserved for road-owned earthwork, carries the
road skirts since `TERRAIN-07-B` (`skirt_*` keys; UV `y` is the bottom-vertex attribute), and a
bridge's fascias go with its concrete.

### Preview, commit, undo and load

**Context.** The road tool works on an immutable, versioned local context: the graph
neighbourhood, terrain samples, sites, and zoning and field reservations. It is captured when the
tool activates and refreshed when the simulation publishes a new generation. No per-pose work takes
the core lock, which also resolves the hover core-lock bug `ROAD-70` in
[`roadmap.md`](roadmap.md) (`get_agriculture_field_overlay_revision`, fixed in `ROAD-68-E`: the
render snapshot publishes the revision and the getter reads it lock-free).

As built (`ROAD-68-E` part 1): the context is the published view plus the preview's private
graph replica. The view shares terrain and water until their own generations change, the
roadbed in `O(1)` and the published road chunks in `O(1)`, and shares one graph copy with the
command's routing rebuild (undo and road bulldoze offer theirs; a commit's publication makes it);
the replica follows by the records
of the owners whose pieces the roadbed rebuilt. The commit tests the pieces it installs against
fields and painted cells, and its stroke corridor against parcels ([open questions 1 and
25](#road-68-open-questions)); since part 2 round 3 the view shares the field index (an `Arc`
the allocator copies on write) in `O(1)`, and since round 6 a persistent copy (`imbl` maps,
cloned in `O(1)`) of the whole zoning cell store, whose live maps stay plain: each publication
copies exactly the entries the live store's write journal names
([question 32](#road-68-open-questions)), beside the commit's graph copy on the shared pool
(round 7); the preview runs the commit's own test on its installed and replaced pieces against
the store's reserved cells, so it shows `field_overlap` or `zoning_cell_overlap` before the
click. Since round 5 every lot derivation is one planner (`CellLotPlanner`: cell generation and
lot claims on a cell store and lot queue it changes in place, against a parcel store it only
reads, recording its parcel changes as the store calls they stand for) and one installer that
makes those calls on the parcel store the plan read; the preview is to run the same planner on
its own copies (question 21 (b)) and draws nothing of it (user decision). The terrain
inputs are a persistent map published with the view in `O(1)` (round 3): per render patch the
refined cache entry, the assembly ledger, the site ownership and the building sites within the
widest road query margin. The core notes every patch whose inputs may change (generation bumps,
ledger stamps and acknowledgements, cache inserts and removals, ownership refreshes, site
changes within that margin) and rebuilds only those patches at the next publication; a world
reset or global terrain generation rebuilds every patch.

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

As built (`ROAD-68-E` part 1): stages 1, 2 and the road half of 4. The road payload is every
render chunk the stroke changes, built by the commit's own emission and splice
(`RoadRenderer::generate_preview_chunks`) into the view's published chunks and sealed, so a
stroke publishes exactly the previewed chunks. Since `ROAD-68-F` part 1 that holds for strokes
with crossings and signs too ([open question 26](#road-68-open-questions), closed): the
emission (`emit_into`) reads only the graph, the pieces and the terrain, never lanes. Zebra
paint frames and crossing signs take the pieces' crossings
(`RoadbedSystem::for_each_crossing`, oriented by `CrosswalkMarking::of_arm`, the rule the
pedestrian lanes own them by), and street lights their span sections and the terminal cap's
layout station. Tested bitwise in Rust
(`preview_chunks_with_crossings_and_signs_are_the_committed_chunks`: a T and a pass-through
continuation, flat and sloped) and in `road_junction_preview_test.gd` (every fixture, every
layer, `road_crossings`, `street_lights` and `road_signs`). Godot's preview display swaps the
chunk meshes, which carry the paint; the resident chunks' lights and signs stay shown until the
commit installs its own. The field and cell checks of the preview's pieces and the parcel
corridor check run at the commit only ([open question 25](#road-68-open-questions)). The
fingerprint is the road-tool surface
generation, the roadbed generation and the source and visual terrain generations: the roadbed
generation and the delta's dirty marks decide whether pieces install, so no per-edge or per-node
generation was added.

Stage 3 and the terrain half of 4 (round 3): the patches and tiles whose terrain the preview's
pieces change are the commit's (`road_terrain_tiles` over the preview roadbed's
`take_terrain_changes`, the function the commit's stamps use; round 4: tiles near changed chain
segments, or near changed pieces where a site grades, so a split trunk's far half is not
rebuilt, [patch selection](#footprint-and-terrain)); the preview's site test reads the published
sites of the tile's patch. Each patch's ownership (`road_terrain_patch_margin_m` on the edited
replica and the preview's roadbed, the patches in parallel), assembly scope, margins and site
query come from the one source function the core's requests and the commit's staged validation
also call (`refined_terrain_patch_source`); the
ledger is the published one with the preview's tiles stamped at a provisional generation later
than every stamp, as the commit's new generation is. The patches build with the commit's job
and entry functions on the shared pool. The terrain half of the fingerprint is each built
patch's source signature: the previous entry it reuses (by identity), the scope, the ownership
flags and margin, and the sites it reads. The payloads are the dictionaries
`poll_ready_terrain_patch_payloads` returns, with the shading masks baked as the payload
workers bake them; Godot stages them with `road_terrain_preview.gd` beside the road chunks.

**Commit.** A click submits the preview's plan as an ordered simulation operation.

- If the fingerprint is still current, the simulation installs the preview's pieces and terrain
  patches without recompiling, then applies the topology mutation (graph, lanes, zoning, parcels).
- If it is stale, the simulation recompiles locally and synchronously.
- Either way the pieces it installs must keep clear of committed fields and painted cells, or the
  staged edit rolls back ([open question 1](#road-68-open-questions)).

Godot keeps showing the preview until the committed generation is acknowledged, then swaps road and
terrain atomically, so no frame shows neither.

As built (`ROAD-68-E` part 1): the click submits the latest preview of exactly that stroke; with a
current fingerprint the commit uses its plan and installs its pieces when `can_install` holds,
otherwise it plans and compiles locally. The commit acknowledgement carries its generation; the
road tool holds the preview until that generation's chunks are installed, and the install
(`NetworkTool._road_mesh_installed`) releases it in the same call. When the commit installed
the preview's pieces, its staged terrain validation builds its own sources and compares their
signatures with the preview's, patch for patch; on a match it installs the preview's entries,
re-stamped with the commit's patch generations, instead of building them
(`preview_terrain_adopted`), otherwise it builds them as before. The comparison is of the
inputs themselves, so a context that missed a change only costs the reuse, never a wrong patch.
The road tool holds the terrain preview until the commit's own patches replace it: the terrain
renderer's `patch_render_will_change` releases it in the call that installs them.

**Undo.** The undo record holds the replaced pieces (`Arc`) and terrain patch revisions; undo
restores them by swapping.

As built (`ROAD-68-E` part 1): undo swaps the replaced pieces back when nothing rebuilt them
since, restores the graph in `O(delta records * log edges)`, and rebuilds the lanes of the
restored closure as a commit does (the agents on them detach and reattach, the connection lanes
of dropped nodes retire, and the entrances fronting the closure are derived again); a test
checks the result equals a full lane rebuild. A dense terrain
restore still rebuilds every lane, as it resets every road product. The record also keeps the
refined terrain entries the edit's validation replaced and the patches' generations after the
edit (round 3); when the roadbed swap held and no generation of those patches moved since,
undo installs the pre-edit entries again at the swap's new generations (a patch that had none
loses the edit's), so the restored terrain is the pre-edit terrain itself; otherwise the touched
terrain rebuilds.

**Load.** Load compiles every piece from the graph in parallel. A junction that fails layout or
the profile solve fails the load with its typed error.

### Commit consequences

Before [`ROAD-68-G`](#road-68-g--commit-consequences-inside-one-frame) the commit rebuilt the CCH
from scratch on the sim thread (`rebuild_cch_and_check`, with an O(edges) connectivity log). It
did not mark the flow fields: they rebuilt in the next running frame, when the hierarchy's build
generation no longer matched theirs (`flow_graph_revision`), and a paused game never ran that
frame. An earlier version of this section said the commit went through
`TransitNetwork::rebuild_pathing`, which marks every field; that is the frame-start path.

Since `ROAD-68-G` ([D7](#product-decisions)) the commit, undo, road bulldoze and the other
topology editors rebuild the CCH as a [pure job](simulation.md#pure-jobs) on a graph copy (the
commit's is the road-tool snapshot's) and adopt it one frame later. The simulation frame holds
until then, so no route query sees the new graph with the old hierarchy, and a late job blocks
only the adoption frame, waited for outside the core lock
([routing contract](entrance_and_exit.md#routing-after-a-topology-edit-road-68-g-d7)). Flow
fields refresh at the adoption frame. Lane invalidation is local; the agents' route-state test
is one parallel pass over the agents.

Since `ROAD-68-G` part 2 the commit's other consequences follow the edit
([local commit publication](#local-commit-publication-road-68-g-part-2-2026-10-24)): the render
publication re-lifts only the rebuilt pieces' lanes and re-emits only their triangles, keeping
the rest of each changed chunk from its published mesh, and the entrance rebuild derives only
the entrances whose inputs the edit changed. The commit still regenerates the edit's dirty
cell-lot regions synchronously ([open question 21](#road-68-open-questions)); since part 3 those
are the corridors of the roads whose geometry or curb frames changed, not of every road the
edit touched ([commit cell lots](#commit-cell-lots-road-68-g-part-3-2026-10-24)).

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
| Chunk rebuild | `O(rebuilt pieces + changed chunks' vertices)`: the rebuilt pieces are emitted and the rest of each changed chunk is copied from its published mesh; lights and signs `O(covered edges)` |

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

- **Size.** There is no line limit (user, 2026-10-24: up-front estimates are not reliable).
  Each gate records the `roadbed/` production line count, tests excluded, and `ROAD-68-F` records
  each family's lines; large growth gets a written reason in the gate's record. Code is cut to
  remove duplication, dead code or needless generality, never to meet a count; comments that
  explain geometry or rules stay. Earlier gate records compare against the former 8,000-line
  estimate.
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
| D4 | Width change on a straight side | A linear taper along both approaches, implemented pre-D ([open question 9](#road-68-open-questions)); `ROAD-68-C` built a radial step at the node. |
| D5 | Footpath meeting a road junction | It joins the sidewalk ring at sidewalk height, with its end pinned there by the profile solve. Its ribbon ends at the sidewalk outer edge on shared vertices. Built pre-D (2026-10-24, [footpath junctions](#footpath-junctions-and-the-bend-crossing-fallback-pre-d)): the ribbon stands at `P + raised` from the outer boundary to a mouth 1 m beyond its farther edge point, so there is no ramp or step at the sidewalk; the mouth of an arm the footpath meets lies at least 1 m beyond it (its zebra stays at the corner, D8). |
| D6 | Roads without sidewalks | Keep the current raised 0.75 m shoulder. |
| D7 | Routing at commit | Confirmed by the user, 2026-10-23. Rebuild the CCH on an immutable graph snapshot as a scheduled job and adopt it at a fixed tick offset. Route queries issued meanwhile wait for adoption. If the job is late, the simulation blocks at the adoption tick, so results do not depend on machine speed. Revisit if the query backlog at adoption is too large at 100k buildings. |
| D8 | Crosswalks | Right outside the curve (user-confirmed 2026-10-24). On crossable arms the mouth moves 3 m beyond the corner tangent station, and the 3 m zebra fills that straight approach: its inner edge is where the curb stops curving. The crossing is square to the road at the strip's middle, its kerb points at the two carriageway edges; sidewalk lanes end on that line, as `ROAD-64` requires. A footpath beside a crossable arm moves only its mouth; the zebra stays right outside the curve, and the node piece covers the run between them (user, 2026-10-24, [open question 17](#road-68-open-questions)). Which arms cross (user-confirmed 2026-10-24, "keep the old behaviour"): every arm of a junction of three or more; exactly one arm of every degree-two node, bend or pass-through, as the legacy `ROAD-64` rule did; none at a road end, whose sidewalk wraps the cap. Only car roads with sidewalks cross, and node overrides adjust the rule. A pass-through node has no node piece, so its crossing is span-owned: it lies on the arm's span from the node out to 3 m, where the span keeps a section row on the crossing line; that row's carriageway-edge vertices are the kerb points. No other zebra sits on a span. |

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
| `ROAD-68-D` | done (2026-10-25, part 5) | Footprint chains and terrain patches; watertight terrain, road chunks and site pads in shared frames |
| `ROAD-68-E` | implemented, acceptance open (2026-10-25, part 2 round 7) | Synchronous preview, commit adoption, undo and load: part 1 has the context, the preview's road and terrain, commit adoption, local undo (pieces, lanes, terrain) and the load check; part 2 the user's decisions on the open questions (order-independent strokes, overlap checks, node moves, pinned knots, the cell store's journal-synced view copy); open: the terrain stage's budget (`TERRAIN-07`) and question 21 (b)'s preview half (paused, question 33) |
| `ROAD-68-F` | done (2026-10-25, part 2 round 4) | Footpaths, mixed profiles, bridges, tunnels, crossings, street furniture and overlap queries: part 1 builds zebra paint, street lights and signs from the pieces for preview and commit alike (question 26 met); part 2 the road families without terrain (roads without sidewalks, mixed profiles, bridge decks, tunnel road pieces, one overlap walk), and round 4 fixed the preview payload's codegen regression. Moved to `TERRAIN-07`: bridge abutments and tunnel portals (`TERRAIN-07-C`, with closing the grounded chains at structures, open question 19), building sites, driveways and paving (`TERRAIN-07-C`), bridge fascias (`TERRAIN-07-B`). Left open: question 17's item on footpaths in a sector without a sidewalk |
| `ROAD-68-G` | implemented, acceptance open | Commit consequences inside one frame (routing, flow fields): D7 landed; part 2 made the publication, entrances and agent pass follow the edit; part 3 made the commit's lots follow the changed roads (28.6 ms at 100k, 41.7 before); part 4 21.8 ms; part 5 fixed the pure-job deadlock, 19.3 ms median before the `ROAD-68-D` rebase; on `road` the 100k commit is rejected and its terrain validation costs 15–69 ms (`ROAD-68-D` part 6); the commit still misses a frame ([open question 21](#road-68-open-questions)) |
| `ROAD-68-H` | open | Rendered acceptance and final cleanup |

**Plan changes (2026-10-24, user).** Pre-D work, before terrain:
- open questions 14 (D5 footpath junctions) and 15 (drawing-order dependence)
- question 13's bend fallback
- question 9's D4 taper

Questions 14 and 13 are done ([footpath junctions and the bend crossing
fallback](#footpath-junctions-and-the-bend-crossing-fallback-pre-d)).

`ROAD-68-E` also takes open questions 1, 2, 7 and 8, and question 11's follow-up (c). `ROAD-68-F`
keeps the rest of its families. The crossings, D4 and D5 are now pre-D work.

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

`ROAD-68-D` (25): `ROAD-68-D` re-enables 24; 23 pass, and
`kuopio_recorded_profiles_and_terrain_preserve_quality_limits` moved to `ROAD-68-E` (a recorded
stroke fails node 5 with `node_piece_failed` at `79e7a476` as well, so it predates `ROAD-68-D`;
the recorded placements predate `ROAD-68-C`, [open question 8](#road-68-open-questions)). The
last, `cars_and_walkers_enter_and_exit_on_compiled_yards`, passes since part 5 (site pads in the
terrain patch, open question 23); none stays disabled under `ROAD-68-D`.

| Module | Tests | Reason |
| --- | --- | --- |
| `nodes::sim::core::tests::building_site_terrain` | `access_snapshot_pad_ownership_overrides_pending_visual_terrain`, `cars_and_walkers_enter_and_exit_on_compiled_yards`, `cars_pitch_and_roll_on_compiled_yards_in_both_access_directions`, `engineered_queries_reject_render_rejected_ground`, `interpolated_vehicle_batch_keeps_lane_poses_and_supports_changed_xz`, `kuopio_reference_supports_zoned_graded_yards_without_changing_fixture`, `large_paved_yards_remain_flat_over_hills_and_depressions`, `new_world_clears_populated_city_and_engineered_terrain`, `site_changes_far_apart_refresh_only_the_patches_they_touch`, `small_commercial_pads_leave_a_watertight_sloping_frontage`, `yard_hedges_stand_on_the_graded_lawn_of_a_sloping_yard`, `yard_material_changes_reuse_cdt_but_not_final_paving_buffers`, `zoned_yards_cover_uphill_and_downhill_frontages` | the frontage paving seam takes road heights only where the road's outer chains enter the terrain CDT (moved from `ROAD-68-B`) |
| `nodes::sim::core::tests` | `planned_topology_terrain_rejection_restores_split_dependents` | road terrain patches |
| `nodes::sim::editing::tests` | `accepted_road_has_current_terrain_payloads_before_publication`, `terrain_rejection_restores_split_dependents_and_preserves_undo_history`, `undoing_road_restore_invalidates_derived_render_cache`, `undoing_road_restore_invalidates_road_locked_terrain_state` | road terrain patches |
| `nodes::simulation_node::tests::async_payload` | `conservative_road_grading_pad_filters_only_margin_only_engineered_patches`, `unmatched_surface_preserves_road_ownership_while_site_ownership_refreshes` | road terrain patches |
| `nodes::simulation_node::tests::cdt` | `terrain_cdt_adjacent_tiles_emit_identical_shared_side_vertices`, `terrain_cdt_guides_use_shared_source_height_and_final_grade_limits`, `terrain_cdt_input_keeps_multi_loop_tie_in_guides_unconstrained`, `terrain_cdt_local_bounds_expand_for_required_grading_distance` | road terrain patches |

`ROAD-68-E` (5 at the gate; part 1 re-enabled
`undoing_fourth_junction_mouth_restores_exact_pre_edit_surface_topology`, and part 2
`sloped_junctions_do_not_depend_on_drawing_order`, which now covers the T, Y and end-to-end
fixtures, and the three tests relabelled from `ROAD-68-F` below; part 2 round 2 the Kuopio saved
reference, rewritten as `kuopio_saved_reference_loads_as_saved` on the regenerated save, and the
hillside houses without their terrain step; round 3 the recorded Kuopio strokes, as
`kuopio_recorded_strokes_commit_with_every_node_solved` and
`kuopio_recorded_profiles_preserve_quality_limits` ([open question 31](#road-68-open-questions)),
and removed the mid-span order test, which [open question 30](#road-68-open-questions)'s rule
test replaces): none stays off.

`TERRAIN-07` (4: two split off by `ROAD-68-E` part 2 round 2, one by round 3, one by `ROAD-68-F`
part 2): the user's rewrite of the terrain surface core makes the exact CDT cut, refined patches
and site paving partition legacy, so these terrain-pipeline assertions are not kept passing on it:

| Module | Tests | Reason |
| --- | --- | --- |
| `nodes::sim::core::tests::road_terrain_plan` | `kuopio_saved_reference_terrain_builds_clean` | patch count and refined CDT patches of the regenerated Kuopio reference (passes today) |
| `nodes::sim::core::tests::road_terrain_plan` | `kuopio_recorded_terrain_builds_clean` | exported and tile seams, window bounds and CDT constraints after every recorded Kuopio stroke (passes today) |
| `nodes::sim::core::tests::zoning_buildability` | `hillside_houses_terrain_builds_clean` | refined CDT patches after the hillside houses on the regenerated fixture (passes today) |
| `simulation::terrain::cdt::tests::builder` | `cdt_promotes_high_delta_omitted_tunnel_source_to_retaining_wall` | tunnel portal boundaries require walls where widened samples cannot tie in; portals are part of the terrain rewrite (relabelled from `ROAD-68-F` by its part 2, with abutments, building sites, driveways and paving) |

`ROAD-68-F` (0; 7 at the gate, two re-enabled by the
[C follow-up](#tighter-corners-and-junction-crosswalks-follow-up), one by the
[footpath junctions](#footpath-junctions-and-the-bend-crossing-fallback-pre-d) and one by
[`ROAD-68-F` part 1](#markings-lights-and-signs-from-the-layout-road-68-f-part-1-2026-10-25)
(`a_residential_street_gets_evenly_spaced_lights_on_one_side_behind_the_kerb`), one added by
`ROAD-68-D`; the two node-move tests and `bridge_entry_points_never_use_the_global_pool` were
relabelled `ROAD-68-E`, which owns open question 2, and pass since `ROAD-68-E` part 2; the
tunnel portal CDT test, added by `ROAD-68-D`, was relabelled `TERRAIN-07` by
[`ROAD-68-F` part 2](#remaining-families-road-68-f-part-2-2026-10-25)). None is left.

`ROAD-68-K14` (0; 2 at `ROAD-68-K14`, both in `nodes::sim::core::tests::road_terrain_plan`,
`ROAD-68-K13`'s one relabelled): `kuopio_saved_reference_profiles_preserve_quality_limits` and
`kuopio_recorded_profiles_preserve_quality_limits` failed on up to 0.36 m past the 5 m
source-delta limit in junction blends ([open question 39](#road-68-open-questions)) and pass again
since `ROAD-68-K15`. None is left.

The 22 other `ROAD-68-F` tests (street lights, road signs, vegetation clearance and the site
support target) pass on span sections and were re-enabled in `ROAD-68-B`.

Godot scripts skipped by `run.sh --test`: none since `ROAD-68-E` part 2, which checks node moves
against painted cells (open question 2) and runs `zoning_cells_tool_test` again (it was skipped
until `ROAD-68-F`, relabelled `ROAD-68-E`).

`network_tool_chunk_renderer_test`, `road_junction_preview_test` and `road_preview_stream_test`
(`ROAD-68-E`: the preview's road mesh, junction scene and pose stream) are rewritten for the
synchronous preview and run again since `ROAD-68-E` part 1.

`zoning_road_tool_test` (`ROAD-68-B`: parcel lots need road frontage heights) passes again and
runs since `ROAD-68-B`. `zoning_reference_test` (`ROAD-68-C`: lots exclude the junction sidewalk
footprint) passes again and runs since `ROAD-68-C`.

#### `ROAD-68` open questions

These need a user decision; work that does not depend on them continues.

1. **Planned overlap checks before commit — decided 2026-10-24 (user): restore in `ROAD-68-E`** from the preview's
   junction and span pieces. The legacy commit tested the planned junction and
   span footprints against painted cells and fields (`PlannedRoadSurfaceQuery`). `ROAD-68-A` keeps
   only the corridor checks (`cells_overlap_road_corridor`, field corridor), in both the commit and
   plan readiness, so a junction corner may overlap painted cells until `ROAD-68-E`/`ROAD-68-F`
   restore the check from preview pieces. Options: accept on the branch (current), or block commits
   that touch reserved cells near nodes.
   **Implemented in `ROAD-68-E` part 2 (2026-10-25), at the commit.** The commit tests the pieces
   it installs, which are the preview's own when it adopts them and its local compile otherwise,
   against committed fields and painted cells (with their occupied grace claims) before it accepts
   the edit: each piece's top triangles against the fields and the reserved cells in its bounds
   (`ReservedOverlap`, `FieldClearance::overlaps_road_piece`,
   `ZoningSystem::cells_newly_covered_by_road_piece`); an overlap rolls the staged edit back with
   `field_overlap` or `zoning_cell_overlap`. Only land the edit newly covers counts: a cell is
   refused when a new piece covers it and none of the pieces the edit replaces did, a field when
   a new piece overlaps one and no replaced piece overlapped any. Cell grids are laid out against
   the straight road corridors (`overlaps_road_corridors`), so cells beside an existing junction
   corner or road end may already lie under its piece; testing every rebuilt piece refused every
   edit that re-lays such a junction (found with the narrow-marks digest test's node move). The field and cell corridor checks are gone from the
   commit and from plan readiness (`RoadEditPlan::status`): a plan has no pieces to test, so a ready
   plan is refused at the commit with the graph, undo history and roadbed unchanged. The parcel
   corridor check stays. `a_field_in_a_junction_corner_refuses_the_road` (a field in a T's curb
   return, clear of both roads' corridors) fails with the corridor checks and passes now. Still
   open: the preview does not show the overlap before the click, because the view publishes no
   field or cell index ([open question 25](#road-68-open-questions)).
2. **Node moves over painted cells — decided 2026-10-24 (user): restore in `ROAD-68-E`**, with question 11's (c). The node-move reservation check compiled a local surface.
   It is removed until `ROAD-68-F`; node moves are currently unchecked against cell reservations.
   **Implemented in `ROAD-68-E` part 2 (2026-10-25).** A node move tries the move on the graph,
   compiles the moved closure into an `O(1)` copy of the published roadbed, puts the graph back from
   a local checkpoint and refuses, with nothing else touched, when a node of the closure fails its
   layout or profile solve or a piece newly covers a field or painted cells (`SimCore::try_graph_edit`,
   question 1's test; the reason goes to `last_road_rejection`); otherwise the checkpoint becomes
   the move's undo record and the move proceeds as before. The narrow-marks digest test's two
   1 m moves of painted grid junctions carry their roads onto painted lots, so they are refused
   now, and the test checks that the refusals change no product. Both
   movement tests, `bridge_entry_points_never_use_the_global_pool` and `run.sh`'s
   `zoning_cells_tool_test` run again (the global-pool test's save now settles the routing hold
   first, `save_game_settled`: a harness change `ROAD-68-G`'s D7 required while the test was off).
3. **Site CDT margin — decided 2026-10-24 (user): resolved by `ROAD-68-D`'s shared grading envelope.** Building-site loops used the legacy seam-vertex grading margin for their
   sample bounds. They now use the base local sample margin. Steep site tie-ins may sample less
   terrain until `ROAD-68-D` defines the grading envelope for both roads and sites.
   **Done in `ROAD-68-D`:** site loops take their influence margin from the shared envelope
   function (`grading_envelope_margin_m`, vertex heights against the terrain below), as road
   loops take theirs from their rays ([footprint and terrain](#footprint-and-terrain)).
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
5. **Straight sectors narrower than a half turn (`ROAD-68-C`) — decided 2026-10-24 (user): keep the miter.** The [round join](#junction-layout)
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
   **Pre-D:** at `pi` and above the boundaries now miter on the bisector too, on the first offset
   segment extended behind the node, which removes a rounding-sized arc of sliver strips; a width
   change tapers instead of stepping (D4, question 9).
6. **Obtuse corners between unequal widths (`ROAD-68-C`) — decided 2026-10-24 (user): keep the backward
   extension.**
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
7. **Stable arms pinned at both ends (`ROAD-68-C`) — decided 2026-10-24 (user): in `ROAD-68-E`, keep pinned mouth
   knots through the resample.** The profile solve pins an arm to `P` up to its
   mouth with a knot there. An adapted arm takes the knot in its control polyline, so it survives
   every later solve. A stable arm's control polyline must not change, so it takes the knot in its
   physical profile only, and keeps its physical knots on later solves unless that solve adds new
   supports. When the other end's solve does add supports, it resamples the physical profile on
   the control polyline and drops this end's knot; the node then fails with `MouthOffPlane`
   (`junction_profile_mismatch`). No fixture hits it. Options: accept the rejection (current); keep
   pinned physical knots through the resample; or let the solve add mouth knots to stable control.
   **Pre-D (question 15):** the case no longer exists. Every solve derives an edge's physical
   profile from its control polyline with the mouth knots of both ends' current pins, so no solve
   can drop the other end's knot, and nothing distinguishes stable from adapted arms. `ROAD-68-E`
   only needs to confirm it on the preview path.
   **Confirmed in `ROAD-68-E` part 2:** `a_far_junction_keeps_the_near_mouth_pinned_on_the_preview_path`
   gives the trunk link east of a side road a junction at its far end in a preview; the near
   junction is in the preview's closure and compiles, and the commit installs the preview's piece
   with no failed node, flat and sloped. No code change.
8. **Saves from before `ROAD-68-C` (`ROAD-68-C`) — decided 2026-10-24 (user): accept the load repair; regenerate
   the reference fixtures with `ROAD-68-E`'s load rebuild; no migration code.** Their clips come from the legacy setback rule
   (about 5.5 m at a right-angle T) and their profiles are pinned only that far, while layout mouths
   sat at 9.5–16 m at the gate, and closer with the follow-up's 3.5 m radius (2.5 m closer at a right
   angle). On load most of their junctions fail the 1 mm mouth check, and the existing
   load repair re-solves them after recomputing their clips: the Kuopio reference fixture loads,
   but about ten of its junction profiles change. Options: accept the load repair (current); re-pin the
   reference fixtures; or a save migration with `ROAD-68-E`'s load rebuild.
   **`ROAD-68-E` part 2, round 2 (2026-10-25): the Kuopio save is regenerated; the recorded
   placements and the hillside fixture cannot be yet.**
   - **Kuopio save.** Loaded by this build (the load repair re-solves the rejected junctions) and
     saved with `save_game_settled`, nothing else: only `network_edge_geometry` (4,068 → 5,073
     rows, the repaired profiles' support knots) and `saved_lanes` (2,231 → 2,310) change; every
     other table keeps its row count. `placements.json` pins `550a6e82…` (was `5ac4cb0f…`); all
     8 `tools/test_road_terrain_replay.py` tests pass (70 live edges, source terrain unchanged).
     The saved-reference test becomes `kuopio_saved_reference_loads_as_saved`: no junction is
     repaired, every saved control and physical point loads bitwise and no node fails. Its refined
     terrain check (28 clean patches; 27 on the legacy save) moved to
     `kuopio_saved_reference_terrain_builds_clean`, ignored under `TERRAIN-07` (the user's
     rewrite of the terrain surface core makes the CDT patch pipeline legacy; it passes today).
     The load repair itself stays covered by
     `headlight_house_junction_full_load_compiles_with_terrain_earthwork`.
   - **Recorded placements.** `kuopio_recorded_profiles_and_terrain_preserve_quality_limits`
     replays the recorded strokes on the source world, so it does not read the save. Case
     `kuopio_05`, attempt 14 (accepted by the legacy compiler) is refused today: node 5 fails its
     piece at the commit with `MouthOffPlane { arm: edge 0, End }` (`render_geometry_invalid`; the
     commit's detail names the error since this round). That is not a fit rule refusing the
     stroke: the profile solve finalized an arm more than 1 mm off its node's plane, a defect in
     the solve for this junction. The strokes are recorded input, not derived data, so
     regenerating them would hide it; the test stays off until the solve is fixed (next step:
     reproduce `kuopio_05` attempts 11-14 in a unit test of the junction solve). Its patch seam,
     window and CDT assertions belong to the legacy terrain pipeline (`TERRAIN-07`) and move
     behind that tag when the road half runs again.
   - **Hillside fixture.** `residential-buildability.json` holds legacy clips and copies its
     control polylines as physical profiles; node 2 fails `MouthOffPlane`, so its tiles report
     `roadbed_chain_open_end`, which only the test's last step (the refined terrain) saw: the
     feasibility verdicts and the five houses, the family house on parcel 7, pass. The test runs
     again without that step, which moved to `hillside_houses_terrain_builds_clean`, ignored under
     `TERRAIN-07`. Re-solving the clips and profiles as the load does makes the roads overlap the
     recorded parcels (`OverlapsExistingParcel`), so regenerating the fixture means regenerating
     its parcels, which renumbers the lots the test names; not done.
   **`ROAD-68-E` part 2, round 3 (2026-10-25): the recorded placements run and the hillside
   fixture is regenerated.**
   - **The `MouthOffPlane` of `kuopio_05` attempt 14: a plan-excerpt defect.** The stroke ends
     on attempt 11's road east of attempt 12's T, more than 100 m from attempt 13's T (node 5).
     The solve re-derives every edge at the T it splits, including the link to node 5, and pins
     that link's far end from node 5's own solve ([Heights](#heights), "Scope and cost"). The
     plan solves on a local excerpt that held one ring of incidence around the edges near the
     stroke, so node 5 had only the link: one plane incident, no pin, and the committed link
     left node 5's plane, which then failed its mouths. The live graph's solve was right; only
     plans (commit and preview) were wrong. Fix: the excerpt takes two rings of incidence, and a
     plan whose affected nodes or their edges' far nodes lack an arm in the excerpt is refused
     (`validate_against_graph`). The terrain spike's `city_grid_t` (closed 96 m blocks, a T split
     between two streets, 8% slope) failed the same way (`node_piece_failed node=15`) and
     commits now (`a_street_grid_t_split_on_an_8_percent_slope_commits`, fails with one ring).
     Where no node failed, one-ring plans still committed profiles a fresh solve would not
     give (the same grid flat: edge 9 without its far pin), so the fix changes outcomes: gate
     G's determinism digest moves from `6252b7ba71c5a6f1` to `da3d23b6c8b63d20` by this fix alone,
     and a commit beside the painted city no longer rebuilds an unchanged far link
     ([round 3 record](#part-2-round-3-2026-10-25-branch-road68-e3-work-in-progress)).
   - **Recorded placements.** The replay is split: `kuopio_recorded_strokes_commit_with_every_node_solved`
     (every recorded stroke commits, no node fails, 158 profiles; failed before the fix at
     `kuopio_05` attempt 14) runs; the fixture's quality limits
     (`kuopio_recorded_profiles_preserve_quality_limits`), which the attempt-14 failure hid,
     failed on three profiles and run again since [open question 31](#road-68-open-questions)'s
     plane sample; the patch
     seam, window and CDT assertions moved to `kuopio_recorded_terrain_builds_clean`, ignored
     under `TERRAIN-07` (passes today).
   - **Hillside fixture.** Regenerated with the load rebuild, no migration code: the legacy
     fixture's roads were loaded as before and passed through `rebuild_loaded_graph_runtime`
     (node 2 rejected, re-clipped and re-solved: clips 6.29 m to 11.80 / 11.81 / 8.56 m, every
     physical profile re-derived), and each parcel was re-authored on the same edge and side at
     the same frontage point: the point its legacy `t` named on the legacy profile, as a `t` on
     the regenerated profile (all sixteen move by 0 m in plan; `t` changes in the fourth
     decimal because the physical length grew by up to 0.05 m). Parcel ids are unchanged, so the
     family house still lands on parcel 7. The fixture now stores each edge's control polyline
     and physical profile (`points`, `physical`) and the regenerated clips; the loader takes
     them as a save holds them and checks every node builds. The source grid and nodes are
     byte-identical. `hillside_residential_sites_select_constructible_houses` passes, and so
     does the `TERRAIN-07` half (`hillside_houses_terrain_builds_clean`, still ignored there).
   **`ROAD-68-K13` (2026-10-25): the regenerated Kuopio save keeps the recorded session's
   profiles.** Its control polylines and node heights are the session's stroke solve's (before
   `2ec65ca9`), which the load rebuild re-derives from and cannot change; on 21 of its 70 edges
   they break the replay's quality limits, up to grade 4.56 and 37.6 m off the ground
   ([open question 38](#road-68-open-questions); no maximum grade, question 37).
9. **Width steps on straight sectors (`ROAD-68-C`) — decided 2026-10-24 (user): build D4's taper at the start of
   `ROAD-68-D`.** D4's radial step has no face of its own. A
   straight sector between a narrower and a wider carriageway gets the generic strips, which are
   zero-area slivers along the step, and the carriageway-edge curb face runs the whole step, up to
   the wider carriageway edge. The piece is watertight (fixture `width_step`), but beyond the
   narrower arm's sidewalk a 12 cm face stands at the wider carriageway's edge and the outer chain
   runs along the step at raised level, which `ROAD-68-D` will see. Options: accept until
   `ROAD-68-F`'s taper (current); build explicit step faces and chain segments now.
   **Resolved pre-D: the taper is built** ([junction layout](#junction-layout),
   [pre-D](#edit-order-independence-pre-d)). The fixture `width_step` and new width changes at a
   four-way, a bend and nearly straight sectors either side of `pi` have no sliver triangles,
   curb faces only where a raised band meets the carriageway, and outer chains along their
   sidewalk; with the step restored the test fails on `width_step`.
10. **Crossings before `ROAD-68-F` — resolved 2026-10-24: keep the layout rule and move the
    lanes, including the bend rule (user decision).** The layout's crossable rule (car-road arms
    with sidewalks at junctions of three or more arms; bend arms only when the node forces a
    crossing; none where a node override removes it) now decides both the 3 m mouth extension and
    the pedestrian lanes' crossings, which take the layout's zebra strip
    ([C follow-up](#tighter-corners-and-junction-crosswalks-follow-up)). Nodes without a layout
    have no crossing. The two disabled lane and sign tests pass again. Open question 12 later
    restored one crossing per degree-two node.
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
      a bug. **Part 2 (2026-10-25):** node moves, crosswalk overrides and edge class changes try
      their graph edit, compile its closure into a scratch roadbed and refuse when a node fails or
      a piece newly covers reserved land (question 2, `SimCore::try_graph_edit`; the crosswalk
      override's earlier layout-only test is replaced). **Round 2 (2026-10-25):** the other three.
      - *Terrain edits.* The terrain sync re-graded every standard road and copied its control
        polyline over its physical profile without the junction solve, so every sloped junction
        lost its pins: a 2 m bump beside the sloped T left node 2 failed
        (`a_terrain_edit_beside_a_sloped_junction_leaves_no_failed_node`, failed before). The sync
        now re-solves every node's profiles from the re-graded polylines (`O(nodes + edges)`, as
        the sync already is), and the test checks the edit applies with freshly solved profiles.
        A terrain edit that still leaves a failed node is rolled back from the undo record it
        pushed, before anything publishes (`node_piece_failed` in `last_road_rejection`).
      - *Road bulldoze.* The removal is tried first (`try_graph_edit`: delete the edge locally,
        re-clip and re-solve both ends, compile the closure, restore) and refused with nothing
        touched when a node fails. No removal among the explored ones fails (a T with a 2+2 and
        a 1+1 half of 20 or 30 m, a side road split 20 m from the T, flat and sloped), so the
        refusal path is the shared mechanism the node-move tests drive; the accepted path stays
        bitwise (`node_moves_and_removals_leave_freshly_solved_profiles` and the bulldoze tests).
        The removal itself still rebuilds adjacency for the whole graph (`O(edges)`).
      - *Undo.* Undo restores a state that compiled when it was published, so it is not refused:
        debug builds compile the restored closure and assert no node fails.
      - *Round 3 (2026-10-25), the terrain spike's `t_cutfill`.* The `TERRAIN-07` spike (branch
        `road68-Bspike`, on `359406c1`, before round 2) saw a T drawn on flat ground lose its
        junction piece when the ground was then sloped 8% under it. On this branch the edit
        stands with every node built and freshly solved profiles
        (`an_8_percent_slope_under_a_flat_t_leaves_every_node_built`); with round 2's re-solve
        removed it fails, so round 2's terrain-sync fix is what the spike was missing.

12. **Walkers switch sides only on crossings (`ROAD-68-C` follow-up) — resolved 2026-10-24: the
    user chose "keep the old behaviour".**
    - **The problem.** Under the first D8 rule (open question 10), walkers crossed only on
      junction arms and forced bend arms. Unforced bends and pass-through nodes had no crossing.
      The walking planner routes on the road-node CCH, which has no sidewalk sides, and its cost
      forecasts use a whole-network proof that every node lets every arriving sidewalk reach every
      leaving one (`ConnectorTargets::complete` in `agents/tick/planning/candidate/query.rs`). One
      node without a crossing made that proof false for the whole network: walk forecasts quoted
      per lane pair, and a walk that had to change sides without a crossing on its node path
      failed to plan. The street-grid benchmark city's four corner bends made its proof false.
    - **Decision.** Every degree-two node crosses once, on the legacy-ordered arm, and junctions
      keep a crossing on every arm ([D8](#product-decisions)). A bend's crossing is its node
      piece's strip right outside the curve; a pass-through node's lies on its arm's span beside
      the node. A road end's walk round its cap is its side change. The proof holds again,
      including on the street-grid city
      ([follow-up 2](#one-crossing-per-degree-two-node-follow-up-2)).
    - **Later.** Placing crossings where walkers need them, rather than by this mechanical rule,
      remains `ROAD-58`.
13. **Degree-two crossings that do not fit (`ROAD-68-C` follow-up 2) — decided 2026-10-24 (user);
    done pre-D.** A bend crossing that does not fit moves to the other arm, like the pass-through
    rule; reject only when neither arm fits ([done](#footpath-junctions-and-the-bend-crossing-fallback-pre-d):
    `NodeArms::layout` moves the default crossing to the other arm only when the first arm does
    not fit it, that is when the layout fails with `ArmTooShort` or `CornerDoesNotFit` on that
    arm, the two errors whose budgets the crossing's depth enters; any other error rejects the
    node at once, and when the other arm fails too the node fails with the first arm's error. The
    arms then record the moved crossing, so nothing reads the default one; a forced crossing never
    moves ([pre-D review fixes](#pre-d-review-fixes)). The remaining choices below stand:
    - A bend's crossing arm takes the 3 m strip as a junction arm does. If its layout then does
      not fit, the edit is rejected with the existing reasons (`junction_arm_too_short`,
      `junction_corner_does_not_fit`). So a bend with a short arm that `16354d77` accepted can be
      rejected now, and a save holding one fails to load (open question 11).
    - A pass-through arm crosses only if the strip fits within half its edge, measured
      horizontally: the budget a layout gives each end, so the strip always ends before the far
      node's mouth. Otherwise the default crossing goes to the other arm, and with neither the
      node has none.
    - An override of `false` on a degree-two node's first arm moves the default crossing to the
      other arm, as the legacy rule did; `false` on both arms leaves the node without one.
    - Nodes with a footpath arm have layouts since question 14; their road arms cross by the same
      rule.

14. **Road junctions with a footpath arm (D5) — decided 2026-10-24 (user): pull forward before
    `ROAD-68-D`; done pre-D ([footpath junctions](#footpath-junctions-and-the-bend-crossing-fallback-pre-d)).**
    - **The gap.** `NodeArms::collect` returns `None` for any node with a footpath arm, so such
      a junction has no layout. Its road arms get no zebra and no corner walks, and the spans
      simply end.
    - **Effect on walking.** Walkers cannot cross the road there. `ConnectorTargets::complete`
      becomes false network-wide, so walk forecasts degrade everywhere.
    - **Effect on the perf contract.** This may explain the baseline-30k hour mean rising from
      3.31–3.40 to 4.10–4.11 ms in the `9d0b187f` perf-contract run. That is unconfirmed: the
      snapshots were regenerated and no control build ran.
    - **Decision.** Give these junctions a normal layout. Road arms keep their crossings and
      corners, and the footpath joins the sidewalk ring at sidewalk height (D5).
    - **Verification.** Check the proof on the perf-contract cities, then re-run the economy
      performance contract against a matched control.
    - **Result (2026-10-24).** The perf-contract cities are street grids without a single
      footpath, and their walking proof holds on both tiers
      (`perf_contract_city_walking_proof_holds`, on the shared snapshots). Footpath junctions
      therefore cannot explain the 3.31–3.40 → 4.10–4.11 ms rise. The matched A/B of the
      integrated pre-D state against `8712b69f`
      ([below](#footpath-junctions-and-the-bend-crossing-fallback-pre-d)) gives identical work
      counts and hour means within run-to-run spread: the control's own baseline-30k mean moved
      4.09–4.57 ms across two sessions of the same day. The earlier rise compares runs from
      different sessions and cities (pre-`ROAD-68` `SIM-02-K` part 7 against regenerated
      `ROAD-68-C` cities); attributing it needs a control from before `ROAD-68` in one session.
      The proof was withdrawn on networks with a footpath dead end, whose single mouth had no
      turn-back connector; that is fixed.
15. **Drawing-order dependence (`ROAD-68-C`) — decided 2026-10-24 (user): root-cause before
    `ROAD-68-D`. Fixed on flat ground pre-D; the slope remainder is question 16.**
    - **The symptom.** The Y reference fixtures render about one colour level differently at
      the junction core after follow-up 2.
    - **Likely cause.** Each Y is drawn stem first, so its node is briefly a bend, and the bend
      crossing's mouth knots may persist in the profile.
    - **Rule.** Invariant 6 requires that an edit history producing the same graph produces the
      same pieces. Fix the cause, and add an edit-order test.
    - **Root cause.** Four history channels, found by committing the reference Y and T fixtures
      in different orders through `SimCore::commit_road`
      ([pre-D](#edit-order-independence-pre-d)): the finalizer wrote mouth knots, 2 m supports and
      plane heights into the control polylines of newly drawn arms, and kept old physical knots on
      existing ones; its corridor ranking gave existing roads a bonus, so a Y's plane tilted
      towards whichever branch came first; junction layouts read the physical profile, so clips
      depended on the previous solve's knots; and strokes took their heights from the highest
      visible road triangle, so a branch drawn over a terminal cap climbed its 12 cm sidewalk.
    - **Result.** All four are fixed. Flat T, 2+2 T and Y fixtures (25°, 40°, 60°) now produce
      bitwise-identical graph profiles, clips, node pieces and spans in every tested order
      (`road_edit_order::flat_junctions_do_not_depend_on_drawing_order`). On the 2.5% slope
      they still differ, by question 16.
16. **Strokes pinned to the pieces their edit rebuilds (pre-D, from question 15) — decided
    2026-10-24 (user): option (a) with a crossing exception, in `ROAD-68-E`.** A stroke never
    reads heights from the pieces its own edit rebuilds: the node pieces at the nodes it joins and
    their incident spans. At a mid-span crossing it still meets the crossed road at that road's
    height, read from the crossed edge's graph profile, not from its pieces, so a road crossing a
    raised road keeps meeting it on top. Which road was drawn first still sets a crossing's
    height; that is the authored-state rule, not a history channel. The preview's topology plan
    (`ROAD-68-E`) runs before stroke preparation, which (a) needs; the sloped edit-order test
    re-enables there.
    - **The problem.** A stroke's prepared profile pins to the visible carriageway it crosses,
      so a road meets a road it crosses or joins at that road's height. Near the node it joins,
      that carriageway belongs to the node piece and spans the edit itself rebuilds: a terminal
      cap, a bend or a neighbouring branch, depending on drawing order. On flat ground those
      heights equal the terrain bitwise. On a slope they are the node plane `P(x, z)` or a span's
      interpolated profile in `f32`, which differ from the terrain sample by about one ulp.
    - **Effect.** The authored control heights near the node differ by an ulp between drawing
      orders, so strictly the graphs differ. In a symmetric Y the two corridors (stem with
      either branch) then tie to rounding, and the tie decides which branch takes the plane's
      authority and supports: a visible difference in the core's tilt.
    - **Evidence.** `road_edit_order::sloped_junctions_do_not_depend_on_drawing_order` (ignored
      until decided) fails on the T at index 1 and 5 of the through halves and on the Y at index 1
      of each branch.
    - **Options.** (a) Exclude the edit's own dirty closure (node pieces at the nodes it joins or
      splits, and their incident spans) from stroke support; this needs the topology plan before
      the profile preparation. (b) Accept: different stroke heights are a different graph.
      (c) Pin strokes to the terrain within the node extent of nodes they join.
    - **Current default.** Carriageway-only pins (pre-D); nothing else changed.
    - **Implemented in `ROAD-68-E` part 2 (2026-10-25).** `plan_road_edit`
      (`road_edit/support.rs`) prepares and plans the stroke, reads off the plan the pieces it
      rebuilds (`RoadTopologyPlan::rebuilt_pieces`: the spans of its dirty source edges, the pieces
      of its solved and merged source nodes, the source edges it splits and the nodes its ends
      join) and, while the last preparation read the carriageway of such a piece, prepares and
      plans again with those pieces excluded too. The exclusion only grows and every further pass
      adds a piece the previous one read, so the passes end: one for a stroke over open terrain,
      two for a join or crossing. Over an excluded span of an edge the stroke splits, a sample
      takes that edge's physical profile height (pinned); a stroke end that joins a node takes the
      node's graph height, so a raised road end is still joined and snapped (the join rule alone
      would leave that end at the source terrain, up to the fill height below the node). The bridge
      and tunnel classification reads the same support. The commit, the preview and the editor's
      checks all plan through this function. `sloped_junctions_do_not_depend_on_drawing_order` runs
      again on the T fixtures (through road first and last) and passes; before, the sloped test
      failed on the Y at the first branch's samples beside the node (fresh run) and, by the
      evidence above, on the T. `a_crossing_of_a_raised_road_meets_it_on_top` covers the
      crossing exception on a road filled 1 m and more over a pit. The reference probe changes in
      `y_25/true` only (re-recorded with attribution). With [open question 30](#road-68-open-questions)
      (a) the sloped Y fixtures are order independent too; the cross and two-junction fixtures
      still depend on drawing order through mid-span meetings (question 30).

17. **Footpath attachment choices (pre-D, D5) — zebra placement decided 2026-10-24 (user): the
    zebra stays at the corner and only the mouth moves; done in the
    [pre-D review fixes](#pre-d-review-fixes). The other items below keep their defaults.**
    - **Mouths move for footpaths beside an arm.** A footpath whose edge meets a road arm's side
      run (roughly within 35° of a two-lane arm, and every footpath across a straight road) puts
      the arm's mouth 1 m beyond the meeting point. A crossable arm's zebra stays right outside the
      corner (D8's strip from the tangent station out 3 m) and the node piece covers the run
      between the zebra and the mouth. Without the mouth move the footpath's edge would end on or
      beyond the mouth (fixture `through_road`: at `s_min` exactly). Before the decision the zebra
      moved out with the mouth; attaching footpaths to spans was the third option.
    - **Rejections.** A footpath shorter than about twice its mouth station (8–12 m) now fails
      `junction_arm_too_short`, and an edge that meets no boundary inside the node fails
      `junction_layout_invalid` (`FootpathDoesNotJoin`). Saves holding such a footpath fail to
      load (question 11). Before, footpath nodes had no layout and always passed.
    - **Old saves.** A save from before this change holds unpinned footpaths and legacy clips at
      footpath nodes; the load repair re-solves them (question 8), as
      `headlight_house_junction_full_load_compiles_with_terrain_earthwork` now expects.
    - **Join route.** An attached footpath joins its sector's walk where its centreline meets it,
      not at the point nearest its end, which at shallow angles cut across unpaved ground. The walk
      is the corner route extended along both arms' walkways out to their mouths, since a footpath
      beside a crossable arm can meet the sidewalk between the crossing line, where the sidewalk
      lanes end, and the mouth (fixture `t_beside_arm_shallow`, 20° off the arm); the join then
      walks in along the sidewalk to that lane end. The centreline runs from beyond the sector's
      outer boundary into the core between the two mouths, so it always meets the walk.
    - **Sectors without a sidewalk (`ROAD-68-F` part 2; decided by the user, 2026-10-25: as is
      until a tool can draw roads without sidewalks, then walkers on shoulders, option (d)).** A footpath in a sector bounded by
      a road side without a sidewalk (a car-only road or shoulder) has no corner route to join, so
      it reaches only the other footpaths there, and the walking proof is withdrawn on such a
      network. The legacy rule joined it to the corner between the nearest sidewalk arms, across
      the car-only road. No fixture or perf city has one.
      - **Evidence (2026-10-25).** The proof (`ConnectorTargets::prove_complete`) requires foot
        lanes both ways on every live edge, car-only ones included, so a single road without
        sidewalks withdraws it on its whole network whatever its junctions do
        (`a_road_without_sidewalks_withdraws_the_walking_proof`: the reference T keeps it, the same
        T with a side road without sidewalks loses it). No road tool authors such a road today
        (strokes always carry sidewalks); saves may hold them.
      - **Options.** (a) keep: the footpath joins only the footpaths there; (b) the legacy join
        across the car-only road to the nearest sidewalk corner, an unmarked crossing walkers take
        without a zebra, against question 12's rule that walkers switch sides only on crossings;
        (c) reject such a footpath (`FootpathDoesNotJoin`); (d) let a road without sidewalks carry
        walkers on its shoulder (foot lanes on the shoulders, joined at its nodes), which the proof
        would then need, so it could hold on networks with such roads.
      - **Recommendation.** (d) if roads without sidewalks become authorable; until then (a). No
        option keeps the walking proof exact on a network with a road without sidewalks, since the
        road alone withdraws it, so it is not taken under the delegation and stays open for the
        user.
18. **Grading envelope cap (`ROAD-68-D`) — decided 2026-10-24 (user): the probed reach, capped at
    the 64 m tile halo.** Each ray is probed until the 1:2 tie-in meets the terrain, at most
    `GRADING_ENVELOPE_MAX_MARGIN_M`.
    - **`TERRAIN-07` (2026-10-25):** carried into its product decision TD5 (64 m cap, proposed); the
      reach becomes an exact bound under that cap ([T1](terrain.md#terrain-07-open-questions)).
    - **The gap.** `roads.md` capped the envelope at `EARTHWORK_MAX_MARGIN_M`, which no longer
      exists in code; it was 18 m in the deleted legacy `surface/earthwork.rs` (ray search 4–18 m
      at slope 0.5), used for patch selection. `earthworks.md` "Geometry Decision" fixes the rule
      (the required grade-limited tie-in envelope) but no value.
    - **Evidence.** The legacy CDT guide path never applied 18 m: it probed each ray until the 1:2
      tie-in met the terrain. `terrain_cdt_local_bounds_expand_for_required_grading_distance` (a
      16 m fill must widen local bounds past 33 m) and
      `terrain_cdt_guides_use_shared_source_height_and_final_grade_limits` (a 16 m guide ring on a
      20 m base margin) encode that rule; an 18 m cap fails the first.
    - **Default.** The probed reach, capped at the 64 m tile halo
      (`GRADING_ENVELOPE_MAX_MARGIN_M`): no tile can carry a tie-in to a footprint outside its halo,
      and patch selection pads edits by the same cap. Both tests pass.
    - **Options.** Keep the halo cap; cap at 18 m and accept narrower tie-ins on high fills; or a
      larger cap with a larger halo (more loops per tile).
19. **Open chain ends at structures and failed nodes (`ROAD-68-D`) — decided 2026-10-24 (user):
    the typed error `roadbed_chain_open_end` until `ROAD-68-F` builds abutments and portals.**
    - **`TERRAIN-07` (2026-10-25):** superseded there: inside and outside come from the pieces' top
      triangles, so an open end flips no region ([T3](terrain.md#terrain-07-open-questions));
      abutments and portals in [T14](terrain.md#terrain-07-open-questions).
    - **`TERRAIN-07-C` round 2 (2026-10-26):** their shapes are
      [T22](terrain.md#terrain-07-open-questions) (decided under the user's delegation, for
      review). A structure's free end is capped and a node piece where only structures meet
      stands off the terrain; the grounded chains that end at a structure close across its mouth
      row through a wall run the roadbed publishes (T22 3 (a)), built in round 3, so until then
      the rejection below still stands.
    - **`TERRAIN-07-C` round 3 (2026-10-26): the rejection is gone for bridges and tunnels.** A
      grounded piece publishes a wall run across each structure mouth it meets
      (`PieceMesh::wall_runs`: a node piece at a bridge or tunnel arm's mouth, a standard span at
      a pass-through's shared mouth row), `terrain_loops` joins through it, and its segments are
      structure sources. `a_bridge_and_a_tunnel_drawn_from_a_road_commit_and_undo` draws a bridge
      (6 m rise) and a tunnel (6 m fall) through the player's path from a T's side road end and
      straight on from a road end, flat and on the 2.5% slope: all eight commit, publish one wall
      run, and undo restores every piece bitwise. With the wall runs left out of the loops the
      first case fails as before (`render_geometry_invalid roadbed_chain_open_end patch=(1,2)`).
      A failed node's gap still fails its rectangle; that error stays.
    - **Until `TERRAIN-07` (measured by `ROAD-68-F` part 2, 2026-10-25).** Abutments and portals
      moved to `TERRAIN-07`, so the error stands on the legacy terrain: a bridge or tunnel stroke
      that joins a grounded road is rejected at the commit (`render_geometry_invalid
      roadbed_chain_open_end`; a stroke from the T's road end rising 8 m, flat and sloped), so
      structures cannot be built from a road in play until then. A free-standing structure commits
      (`structural_road_plans_adopt_with_and_without_terrain_cutouts`), and the class change
      (`set_edge_class_internal`) compiles only the closure's pieces, not the terrain, so it
      accepts the same open ends; its terrain patches then fail to build.
    - **The case.** Bridge and tunnel spans publish no terrain chains (no mid-span clipping), and
      their terminals have no node piece, so a grounded chain that meets one ends inside a tile's
      rectangle. A failed node publishes as a gap (question 11) and leaves the same open ends.
    - **Default.** The tile fails with `roadbed_chain_open_end`; the patch is road-locked at the
      envelope cap so its build reports the error instead of raw terrain under a road. A failed
      node already rejects the edit; a bridge or tunnel leaves its patches without refined terrain
      until `ROAD-68-F` builds abutments and portals.
    - **Options.** Keep the error until `ROAD-68-F`; or close a grounded chain across the mouth row
      it ends at (the exact boundary of the grounded footprint, but curb pairs share XZ with two
      heights, so the seam steps 12 cm at the curb).
20. **Supplier work-count limits on `ROAD-68` cities (`ROAD-68-G` attribution; attributed
    2026-10-25: correct topology, no defect; the re-baseline waits for the user).** In one
    session the last pre-`ROAD-68` code (`d6c80f91`) and the `ROAD-68-G` base (`c59955e6` plus
    the gate's test-only harness) ran the economy performance contract A B B A, each on its own
    generated cities ([attribution](#attribution-of-the-perf-contract-rise-road-68-g)). Five of the
    seven failing work-count limits already fail before `ROAD-68`: baseline `buyer_sites_built`,
    `staffing_trials` and `job_search_keys`, fiscal `buyer_sites_built` and `cch_searches`. Two
    fail only on `ROAD-68` cities: baseline `supplier_searches` (15,394 → 63,981, 4.16× against a
    3.43× limit; before: 22,712 → 62,900, 2.77×) and `supplier_routes` (261,289 → 1,346,656,
    5.15×; before: 393,202 → 1,232,340, 3.13×). The 10k city does a third fewer supplier searches
    and routes, and the 30k city 2% more searches and 9% more routes, so the ratio crosses the
    limit from both ends. Both cities have the same street grid; what differs is the roadbed,
    lanes and walking connectors `ROAD-68` builds on it. The limits are unchanged here. Options:
    find which supplier candidates the new lanes add or remove at each tier and fix the cause if
    it is a defect; or accept the new counts and re-baseline these two limits (user decision).
    **Decided by the user, 2026-10-25: investigate first.** Find which supplier candidates the
    new lanes and walking connectors add or remove at each tier and fix the cause if it is a
    defect; re-baseline the two limits only if the counts follow correct road topology.
    **Attributed 2026-10-25 ([supplier counts](#supplier-counts-on-road-68-cities-open-question-20-2026-10-25)):
    no defect.** The lanes add or remove no supplier: both builds' cities have the same graph,
    lane count and car component, every building keeps its door and lanes, and every route
    quote and search succeeds. The counts follow the city, not the build: each build run on the
    other's snapshot reproduces that snapshot's 10k searches. The 10k census of the `ROAD-68`
    city needs a second closure pass in 6 of 24 hours against 20 (30 passes against 44), and
    each pass repeats the staffing sweeps' searches; the 30k census takes two in every hour in
    both. The cities differ because each build grew them for twelve days with its own quote
    times, which follow the layout's longer junction clips (10 m mouths against the legacy
    5.5 m setback). Per closure pass both cities scale within the 3.43× limit (searches 2.60×,
    routes 3.22× on `ROAD-68` cities; 2.54× and 2.87× before). Recommended values (user
    decision, not taken; the limits are unchanged): (a) recommended: divide each tier's
    `supplier_searches` and `supplier_routes` by its census closure passes, a new work counter
    the runner also reports, and keep the 1.2 × household-ratio limit; (b) keep raw counts with
    per-counter factors of at least 1.46 (searches, 4.16×) and 1.81 (routes, 5.15×) of the
    household ratio, which fit only the present cities; (c) keep the limits and list both
    with the five known failures.

21. **The commit's other stages exceed a frame (`ROAD-68-G` measurement) — cell lots decided
    2026-10-24 (user): profile and speed up lot regeneration first; if it still does not fit,
    `ROAD-68-E` builds the lots in the preview and the commit installs them (option (b)). The
    other stages stay open until the commit fits a frame.** With the CCH
    off the commit (D7), part 1 measured click → acknowledgement at 2.4 s at 100k residents.
    Part 2 ([local commit publication](#local-commit-publication-road-68-g-part-2-2026-10-24))
    found that the harness never acknowledged an upload; with Godot's acknowledgements modelled,
    publication, entrances and the agent pass follow the edit. Part 3
    ([commit cell lots](#commit-cell-lots-road-68-g-part-3-2026-10-24)) made the lot
    regeneration follow the changed roads and generation cheaper, bit for bit (28.6 ms at 100k
    against a 41.7 ms control). Part 4
    ([commit stages](#commit-stages-part-4-road-68-g-2026-10-24)) first fixed a difference in
    part 3's marks (undo queued fewer regions), then lifted each new lane once, spliced into
    reserved buffers, invalidated cells without the lots' frontage reach and made the agent
    passes' set probes cheap, bit for bit: per sample at 100k the commit takes 28.3 / 21.8 /
    18.3 ms (median 21.8; same-session control 41.8), at 2k 24.8 / 11.9 / 4.0 ms. Part 5
    ([commit stages, part 5](#commit-stages-part-5-road-68-g-2026-10-25)) fixed the pure-job
    deadlock (no pool worker waits for a pure job), put the new-lane lift and the chunk seal
    back on the pool and made the cell road-overlap test exact without rounding: before the
    `ROAD-68-D` rebase, 23.5 / 19.3 / 16.1 ms at 100k (median 19.3; control 41.6) and 4.5 / 9.0
    / 12.4 / 12.9 ms at 0 / 2k / 10k / 30k. On `road` after `ROAD-68-D` (`e9fee678`) the 100k
    commit is rejected (`render_geometry_invalid`) and the commit's terrain validation takes 15
    to 69 ms (median) from 0 to 30k, in the control too; `ROAD-68-D` part 6 owns both, and the
    gate is measured on `road` again once it lands. Part 6
    ([commit stages, part 6](#commit-stages-part-6-road-68-g-2026-10-25)) found the first sample
    to be the larger, grid-adjacent edit rather than a cold start, and put its reattachment
    search, street lights and signs and retained frontage lines on the pool, bit for bit: before
    the rebase, 20.5 / 17.9 / 16.0 ms at 100k against 23.4 / 19.2 / 13.7 ms (without the varying
    snapshot publish 19.3 / 13.7 / 12.0 against 22.3 / 15.0 / 12.6). On `road` with
    `ROAD-68-E1` the snapshot publish is below 0.7 ms, but the tool snapshot grew to 5–11 ms at
    10k and 30k, in the control too (`ROAD-68-E1`'s). The acceptance (under 16.7 ms at 100k
    buildings) is still not met. At 100k, second sample (part 4; part 5 changes the publication
    to 3.3 ms and the lane lift as below):
    - **Snapshot publish, 4.2 ms**, and the **tool snapshot, 2.5 ms** (`ROAD-68-E`'s road-tool
      context). The publish drops the previous snapshot inside the view's write lock on the sim
      thread; dropping a warm, unshared copy of the live graph costs only 0.66 ms, so most of it
      is freeing what the old snapshot alone still holds (not split further). The tool snapshot
      is the graph clone (24 % of the commit thread's samples in a 100k profile, most of it
      cloning every node's lane-connection map).
    - **Cell lots, 3.5–8.9 ms.** 43 % of generation is the per-footprint legality test against
      the roadbed (`cell_site_blocked`, parallel); the candidate index and sort stay serial.
    - **Render publication, 4.7–9.8 ms** (3.3–5.1 ms in part 5): street lights and signs for
      every covered edge (13 %), piece surfaces (12 %), the new lanes' lift (on the pool again
      since part 5).
    - **Agent passes, 1.8–3.7 ms + 0.2 ms**, still `O(A / P)`.
    - **Options.** (a) Ruled out (`SIM-02-H`). (b) Derive the commit's lots from the preview's
      plan in `ROAD-68-E`, so the commit installs them (the user's fallback): removes 3.5–8.9 ms.
      (c) In `ROAD-68-E`'s publication, swap the view under the lock and drop the old one on the
      shared pool, and share the graph replica instead of cloning it: removes up to 7 ms.
      (d) Per-edge agent lists and an index of planned routes by node, so the agent passes
      visit only the edit's agents. (e) Splice street lights and signs like piece triangles
      where their street is unchanged.
      Recommendation: (c) in `ROAD-68-E` round 2 (it owns the publication), then (b) if the
      first, grid-adjacent sample still misses; (d) and (e) only if needed after both.
      Estimate from this breakdown: with (c) the median commit takes about 15 ms and the first
      sample about 25 ms; with (b) as well, about 10 and 16 ms, and about 2 ms less each once
      the pure-job deadlock is fixed and the new lanes are lifted on the pool again (done in
      part 5: median 19.3 ms before the rebase, about 13 ms expected with (c)). `ROAD-68-G`
      stays open until the commit fits a frame.
    - **`ROAD-68-E` part 2 (2026-10-25):** option (b) is not started; it is measured first with
      gate G's harness on this branch. Historical evidence (gate D part 6's own run on its final
      binary, eight workers, not re-run here): click to acknowledgement at 100k residents takes
      120 / 79 / 23 ms per sample, most of it the commit's terrain ([open question
      29](#road-68-open-questions)), so the lots alone (3.5-8.9 ms) cannot make the commit fit;
      the measurement must also cover part 2's reservation checks on the painted city.
    - **`ROAD-68-E` part 2, round 2 (fresh, 2026-10-25 07:35-07:56 EET):** gate G's harness
      (`benchmark_populated_commit_breakdown`, `METRUM_COMMIT_TIERS=0,10000,30000,100000`,
      release library tests, 8 workers, P-cores, both timing locks), C = `road` `3f24cff2`
      (before E2) as the same-session control, E = `359406c1` (E2 round 1), C E E C, three
      samples per tier and run; medians of the six samples, ms, C / E:

      | Stage | 0 | 10k | 30k | 100k |
      | --- | --- | --- | --- | --- |
      | click → ack | 14.6 / 14.3 | 28.0 / 29.2 | 74.3 / 81.2 | 79.9 / 87.4 |
      | plan | 0.20 / 0.35 | 0.21 / 0.35 | 0.24 / 0.39 | 0.26 / 0.42 |
      | roadbed and terrain (`surface`) | 11.5 / 10.9 | 18.5 / 19.5 | 63.9 / 70.2 | 66.9 / 74.2 |
      | entrances and cell lots (`buildings`) | 0.00 | 4.45 / 4.36 | 4.51 / 4.20 | 4.98 / 5.00 |
      | render publication (`mesh`) | 2.09 / 2.35 | 3.64 / 3.74 | 3.55 / 4.15 | 2.28 / 2.31 |
      | tool snapshot | 0.03 | 0.29 / 0.30 | 0.70 / 0.83 | 2.70 / 2.73 |

      100k per sample, click → ack: C 120.3 / 79.8 / 22.6 and 118.6 / 80.0 / 22.4, E 127.1 /
      87.8 / 22.1 and 129.3 / 87.0 / 22.2. Lot and generated-cell digests equal C's in all 24
      sample pairs; determinism (below) unchanged. E adds 6-9 ms to `surface` beside the painted
      city (all three 30k samples, the first two at 100k; not the open-terrain third). It is not
      the reservation checks of question 1: in a profile of E's commit windows at 30k (perf,
      LBR, 8 workers; a profile, not a timing) `i_overlay` and the reservation functions take
      under 0.5% of the samples, and terrain grading, site paving and the terrain CDT dominate.
      The same profile of C has 10% fewer cycles in its commit windows (3.32 against 3.70 G), the
      difference spread over the road loops' grading (`road_loop_grading` +32%), the road
      terrain loops (`terrain_loops` +30%) and terrain sampling (+7%): E's commit grades more
      road loop, so its closure or pieces differ from C's on this edit; the cause is not found
      yet. On this round's final code (F) one C F pair (09:08-09:19, same settings) gives
      `surface` medians C / F of 10.7 / 13.0, 18.1 / 18.6, 68.0 / 64.3 and 70.8 / 71.9 ms (0 to
      100k) and click → ack 13.6 / 16.6, 27.0 / 28.5, 81.3 / 75.0 and 83.9 / 85.5: one pair is
      too noisy to tell whether the meeting-height change (question 30), which also moves where a
      stroke ending on a road mid-span meets it, removed the rise; re-measure C F F C. The
      terrain stage (question 29, gate D
      part 7) still dominates every miss, and the lots are 3-5 ms of `buildings`, so option (b)
      alone cannot bring the commit under 16.7 ms; it is not started this round (it needs the
      zoning cell store in the road tool's view, as question 25 (c) does). Target missed, open.
    - **`ROAD-68-E` part 2, round 3 (2026-10-25): (b) not started; what it needs.** The view
      now holds the cell reservations (question 25 (c)), but lot derivation reads far more than
      the reserved masks: `SimCore::regenerate_cell_lot_regions` generates cells
      (`prepare_generated_cells`, the store's valid masks, frontages, curve sources, road
      alignments and chunk completeness) and derives lots (`derive_cell_lots`, the lot claims
      and parcels) on the post-edit graph and roadbed, tests footprints against building sites
      (`cell_site_blocked`, the allocator's site index), takes the catalog's lot sizes, and then
      reattaches occupants (`sync_cell_lot_attachments`, the allocator). A preview that derives
      the commit's lots therefore needs the whole cell store persistent (its `CellMap`s as
      `imbl` maps, so the view shares it and the preview generates on a copy-on-write copy),
      the site index and lot sizes in the view, and an install step that swaps the derived
      blocks, frontages and claims in when the store's revision is the preview's. Lot
      generation is the commit's own hot path (`ROAD-68-G` part 3), so the persistent store
      needs its own same-session comparison first. Not done this round; the `buildings` stage
      (entrances and lots) is 2.6-4.1 ms of the misses. Fresh (11:12-11:33, gate G's harness,
      C = `road` `0eabfe36`, C E E C; [round 3 record](#part-2-round-3-2026-10-25-branch-road68-e3-work-in-progress)):
      the plan-excerpt fix of question 8 brings the 100k click → ack median from 80.0 to 21.3 ms
      (samples 1 and 2 no longer re-derive an unchanged far link); the third sample takes
      15.0-15.4 ms, the first two 130 and 21 ms. Target still missed, open.
    - **Decided by the user, 2026-10-25: the preview does not show the lots it derives; they
      appear at the commit.** The preview derives the commit's lots only so that the commit
      can install them: it draws, uploads and publishes no cell or lot geometry, and its render
      payload stays the road chunks and terrain patches it has today.
    - **`ROAD-68-E` part 2, round 4 (fresh, 2026-10-25 14:28-15:00 EET): the cell store is
      persistent; the lots are not derived in the preview yet.** The store's maps are `imbl`
      maps on the same hasher (frames an `imbl` vector), so the road tool's view holds the
      whole store in `O(1)` and round 3's mirror index is gone (question 25). Lot and generated
      digests are unchanged in every sample, and so is gate G's determinism digest. Measured
      with gate G's breakdown against `road` at round 4's part 1 (`096df250`), the commit's
      `buildings` stage (entrances and cell lots) is 0.3-2.0 ms slower in 8 of 9 tier samples
      (C against C: 0.06-0.71 ms), medians 3.90 / 4.83, 3.30 / 3.38 and 3.24 / 3.94 ms at 10k,
      30k and 100k ([round 4 record](#part-2-round-4-2026-10-25-branch-road68-e4-work-in-progress)).
      Open: the regression (to attribute first: copy-on-write after each publication, since
      the view holds the previous store and a block entry carries its whole 656-byte block,
      against `imbl`'s lookups; an unshared lot regeneration is not slower), then the site
      index and lot sizes in the view, a lot plan the preview derives on its store copy
      against its edited replica and installed roadbed, and the commit's install of that plan
      when the store's and the parcels' revisions are the preview's. The first commit beside a
      painted city spends its extra time in the commit's terrain validation, which
      `TERRAIN-07` replaces (same record). Target still missed, open.
    - **`ROAD-68-E` part 2, round 5 (fresh, 2026-10-25 16:37-17:56 EET): the regression is
      the persistent maps themselves; the lot derivation is one planner and one installer.**
      A build whose view shares nothing is as slow as round 4's, and so are builds with the
      store's values behind `Arc` and writes that read first (kept) and with generation's
      temporary index in plain maps (dropped): copy-on-write is not the cost
      ([round 5 record](#part-2-round-5-2026-10-25-branch-road68-e5-work-in-progress)). The
      store design the preview's lot plan needs is [question 32](#road-68-open-questions),
      decided by the user: profile first, then a plain live store with a journal-synced
      persistent copy in the view if no ~1 ms cut is found. Every lot derivation now runs on `CellLotPlanner` and installs its recorded parcel calls
      (`install_cell_lot_plan`), bitwise; the preview does not plan yet (it needs question 32,
      the parcel view, the lot sizes and the explicit-site index in the view). Target still
      missed, open.
    - **`ROAD-68-E` part 2, round 6 (2026-10-25): question 32 built; the preview half paused
      by the user.** The live cell store keeps plain maps again and the road tool's view a
      journal-synced persistent copy, with one planner generic over both
      ([question 32](#road-68-open-questions)). The user decided (question 33, 2026-10-25) that
      no lots are planned per preview pose: 21 (b)'s preview half waits until the commit is
      measured with `TERRAIN-07`'s terrain validation, and if the 100k commit then still
      misses 16.7 ms, lot regeneration is deferred to a job adopted one frame after the commit,
      as routing adoption is (D7). Target still missed, open.
22. **The 2 ms terrain stage on the sloped T (`ROAD-68-D`) — resolved 2026-10-24 (part 3): met,
    no decision needed.** The grading pass computed its loop's orientation once per edge
    (`O(V^2)` per loop); computed once per loop, bitwise unchanged, the T takes 1.91 / 1.89 ms,
    and 1.85 / 1.88 ms with the constructive station rule (A B B A against the part 2 binary at
    2.20–2.25 ms, [part 3](#road-68-d--footprint-and-terrain)). Options (a)–(c) below are not
    needed; (d) is withdrawn. The record of part 2 follows.
    - **Target.** The T fixture's tiles total at most 2 ms on four workers (gate acceptance and
      [stage budgets](#stage-budgets)).
    - **Evidence.** `benchmark_roadbed_terrain_tiles` (release, 4 workers, P-cores, both timing
      locks, A B B A against the first `ROAD-68-D` WIP binary in the same session): total p50
      2.92/2.90 ms for the control, 2.33/2.35 ms now; the patch plan fell from 1.39–1.40 to
      0.75 ms with one ray pass per loop and rings only for rays that reach the tile. The 12 tile
      CDTs on four workers take the other 1.6 ms (tile p50 0.35 ms, slowest about 1.15 ms). A
      flat profile of the candidate: the grading pass 15% (still every ray of every halo loop
      probed once per tile, 16 tiles), `widening_tie_in_sample_against_any_road_loop` 7% and
      `point_inside_any_road_footprint` 3% (linear in the tile's loop edges per sample),
      R-tree bulk loads and sorts for constraint noding about 6%, Spade about 10%.
    - **Options.** (a) probe each chain segment's rays once per plan and share the reaches between
      the tiles of the plan (the rays of a chain segment are the same in every tile; about 9x
      fewer probes); (b) index the tile's loop edges once per canonicalization (the R-tree the
      noding already builds) for the widening and footprint tests, `O(log E)` per sample instead
      of `O(E)`; (c) plan and triangulate each tile in one task instead of a plan barrier
      followed by the tile CDTs; (d) accept a 2.5 ms budget for the T. (a) and (b) keep every
      output bitwise; (c) changes only scheduling.
23. **Seams between meshes in different frames (`ROAD-68-D` part 4) — decided by the
    coordinator under the user's delegation, 2026-10-24, confirmed by the user 2026-10-25: (a).** The yard test
    `cars_and_walkers_enter_and_exit_on_compiled_yards` samples exactly on the terrain/foundation
    seam (coordinator's decision of 2026-10-24: a seam point must be covered).
    - **`TERRAIN-07` (2026-10-25):** superseded for terrain: graded terrain shares no vertices with roads
      or pads, so the open T-vertex and filler items go with the CDT at `TERRAIN-07-G`.
    - **Evidence.** Kuopio yard fixture, after part 4. Building 1, sample (65.538, −95.030): the
      foundation's paved triangle misses it by 2.7 µm across the footprint side, and the terrain
      faces along the same side miss it too. The foundation is a world-space f32 mesh
      (`foundation_mesh`, uploaded by `get_building_site_mesh_data_internal`); the terrain is
      patch-local f32 (`(x − centre) as f32` from f64), and each partition adds its own split
      point where a yard triangle edge crosses the side: (66.766045, −92.73785) in the
      foundation, (66.766052, −92.737839) in the terrain, 13 µm apart. In general the two frames
      also round the same footprint vertex differently. Building 3, yard centre
      (75.703, −978.962): the centre lies on the diagonal of the yard's two triangles;
      `SitePavingPartition::partition` clips each foundation triangle by each yard triangle on
      its own, so one side of the diagonal keeps an unsplit edge while the other is split at
      (76.058914, −977.6029), and that T-vertex, rounded to f32, opens a crack the centre falls
      into (it misses both paved neighbours by under 4 µm). **Fixed in part 4:** a convex surface
      is clipped against as one polygon (no internal diagonal), and `split` computes a segment's
      split point from its lexicographically lower end, so the two faces sharing a segment split
      it at the same point; building 3 is covered, and building 0's seam sample (u = 0.9,
      v = 0.9, on a footprint side like building 1's) now misses instead, by the frame split
      alone. The same frame split exists between the world-space road
      chunks and the patch-local terrain along every road seam.
    - **Options.** (a) Build the foundation's faces into the terrain patch buffers, in the
      patch's frame and through the same material partition as the terrain paving (clip each
      foundation triangle by the patch), and stop uploading the separate world-space foundation
      mesh; the seam is then one shared edge by construction. (b) Keep two
      meshes and give coverage a seam tolerance (rejected: tolerance). (c) Move the samples off
      the seam (rejected by the coordinator's decision). **Recommendation: (a)**, with the road
      seam (question 24) following the same rule.
    - **Scope (coordinator, same delegation):** (a) in `ROAD-68-D`, with one frame per shared
      seam for road chunks and terrain as well, and one shared height function for filler and
      tile sides; the yard test is re-enabled with it.
    - **Done in part 5** ([D part 5](#road-68-d--footprint-and-terrain)). Each site's pad is
      composed into the terrain patch buffers: the footprint triangulated in f64, clipped to the
      patch by the canonical rectangle clip (`terrain/rect_clip.rs`), converted once by the
      terrain's own `render_frame_local` and run through the same `SitePavingPartition` as the
      terrain faces. `get_building_site_mesh_data_internal` no longer uploads a world-space
      foundation (`BuildingSiteClient::foundation_mesh` is gone). Road chunks take the frame of
      the terrain patch over them (the cell centre, `render_frame_centre`), converted from the
      pieces' f64 positions by the same function, and are clipped at chunk lines. The yard test
      passes again; building 0's last miss was a 17 µm sliver the test's height query dropped
      under `SAMPLE_EPSILON_M`, now an exact point-in-face test.
    - **Still open.** (1) Seam edges are shared at vertex level only: where a seam crosses a CDT
      tile line inside a patch, the CDT loop clip (`terrain/cdt/loop_clip.rs`, overlay grid and
      `CDT_EPSILON_M` matching) adds a seam vertex the road lacks, and on `y_60` a terrain seam
      edge runs straight past a road vertex 1.85 cm from its end (a near-collinear chain vertex
      the terrain does not keep): T-vertices on straight seam edges, 4 to 16 per reference
      fixture, allowed by `reference_junction_road_and_terrain_share_seam_vertices`.
      Recommendation: let the CDT clip loops at tile sides with `rect_clip`'s canonical split and
      the road render clip at the same tile grid, and keep every chain vertex. (2) A truncated
      last patch at the world border (world size not a multiple of the patch span) centres its
      frame on its own middle, the road chunk over it on the full cell. (3) Filler and tile side
      heights come from two bilinear evaluations (patch snapshot and terrain system) that agree
      on sample points but can differ in the last bit between them ([watertight
      patches](#footprint-and-terrain)); the reference fixtures do not need the shared height
      function, so it is not built.
24. **Road surface pinholes on slopes (`ROAD-68-D` part 4) — decided by the coordinator under
    the user's delegation, 2026-10-24, confirmed by the user 2026-10-25: (a), in `ROAD-68-D`; done in part 5.** With the terrain watertight,
    the reference screenshots keep 9 single sky pixels, all on the sloped fixtures and none on
    the flat ones (`pinholes.py`): 8 inside the carriageway (`t`, `four_way` and
    `t_footpath_beside_arm` at one screen point near the side arm's curb, three in a row on the
    junction of `y_25_branches_first` but none in `y_25`, one in `y_40`) and 1 on the outer seam
    between a sidewalk and the terrain (`t_mixed_4lane_through`). The terrain under a road is
    now omitted, so any crack in the road surface shows the sky; before `ROAD-68-D` the
    unclipped terrain hid it. Attribution (a temporary test edge-matching every surface layer
    of the published road chunks in world coordinates, chunk origin added in f64, with exact
    and with 0.1 mm vertex matching): 72 to 86 edges per fixture with road on both sides
    border one face only, flat and sloped alike, so they are topology, not rounding. They are
    T-vertices: on `t`, the edge (−3.25, 6.5)–(−3.25, 9.2857) on one side against
    (−3.25, 6.5)–(−3.25, 7.25) on the other, beside the side arm's zebra, exactly at the `t`
    family's pinhole; on `y_25_branches_first`, the edge (0, 0)–(7.5079, −1.6645) along the
    lower branch (the row of three pinholes) and a cluster at the upper branch's mouth,
    (29.3–32.0, 3.2–3.8) (pinhole (1405, 543)). On flat ground a T-vertex lies on the plane of
    the edge it meets, so nothing opens; on a slope its height leaves that edge and the crack
    shows the sky. The `y_40` and `t_mixed_4lane_through` pinholes have no open edge within
    1.5 m; the latter lies on the outer seam, where the road chunks (chunk-local f32, minus the
    chunk's minimum corner, `push_triangle_to_layer`) and the terrain (patch-local f32, minus
    the patch centre) round a shared vertex in two frames (the frame split of question 23).
    The two Y drawing orders also differ in height on the slope (junction centre 3.2651 m drawn
    branches first, 3.1992 m drawn stem first), so invariant 6 fails for those images; that
    belongs to question 16. The pieces themselves are closed: edge-matching every band and curb
    face triangle of the sloped `t` and `y_25_branches_first` pieces in f64 finds no open edge
    with road on both sides, and no piece triangle collapses in f32; crosswalks add only
    per-vertex shader data, no cut-outs. So the open edges arise in the render emission
    (`emit_curb_faces` drops faces under `MIN_RENDER_TRIANGLE_DOUBLE_AREA_M2`, and each
    triangle is moved into its centroid's chunk frame). Options: (a) find where the render
    emission leaves one side of a shared edge only, make both sides take the
    same vertices, add the road-chunk edge-matching test beside
    `reference_junction_terrain_is_watertight`, then give road chunks and terrain one frame per
    shared seam (question 23's rule); (b) hide the cracks under terrain again (rejected:
    terrain must stay clipped). **Recommendation: (a).**
    - **`TERRAIN-07` (2026-10-25):** the road side stays: terrain is still discarded under roads, so a
      road crack would show, and the road chunk watertightness test stays.
    - **Correction and fix (part 5).** Crosswalks did cut the asphalt:
      `crosswalks::emit_triangle` split every carriageway triangle its zebra's rectangle touched
      along the rectangle's lines, extended across the whole triangle, and left neighbours
      outside the rectangle and the curb faces whole. With the 6.5 m carriageways' zero margin
      the rectangle's corners lie on the kerb, hence (−3.25, 7.25). The test
      `reference_junction_road_chunks_are_watertight` failed every fixture with 72 to 138 open
      edges; with zebras off, the T fixtures passed and the Y and bend fixtures kept 22 to 28,
      edges whose two faces sat in different chunks and rounded their chunk-local f32 apart.
      Now a zebra cuts nothing: every asphalt triangle meeting the bar rectangle carries the
      crossing's affine paint frame, and the shader draws only the bars (fewer than a bar pitch
      of margin is left to the kerbs, so no extra bar fits); and a triangle crossing a chunk line
      is clipped at it by the canonical rectangle clip, in the chunk's centre frame. The test
      passes on all 20 fixtures, flat and sloped.
25. **Corridor checks in the synchronous preview (`ROAD-68-E` part 1) — decided by the coordinator
    under the user's delegation, 2026-10-24, confirmed by the user 2026-10-25: option (a).**
    - **Evidence.** The old hover path ran the parcel and field corridor checks under a per-pose
      `try_lock_core` (`road_candidate_dictionary_with_parcel_clearance`). The spec forbids
      per-pose core work ([Preview](#preview-commit-undo-and-load)), and the view holds no parcel
      or field index.
    - **Options.** (a) run them at the commit only (`RoadEditPlan::failed_check`): the preview shows
      a parcel or field overlap only once the click is refused, until `ROAD-68-E` part 2 plans the
      overlap checks from the preview's pieces against indices published with the view
      ([open question 1](#road-68-open-questions)); (b) keep the per-pose `try_lock_core`, which
      reintroduces the `ROAD-70` class of stall; (c) publish the parcel and field indices with the
      view now, an `O(parcels)` copy per publication unless they become persistent.
    - **Recommendation.** (a): no stall, no city copy, and part 2 replaces it with exact planned
      checks. A refused click keeps its reason on the stroke until the pointer or the context moves.
    - **Part 2 (2026-10-25).** Question 1 replaces the field and cell corridor checks at the
      commit with checks of the installed pieces; the parcel corridor check stays at the commit.
      The preview still learns of a field, cell or parcel overlap only from the refused click: a
      per-pose check needs the field and cell indices in the view, which (c) describes and which
      stays open until they are persistent.
    - **Part 2, round 2 (2026-10-25): (c) not started.** The preview has the pieces the commit
      installs (`RoadPreviewPieces::delta`) and the pieces they replace (the view's roadbed), so
      it can call `ReservedOverlap::first` once that takes the two indices instead of the core.
      The field index (`FieldClearanceIndex`) is small and changes only on field edits, so the view
      can share it as an `Arc` replaced on change. The cell reservations live in the cell store's
      blocks (a `HashMap` per store, painted masks, lot claims, frames); a copy per publication is
      `O(painted cells)` on every paint. Next step: a persistent reservation index beside the store
      (reserved blocks only, in an `imbl` map of `Arc` blocks, updated where paint and claims
      change, published in `O(1)`), leaving lot generation's maps as they are; then the preview
      reports `field_overlap` or `zoning_cell_overlap` before the click with the commit's own
      function, and question 21 (b) builds on the same view.
    - **Part 2, round 3 (2026-10-25): (c) for fields and cells, done.** The cell store mirrors
      every block's reserved mask into `CellReservations` (an `imbl` map per 512 m chunk of
      reserved blocks and their frames) where paint and lot claims change it (`set_profile`,
      `assign_lot`, `clear`): `O(chunks the block reaches * log blocks)` per changed block, none
      when a claim lands on painted cells. The allocator holds the field index as an `Arc`,
      written through `field_clearance_mut` (copied once after a publication shared it,
      `O(fields)`; a swap removal touches it only when either building holds a field). The view
      takes both in `O(1)`; a paint publishes the view (unchanged roads publish in `O(1)`), as
      field commits already did. `ReservedOverlap::first` takes the two indices, and the
      preview calls it on the pieces its delta installs and replaces, as the commit does, and
      turns an overlap into the stroke's invalid reason, which the road tool shows ("Road
      overlaps a farm field", "Road overlaps painted zoning cells") without submitting the
      click. The cell test reads the index in both, so the same function on the same data
      decides; `the_reservation_index_follows_paint_erase_and_undo` checks the index visits the
      store's reserved cells exactly. The field and painted-cell refusal tests now assert the
      preview's reason before the commit's, and that both accept once the land is released.
      Lag: a redevelopment grace claim that ends in a simulation tick shrinks the reservations
      without a publication, so until the next one the preview may refuse what the commit
      accepts (never the reverse). Parcels keep the commit's corridor check.
    - **Part 2, round 4 (2026-10-25).** The cell store itself is persistent now (question 21
      (b)), so the view holds the store, and round 3's mirror index (`CellReservations`, kept
      beside the blocks in `set_profile` and `assign_lot`) is removed: the commit and the
      preview test the store's own reserved masks (`CellStore::visit_reserved_footprints_in_bounds`),
      one index instead of two. The same refusal tests pass; the lag above is unchanged.
26. **Preview payloads with crossings or signs (`ROAD-68-E` acceptance) — closed 2026-10-25 by
    `ROAD-68-F` part 1: met, bitwise.** The preview's chunks equalled the committed chunks only
    when the edit's closure had no crossing and no road sign: the renderer built crosswalk paint
    and signs from lanes, and a preview builds no lanes. Since `ROAD-68-D` part 5 a zebra no
    longer cuts the asphalt, so only the per-vertex paint frame under the bars and the signs
    differed. Now one emission (`render/road/mod.rs` `emit_into`), which the preview and the
    commit both call, reads crossings from the pieces: a node piece's layout crossings
    (`NodePiece::crosswalk_kerbs`) or, at a pass-through node, the span row's kerb vertices the
    span piece keeps (`SpanPiece::crosswalk_kerbs`), through `RoadbedSystem::for_each_crossing`;
    `CrosswalkMarking::of_arm` orients them as the lanes own them. Lanes and pieces take the
    kerbs from the same functions (`crossing_kerbs`, `crossing_kerbs_right_first`), so the
    crossing contract holds bitwise. Rust: `preview_chunks_with_crossings_and_signs_are_the_committed_chunks`
    (T and pass-through continuation, flat and sloped: paint, signs and lights present, every
    changed chunk's payload equal); Godot: `road_junction_preview_test.gd` compares every
    previewed chunk with the committed one for all 14 fixtures, crossings, lights and signs
    included ([`ROAD-68-F` part 1](#markings-lights-and-signs-from-the-layout-road-68-f-part-1-2026-10-25)).
27. **Godot stage timings at the CPU's idle clock (`ROAD-68-E` part 1) — decided by the coordinator
    under the user's delegation, 2026-10-25, confirmed by the user 2026-10-25: option (a).**
    - **Evidence.** The headless stream test sleeps 16.7 ms between inputs; on an otherwise idle
      machine the P-cores then stay at the 800 MHz floor (0.77-0.9 GHz measured through the motion
      phases), and every preview stage runs about six times slower: Godot stage and install
      2.1-3.0 ms p50 against 2.0, native stages 1.0-2.0 ms. At operating clock the same library
      measures 0.41-0.50 ms for the Godot stage and harness-like native stages (gate E round 3).
      In the game, rendering and the simulation keep the clock up; a paused, empty city at 60
      fps may not.
    - **Options.** (a) state the stage budgets at operating clock and measure the stream with
      back-to-back frames (`METRUM_PREVIEW_STREAM_INTERVAL_SEC=0`), keeping the idle-clock
      numbers as an observation; (b) hold the budgets at the idle clock too, which needs about a
      sixfold cut of every stage; (c) keep the main thread busy between frames (a spin), which
      wastes power to win a benchmark.
    - **Recommendation.** (a): the budgets describe work, and the end-to-end 16.7 ms holds even at
      the idle clock (_process max 9.7 ms). The idle-clock numbers stay recorded with every run.
    - **Since terrain is in the preview (round 5) — open, the 16.7 ms frame is missed at the idle
      clock.**
      - **Evidence.** Round 4's library in the stream test (gate E round 5 record): back to back,
        every frame fits (864 of 864, `_process` max 8.8 ms, terrain 3.5-5.1 ms p50); with the
        1/60 s timer, `_process` takes 11.8-13.5 ms p50 and 38 of 864 frames exceed 16.7 ms (max
        21.1 ms). The terrain stage is what grew: 8.1-9.8 ms p50 (max 15.4) at the idle clock,
        1.7-2.6 times its operating-clock time, while every other stage together stays at 3-4 ms
        p50. The round-2 evidence above (max 9.7 ms) predates terrain in the preview.
      - **Options.** (a) Meet the terrain stage's 3.0 ms budget at operating clock (gate D part 6,
        its item (c): the terrain build cost) and measure the idle-clock stream again; at the
        measured factor a 3.0 ms stage takes 5-8 ms p50 at the idle clock, a frame of about
        9-12 ms p50, and whether the slowest frames fit needs the run. (b) A tighter terrain
        budget, set from that idle-clock run. (c) Build terrain across frames: the
        asynchrony the spec forbids without a measured reason, and a frame would show a road
        without its terrain. (d) Keep the clock up while the road tool is active (question 27
        (c)), which spends power to hide the cost.
      - **Recommendation.** (a), then decide between (b) and (d) from the idle-clock stream on
        gate D part 6's terrain code; the 16.7 ms target is not relaxed.
28. **Debug face sources name tile-local chain runs (`ROAD-68-E` round 4) — decided by the
    coordinator under the user's delegation, 2026-10-25, confirmed by the user 2026-10-25: (a).**
    - **Evidence.** The CDT's `RoadBoundary` source named the graph piece, chain and segment of
      each boundary edge, and a tile's window fingerprint hashed them. A T that splits a 220 m
      trunk renumbers the far half (another edge's span, its chain segments shifted by the
      split) although its vertices stay bit-identical, so on the sloped T all 15 windows changed
      fingerprint and re-triangulated, 11 of them with no geometric change within 40 m. The
      terrain stage measured 7.0-8.3 ms p50 against its 3.0 ms budget.
    - **Options.** (a) Name each source by its tile-local chain run and place in the run, ordered
      by vertex bits ([source tags](#footprint-and-terrain)): products depend on the footprint
      only, and the debug display's face sources (`*_face_sources`, the terrain inspector's
      summary) show run numbers instead of edge and node ids. (b) As (a), and map runs back to
      pieces in the debug export only, by looking up the current roadbed's chains at the run's
      first segment; costs a lookup per exported source, changes no product. (c) Keep graph
      labels and rebuild every tile a renumbered piece reaches (the whole trunk), which misses
      the budget for no geometric reason.
    - **Recommendation.** (a) now; (b) when the debug inspector needs edge ids again.
29. **The commit's terrain at city scale (`ROAD-68-D` part 6) — open, target missed.**
    - **`TERRAIN-07` (2026-10-25):** superseded: the graded bake follows the dirty tiles (spike: 1.64-2.52
      ms per populated edit at 2k and 30k); the 3.0 ms target moves to `TERRAIN-07`'s budgets.
    - **Evidence.** With the city's refined terrain built and `ROAD-68-E`'s chain rule, a T
      split's terrain validation (`surface`) takes 10-12 ms in open terrain and 55-73 ms beside
      city patches, at 0 to 100k residents alike (eight workers; [part 6
      record](#road-68-d--footprint-and-terrain)), against the 3.0 ms stage budget. The cost
      follows the 512 m render patches the edit's stamped tiles fall in: per patch, the road
      ownership refresh re-grades the loops of all 64 tiles (`road_terrain_patch_margin_m`,
      after a `visual_patch_snapshot` copy), the stamped tiles and their neighbours
      re-triangulate, and the patch's mesh buffers and paving partition are recomposed over
      every site in it. A profile of the three 2k commits (four workers, commit windows only,
      profiled, not acceptance) puts 45% of all samples in the window plans
      (`terrain_cdt_planned_window_for_tile`), 32% in building-site grading guides
      (`append_building_site_grading_guides_from_parts`), 28% of them in the guide
      de-duplication's `HashSet<(i64, i64)>` (SipHash, and 17% in its rehash as the set grows),
      25% in the tiles' road loops (18% their grading rays), 12% in the CDT and 11% in
      heightmap sampling. Inside a window, `ROAD-68-E` round 4's profile adds Spade's bulk load
      and predicates and the per-sample loop-edge scans of canonicalisation
      (`widening_tie_in_sample_against_any_road_loop`, `point_inside_any_road_footprint`).
    - **Options.** (a) Make the patch-level work follow the stamped tiles: a patch's road margin
      from per-tile margins kept beside the assembly ledger (derived state, so measured first,
      per the no-stage-caches rule), and composition per tile, so a patch payload joins
      unchanged tiles' buffers and pads. (b) Index loop edges for the per-sample scans (question
      22's option (b)). (c) Smaller render patches near roads, which multiplies Godot's patch
      count.
      (d) De-duplicate site grading guides without a growing SipHash set (the guides keep
      their first-wins order: a pre-sized set with an integer hasher, or sorted keys).
    - **Recommendation.** (d) now (local, output-preserving), then (a), measured with the commit
      breakdown at every tier, then (b).
    - **`ROAD-68-E` part 2 round 3 (2026-10-25).** Part of the cost beside the city was not the
      terrain's: the edit plan's one-ring excerpt re-derived, wrongly, a far link the edit does
      not change (question 8), whose pieces and terrain the commit then rebuilt. With the
      excerpt fixed, `surface` at 100k takes 10.4-10.7 ms instead of 66-67 ms in the harness's
      second sample and 6.9-7.4 instead of 11.4-11.7 ms in the third; the first sample (the
      city's first commit) is unchanged or slower (82-85 against 73-74 ms) and still misses.
30. **Drawing order on slopes after question 16 (`ROAD-68-E` part 2) — decided by the coordinator
    under the user's delegation, 2026-10-25, confirmed by the user 2026-10-25: (a); joins implemented, mid-span
    meetings follow the crossing rule exactly (round 2), and the order test asserts that rule
    (round 3).**
    - **Evidence.** With question 16 implemented the sloped T fixtures are bitwise independent of
      drawing order; the other sloped fixtures are not, through two rules that are not piece
      reads. (1) *Terminal extensions.* A stroke from a road end is solved together with the road
      it extends as one vertical corridor (`prepare_terminal_road_extension`), which rewrites that
      road's control polyline. Every Y draws its second stroke from the first one's end, so the
      order decides which two roads are solved together: on the 2.5% slope `y_25` in order
      `[0, 2, 1]` differs from `[0, 1, 2]` by one ulp at index 5 of a branch, whose samples and
      support are identical in both (one is the extension solve, the other a plain solve). (2)
      *Mid-span meetings.* By question 16's crossing rule a stroke meets a road it splits mid-span
      at that road's graph profile, so the road drawn first sets the meeting height: the cross's
      junction differs by about 7 mm between orders, and the two-junction fixture's junction at
      `(-30, 0)` by two ulps (side road first: its end at the terrain; through road first: the
      through road's profile between two of its samples). The T at the origin agrees by chance.
    - **Options.** (a) Drop the extension's combined re-solve: a stroke from a road end joins it
      like any node, at the node's graph height, and the bend's plane solve keeps the joint
      continuous; no stroke then rewrites another road's control polyline, joins at existing nodes
      are order independent (the Y), and mid-span meetings stay question 16's authored-state rule,
      which the sloped test then asserts directly for the cross and the pair. (b) Keep both as
      authored-state rules: the sloped Y, cross and pair are different graphs in different orders
      and leave the test. (c) Also make mid-span meetings order independent, by letting a stroke
      that passes an existing road end re-height that node to its own profile, which edits the
      other road.
    - **Recommendation.** (a): it removes the last history channel at joins without changing the
      user's crossing rule; (b) gives up invariant 6 for every Y, and (c) edits authored roads.
    - **Implemented (joins, part 2).** The terminal extension is gone
      (`prepare_terminal_road_extension`, `RoadExtensionReprofile`, the validation corridor and
      the editor's re-profile of the extended edge): a stroke from a road end is prepared like
      any stroke, its end at the node's graph height (question 16's joined-end rule), and the
      node's plane solve joins the two roads. All six orders of the sloped Y fixtures are bitwise
      equal now and run in `sloped_junctions_do_not_depend_on_drawing_order`;
      `a_stroke_from_a_road_end_joins_it_continuously_on_a_slope` checks, straight on and bending
      34°, that the extended road's control profile is untouched, both arms end at the joint
      node, their grades stay within the design grade and the grade change across the joint
      within the profile's curvature rule. `a_stroke_from_a_road_end_adopts_like_a_cold_add`
      replaces the extension's adoption test and checks the extended road keeps its profile.
    - **Visible change, for review.** An extended road keeps its own vertical profile: before,
      the extension re-solved its control polyline and its end node's height together with the
      new road as one corridor, so the joint was smoothed across both and the old road's last
      stretch could rise or fall towards the new one. Now the old road and its end node stay as
      they were, the new road starts at that height, and the joint is the plane solve's (a bend
      or pass-through between two independently solved profiles). On the 2.5% reference slope the
      difference is at rounding level: the five sloped probe fixtures whose second stroke starts
      at a road end hash differently (re-recorded), and the drawing-order differences were one
      ulp. On curved terrain a crest or dip at the joint is no longer shared between the two
      roads; the size of that change on real terrain is not measured yet (the Kuopio replays wait
      for question 8's fixture regeneration).
    - **Mid-span meetings (part 2, round 2).** A meeting node now lies exactly on what the
      first-drawn road had there: a crossing (`collect_crossing_splits`) and a stroke end that
      snaps onto a road mid-span (`collect_endpoint_snap_splits`) take the crossed road's graph
      profile height at the split, from `RegionGraph::profile_height_at`, the one function the
      stroke's support also reads (question 16); before, the node took the new road's
      interpolated height, 17 mm off the first road on the sloped cross when the z road is drawn
      first. So the split leaves the crossed road's profile as it was and no stroke edits the road
      it meets; a road end the stroke passes over keeps its node's height, as before.
      `a_mid_span_meeting_lies_on_the_first_drawn_road` checks every order of the sloped cross and
      pair bitwise against the graph just before the second road (failed before). The reference
      probe is unchanged (its four-way meets at the origin, where both roads agree).
      **Order independence is not attainable under the crossing rule** and is not claimed: the
      two roads' profiles at the sloped cross's meeting differ by an ulp (3.7 m, 1 ulp apart), so
      `sloped_mid_span_meetings_do_not_depend_on_drawing_order` stays ignored with that reason.
      Only option (c), re-heighting the first road, would make the orders equal, and it edits an
      authored road.
    - **Round 3: the test follows the decision (coordinator under the user's delegation,
      2026-10-25, for review).** Question 16 makes the first-drawn road set a crossing's height
      (authored state), and round 2 built exactly that, so the ignored order-independence test
      for mid-span meetings is removed instead of kept waiting: on the slope the cross and the
      two-junction fixture are asserted by the rule, the meeting on the first road's profile, in
      every order (`a_mid_span_meeting_lies_on_the_first_drawn_road`), and flat they stay in the
      order test. The joins stay order independent and are asserted so on the slope: the T and Y
      fixtures and, new, end-to-end joins (a road drawn as two strokes meeting straight on, a
      pass-through node, or bending 34 degrees, either stroke first;
      `sloped_junctions_do_not_depend_on_drawing_order`, also flat).
31. **Recorded Kuopio profiles beyond the 5 m source-delta limit (`ROAD-68-E` part 2 round 3) —
    decided by the coordinator under the user's delegation, 2026-10-25, confirmed by the user 2026-10-25: (a), as the
    plane sample at the mouth station where it lies beyond 12 m; done, the limits hold.**
    - **Evidence.** With `kuopio_05` attempt 14 fixed (question 8), the recorded replay's fixed
      quality gates (offset from the source terrain at most 5 m, grade at most 1.0, pitch change
      at most 30 degrees per knot, as the rendered replay checks) fail on three profiles:
      `kuopio_03` attempts 7 and 8, edge 2, 5.76 m (grade 0.41); `kuopio_07` attempt 20, edge 0,
      5.08 m (grade 0.51). Both lie beside a junction on steep ground. `kuopio_03` attempt 7
      ends a 2+2 stroke on a road climbing about 38% near the node: the plane, fitted to the
      arms' control points 12 m out (the "Heights" sample), has that grade, and the 24.4 m mouth
      station of the wide T holds it beyond where the hill flattens, so the pinned profile rises
      to 4 m above the ground at the mouth and 5.76 m in the 32 m blend. `kuopio_07` attempt 20
      crosses a road climbing up to 35% (its terrain-following control) with a contour road;
      neither corridor fits the other's samples, the straighter tie goes to the flatter corridor
      (3% plane), and the steep road is held to it over its 15.85 m mouth station, 5.08 m over
      the ground 28 m out. The limits pass on the legacy setbacks (about 5.5 m); the gate C
      mouths and the plane rules are what changed. Since gate C the attempt-14 failure stopped
      the replay before these checks.
    - **Options.** (a) Sample each arm for the plane at its mouth station rather than at 12 m
      (the plane then meets every authority arm's control profile where its pin ends, and the
      blend starts from the control height); it changes every sloped junction's plane, so the
      probe and the sloped digests are re-recorded. (b) Rank a corridor whose plane keeps every
      arm within the source-delta limit before the flatter one (rule 3 of the ranking), so the
      steep road keeps its grade at a fall-line crossing. (c) Accept and re-baseline the
      fixture's limit, which relaxes a recorded acceptance target and needs the user.
    - **Recommendation.** Measure (a) and (b) on the recorded replay, the reference probe and
      the sloped fixtures before deciding: (a) addresses `kuopio_03` (an authority arm held past
      the hill's knee), not `kuopio_07`, where the steep road is a non-authority arm and only (b)
      changes which road keeps its grade (with the fall-line corridor the contour road, 57
      degrees off it, would take about 16% across its 15.85 m mouth, about 2.6 m). The quality
      test stays ignored under this question until then.
    - **Measured and decided (round 3).** (a) as `max(12 m, mouth station)`, at most half the
      edge (`junction_profile_incident_samples`; the clip is current before the solve): the
      plane never takes a sample inside the stretch it pins. On the recorded replay every
      profile passes the limits, `kuopio_07` included: its steep arm's 15.85 m mouth moves the
      sample out, which changes both corridors' grades and the ranking's outcome, so (b) is not
      needed. The reference probe changes only the four sloped Y fixtures (`y_25_branches_first`,
      `y_25`, `y_40`, `y_60`: graph, nodes, spans, walkways, lanes and queries, not the layouts),
      whose narrow-angle mouths lie beyond 12 m; re-recorded with that attribution. Every flat
      fixture, the T, four-way, bend, terminal and footpath fixtures on the slope, the edit-order
      tests (T, Y and end-to-end joins in every order) and the narrow-marks digest test are
      unchanged. `kuopio_recorded_profiles_preserve_quality_limits` runs again.
    - **Visible change, for review.** On slopes, a junction whose mouth lies beyond 12 m (narrow
      Y angles, wide roads at oblique angles) takes its plane from the arms' control heights at
      the mouths instead of 12 m out, so its tilt follows the ground over the junction's own
      extent; on even slopes the difference is millimetres, on a hill's knee it removes the
      embankments above (5.76 m and 5.08 m beside the recorded junctions).
    - **On `ROAD-68-F` part 2 (round 4).** Rebased onto part 2's families, the same sample
      changes six of their product-probe entries, attributed by a build of the combined tree
      with this sample reverted, which gives part 2's table exactly: the sloped `rural_y_40`,
      `band_width_step`, `band_width_step_curved` and `width_pass_through`, and
      `width_bridge_pass_through` flat and sloped (its bridge climbs 6 m), whose mouths lie
      beyond 12 m (the narrow Y and the D4 tapers that part 2 lays out at straight profile
      changes). So the visible change above also covers those junctions; in the reference
      probe `y_25/true`'s lanes follow both part 2's minimum run and this sample.
32. **The cell store the preview plans lots on (`ROAD-68-E` round 5, from question 21 (b)) —
    decided by the user, 2026-10-25: profile first; (b) if no ~1 ms cut is found.** Round 4 made the zoning cell store persistent (`imbl` maps) so that the
    road tool's view holds it in `O(1)` and the preview can plan lots on a copy. That store
    makes the commit's `buildings` stage 0.3-1.7 ms slower at 10k-100k residents (round 5
    record), and the cost is the persistent maps themselves, not their sharing: a build whose
    view shares nothing is as slow, as are builds with the values behind `Arc` and every write
    reading first, and with cell generation's temporary candidate index in plain maps. The
    acceptance (`buildings` within the control's spread) cannot be relaxed.
    - **Options.** (a) Keep the persistent store and find the operations that cost (a profile
      of the commit's cell-lot windows, C against the persistent build, is the next step; the
      reads are spread over generation's conflict and support passes and lot derivation, so a
      large cut is not expected). (b) Keep the live store in plain maps, as before round 4, and
      give the view a persistent copy brought up to date at each publication from the store's
      own write journal (every block, chunk entry, frontage, curve source, frame and alignment
      write records its key; the publication copies those entries, `O(changes × log n)`): an
      exact derived copy, never stale, at twice the memory; the planner is one function
      generic over the two map kinds, the preview plans on a copy of the view's store, and the
      commit installs a matching plan by writing its changed entries into the live store,
      otherwise plans on the live store itself. (c) Plain live store and no copy in the view:
      the preview cannot plan lots, so question 21 (b) is not met.
    - **Recommendation:** profile first (a, an hour); if no cut of about 1 ms is found, (b).
      The parcel store needs the same kind of view for the preview's plan (it is a plain
      store copied in `O(parcels)`), so (b)'s journal design would serve both.
    - **Decided by the user, 2026-10-25: profile first; (b) if no ~1 ms cut is found** that
      brings `buildings` within the control's spread: a plain live store, plus a persistent
      copy in the view kept up to date from the store's write journal at each publication,
      exact and never stale, at about twice the cell store's memory, with one generic planner.
      Its measurement records the extra memory at 100k residents.
    - **Profile done (round 5, 18:14-18:17):** the extra time is spread over every lap of
      generation and over derivation, with no single ~1 ms cut
      ([round 5 record](#part-2-round-5-2026-10-25-branch-road68-e5-work-in-progress)); (b) is
      next.
    - **Built (round 6, branch `road68-E6`):** the live store keeps plain maps and the view a persistent copy that each
      publication brings up to date from the live store's write journal, with every algorithm
      on the store one function generic over the two kinds
      ([round 6 record](#part-2-round-6-2026-10-25-branch-road68-e6-work-in-progress)). Products
      and determinism are unchanged; `buildings` is within the control's spread once the breakdown harness
      publishes its first view as the game does (one 10k sample 0.06 ms above the control's
      higher run, inside the control's own run-to-run difference). Round 7 hides the follow
      behind the commit's graph copy on the pool, so the tool snapshot is at the control's, and
      profiles render publication and the two preview stages: identical work, code layout under
      question 36 ([round 7 record](#part-2-round-7-2026-10-25-branch-road68-e7)). The copy holds 175,055 entries at 100k
      residents: 16.5 MB of keys, values and lists, about 20.7 MB with each entry's pointer and
      `Arc` header (map nodes not counted), next to the live store's own 16.5 MB.
33. **Where the preview's lot plan runs (`ROAD-68-E` round 6, from question 21 (b)) — decided by
    the user, 2026-10-25: no per-pose lot planning; question 21 (b)'s preview half is paused
    until the commit is measured with `TERRAIN-07`; if it then misses, lot regeneration is
    deferred one frame, as routing adoption is (D7).**
    - **Evidence.** A preview plan does the commit's lot work: the marks on copies of the cell
      store and queue, cell generation and lot derivation over every queued region. That is
      the commit's `buildings` stage, several milliseconds per sample at 10k-100k residents
      (round 5 and 6 records), against the preview's 7.5 ms per-pose stage total
      ([stage budgets](#stage-budgets)), on every previewed pose.
    - **Options.** (a) Plan synchronously on every pose. (b) Plan in a pure job per pose and
      let the commit install a finished, matching plan. (c) Plan no lots in the preview; if
      the commit still misses a frame once measured with `TERRAIN-07`'s terrain, defer lot
      regeneration to a job adopted one frame after the commit, as routing adoption is (D7).
    - **Decided by the user, 2026-10-25: (c).** The user's rule for question 21 was
      conditional: speed up lot regeneration first, and plan in the preview only if the commit
      still does not fit a frame. That cannot be judged until `TERRAIN-07` replaces the legacy
      terrain validation, which takes 10-90 ms of today's commit. Per-pose plans, synchronous
      or in jobs, are not the next step.
35. **Worn zebra bars on residential streets (`ROAD-68-F` part 1, from the gate F notes) —
    decided by the coordinator under the user's delegation, 2026-10-25, confirmed by the user 2026-10-25: (a).**
    - **Evidence.** Two bars of the upper branch's zebra on `y_25_slope_oblique` looked faded
      and patchy and were taken for z-fighting on the sloped node plane. They are the shader's
      studded-tyre wear (`fr_zebra_paint`): it thins the paint in the wheel paths, 0.9 m either
      side of each lane centre, by up to `age * 0.9 * 1.5`, for every crossing whatever its
      traffic. On the 7 m two-lane fixtures the inner wheel paths (2.65 and 4.35 m from the kerb)
      fall on the third and fifth of the seven bars; the outer ones fall in gaps. The flat Y
      shows the same bars worn; the stem's crossing (cosmetic age 0.96) and the upper branch's
      (0.71) wear, the lower branch's (0.29) is crisp. Nothing overlaps: since `ROAD-68-D` part 5
      the paint is a vertex frame on the one asphalt layer.
    - **Options.** (a) keep: Finnish winter traffic runs on studded tyres, so even residential
      zebras wear in the wheel paths, and the look-dev against street photographs chose it;
      (b) scale the wheel-path paint wear by the asphalt's traffic factor, as the ruts are
      (none on a 1 + 1 street at 50 km/h), so residential zebras only fade evenly with age;
      (c) cap the wear so a bar keeps at least half its paint.
    - **Recommendation.** (a): it is the designed, photo-matched look, not a rendering defect;
      (b) if the user finds residential crossings too worn.
36. **Timing comparisons that follow the codegen-unit partition (`ROAD-68-F` part 2 round 4) —
    decided by the user, 2026-10-25: (a); the build stays at 16 units.** Release builds split the crate into 16 codegen units; any added code (tests included,
    in the release test binary every benchmark runs from) can move unrelated functions between
    units and change what is inlined and which copies are elided, so a same-session A/B can show
    a stage regression with no change in work.
    - **Evidence (2026-10-25).** Three cases with equal products: part 1's 30k commit `surface`
      (+7-8 ms; the site paving query's envelope fold out of line); round 3's preview payload
      (+0.06 ms; the chunk router out of line and two more polygon copies in the triangle clip,
      fixed structurally in round 4); round 4's rebase onto `096df250` (`ROAD-68-E` round 4): the
      rebased fix's preview terrain stage is 0.23 ms (5%) above `road`'s, which shares every
      terrain function with it; a profile puts it in the legacy terrain CDT's R-tree code
      (`road_exterior_support_point` +2,100 samples per run, `source_sample_parameter_on_road_
      constraint` +1,200, `rstar`'s bulk-load insertion sort +930, at 12 kHz).
    - **Options.** (a) keep 16 units and fix each hot path found structurally, as round 4 did,
      and attribute every unexplained stage change by profile; (b) build release with
      `codegen-units = 1` (with or without thin LTO), so no added code moves another function's
      inlining; release and test builds take longer and every gate's timing baseline moves once;
      (c) accept a stage change only when it reproduces across two builds whose partition
      differs (as round 4's Cx build did).
    - **Recommendation.** (b), measured first: the release test build time (2.5 min now) and the
      stage and commit timings against the 16-unit control; keep (a) for the code each gate
      owns. The legacy terrain CDT is `TERRAIN-07`'s and is not optimised under `ROAD-68`.
    - **Decision (user, 2026-10-25): (a).** The build configuration stays as it is (no
      `codegen-units` change). A stage regression against a same-session control counts as code
      layout only when a profile shows identical work and the products are bitwise equal, as
      round 4 showed; hot paths a gate owns are fixed structurally. The 0.23 ms in the legacy
      terrain CDT is accepted as layout; `TERRAIN-07` deletes that code.
37. **No maximum road grade (`ROAD-68-K13`, from the `TERRAIN-07-A` wall census) — decided
    2026-10-25 (user): a hard maximum of 1:3 (33%), refused with a typed `road_too_steep`, and a
    warning from 16% (`ROAD-54`'s design grade) shown in the road tool but not refused;
    implemented in `ROAD-68-K14`.** The user chose the value from a real-world comparison: the
    world's steepest streets are about 35% (San Francisco's steepest 31.5%), and 1:2 (50%)
    matches no real street.
    - **Evidence.** Nothing bounds a road's grade. The stroke solve
      (`solve_standard_road_profile`) treats 16% as a target and raises each 6 m span's limit to
      the slope of its own targets (the terrain under it, or a carriageway the stroke pins to), so
      it follows any hillside. Replayed today, the recorded Kuopio strokes reach 0.725
      (`kuopio_12`, edge 1); of the replayed city's 70 final edges, 63 exceed 10%, 47 exceed 16%,
      21 exceed 1:3 and 4 exceed 1:2 (0.50-0.73, in `kuopio_04`, `kuopio_06`, `kuopio_07` and
      `kuopio_12`). The [`ROAD-54`](#road-geometry-validation-road-54) validator measures grade
      (illegal above 16%), but placement does not refuse it (the hill policy). A load checks only
      that every piece builds and every bend or junction has a plane, so the reference save's
      grade-4.56 span loads (question 38). The recorded replay's grade 1.0 is a test limit
      (`placements.json`), not a rule. Under `TERRAIN-07`'s TD7 every road steeper than the 1:2
      tie-in slope walls itself (terrain.md, the T2 wall census), and a road end's node piece
      continues the end grade over its cap: the saved edge 13's dead end reaches 22.65 m below
      the ground. The saved edges 13 and 14 also show what a 16% clamp without a rejection
      builds (question 38).
    - **Options.** (a) A grade limit with a typed rejection (`road_too_steep`, with the measured
      grade and the limit in the road tool, as `ROAD-54`'s radius and arm refusals): the stroke
      solve relaxes a span only up to the limit, cutting or filling where the ground is steeper,
      placement refuses an edit whose finalized physical profiles exceed it or whose ends cannot
      be joined within it, and a load fails on a saved profile over it, as on a failed node. The
      value: at 16% (`ROAD-54`'s illegal tier) 47 of the 70 replayed edges are re-profiled or
      refused (`kuopio_04` attempt 9 climbs 24% on average between its ends, so it cannot be built
      at all); at 1:2 (TD4's tie-in slope, the steepest road the graded terrain meets without a
      wall) 4 are. (b) Automatic structures beyond the limit: the steep stretch becomes a bridge
      or tunnel with piers or portals (a class change inside an edge, larger than `ROAD-68`).
      (c) The current behaviour: no limit, and a road steeper than 1:2 walls itself.
    - **Recommendation (K13).** (a) at 1:2 now, keeping 16% as `ROAD-54`'s design warning and
      choosing any lower hard value with `ROAD-54`'s hill policy; (b) later, as its own feature.
    - **Measured first (`ROAD-68-K14`, fresh, today's code: `road` `4710f59b` with K13's
      test-only commit, before the rule).** Grade by the rule's one grade (`steepest_grade`, the
      steepest segment of at least 5 cm) on every live physical profile. The recorded session in
      one world (the 36 accepted strokes in attempt order, the reference network's 70 edges): 47
      edges over 16%, 21 over 1:3, 4 over 1:2. Over 1:3, by the stroke that last derived them:
      attempt 8 (`kuopio_03`, 2 edges), 10 (`kuopio_04`, 3), 14 (`kuopio_05`, 1), 17 (`kuopio_06`,
      2), 20 (`kuopio_07`, 1), 22 (`kuopio_08`, 2), 24 (`kuopio_09`, 3), 27 (`kuopio_10`, 2), 31 and
      33 (`kuopio_11`, 1 each), 35 (`kuopio_12`, 2) and 37 (`kuopio_12`, 1). Over 1:2: attempt 10's
      0.579, 17's 0.535, 20's 0.503 and 35's 0.725. Eleven of the 21 follow ground steeper than 1:3
      with their control profile (the stroke solve's per-span relaxation: control grades
      0.35-0.58 on ground of up to 0.58), some steepened further by their junction pins (attempt
      35's 0.455 to 0.725); the other ten have controls at 0.19-0.32 that the pins steepen to
      0.34-0.46: a blend that eases an arm onto a flatter node plane over the 32 m blend zone adds
      up to 1.875 times (the smootherstep's steepest slope) the plane-to-control offset over the
      blend's length. Each recorded case on its own source world (the recorded
      tests' replay, 13 cases with attempts 34 and 35 in two): 51 over 16%, 23 over 1:3, 5 over
      1:2.
    - **Implemented (`ROAD-68-K14`).** One module owns the rule (`network/grade.rs`): the grade
      (`steepest_grade`), the maximum (`ROAD_GRADE_MAX` = 1/3), the warning (`ROAD_GRADE_WARNING` =
      0.16), the grade profiles are built to (`ROAD_GRADE_BUILT` = 0.33) and the construction
      within a grade (`grade_envelope`, `construct_within_grade`) that the stroke solve and the
      junction profile solve share ([Heights](#heights)). Placement refuses above 1:3 in
      `validate::edit_violation`, called by the plan the preview and the commit share, so the
      preview shows the refusal before the click with the measured grade and the limit
      ([`ROAD-54`](#road-geometry-validation-road-54)); node edits refuse it in
      `SimCore::try_graph_edit`. The validator's grade tiers become 16% / 1:3 for streets and
      6% / 1:3 for bridges and tunnels: one maximum for every class (decided by the coordinator
      under the user's delegation, 2026-10-25 night, for review: the user's 1:3 named no class,
      and `ROAD-54`'s structural 16% was never refused). Not covered in `ROAD-68-K14`:
      `RegionGraph::sync_to_terrain`, the whole-graph terrain sync, which re-graded control
      polylines to the ground without the rule (covered since `ROAD-68-K15`, below); and loads,
      which keep saved profiles as saved, over the limit or not (option (a)'s load rejection is
      not part of the decision). New tests: `a_road_up_to_one_in_three_commits_and_a_steeper_one_is_refused`
      (20% and 30% commit with the warning, 10% without, 40% is refused in the preview and the
      commit alike) and the validator's 40% / 20% / 12% ramps. Two fixtures steeper than the
      maximum were brought within it, keeping their assertions: the stroke solve's interior-relief
      test (ends at 48%, now 30%) and `hillside_crossing_keeps_existing_corridor_authority_after_splits`
      (a 60% hillside, now 30%).
    - **The rule on the recorded session (fresh, `ROAD-68-K14`).** In one world, all 36
      accepted strokes commit with no failed node; no edge exceeds 1:3 (21 before), 47 exceed
      16% and carry the warning. Measured on the way: building at exactly 1:3 read 0.33334 on
      the derived points and refused seven strokes, hence the 33% build grade; with the stroke
      solve alone, eleven junction strokes (attempts 8, 10, 12, 17, 20, 22, 24, 31, 33, 35 and 37)
      were refused by their arms' blends at 0.34-0.62, hence the blend's construction; holding
      the plane only to the hard zone, not its reused mouth knot 1 cm on, failed attempt 35's
      node (`junction_profile_mismatch`); letting the construction move the control beyond the
      blends too rebuilt terrain-synced roads off their bumps
      (`terrain_stroke_revalidates_cell_growth_and_undo_restores_feasibility` failed), so it
      moves only the blends. The replay's stroke ends now take the height the road
      tool picks (a road's surface, `get_world_surface_height_internal`) instead of the visual
      terrain: an end beside a road the solve cut or filled read as a bridge or tunnel stroke
      and failed its open chain end (attempt 17). The source-delta quality limit fails on 4
      edges, up to 0.36 m past 5 m in junction blends ([open question 39](#road-68-open-questions)).
    - **`ROAD-68-K14`'s open items (`ROAD-68-K15`; decided by the coordinator under the user's
      delegation, 2026-10-25 night, for review).** (1) The whole-graph terrain sync applies the
      rule's construction: `RegionGraph::sync_to_terrain` builds each re-graded standard control
      inside the source band within 33%, else 1:3, its node ends held
      (`grade::construct_within_bounds`), and where the ground leaves none, at the least grade it
      allows (steeper than 1:3). Refusing such a terrain edit is not implemented: a rollback beside
      the failed-node one (`road_too_steep` when the sync leaves a road steeper than 1:3 and than
      before) was built and measured, but `terrain_stroke_revalidates_cell_growth_and_undo_restores_feasibility`
      sculpts a 200 m mound over a lot and the road end beside it, which the rollback refuses, and
      the only mound found that blocks the lot without reaching the road (centred 10 m beyond the
      lot) leaves the lot blocked after its undo, because terrain undo restores the authored
      terrain but not the source terrain the lot reads (terrain.md, "authoritative terrain undo",
      `TERRAIN-07`'s). Recommendation, for the next gate: refuse the terrain edit as the
      rollback did, once the test sculpts a mound its road can follow or terrain undo restores
      the source terrain. (2) Old saves load as saved: a save with grades over 1:3 loads
      unchanged and only new edits are refused (recommended and taken: refusing the load would
      lose the player's city over a rule it predates, and every edit beside such a road stays
      possible because placement refuses only a profile it makes steeper). (3) The first planning
      pass can refuse `road_too_steep` from a carriageway pin its own edit rebuilds before a
      second pass runs (a refused pass has no plan, so no rebuilt pieces to exclude). Fix only if
      it changes a verdict: a probe counting refused passes that read any carriageway found none in
      the debug `nodes::` (492 tests, the Kuopio replays and road tool tests included) and
      `simulation::` (1,294) suites, so it is left as is and recorded.
    - **Timing of the rule (`ROAD-68-K15`, fresh).** Release lib test binaries (`$TEST_BINARY`,
      16 codegen units): C = `road` `4710f59b` (before the rule), K = `d740a332` (the rule, built
      from `ac0480cc`, whose code is the same), F = `road68-K15`'s production code (its two test
      changes came after the build). Both locks, `taskset -c 0,2,4,6,8,10,12,14`, no restic,
      `METRUM_DEBUG=0`; `benchmark_road_preview_stages` with `RAYON_NUM_THREADS=4` and
      `benchmark_populated_commit_breakdown` with `METRUM_COMMIT_TIERS=0,10000,30000,100000` and
      `RAYON_NUM_THREADS=8`. Session 1 (2026-10-25 23:19-23:41): stages C K C K C K C K, breakdown
      C K K C. Session 2 (2026-10-26 00:35-00:57): stages C K F three times, breakdown C F F C. Every
      breakdown digest and every payload vertex count is equal across C, K and F.
      | Preview stage p50, ms | C | K | F |
      | --- | --- | --- | --- |
      | `sloped_t` plan (session 1 / 2) | 0.460-0.471 / 0.463-0.467 | 0.482-0.490 / 0.486-0.487 | 0.485-0.489 |
      | `sloped_branch` plan (session 1 / 2) | 0.404-0.412 / 0.402-0.407 | 0.418-0.425 / 0.420-0.426 | 0.416-0.426 |
      | `sloped_t` payload (session 2) | 0.384-0.417 | 0.381-0.386 | 0.421-0.431 |
      | `sloped_t` terrain (session 2) | 4.618-4.653 | 4.279-4.317 | 4.513-4.529 |
      | `sloped_t` total (session 2) | 5.776-5.841 | 5.465-5.487 | 5.726-5.757 |
      The preview plan stage rises by 0.015-0.022 ms (3-5%) with the rule, beyond C's spread, and
      the commit's plan by 0.02-0.07 ms at every tier (for example 10k sample 0: C 0.468/0.476, K
      0.509/0.515; 100k sample 0: C 0.538/0.582, F 0.600/0.675). It is the rule's work: the
      envelope, the band and the construction in the stroke solve, the grade passes of every
      derived profile, and in K the full `ROAD-54` validator (radius windows included) run again on
      every re-derived profile for its grade alone; a flat profile (perf, LBR, 8 kHz, not
      acceptance) shows `validate_edge` at 27 / 54 / 21 samples in C / K / F, since F checks the
      re-derived profiles and the road tool's warning with the grade check alone
      (`validate::grade_violation`). The remaining rise is open: the stroke solve's band and
      envelope allocate about eight vectors per solve and run twice per two-pass plan. The terrain
      stage and payload move by up to ±0.35 ms and +0.04 ms with code no gate changed (the legacy
      terrain CDT and the payload, `spade`'s incircle inlined in K and F, `rstar`'s sort and the CDT
      clip closures moving between functions in the same profile): code layout under question 36.
      The commit's `surface` stage (the legacy terrain CDT) is bimodal in every binary: 30k sample
      0 reads 110.0-115.3 ms for C in session 1 and 111.5 and 149.8 ms for C in session 2 (K 147.2
      and 148.9 in session 1, F 114.5 and 117.1 in session 2), and F's surface reads 5-10 ms above
      C at 10k sample 0, 30k sample 1 and 100k sample 0 with equal digests and a flat city whose
      profiles take the allocation-free path. Recorded here first as layout, without a profile;
      corrected by the coordinator's ruling on the `ROAD-68-K15` report: question 36 counts a stage
      change as layout only when a profile shows identical work, so the rise was unattributed
      until `ROAD-68-K16` profiled the commit window (below).
    - **The allocation-free stroke solve (`ROAD-68-K16`).** The stroke solve runs on every preview
      pose, so its per-solve vectors were a hot-path allocation (the coordinator's ruling on the
      `ROAD-68-K15` report). Its buffers now stay with each thread and are reused between solves
      (`StrokeScratch` in `road_edit/input.rs`: the simplified alignment, the samples, the smooth
      solve's span limits and heights, and a `grade::GradeScratch` with the runs, held points,
      bounds, envelope, heights and the construction's second pass); the grade functions write
      into caller buffers (`source_bounds` keeps the band's excess in its output), the junction
      blend construction takes the thread's `GradeScratch` (its end distances included), and the
      terrain sync one per sync. Taken for a solve rather than borrowed, so a solve nested on the
      same thread starts from empty buffers instead of failing a borrow. Heaptrack (release lib
      test binaries, the preview stages benchmark on 4 workers, not timing): the stroke solve's
      own allocations fall from 11,812 for C's 1,928 solves (the alignment, the samples' growth
      and the smooth solve's two vectors; `ROAD-68-K15`'s band added about eight a solve) to 41,
      each a buffer's first growth on its thread. A solve still allocates the polyline it returns
      (1,928) and the support's list of the pieces it read (960, `StrokeSupport::height_at`,
      question 16); the junction profile solve's own allocations (`pinned_physical_profile`, the
      profile copy, endpoint distances and support knots: 84,242 in C and in F alike) predate the
      rule and stay open (removed by `ROAD-68-K17`, below). Products are bitwise equal: both
      probes, both Kuopio quality tests, and the re-recorded reference, whose every saved profile
      point keeps its bits and storage order (question 39).
    - **Timing (`ROAD-68-K16`, fresh, 2026-10-26).** Release lib test binaries (`$TEST_BINARY`,
      16 codegen units): C = `road` `4710f59b` (before the rule), F15 = `ROAD-68-K15`'s timed binary
      (`road68-K15` on `ac0480cc`, with its since-dropped rollback), R = `road` `b3627ada` (gate
      E, `TERRAIN-07-A`, the rule without `ROAD-68-K15`), F = `road68-K16`'s final code (built
      before two whitespace-only edits). Both locks, `taskset -c 0,2,4,6,8,10,12,14`, no restic,
      `METRUM_DEBUG=0`; 02:33-03:05: `benchmark_road_preview_stages` with `RAYON_NUM_THREADS=4`,
      C F15 F R R F F15 C twice, then `benchmark_populated_commit_breakdown` with
      `METRUM_COMMIT_TIERS=0,10000,30000,100000` and `RAYON_NUM_THREADS=8`, C F R R F C. Every
      payload vertex count and every breakdown digest is equal across the binaries. A
      `cargo check` run without the build slot or timing lock (`TERRAIN-07-C` round 2,
      02:50:40-02:51:30) overlapped one of these breakdown runs, so that run's timings are not
      acceptance evidence; the commit-breakdown conclusion rests on the other runs, and
      `ROAD-68-K17`'s same-session timing on `road` `ef12bf26` re-measures it (below: P, before
      `ROAD-68-K15`, against C, `ROAD-68-K16`'s code, shows no commit stage beyond the same code's
      run-to-run spread).
      | Preview stage p50, ms | C | F15 | R | F |
      | --- | --- | --- | --- | --- |
      | `sloped_t` plan | 0.461-0.471 | 0.476-0.494 | 0.489-0.491 | 0.482-0.486 |
      | `sloped_branch` plan | 0.403-0.413 | 0.421-0.425 | 0.425-0.427 | 0.421-0.422 |
      | `sloped_t` payload | 0.382-0.383 | 0.417-0.457 | 0.378-0.387 | 0.381-0.384 |
      | `sloped_t` terrain | 4.607-4.640 | 4.497-4.604 | 4.471-4.521 | 4.342-4.412 |
      | `sloped_t` total | 5.759-5.818 | 5.732-5.858 | 5.683-5.730 | 5.530-5.619 |
      F's plan stage is 0.003-0.009 ms below R's: what `ROAD-68-K15`'s band and this solve add
      on today's `road` is gone. Its rise over C, 0.015-0.021 ms on `sloped_t` and 0.008-0.019 ms
      on `sloped_branch` per pose, is the rule's own work, measured per pose by profile: the
      preview stages under `perf record -c 200000` (user cycles and instructions, LBR call
      stacks, the same binaries and workers, not acceptance), each sample's leaf and caller frames
      resolved with their inlined frames (`llvm-addr2line -i`), count as the rule's when one lies
      in `network/grade.rs` or in `build_within_grade`, `grade_violation` or `record_edit_grade`:
      F 320 cycle samples (64 M cycles, all under `plan_road_edit`), C none, over 800 previewed
      poses: 0.016 ms per pose at the P-cores' 4.9 GHz. That is the band's terrain samples, the
      envelope and construction, and the grade of every derived profile; no allocation is left in
      it. The commit's plan shows no rise beyond its spread at any tier (10k sample 0: C
      0.473/0.477, R 0.474/0.490, F 0.465/0.416 ms).
    - **The commit's `surface` rise, attributed (`ROAD-68-K16`).** The breakdown above does not
      reproduce it with F: at `ROAD-68-K15`'s three samples the surface stage reads 10k sample 0
      C 89.4/89.9, R 91.8/87.7, F 88.4/85.0 ms; 30k sample 1 C 52.0/51.9, R 55.6/51.4, F 50.5/51.0;
      100k sample 0 C 76.8/76.0, R 74.4/76.4, F 76.0/74.9. A profile of the commit windows only
      (the breakdown at 10k, 30k and 100k, 8 workers, `perf record -D -1 --control` driven by the
      benchmark's `METRUM_ECON_PERF_CONTROL` fifo around each commit, `-c 200000` user cycles and
      instructions, nine commits per binary, not acceptance) attributes F15's: over the surface
      stage's functions (the legacy terrain CDT and grading, site paving, `rstar`, `spade`) F15
      retires 10.4% more instructions than C (+2,336 samples) and spends 8.1% more cycles;
      +2,175 of those samples lie in one inlining boundary of the site paving R-tree's envelope
      query, code `ROAD-68-K15` does not touch: `rstar`'s `SelectInEnvelopeFuncIntersecting` for
      `PavingRegion` is out of line in F15 alone (+1,876), its caller `SelectionIterator` has
      1,238 fewer, `rstar`'s point generators +1,154 and the line R-tree's selection +292; the
      other functions net +161. The same queries with the same products, the envelope test called
      instead of inlined: the codegen-unit partition, question 36's part 1 case in the same
      function, so layout by the profile, and gone in F (surface functions 4.7% fewer
      instructions than C and 1.2% fewer than R; all commit-window functions 2.8% fewer than C
      and 0.6% more than R).
    - **The allocation-free junction profile solve (`ROAD-68-K17`).** The junction profile solve
      runs twice on every preview pose (both planning passes) and on every commit, and
      `check_node` solves each checked bend and junction again, so its per-solve collections were
      the same hot-path bug as the stroke solve's. Heaptrack (release lib test binaries, the
      preview stages benchmark on 4 workers, 962 previewed poses and 1,928 solves; allocation
      counts, not timing), C = `road` `ef12bf26`, F = `road68-K17`: the solve and the node checks'
      single-node solves allocate 163,224 times in C (170 a pose): the support knots' end
      distances 58,604, the endpoint samples' distances 17,332, the surface-edge sets 16,378, the
      incident lists and their node map 16,376, the pin map, node ids, phase partition and changed
      set 12,524, the node solves' samples, corridor list, authority set and plane offsets 11,556,
      the profile copy and the changed profiles 8,668, the knot inserts 8,304, the R-tree's
      removals 6,742 and the blend distances 6,740. F allocates 11,619 (12.1 a pose), none of them
      a working collection: 6,742 in `rstar`'s removal (its traversal stack, one per edge the solve
      takes out of the R-tree and puts back), 3,736 where a changed profile outgrows its edge's
      buffer (the preview's excerpt copies each profile at its length, so the edge a stroke adds
      grows by its support knots), 1,084 where the caller's dirty-edge set grows to take the
      changed edges (counted under the caller in C), and 57 first growths of the threads'
      buffers; the node checks allocate nothing. The solve's buffers stay with each thread
      (`ProfileSolveScratch` in `graph/rebuild/junction_profiles.rs`, taken for a solve as the
      stroke solve's are): the derived edges and their nodes as sorted lists instead of hash
      sets, the incidents of all nodes in one list sorted by node, attachment and incident order
      instead of a map of per-node lists, the pins in a slot per derived edge end instead of a map,
      the samples, the profile being derived with its end distances, and the blend construction's
      `GradeScratch` (now the solve's, so `GradeScratch::with` and its thread-local are gone);
      the best authority corridor is kept as the candidates are ranked instead of sorting a list,
      a sample's distance is accumulated along the control polyline instead of building the end
      distances, and a changed profile is copied into the edge's own buffer. The finalizer
      inserts the changed edges into the caller's set (`finalize_junction_endpoint_profiles(nodes,
      terrain, changed)`) instead of returning a new one. The plan as a whole allocates 450,266
      times in F against 604,765 in C. Left: the R-tree removals (re-indexing only the changed
      edges would change the tree's shape, whose traversal order `get_edges_near_aabb`'s callers
      use, so it is not behaviour-preserving), the changed profiles (output), and in the callers
      the clip rebuild's edge list, mouth map and R-tree removals (13,680) and `check_node`'s node
      plans (224,448 in `junction_rejection`, roadbed code; 247,558 in C). Products are bitwise
      equal (debug, fresh): both probes, the narrow-marks digest, the Kuopio quality tests (9
      passed, 7 ignored), the save determinism test, and the Kuopio reference re-recorded twice
      at `3c1dbc27…` with the fixture unchanged; every payload vertex count and breakdown digest
      below is equal.
    - **Timing (`ROAD-68-K17`, fresh, 2026-10-26).** Release lib test binaries (`$TEST_BINARY`,
      16 codegen units): P = `road` `5b138f8e` (before `ROAD-68-K15` and `ROAD-68-K16`), C =
      `road` `ef12bf26`, F = `road68-K17`'s final code. Both locks, `taskset -c
      0,2,4,6,8,10,12,14`, no restic, `METRUM_DEBUG=0`, no compiler, cargo or Godot process at any
      10 s sample of the session; 05:22-05:54: `benchmark_road_preview_stages` with
      `RAYON_NUM_THREADS=4`, C F F C twice, then `benchmark_populated_commit_breakdown` with
      `METRUM_COMMIT_TIERS=0,10000,30000,100000` and `RAYON_NUM_THREADS=8`, P C F F C P. Every
      payload vertex count and every breakdown digest is equal across the binaries.
      | Preview stage p50, ms | C | F |
      | --- | --- | --- |
      | `sloped_t` plan | 0.470-0.480 | 0.474-0.480 |
      | `sloped_branch` plan | 0.417-0.424 | 0.410-0.416 |
      | `sloped_t` roadbed | 0.319-0.331 | 0.322-0.327 |
      | `sloped_t` payload | 0.378-0.404 | 0.378-0.381 |
      | `sloped_t` terrain | 4.242-4.367 | 4.444-4.453 |
      | `sloped_branch` terrain | 4.570-4.604 | 4.644-4.707 |
      | `sloped_t` total | 5.413-5.559 | 5.614-5.623 |
      The plan stage, which holds the solve, is unchanged on `sloped_t` and 0.001-0.014 ms lower
      on `sloped_branch`, about what the 158 allocations a pose it no longer makes cost. The
      terrain stage reads 0.08-0.21 ms higher on `sloped_t` and 0.04-0.14 ms on `sloped_branch` in
      every run, in code no gate changed (the legacy terrain CDT, which `TERRAIN-07` deletes).
      Attributed by profile
      (the preview stages under `perf record -c 200000`, user cycles and instructions, the same
      binaries and workers, C F C F, not acceptance; each leaf sample counted at its innermost
      inlined source function, `llvm-addr2line -i`, so a function counts the same inlined or
      not): F retires 6,950 / 7,516 more instruction samples and spends 6,042 / 8,065 more cycle
      samples than C over the run; 4,440 / 4,485 of those cycle samples lie in `rstar`'s point
      generators (the envelope arithmetic of the terrain CDT's line R-trees: `rstar`'s `point.rs`
      with `core`'s array drain and `try_trait`, out of line and scalar in F, inline as SSE2
      intrinsics in C, which retire 7,227 / 7,096 fewer instruction samples there), and another
      1,292 / 1,715 in `core`'s pointer, iterator and slice helpers whose inlining moved; the
      terrain CDT's, `spade`'s and `robust`'s own code takes 167 / 519 more cycle samples (0.2%,
      0.5%) and 2,722 / 2,151 fewer instruction samples. The same work with the same products
      compiled differently: the codegen-unit partition under question 36, which keeps this
      layout case as recorded and leaves the legacy CDT to `TERRAIN-07`.
      | Commit breakdown, ms (two runs each) | P | C | F |
      | --- | --- | --- | --- |
      | 10k sample 0 plan | 0.437 / 0.489 | 0.425 / 0.578 | 0.435 / 0.438 |
      | 10k sample 0 surface | 87.34 / 86.00 | 84.75 / 86.56 | 84.61 / 89.61 |
      | 10k sample 0 commit | 107.36 / 107.17 | 105.21 / 106.72 | 105.00 / 109.34 |
      | 30k sample 1 plan | 0.408 / 0.452 | 0.406 / 0.425 | 0.490 / 0.398 |
      | 30k sample 1 surface | 52.31 / 49.90 | 53.54 / 54.01 | 50.24 / 53.99 |
      | 30k sample 1 commit | 58.49 / 55.98 | 59.98 / 60.71 | 56.64 / 60.39 |
      | 100k sample 0 plan | 0.542 / 0.565 | 0.593 / 0.460 | 0.564 / 0.547 |
      | 100k sample 0 surface | 75.99 / 72.30 | 76.96 / 74.62 | 75.42 / 76.79 |
      | 100k sample 0 commit | 95.63 / 92.12 | 96.86 / 92.41 | 95.65 / 96.25 |
      The commit's plan stage shows no change beyond C's spread at any tier for F, and none for C
      against P. The clean re-measure of `ROAD-68-K15` and `ROAD-68-K16`'s commit stages (P
      against C; the coordinator's addition, since an unlocked `cargo check` overlapped one of
      `ROAD-68-K16`'s breakdown runs, above): no stage rises beyond the same code's run-to-run
      spread. At `ROAD-68-K15`'s three samples the surface stage reads 10k sample 0 C at or below
      P, 100k sample 0 C 74.62-76.96 against P 72.30-75.99, and 30k sample 1 C 53.54-54.01
      against P 49.90-52.31, where F, whose surface code is C's, reads 50.24 and 53.99; the 30k
      sample 0 surface is bimodal in every binary (P 83.66 / 126.51, C 121.79 / 125.96, F 121.80
      / 84.81). F's surface lies within C's spread except one 10k sample 0 run (89.61; its other
      run 84.61, below C's).
    - **Gate checks (`ROAD-68-K17`, fresh).** `cargo clippy --all-targets`: the baseline's 59
      warnings, none added (run before a token-identical reformat of the new code). `./run.sh
      --test` on the final code with `DISPLAY` and `WAYLAND_DISPLAY` unset (06:00-06:19): exit 0;
      debug lib 1,944 passed, 0 failed, 172 ignored; integration 4 passed; every Godot script
      passes, no `SCRIPT ERROR`; skipped only the shader global probe and the rendered vegetation
      tests (headless). The ignored gate tests are `road`'s (`TERRAIN-07` only), and `roadbed/`'s
      production code is unchanged (its test fixtures take the finalizer's new argument).
      `graph/rebuild/junction_profiles.rs` grows from 1,185 to 1,357 lines: the scratch and its
      incident list (about 90 with their comments), the per-end pin slots, the single-node solve
      shared by `check_node` and the plane readers, and the test-only copy of the profile
      derivation that returns a vector; `grade.rs` loses `GradeScratch::with` and its
      thread-local.
38. **The Kuopio reference save keeps the legacy stroke solve's profiles (`ROAD-68-K13`) —
    decided 2026-10-25 (user): (a), re-record the reference network with today's code, after
    question 37. The re-recorded network is interim: the user will provide a new Kuopio save once
    `TERRAIN-07` and `ROAD-68` are implemented, and it replaces this one through the same pinning
    steps (below).**
    - **Evidence.** The five tallest walls of the `TERRAIN-07-A` wall census (43.9, 37.6, 19.3,
      16.5 and 6.3 m) stand on the saved edges 13, 14 and 40. Edge 13 rises from its dead end
      (node 16, 128.08 m, on the ground) to 155.20 m within 5.95 m (grade 4.56), and edge 14 from
      node 18 (132.79 m) to 156.48 m within 5.94 m (3.99); both then climb at 16% on embankments
      of up to 37.6 and 31.2 m to the four-way node 20 at 171.67 m, 29.43 m above its 142.24 m
      ground. Its plane is flat (grade 0.0017) and its piece stands 24.2-34.6 m above the
      ground; its other arms, edges 15 and 16, start 29.4 m up. The dead ends' terminal planes
      take the 4.55 and 3.99 end grades, so node 16's piece reaches 22.65 m below the ground and
      node 18's 19.83 m. Edge 40 leaves the four-way node 49 (131.07 m, 8.21 m below its 139.28 m
      ground) through control knots at 131.06, 132.77, 131.04 and 134.63 m, 4.9, 10.9, 12.0 and
      14.0 m out (grades up to 1.79); the solve pins it to the node plane up to its 10.09 m mouth
      and blends back to that control by 46.6 m, at up to 1.08 (28 m out), 9.7 m below the
      ground. Edge 44 (`kuopio_10`) has a control grade of 2.49 beside its bend.
    - **Cause: the recorded session's stroke solve, not today's code.** The save is the session
      recorded on 2026-09-08, whose stroke solve clamped every span to 16% with both ends pinned;
      the per-span relaxation came the next day (`2ec65ca9`). Where a stroke's ends lie further
      apart than 16% allows (`kuopio_04` attempt 9 climbs 77.6 m over 321.2 m), the clamp's last
      backward pass leaves a 16% line from the upper pin and the rest of the difference in the
      span beside the lower one: 205.65 − 0.16 × 315.3 = 155.20 m at 5.95 m, as saved. The
      crossing at node 20 took that line's 171.69 m, and attempt 10's road was clamped from it
      the same way (156.44 m at its 5.94 m; 156.48 m saved). The legacy save before `ROAD-68-E`
      part 2 round 2 (the parent of `0a52d9d7`) holds the same control polylines and node
      heights for these edges; the round's regeneration ([question 8](#road-68-open-questions))
      re-derived only clips and junction pins from them, and every load keeps them bitwise
      (`kuopio_saved_reference_loads_as_saved`), so regenerating through the load rebuild cannot
      change them. Replayed today, the same strokes build `kuopio_04`'s four-way at 142.20 m on
      its 142.24 m ground (arm grades 0.30-0.58, following the hill, at most 2.6 m off the
      ground) and `kuopio_09`'s at 139.11 m on 139.28 m (at most 0.46 and 2.15 m). Neither the
      profile solve, its pinned knots and node planes, nor the terrain-sync re-solve (never run
      on the save) made these profiles.
    - **Why the recorded tests passed.** `kuopio_recorded_profiles_preserve_quality_limits`
      replays the strokes on the source world and never reads the save, and the replay tool
      keeps the save as historical evidence that is never a Y target
      (`tools/test_road_terrain_replay.py`). On the save's own network the replay's limits fail
      on 21 of 70 edges: 19 lie more than 5 m off the ground (up to 37.6 m) and edges 13, 14, 40
      and 44 exceed grade 1.0 (`kuopio_saved_reference_profiles_preserve_quality_limits`, which
      shares the replay's measurement, fails so; it is ignored under this question).
    - **Options.** (a) Re-record the reference network from the recorded strokes with today's
      stroke solve: the 36 accepted strokes in attempt order on the source world in one session
      (measured 2026-10-25: all 36 commit, 70 live edges, no failed node, every edge within the
      replay's limits), saved with `save_game_settled`; `placements.json` is re-pinned and the
      tests that name saved edges are re-baselined (the Kuopio yard tests, and on `TERRAIN-07-A`
      the census sites, edge 43's removal and edge 69's hillside T). (b) Keep the save as a legacy
      load fixture and run the geometry and terrain checks (the census, wall sites, pruning
      exactness and Kuopio edits) on the replayed city, which `benchmark_kuopio_graded_edits`
      already builds as its background. (c) Keep everything as it is; the census reports these
      walls as legacy data.
    - **Recommendation (K13).** (a), after question 37, since a grade limit changes the solve and
      so what the re-recording produces: the reference then measures today's roads, and the legacy
      profiles stay in git history as the evidence the replay tool cites.
    - **Pinning a Kuopio reference save** (the re-recorded one now, the user's save later). The
      save path is `benchmarks/fixtures/kuopio-terrain/kuopio-terrain-map.sqlite`; no migration
      code, a save written by `save_game_settled` only.
      1. Write the save. Re-recorded: `record_kuopio_reference_save` (ignored, a manual
         recorder in `road_terrain_plan`, run with `--ignored --exact`) replays the manifest's
         accepted strokes in attempt order on the source world in one core
         (`replay_kuopio_accepted_strokes_in_one_world`, each end at the height the road tool
         picks there), saves, loads (which builds the lanes the bulk commits defer) and saves
         again at the camera and the header save time the old save held (`save_game_settled_at`),
         so re-recording the same city writes the same bytes and the pinned hash changes only
         with the city (`ROAD-68-K16`). The user's save is copied to the path; it must be
         played on the manifest's source world.
      2. `python3 tools/road_terrain_replay.py --pin-reference --output
         benchmarks/fixtures/kuopio-terrain/placements.json` checks the save's terrain against
         the source world and re-pins `reference_save_sha256` and `reference_live_edges`.
      3. Run `tools/test_road_terrain_replay.py` and compare every table's row count with the
         previous save's (only the network tables may change).
      4. Run the Rust tests that read the save: `kuopio_saved_reference_loads_as_saved`,
         `kuopio_saved_reference_profiles_preserve_quality_limits` and the Kuopio yard tests in
         `building_site_terrain` (`kuopio_yard_fixture` and the tests placing yards on its
         roads). The yard tests name their roads by place (`kuopio_yard_edge`: the live edge
         whose midpoint lies nearest the recorded midpoint, on the same side), so a re-recorded
         network keeps them; a different city needs new places with the same character (a flat
         street, a steep one, a junction arm), chosen and recorded with the reason.
      5. On `TERRAIN-07-A`: its census sites, edge 43's removal and edge 69's hillside T name
         saved edges and are re-picked the same way, then its Kuopio census and edits re-run.
    - **Re-recorded (`ROAD-68-K14`, fresh).** Steps 1-4 on this branch, with question 37's rule:
      all 36 strokes commit, 70 live edges and 80 nodes, as before (the same edge ids at the
      yard roads, 3, 9, 15 and 18, within 1 m of their recorded midpoints). Only
      `network_edge_geometry` changes its row count (5,073 to 4,256); `saved_lanes` (2,310) and
      `lane_connections` (1,416) keep theirs, because the recorder saves, loads and saves again so
      the load builds the lanes the replay's bulk commits defer. `placements.json` pins
      `0f20a663…` (was `550a6e82…`); the 8 replay-tool tests pass.
      `kuopio_saved_reference_loads_as_saved` passes (every saved point loads bitwise, no failed
      node), and so does the `TERRAIN-07` half `kuopio_saved_reference_terrain_builds_clean`.
      `kuopio_saved_reference_profiles_preserve_quality_limits` fails only on the four edges of
      question 39 (edges 13, 14, 27 and 36, up to 5.36 m; no grade over 1:3) and stays ignored
      under that question. Yard tests: the third road's yards (edge 15), refused on the recorded
      network beside its four-way 29 m above the ground, now both build, so
      `kuopio_yard_fixture` places 6 houses instead of 4 and refuses the second and fourth roads'
      yards on side +1 as before; every `building_site_terrain` test passes. `TERRAIN-07-A`
      re-runs its Kuopio census and edits on this save.
    - **Step 5 done (`TERRAIN-07-A` round 4, fresh).** The re-recorded save keeps all 70 edges'
      ids and midpoints (within 1 m), so the hillside T and the dead end's removal, now found by
      place (`KUOPIO_HILLSIDE_T`, `KUOPIO_DEAD_END`), resolve to edges 69 and 43 again. The wall
      census falls from 430 walled samples (highest 43.9 m, on the saved edges 13, 14 and 40) to
      24 (0.37 m, between edges 41 and 44, which leave node 50 at 1:3 in diverging directions:
      a real TD7 conflict). Census, edits and the gate's acceptance:
      [terrain.md, the gate A record](terrain.md#terrain-07-a-record).
39. **Junction blends beside arms cut to the source-delta limit (`ROAD-68-K14`, from question 37)
    — decided by the coordinator under the user's delegation, 2026-10-25 night, for review: (a);
    implemented in `ROAD-68-K15`.**
    - **Evidence (fresh, `ROAD-68-K14`).** With the grade rule, the recorded session in one world
      (36 accepted strokes) commits every stroke with no failed node and no edge over 1:3, but two
      edges leave the replay's 5 m source-delta quality limit beside a junction: edge 14
      (`kuopio_04` attempt 10) 5.21 m and edge 27 (`kuopio_06` attempt 17) 5.36 m from the
      ground, 14-26 m from the mouth, inside the 32 m blend. Their controls lie at the limit (the
      stroke solve cut them to 33% across ground up to 58% steep), and the node plane, fitted to
      the arms' control heights 12 m or a mouth station out, lies further from the ground there;
      the blend follows the plane. Before the rule those controls followed the ground and the
      same strokes stayed within 5 m. The stroke solve keeps its samples within the limit
      exactly (`f32` bounds), so no control knot leaves it.
    - **Options.** (a) Build the junction blend inside the same source band as the stroke solve:
      the profile solve reads the source terrain (every caller has it), and an arm whose plane
      and band leave no profile within 1:3 is refused as `road_too_steep`. (b) A margin from the
      band in the stroke solve near ends that join nodes (no rule gives its size). (c) Accept
      blends a little past 5 m beside junctions and re-baseline the replay's limit (relaxes a
      recorded acceptance target; needs the user).
    - **Recommendation.** (a): it is the same construction under the same constraint, and
      question 37's rule already counts the source-delta limit among the constraints that may
      force a refusal. It also has to hold the band where a blend is within the grade but off the
      ground, so it changes more junction profiles than the grade construction does. Taken as
      decided above; `ROAD-68-K14` ran out of time before implementing it, and its two quality
      tests stay ignored under this question until it lands (next steps in the K14 handoff:
      terrain into `RegionGraph::finalize_junction_endpoint_profiles` and its callers, the band
      in `build_within_grade`, triggered by a blend off the band as well as by grade, and a
      `road_too_steep` refusal where the band and the grade leave no blend).
    - **Implemented (`ROAD-68-K15`).** The source terrain reaches the junction profile solve
      (`RegionGraph::finalize_junction_endpoint_profiles(nodes, terrain)`) from every caller with a
      world: the road edit plan's validation graph, the unplanned bulk commit, node moves, road
      removals, the crossing toggle, the load repair and the terrain sync; `None` only from graph
      fixtures without a world (the probe fixtures, graph unit tests and the graph-only non-bulk
      `TransitNetwork::add_road`, which no gameplay commit takes). The band, its widening near held
      points and the construction are the stroke solve's own, moved into `network/grade.rs`
      (`source_bounds`, `maximum_envelope`, `construct_within_bounds`, `least_grade`), so the two
      solves share one definition ([Heights](#heights)). The construction is triggered by a blend
      point more than 5 m from the ground as well as by grade, moves only blended points, and where
      the held points and the band leave no blend within 1:3 it builds the blend inside the band
      at the least grade they allow: steeper than the maximum, so the plan's existing grade check
      refuses the edit (`road_too_steep`, with that grade) and no separate check is needed.
      Measured on the way: with the widening shrinking at the grade the profile is built to (the
      stroke solve's rule in `ROAD-68-K14`), a blend leaving a plane 1-3 m above the band on flat
      ground ran exactly along the band's edge, `f32` rounding closed the 33% attempt, and the 1:3
      attempt then built the blend's descent at exactly 1:3, which read above the maximum and was
      refused; the widening now shrinks at the 16% design grade in both solves (the stroke solve's
      33% attempt is otherwise unchanged; its 1:3 attempt and its refusal grade read a band at
      least as wide as before). New test:
      `junction_blends_keep_the_source_band_or_read_steeper_than_the_maximum` (a plane 6 m above
      flat ground: the blend is back within 5 m once the 1 m excess has run out, and without the
      band it is not; ground falling 10 m at 1:1 past the mouth: within 1:3 without the band,
      steeper than 1:3 and within 5 m with it).
    - **Results (fresh, `ROAD-68-K15`, debug).** `kuopio_recorded_profiles_preserve_quality_limits`
      (13 cases, every stroke) and `kuopio_saved_reference_profiles_preserve_quality_limits` pass
      again and are no longer ignored. The reference save is re-recorded and re-pinned by
      question 38's steps: `1607f880…` (was `0f20a663…`), 70 live edges; only 18 profile points of
      edges 13, 14, 27, 35 and 36 move (at most 0.363 m, edge 27), with those edges' lengths and
      costs; every table keeps its row count, and `lane_connections` holds the same set of rows
      in another order. `kuopio_saved_reference_loads_as_saved`, the `TERRAIN-07` half
      `kuopio_saved_reference_terrain_builds_clean`, the 8 replay-tool tests and every
      `building_site_terrain` test pass (18 passed, 4 ignored, as before); `TERRAIN-07-A` re-runs
      its Kuopio census and edits on this save.
    - **Landed with `ROAD-68-K16`** (branch `road68-K16`, `ROAD-68-K15`'s commit rebased onto
      `road` `b3627ada`, gate E and `TERRAIN-07-A`; only `roadmap.md`'s status paragraph
      conflicted, both sides kept). The `lane_connections` row order that differed between two
      re-records came from the save itself: it wrote the rows in hash-map iteration order, which
      differs between two saves of the same city ([`simulation.md`](simulation.md#save-bytes-road-68-k16));
      the save now writes them in key order. The reference is re-recorded by question 38's steps:
      two re-records give the same bytes, `3c1dbc27…` (was `1607f880…`); against `1607f880…`,
      written by `ROAD-68-K15`'s solve, every table holds the same rows in the same storage order
      except `lane_connections`, the same 1,416 rows now sorted, so every saved profile point is
      bitwise unchanged by `ROAD-68-K16`'s allocation-free solve (question 37). The 8 replay-tool
      tests, the saved-reference tests and every `building_site_terrain` test pass (18 passed, 4
      ignored).
    - **Gate checks (`ROAD-68-K16`, fresh, on its final code).** `cargo clippy --all-targets`: the
      baseline's 59 warnings, none added. `./run.sh --test` with `DISPLAY` and `WAYLAND_DISPLAY`
      unset: exit 0; debug lib 1,935 passed, 0 failed, 171 ignored; integration 4 passed; every
      Godot script passes, no `SCRIPT ERROR`; skipped only the shader global probe and the
      rendered vegetation tests (headless). Disabled tests: 4, all `TERRAIN-07`. `roadbed/` is
      unchanged by `ROAD-68-K16`.

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
| `STRAIGHT_SECTOR_EPS_RAD` (`eps`) | 1° | covers the direction noise of a split straight road (f32 knots, below about 1e-3 rad anywhere in a 20 km world); a visually straight width change tapers (D4; a radial step until pre-D) |
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
  [follow-up](#tighter-corners-and-junction-crosswalks-follow-up) moved them onto the layout, and
  [follow-up 2](#one-crossing-per-degree-two-node-follow-up-2) gives every degree-two node one
  crossing again.
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

**Consequence.** Walkers changed sides only on crossings ([open question
12](#road-68-open-questions)); [follow-up 2](#one-crossing-per-degree-two-node-follow-up-2)
restores a crossing at every degree-two node.

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

The 45° bend has no crossing, as the bend rule then said.

#### One crossing per degree-two node (follow-up 2)

**Status:** done 2026-10-24. The user resolved [open question 12](#road-68-open-questions) as
"keep the old behaviour".

**The rule** (`roadbed/arms.rs`, `crossable_arms`; [D8](#product-decisions)). One function
decides the crossings for the layout, the spans and the lanes:

- Junctions of three or more arms: every car-road arm with sidewalks, unless an override of
  `false` removes it. Unchanged.
- Degree-two nodes, bends and pass-through nodes alike: the legacy `ROAD-64` rule. Arms are taken
  by direction angle in `(-pi, pi]` (atan2 of the direction to the first distinct centreline
  point), then edge index. An override of `true` crosses, `false` does not, and an arm without one
  crosses only if no earlier arm did. One crossing by default, on the first arm.
- Road ends: none; the sidewalk walks round the cap.

**Bends.** The crossing arm is crossable in the layout, so its zebra is the node piece's strip
right outside the curve, its mouth 3 m beyond the tangent, exactly as on junction arms.

**Pass-through nodes** (`roadbed/span.rs`, `span/model.rs`). There is no node piece, so the
crossing is span-owned:

- The strip runs on the crossing arm's span from the mouth at the node out to 3 m (span stations),
  and the span keeps a protected section row on its middle, the crossing line.
- `pass_through_crossing` builds the same span model the compile does: the far node's planned
  mouth (`plan_node`), or the graph clip where it has no piece. Its section on the crossing line is
  bitwise the span's row there, whichever of lanes and roadbed runs first.
- Kerb points are that row's carriageway-edge vertices (`RowLayout::carriageway_edge_slots`), so
  the crossing is square to the road. The crossing arm's sidewalk lanes end on the row's walkway
  centres, at its horizontal station.
- The other arm keeps the walk of a node without a layout: its sidewalks stop
  `SIDEWALK_NODE_INSET` beyond the node, and the two sides join straight.
- An arm crosses only if the strip fits within half its edge ([open question
  13](#road-68-open-questions)).
- `SpanPiece::crossing_strip_m` publishes the strip; lane markings stop at it, and crossing signs
  find it on the span's own sections.

**Walking proof.** `ConnectorTargets::complete` holds again. A road end's cap walk links its two
sidewalks, so it counts as a side change.

**Tests.** Restored to their pre-follow-up expectations:

- `test_pedestrian_crosses_junction` (a straight road again)
- `prepared_commute_queries_match_connector_paths_without_growing_storage` (no forced crossings)
- `incremental_rebuild_repairs_connectors_at_expanded_edge_far_end` (the bend's default crossing)
- `test_crosswalk_counts`: one crossing at the pass-through node. The road end keeps none.

New or extended:

- `lanes::tests::crossings`: the bend fixtures cross by default (45° and across the west
  direction), and pass-through fixtures (straight both ways and bent 8°, flat and at 2.5%) join
  the corner-walk and full/local rebuild repeat tests. `degree_two_nodes_cross_once_on_the_legacy_first_arm`,
  `pass_through_crossing_lies_beside_the_node_on_the_span_row` (strip from the node out to 3 m,
  kerb points bitwise the span row's vertices, square, on the carriageway, walks on the kerb
  segment) and `crossing_overrides_at_degree_two_nodes_follow_the_legacy_rule` (bend and
  pass-through: default, forced, moved, removed, both).
- `walking_proof_holds_with_one_crossing_per_degree_two_node` (planner): a straight road split by
  pass-through nodes and a road with two bends are complete, also with an override of `false` on
  either arm; with no crossing at one node the proof is withdrawn.
- `populated_city::walking_proof::street_grid_walking_proof_holds_at_its_corner_bends`: the 400 m
  street grid has exactly four degree-two nodes, its corners, and its proof holds.

**Checks** (fresh, on the follow-up 2 sources):

- Debug library suite (`RAYON_NUM_THREADS=4 METRUM_DEBUG=0`): 1,812 passed, 0 failed, 185 ignored
  (follow-up: 1,806, 0, 185; six new tests).
- Clippy (`--all-targets`) reports only the 59 legacy-tree warnings, and `cargo doc` reports no
  missing docs.
- `./run.sh --test` (isolated profile, no display) exits 0: the library suite as above,
  `tests/growth_scenario.rs` with 4 passed, and the Godot bridge suites with the same results as
  the follow-up (no script errors, `zoning_reference_test` 8 layouts, the same four scripts
  skipped).
- `roadbed/` holds 6,479 production lines (follow-up: 6,136), under the 8,000 cap.

**Economy performance contract** ([`PERF-01`](economy.md#performance-contract-perf-01perf-05),
2026-10-24). `16354d77` changed walk forecasts without re-running it, so it ran here on the final
release library tests (`$TEST_BINARY`), pinned with `taskset -c 0,2,4,6,8,10,12,14` and
`RAYON_NUM_THREADS=8`, under the exclusive timing lock with no build running:

```bash
RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14 ./run.sh --perf-contract --binary "$TEST_BINARY"
```

- Both runs regenerated the 10k and 30k snapshots, because the fixture revision hashes the
  fixture source, which gained the walking-proof test module. Generation took 25.3/90.6 s
  (baseline) and 14.7/41.6 s (fiscal stress), outside the runtime budget. The regenerated cities
  end day 12 with 5,311 and 15,180 households; the recorded runs had 5,330 and 15,180.
- Two runs gave identical work counts. Hour means: 0.83–0.86 ms (baseline 10k), 4.10–4.11 ms
  (baseline 30k), 6.42–6.47 ms (fiscal 10k) and 28.19–28.66 ms (fiscal 30k). The latest recorded
  run (`SIM-02-K` part 7, [`simulation.md`](simulation.md)) had 0.81–0.92, 3.31–3.40, 8.36–8.51
  and 26.09–26.64 ms, before `ROAD-68` changed the roads, lanes and walks the cities are built on.
  No control binary ran, so the differences are not attributed.
- No time limit fails. All 14 isolated phases, both locality checks and the agent checks pass.
- Seven work-count limits fail: baseline `buyer_sites_built`, `staffing_trials`,
  `supplier_searches`, `supplier_routes` and `job_search_keys`, and fiscal-stress
  `buyer_sites_built` and `cch_searches`. The recorded runs also fail only work-count limits,
  among them these scaling families and the fiscal `cch_searches` limit at its edge, but they do
  not list the full set, so the docs alone cannot rule out a changed member.

**Screenshots.** The [reference harness](#rendered-reference-screenshots) re-ran on the debug build
(48 images, no fixture failures). The 45° bend's plan view shows one zebra on its 45° arm, the
legacy-first arm, right outside the curve on the node piece, with the span starting at the moved
mouth. The T, four-way and terminal images are byte-identical to the follow-up's. The Y images
differ only in the junction core: by at most one colour level in the plan views, and in the
oblique views in the pattern where the unclipped terrain shows through the core (`ROAD-68-D`).
Each Y is drawn stem first, so its node is a bend, now with a crossing, before the branches
arrive.

#### Edit-order independence (pre-D)

**Status:** done 2026-10-24 for [open question 15](#road-68-open-questions) on flat ground
(slopes: open question 16) and for the D4 taper ([open question 9](#road-68-open-questions)).

**The rule.** Physical profiles, clips and pieces derive from the final topology and authored
state only (invariant 6):

- **Profile solve** (`graph/rebuild/junction_profiles.rs`). `finalize_junction_endpoint_profiles`
  takes only the affected nodes. For every road edge touching them it derives the physical
  profile from the control polyline: a knot at each pinned end's mouth (the clip), that end's
  2 m supports where it needs a vertical curve, heights on the end's plane up to the mouth and
  eased back over the blend zone. Pins at both ends are re-derived, the far end's from the same
  plane solve of its arms. Control polylines are never rewritten, and the edit-role inputs
  (adaptable and authored edges, stable incidents, the previous-plane path, the authored-edge
  ledger in `TransitNetwork`) are gone.
- **Corridor authority.** Ranked by geometry only: a corridor whose plane every other arm
  already meets within the 5 cm support tolerance comes first (a fall-line road crossed by a
  contour road keeps its grade, as the existing-road bonus achieved before), then the
  straightest and flattest pair, then incident order. A junction's other arms take supports.
- **Layout input** (`roadbed/arms.rs`). Arm knots are the control polyline's XZ, as the
  [junction layout](#junction-layout) contract says, not the physical profile, so clips never
  read the knots the solve inserts. Heights and span stations come from the physical profile at
  the same horizontal station. Node kinds (`incident_direction`) and the pass-through crossing
  fit use the control polyline too.
- **Stroke preparation** (`road_edit/input.rs`). A stroke pins only to visible carriageways, never
  to raised sidewalks, curbs or caps ([open question 16](#road-68-open-questions) covers the
  remaining carriageway pins on slopes).

**Tests.** `nodes::sim::core::tests::road_edit_order`: the reference T, 2+2 T and Y (25°, 40°,
60°) fixtures, committed in two to four orders each through `SimCore::commit_road`; graph control
and physical profiles, clips, node-piece and span triangles must be bitwise equal. Flat: passes
(it failed before on every fixture). Sloped: ignored on question 16. The finalizer tests in
`graph/rebuild/tests.rs` now assert unchanged control polylines and supports in the physical
profile. `ROAD-68` ignores: D 26 (the sloped order test), E 2, F 5.

**D4 taper** (`roadbed/layout.rs`, `node.rs`, `walkways.rs`; [junction layout](#junction-layout)).
- `SectorBoundary::taper` (`BoundaryTaper`) records where a straight sector's boundary reaches
  its arms' own offsets; `run_offset_m`, `run_point` and `for_each_run_station` (the arm's knots
  plus the taper station) place every run, so the core loop (`CoreVertexSource::TaperEnd`), node
  strips, curb faces, outer chains and walkway corner routes share its vertices.
- Mouths reach at least the taper station; the span's own taper stays for pass-through nodes.
- Straight sectors wider than `pi` now miter on the bisector too (first offset segment extended
  behind the node), which removes the rounding-sized arc of sliver strips the plain T had.
- Tests: `a_straight_width_change_tapers_along_both_approaches`,
  `a_width_change_taper_that_cannot_fit_is_rejected` (layout) and
  `width_changes_leave_no_slivers_stray_curb_faces_or_chain_steps` (every node fixture, flat and
  sloped: no top triangle thinner than 1 mm, curb-face edges bordering raised and ground bands,
  chain edges bordering one top triangle). Mutation check: without the taper the node test fails
  on `width_step`.

**Consequences.** Every edge at an affected node re-derives its profile, so an existing arm takes
its mouth knot the first time its node is solved, and removing a branch returns the remaining arms
to their authored grade. The crossing toggle now re-pins its node (it passed no adaptable edges
before, so the solve did nothing). Open question 7's failure cannot occur: both ends' mouth knots
are re-derived on every solve.

**Checks** (fresh, final pre-D sources):

- Debug library suite (`RAYON_NUM_THREADS=4 METRUM_DEBUG=0`): 1,815 passed, 0 failed, 186 ignored
  (follow-up 2: 1,812, 0, 185). `ROAD-68` ignores: D 26, E 2, F 5.
- Clippy (`--all-targets`) reports only the 59 legacy-tree warnings, and `cargo doc` reports no
  missing docs.
- `roadbed/` holds about 6,690 production lines (follow-up 2: 6,479), under the 8,000 cap.
- `./run.sh --test` (isolated profile, no display) exits 0: the library suite as above,
  `tests/growth_scenario.rs` with 4 passed, and the Godot bridge suites with the same PASS lines
  as follow-up 2.
- Screenshots ([reference harness](#rendered-reference-screenshots), debug build, 54 images, no
  fixture failures; new fixture `y_25_branches_first`, the 25° Y drawn branch, branch, stem): on
  flat ground all three views are byte-identical to `y_25`'s; on the slope they differ (plan: at
  most 4 colour levels; oblique: the unclipped-terrain pattern), by open question 16. Against
  follow-up 2 the T, four-way, bend and mixed T change by at most one colour level in under 70
  pixels; the sloped Y cores shade differently, from the geometric corridor choice.

**Benchmarks** (2026-10-24): release, i9-12900K pinned to the P-cores, `RAYON_NUM_THREADS=4`,
Criterion defaults with 200 samples, under the exclusive timing lock with no build running. The
control is the `8712b69f` binary in the same session; rounds alternate control and new.

| Case | Control, two rounds | Pre-D, two rounds | Target |
| --- | --- | --- | --- |
| `roadbed_junction/build/t` | 28.10 / 28.10 µs | 27.81 / 27.72 µs | — |
| `roadbed_junction/build/four_way` | 46.28 / 46.37 µs | 46.56 / 47.52 µs | 100 µs |
| `roadbed_junction/build/six_way` | 80.40 / 79.85 µs | 81.27 / 82.17 µs | 300 µs |
| `roadbed_junction/publish/t_closure` | 91.94 / 94.63 µs | 94.44 / 97.20 µs | — |

The fixtures have no width change, so the taper adds no work there. Unprofiled, the 1–3% growth
on the four- and six-way is most likely the arms' second walk (control knots, then the physical
profile for heights and span stations); the T gains from its straight side's single miter point.
The profile solve is not in these cases.

```bash
cargo bench --bench roadbed_benchmark --no-run
RAYON_NUM_THREADS=4 METRUM_DEBUG=0 taskset -c 0,2,4,6,8,10,12,14 $BENCH_BINARY --bench --noplot roadbed_junction
```

#### Footpath junctions and the bend crossing fallback (pre-D)

**Status:** done 2026-10-24 ([open questions 13 and 14](#road-68-open-questions), user decisions;
new question 17 records the remaining choices). Branch `road68-pre1`.

**Footpath junctions** ([D5](#product-decisions), [junction layout](#junction-layout)):

- `roadbed/arms.rs`: `NodeArms::collect` takes footpath arms. At a node with a road arm they are
  attached (`ArmInput::attached`); the D8 crossing rule and its degree-two case count road arms
  only, and a pass-through pair must be of one kind, so a footpath at a straight road or at a
  road end gives the node a layout. `level_at_node`: an attached arm's band stands at sidewalk
  height in the node.
- `roadbed/layout.rs`: `JunctionLayout::attachments` (`Attachment`, `AttachedEdge`,
  `BoundaryPoint`); edge points on arm-side runs (`attach_to_runs`, before mouths) or inserted
  into the sector's samples (`sample_sectors`); the attached mouth and the ribbon checks
  (`finish_attachments`); `side_run`, `outer_path`, `attached_run` and `ribbon`, the one
  construction of the runs, outer paths and ribbon outlines that node pieces and checks share.
  `LayoutError::FootpathDoesNotJoin`.
- `roadbed/node.rs`: mouth sections of attached arms at `P + raised`; outer chains cut at each
  attached arm; ribbons triangulated as the core is (`triangulate_ring`, band `Footpath`);
  a footpath-only core is band `Footpath`. `terminal_plane` is the one road-end plane, which the
  profile solve pins a road end's footpaths to.
- `graph/rebuild/junction_profiles.rs` (on the [edit-order independent](#edit-order-independence-pre-d)
  solve): footpaths are derived edges too. A node's plane is its roads' (or its footpaths' where
  only footpaths meet); an attached footpath's pin is that plane raised by `CURB_STEP_HEIGHT_M`, up
  to its mouth; at a road end it is `roadbed::terminal_plane` raised, taken after the road's own
  profile is derived. A far end joining two road or footpath edges halves a pin's blend.
  `graph/rebuild/clips.rs`: footpath clips are layout mouths, an attached footpath's at any road
  node.
- Lanes: footpath lanes end at their layout mouths; attached footpaths join their sector's corner
  route where their centreline meets it; dead ends keep a turn-back connector
  ([Pedestrian Junction Crossings](#pedestrian-junction-crossings)).

**Bend crossing fallback** (question 13): `NodeArms::layout` lays the node out again with the
default crossing on the other arm when the first layout fails and the other arm may cross by
default; the node fails with the first arm's error only when both fail. Pieces, clips and lanes all
lay out through it.

**Tests** (new): `roadbed::node::tests::footpaths` on `roadbed::fixtures::footpath_fixtures`,
flat and at 2.5%: a footpath in a T's corner and beside its arm, at a road end straight on and to
the side, across a straight road, on both sides of a bend, and footpath-only nodes of degree one,
two and three. Pieces are watertight with one extra chain per attached footpath, heights are
`P + level` bitwise, attached mouths and edge points stand at `P + raised`, ribbons are footpath
between boundary and mouth, insertion order does not change the hub, and an edit rebuilds its
closure. `lanes::tests::footpaths::footpath_junction_lanes_end_at_their_mouths_and_join_the_corner_walks`
(lanes end at the mouths bitwise, every arriving foot lane reaches every leaving one, joins stay
off the carriageway); `walking_proof_holds_at_footpath_junctions` (planner);
`a_bend_crossing_that_does_not_fit_on_its_first_arm_moves_to_the_other_arm`,
`a_bend_that_holds_its_crossing_on_neither_arm_is_rejected` and
`a_bend_whose_crossing_moved_to_its_second_arm_loads` (save). Re-enabled:
`headlight_house_junction_full_load_compiles_with_terrain_earthwork` (`ROAD-68-F`), which now
expects the load repair to re-solve its legacy profiles (question 17). Updated for D5: the three
legacy footpath lane tests (lanes end at the layout mouth; cars cross a straight road with a
footpath on one junction connector) and `test_dedicated_footpath_centering` (dead-end turn-back
connectors). `ROAD-68` ignores: D 26, E 2, F 4. After the question 16 decision the sloped
edit-order test moved to `ROAD-68-E`: D 25, E 3, F 4.

**Checks** (fresh, on the final sources, rebased onto the [edit-order independent
solve](#edit-order-independence-pre-d) at `03b9f4ed`):

- Debug library suite (`RAYON_NUM_THREADS=4 METRUM_DEBUG=0`): 1,826 passed, 0 failed, 186 ignored
  (`03b9f4ed`: 1,815, 0, 186): ten new tests and one re-enabled; the one new ignored test is the
  perf-city proof, which needs the prepared snapshots.
- Clippy (`--all-targets`) reports only the 59 legacy-tree warnings, and `cargo doc` reports no
  missing docs.
- `./run.sh --test` (isolated profile, no display) exits 0: the library suite as above,
  `tests/growth_scenario.rs` with 4 passed, and the Godot bridge suites with the same PASS lines
  as `03b9f4ed` (no script errors, `zoning_reference_test` 8 layouts, the same four scripts
  skipped).
- `roadbed/` holds 7,388 production lines (`03b9f4ed`: 6,703 by the same count, which reads 6,493
  at follow-up 2), under the 8,000 cap: about 690 for the attachments, ribbons, chains and joins.
- Screenshots ([reference harness](#rendered-reference-screenshots), 54 images, no fixture
  failures; none has a footpath): 32 byte-identical to `03b9f4ed`'s, and 22, mostly plan views,
  differ by one colour level in at most 67 pixels. The roadbed fixtures' node pieces, spans and
  lanes are bitwise unchanged (checked on the T before the rebase). The difference is the asset
  import, not the code ([pre-D review fixes](#pre-d-review-fixes)): the two runs used different
  worktrees, each importing afresh.

**Benchmarks** (2026-10-24, `roadbed_benchmark`, release, i9-12900K pinned to the P-cores,
`RAYON_NUM_THREADS=4`, Criterion defaults with 200 samples, exclusive timing lock, no build
running; the `8712b69f` binary as control, run A B B A on the integrated pre-D sources):

| Case | Control (`8712b69f`) | Pre-D with footpath junctions | Target |
| --- | --- | --- | --- |
| `roadbed_junction/build/t` | 28.12 / 28.08 µs | 28.92 / 28.89 µs | — |
| `roadbed_junction/build/four_way` | 47.20 / 45.89 µs | 47.81 / 47.16 µs | 100 µs |
| `roadbed_junction/build/six_way` | 80.24 / 79.87 µs | 81.78 / 81.54 µs | 300 µs |
| `roadbed_junction/build/t_footpath` | — | 40.36 / 39.95 µs | — |
| `roadbed_junction/publish/t_closure` | 93.90 / 94.28 µs | 92.03 / 96.04 µs | — |

Road junctions cost up to 3% more, not profiled; the likely costs are the outer-path walk that
each sector's chains now take and the arm inputs each build collects. Before the rebase the same
A B B A order gave the same picture (six-way 80.40 / 80.01 against 83.11 / 82.18 µs). A footpath
adds about 12 µs to a T: its run search, the ribbon outline check and the ribbon CDT.

```bash
cargo bench --bench roadbed_benchmark --no-run
RAYON_NUM_THREADS=4 METRUM_DEBUG=0 taskset -c 0,2,4,6,8,10,12,14 $BENCH_BINARY --bench --noplot roadbed_junction
```

**Economy performance contract, matched A/B** ([`PERF-01`](economy.md#performance-contract-perf-01perf-05),
[open question 14](#road-68-open-questions)). Release library tests of `8712b69f` (control, binary
`45add457…`) and of the integrated pre-D sources (`544448d2…`: `03b9f4ed` with this change), on
one set of snapshots generated once by the control and pinned: every run reported
`generation_s: null` for all four fixtures, and the snapshot files' SHA-256 sums were identical
after every run. Runs alternated A B B A, pinned to the P-cores with eight workers under the
exclusive timing lock, no build running:

```bash
RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14 \
  python3 tools/perf_contract.py --binary "$TEST_BINARY" --artifacts "$ARTIFACTS/<run>"   # <run>/snapshots -> one shared directory
METRUM_PERF_RESIDENTS=<tier> METRUM_PERF_CACHE="$ARTIFACTS/snapshots" taskset -c 0,2,4,6,8,10,12,14 "$TEST_BINARY" \
  nodes::sim::core::populated_city::walking_proof::perf_contract_city_walking_proof_holds --exact --ignored --nocapture
```

| Hour mean | Control A1 | Pre-D B1 | Pre-D B2 | Control A2 |
| --- | --- | --- | --- | --- |
| baseline 10k | 0.83 ms | 0.86 ms | 0.90 ms | 0.84 ms |
| baseline 30k | 4.09 ms | 4.44 ms | 4.25 ms | 4.19 ms |
| fiscal 10k | 6.54 ms | 6.69 ms | 6.57 ms | 6.47 ms |
| fiscal 30k | 28.56 ms | 28.42 ms | 28.33 ms | 28.37 ms |
| runner time | 108.8 s | 108.8 s | 108.7 s | 108.8 s |

Work counts are identical in all four runs, and so are the final cities (5,311 and 15,180
households). The earlier session on the pre-rebase sources, with the same snapshots, gave the
same work counts and control 4.57 / 4.15 against 4.37 / 4.42 ms at baseline 30k: the spread
between runs is larger than any difference between builds. No time limit fails. The same seven
work-count limits fail in control and change alike: baseline `buyer_sites_built`,
`staffing_trials`, `supplier_searches`, `supplier_routes` and `job_search_keys`, and fiscal-stress
`buyer_sites_built` and `cch_searches`. All isolated phases, both locality checks and the agent
checks pass. The walking proof holds on both tiers, with no footpath in either city.

#### Pre-D review fixes

**Status:** done 2026-10-24, on the static reviews of the two pre-D branches. Branch
`road68-fix`. Each fix has a test that fails on `c59955e6`, the pre-D head, unless noted.

**Fixes:**

- **Wrap-sector footpaths** (`layout.rs`). Attached arms were kept in angle order, but a sector
  through angle zero (a road end's cap, the last sector of any node) holds them out of that
  order along its outer path, so two valid footpaths there failed `FootpathDoesNotJoin` and the
  outer chains were not cut. They are now ordered by sector and their position on its outer path,
  and ribbons that touch fail. Fixture `road_end_both_sides`: a road end at 60° with footpaths at
  30° and 90°.
- **Zebra at the corner** ([open question 17](#road-68-open-questions), user decision). An
  attached footpath moves only its arm's mouth; the crosswalk comes from the boundaries alone.
  Test `a_footpath_beside_an_arm_moves_its_mouth_but_not_its_zebra`: every arm's crosswalk equals
  the one without the footpath, bitwise, and the met arm's mouth lies 1 m past the meeting point.
  Footpath edges attach to a run only at or beyond its reach, where it lies on its arm's own
  offset curve (it searched at the final offset but took the tapered point before).
- **Footpath joins** (`walkways.rs`, `pedestrian_junctions.rs`). With the zebra at the corner a
  footpath beside a crossable arm can meet the sidewalk between the crossing line and the mouth,
  where no corner route ran, and its lanes were left unconnected without an error. A footpath now
  joins the sector's walk, the corner route extended to both mouths, which its centreline meets by
  construction, and walks in to the sidewalk end. One crossing primitive (the layout's
  `segment_crossing`) and one join-route builder serve layout and legacy nodes. Fixture
  `t_beside_arm_shallow` (20° off the arm): with the route alone it fails the lane and walking-proof
  tests.
- **Bend crossing fallback** ([open question 13](#road-68-open-questions)): only `ArmTooShort` or
  `CornerDoesNotFit` on the first arm moves the default crossing, and `NodeArms` records the move
  (`layout` takes `&mut self`), so `input(i).crossable` agrees with the layout. Test
  `a_moved_bend_crossing_is_the_arms_crossing`.
- **Profile solve** (`junction_profiles.rs`; [Heights](#heights)). Arm directions come from
  `roadbed::incident_direction` in `f64`, the function node layouts and span ends use, and the
  pass-through rule is the layout's `directions_pass_through`; the `f32` first-segment angle,
  which put arms starting on a repeated point at angle zero, and `edge_endpoint_direction_xz` are
  gone. Corridors rank by the lexicographic rule in Heights instead of `-dot - 2 grade`. An arm too
  short for its sample fails the node (`NodeError::NoPlane`, through `check_node`) instead of
  dropping out of the plane; the bend, authority and least-squares solves share one sample pass.
  Test `corridor_ranking_does_not_depend_on_edge_creation_order` (a mirrored Y on a slope whose
  arms start on a repeated point, built in two edge orders).
- **Freshness.** Node moves and road removals rebuilt clips but never re-ran the profile solve,
  so the arms they reach kept pins from the old plane. Both now solve their nodes; test
  `node_moves_and_removals_leave_freshly_solved_profiles` compares every profile with a solve from
  scratch, flat and sloped.
- **No silent fallbacks** (`layout.rs`, `node.rs`). `bisector_station` returns `None`, a typed
  `ArmTooShort`, instead of station zero where an arm's first segment would not move away from the
  bisector; a straight sector rules that out, since its bisector lies within half a degree of the
  station-zero lateral line. `run_offset_m` no longer clamps a station before its run (a debug
  assertion; no caller passes one). A mouth vertex the mouth section does not hold is
  `NodeError::MouthVertex` (`junction_layout_invalid`) instead of a fresh vertex.
  `SectorBoundary::run_start` is the one side-to-station mapping. In a straight sector every boundary has equal offsets by construction,
  so `is_shared_miter` no longer compares floats.

**Rejected or kept, with reasons:**

- The taper trigger compares the arms' band offsets exactly. They are the profile's own `f32` band
  edges widened to `f64` without arithmetic, and the band rule computes each from the edge's width
  and band kinds alone, so equal nominal widths give equal offsets bitwise and a different band
  composition is a `BandMismatch`. A tolerance would leave a step where widths differ by less.
  A band-only difference (equal carriageways) tapers over the 12 m minimum, as documented.
- A debug assertion that every edge whose control changed has both ends in the solve's nodes would
  be true by construction in `finalize_road_geometry` (both ends of each dirty edge go in); the
  node-move and removal callers assert instead that every edge the solve changes is in their undo
  record.
- The road-end plane still lays its node out once more for the solve (a fourth layout of that
  node); bounded by the affected set. Small per-node allocations (`roads`, the attached partition,
  the arm inputs per piece), the `UNPLACED` sentinel and samples inserted at a segment end are
  unchanged.
- `NoPlane` from the solve is a placement rejection (`check_node` runs in validation only, so the
  compile path pays no second solve). A load does not run it: a save holding such a node, which
  placement cannot produce, loads with that node's arms unpinned, as before. The load contract
  ([Preview, commit, undo and load](#preview-commit-undo-and-load)) wants it to fail; that belongs
  with the load path of `ROAD-68-E`.
- The sloped edit-order test stays ignored on [open question 16](#road-68-open-questions)
  (`ROAD-68-E`). The flat test now commits the Y in all six orders, a cross whose second stroke
  splits the first between its knots, and a through road with two side roads 50 m apart, whose
  link edge is pinned at both ends. These, and the node fixtures `width_step_curved` (the width
  step with its wide arm curving away, knotted every 2 m, flat and sloped) and `straight_convex`
  (equal widths over a straight sector just wider than a half turn), pass on `c59955e6` too: they
  close the coverage gaps the review named.

**Screenshot differences explained.** Same-build renders in different worktrees differed by one
colour level in tens of pixels, which the [reference harness](#rendered-reference-screenshots)
records as fixture changes. A probe that commits the reference T (flat and sloped) and Y through
`SimCore::commit_road` and dumps graph profiles, clips, costs, node and span triangles, outer
chains and lanes found `03b9f4ed` and `c59955e6` bitwise identical. The cause is the asset import:
`screenshots-C.sh` imports into a throwaway profile whenever a worktree has no `.godot` cache, and
two fresh imports of the same tree and build differ in 44 of 54 images (at most 23 pixels, one
level each). Renders compare only within one import; this gate's base and fix share one.

**Checks** (fresh, final sources):

- Debug library suite (`RAYON_NUM_THREADS=4 METRUM_DEBUG=0`, inside `./run.sh --test`): 1,830
  passed, 0 failed, 186 ignored (`c59955e6`: 1,826, 0, 186): four new tests; the new fixtures run
  inside existing tests. No test was disabled or re-enabled. `tests/growth_scenario.rs`: 4 passed.
- `./run.sh --test` (isolated profile, no display, every Godot run under a 600 s timeout) exits 0
  with the Godot PASS lines of the pre-D run, no script errors and the same four scripts skipped
  (`network_tool_chunk_renderer_test`, `road_junction_preview_test`, `road_preview_stream_test`
  until `ROAD-68-E`, `zoning_cells_tool_test` until `ROAD-68-F`). `vehicle_ground_support_test`
  passes with moved road-height metrics (tilted poses 2,472 → 2,471, largest centre lift 0.4236 →
  0.4144 m), not analysed further. A worktree needs its LFS content (`git lfs checkout`): with
  pointer files that test fails on its vehicle models and headless Godot does not exit.
- Clippy (`--all-targets`): the 59 legacy warnings only (one in `nodes/asset_document.rs`, 58 in
  `economy/agents/data.rs`, all `redundant_field_names`), no missing docs.
- `roadbed/` production lines: 7,505 by the counting used here, which reads 7,377 for
  `c59955e6` (+128: the sector walk 51, layout 30, arms 26, typed mouth vertices 22). Outside
  `roadbed/`, the profile solve lost 60 lines and `clips.rs` 32.
- Screenshots ([reference harness](#rendered-reference-screenshots), new fixture
  `t_footpath_beside_arm`: a footpath 30° off the T's side road; 60 images, no failures), base and
  fix rendered in one worktree with one import: the 54 images of the earlier fixtures are
  byte-identical, and the footpath fixture's six show the side road's zebra back at the corner
  with the node piece running on to the mouth.

**Benchmarks** (2026-10-24, `roadbed_benchmark`, release, i9-12900K pinned to the P-cores,
`RAYON_NUM_THREADS=4`, Criterion defaults, exclusive timing lock, no build running; control: the
pre-D binary of `8593e166`, whose production code `c59955e6` keeps; rounds A B B A, medians):

| Case | Control | Review fixes |
| --- | --- | --- |
| `roadbed_junction/build/t` | 28.41 / 28.55 µs | 28.91 / 29.01 µs |
| `roadbed_junction/build/four_way` | 47.06 / 47.12 µs | 47.19 / 47.47 µs |
| `roadbed_junction/build/six_way` | 80.92 / 81.35 µs | 80.99 / 80.76 µs |
| `roadbed_junction/build/t_footpath` | 39.94 / 40.26 µs | 40.39 / 40.08 µs |
| `roadbed_junction/publish/t_closure` | 93.35 / 94.34 µs | 99.40 / 96.75 µs |

Node builds are within 2%; the T's 0.5 µs is unprofiled. `t_closure`, the noisiest case (earlier
sessions: 91.9–96.0 µs on unchanged code), reads 2.5–5% higher. The validation-only solve check
and the sector walks of the lane rebuild are not in these cases.

```bash
cargo bench --bench roadbed_benchmark --no-run
RAYON_NUM_THREADS=4 METRUM_DEBUG=0 taskset -c 0,2,4,6,8,10,12,14 $BENCH_BINARY --bench --noplot roadbed_junction
```

#### Roadbed compaction (before E)

**Status:** done 2026-10-24 (user decision: compact before `ROAD-68-E`; the line limit was
dropped the same day, so cuts were kept only where they improve the code). Branch
`road68-compact`, on `road`. No behaviour change: every published product is bitwise equal to
`79e7a476`'s.

**Product probe.** `roadbed::fixtures::ProductProbe` hashes (FNV-1a over exact bits) what the
roadbed publishes, per fixture and component:

- graph clips and physical profiles
- node pieces: positions, road-space coordinates, grouped triangles, outer chains, bounds, query
  grids, mouth sections, crosswalk kerbs and approaches, and node errors (by variant and values)
- layouts, field by field
- span pieces: meshes, sections, mouths and crossing strips
- node walkways: sidewalk and footpath ends, crossings, corner routes and footpath joins
- lanes, with their crosswalks and successors
- surface queries at every fifth top triangle: heights, bands, vertical and oblique rays, discs,
  lane queries, the generic ground grid and the core triangulation

Two tests compare these with values recorded at `79e7a476`, flat and sloped.
`roadbed_fixture_products_match_the_probe` covers every node, footpath, span, pass-through and
short-link fixture of the roadbed tests (93 fixtures). `reference_junction_products_match_the_probe`
covers the rendered reference junctions committed stroke by stroke through
`SimCore::commit_road` (20). Both run in under a second. A failure prints the actual table, so a
gate that changes products on purpose re-records it.

**Line count** (production lines: every line of each non-test file, `#[cfg(test)]` items,
`tests/` and `fixtures.rs` excluded; this counter reads 7,545 for `79e7a476`, where the pre-D fix
count read 7,505 and gate D's 7,542):

| File | `79e7a476` | Compacted |
| --- | --- | --- |
| `layout.rs` | 1,950 | 1,885 |
| `geometry.rs` | 560 | 460 |
| `arms.rs` | 532 | 492 |
| `walkways.rs` | 531 | 517 |
| `system.rs` | 476 | 467 |
| `mesh.rs` | 457 | 420 |
| `query.rs` | 328 | 299 |
| `profile.rs` | 224 | 202 |
| Untouched (`node.rs`, `span*`, `index.rs`, `section.rs`, `roadbed.rs`) | 2,487 | 2,487 |
| `roadbed/` | 7,545 | 7,229 (−316) |

That is −321 code lines, −9 blank lines and +14 comment lines; no comment that explains geometry,
orientation or a rule was dropped, and comments were rewritten only where the code they
described was merged or removed.

**Cuts by kind** (code lines, about):

- **Duplication (about −225).** `geometry.rs`: one clamped-cell function for building grids,
  point lookups and bounds walks; one XZ bounds function, which mesh bounds use too; the ray slab
  test as one loop over two axes. `mesh.rs`: span and node meshes share one assembly
  (`PieceMesh::new`, group starts, grid and bounds); left-facing curb faces are the right-facing
  pair mirrored. `layout.rs`: glam's `lerp` (the same `a * (1 - t) + b * t`); `PathPoint::at`;
  `run_meetings`, the one scan of the attached edges that meet an arm side, for mouths and runs;
  attached runs through `for_each_run_station`; one arm-input lookup in `outer_path`; one crosswalk
  limit in `straight_taper_m`. `arms.rs`: one band constructor for both sides, each arm's two
  sides as one array, one centreline position. `system.rs`: one accessor for a published piece's
  mesh and footprint; `shared_mouth`, the one test for an edge whose two mouths coincide.
  `query.rs`: discs go through `any_top_triangle_in_bounds`; rays walk their cells without a
  vertical special case (`walk_ray_cells` handles a zero XZ direction). `profile.rs`: band
  offsets computed once and mirrored; one sidewalk-or-shoulder choice. `walkways.rs`: one
  end-point function for sidewalk and footpath ends; one step length in the route heights.
- **Needless parameters and indirection (about −60).** The layout's build steps fill the layout
  under construction instead of taking five to seven of its fields each; single-caller helpers
  inlined: the constrained-region triangulation with its two closure parameters, the barycentric
  weights, `is_shared_miter`, `push_road_space_row`, `accepts`.
- **Dead code (about −20).** The unread `Attachment::angle_rad`; the derivable
  `SectorLayout::samples_per_boundary`; the row layout's unread slot index; a ring-length check
  no caller can fail; a failure-count comparison the failure flag already covers; a per-piece
  bounds test in point queries that the piece grid's own padded test repeats; `band_at`, used
  only by tests, is test-only; an alias in the degree-two crossing rule.
- **Cosmetic (about −15).** Ray conversions through `from_array`, the degree-two order as one
  comparison, `CurbStep` by `Default`, the sector walk's two arms by name.

Rewrites made for the count and reverted as no clearer: `LayoutError` as tuple variants, the
`Bounds` type as closures, and a few loop-to-iterator and formatting-driven rewrites.

**Follow-ups at the boundary** (not done here; `node.rs` and `span/` belong to gate D part 2):

- `node.rs` converts to `robust::Coord` in `top` and `fan_core` where `layout.rs` has `orient`.
- `node.rs`: `mouth_section` and `section_at` both build the arm's lateral sign and offset-point
  closure.
- `mesh.rs` keeps `push_section_row` only for `span/model.rs`, which could call
  `push_section_row_at` directly.

**Checks** (fresh, final sources rebased on `road` `4718ac12`):

- Probe: both tests pass; all seven components of all 113 entries equal the values recorded at
  `79e7a476` (built in a worktree at the probe commit, whose production code is `79e7a476`'s).
  The probe was green after every intermediate step as well.
- Debug library suite (`RAYON_NUM_THREADS=4 METRUM_DEBUG=0`, inside `./run.sh --test`): 1,839
  passed, 0 failed, 188 ignored; the two probes are the only new tests, none was disabled or
  re-enabled. `tests/growth_scenario.rs`: 4 passed.
- `./run.sh --test` (isolated profile, no display, every Godot run under a 600 s timeout) exits 0
  with no script errors; it still skips `network_tool_chunk_renderer_test`,
  `road_junction_preview_test`, `road_preview_stream_test` (until `ROAD-68-E`) and
  `zoning_cells_tool_test` (until `ROAD-68-F`). `vehicle_ground_support_test` reports the pre-D
  metrics (2,471 tilted poses, largest centre lift 0.4144 m).
- Clippy (`--all-targets`): the 59 legacy warnings only, the same set as before; no missing docs.

**Benchmarks** (2026-10-24, `roadbed_benchmark`, release, i9-12900K pinned to the P-cores,
`RAYON_NUM_THREADS=4`, Criterion defaults, exclusive timing lock, no build running; control: the
benchmark binary of `79e7a476`'s production code, built this session; rounds A B B A, medians):

| Case | Control | Compacted |
| --- | --- | --- |
| `roadbed_junction/build/t` | 29.05 / 29.04 µs | 24.41 / 24.34 µs |
| `roadbed_junction/build/four_way` | 47.43 / 47.66 µs | 40.41 / 39.35 µs |
| `roadbed_junction/build/six_way` | 81.07 / 82.85 µs | 67.67 / 66.37 µs |
| `roadbed_junction/build/t_footpath` | 40.62 / 40.76 µs | 36.51 / 35.93 µs |
| `roadbed_junction/publish/t_closure` | 98.36 / 95.29 µs | 71.90 / 72.02 µs |

Every case is faster, by 10–27%, well beyond the run-to-run spread; the control matches the
pre-D record. The probe shows the products are unchanged and the benchmark source is the same;
the gain is not attributed yet (the grid build and mesh assembly paths changed most). The
candidate also carries gate G, which does not touch `roadbed/`.

```bash
cargo bench --bench roadbed_benchmark --no-run
RAYON_NUM_THREADS=4 METRUM_DEBUG=0 taskset -c 0,2,4,6,8,10,12,14 $BENCH_BINARY --bench --noplot roadbed_junction
```

### `ROAD-68-D` — footprint and terrain

**Terrain cut superseded (user, 2026-10-25).** The exact CDT cut below landed and is correct, but
it misses the 3.0 ms terrain stage at city density ([open question 29](#road-68-open-questions)).
The terrain surface core is to be rewritten as [`TERRAIN-07`](roadmap.md) (heightmap plus fine
grading and mask layers, road skirts); the legacy terrain surface code is retired with it. The
roadbed side of this gate (outer chains, terrain changes, heights) stays as `TERRAIN-07`'s input.
The terrain side's specification, gates and open questions are in
[`terrain.md`, "Graded terrain surface (`TERRAIN-07`)"](terrain.md#graded-terrain-surface-terrain-07) (draft for review, 2026-10-25); the
contract it consumes is listed under [footprint and terrain](#footprint-and-terrain).

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

#### Progress (2026-10-24, branch `road68-D`, work in progress)

Landed (see [footprint and terrain](#footprint-and-terrain) for the contract):

- `roadbed/terrain.rs`: `terrain_loops(roadbed, graph, rect)` builds a rectangle's loops from the
  outer chains, with `ChainSegment` provenance per edge, orientation-derived holes and the typed
  `TerrainLoopError::OpenEnd`. Span chains now run with the road on the left like node chains
  (they ran the other way); `RoadbedSystem::take_changed_bounds` records replaced pieces' bounds.
- `terrain/cdt/model.rs`: the legacy span, node and handoff sources and their seven supporting
  types are gone; `RoadBoundary { piece, chain, segment, support }` and `BuildingSiteBoundary`
  remain, `SyntheticTestBoundary` is `#[cfg(test)]`. Ordering is the derived `Ord`; the merge rule
  joins segments of one chain; fingerprints and the debug export follow, and the export sends
  only counts, kind codes, primary ids and the Rust label, which `terrain.gd` prints
  (`TERRAIN_CDT_CONTRACT_REVISION` 15). The module-level `dead_code` allow in `terrain/cdt.rs` is
  removed; the eight legacy face-ownership helpers it hid are deleted, with the two tests that
  exercised only them.
- Face classification uses containment parity within a footprint group.
- `terrain/grading.rs`: the shared grading envelope (open questions 3 and 18).
- Per-tile road contributors in the full-patch and local-tile plans, per-tile ownership, the
  tile-keyed ledger and patch selection from changed piece bounds (open question 19 for open
  ends).

Part 2 (2026-10-24, same branch):

- **Seams on tile sides.** The starter town's cross street at `x = 0` failed 46 `scenario` tests
  with `terrain_cdt_pathological_output`: its terminal cap started at `x = -4.999999999999999`,
  one ulp inside the sidewalk line `x = -5`, which is also a render patch side, and the CDT built a
  vertical sliver between the cap and the side. Two constructive fixes: arm knot laterals divide by
  the segment length instead of multiplying by its reciprocal (`249 * (1 / 249)` rounds below one),
  so an axis-aligned arm's runs and cap ends lie exactly on the span's line; and the CDT treats a
  seam lying exactly on a tile side as that side in both tiles
  ([footprint and terrain](#footprint-and-terrain)), with the test
  `a_seam_along_a_tile_side_is_that_side_in_both_tiles`.
- **Recorded fixtures.** `kuopio_recorded_profiles_and_terrain_preserve_quality_limits` fails
  node 5 (`node_piece_failed`) at `79e7a476` too, and the hillside fixture's node 2 fails
  `MouthOffPlane` on its legacy clips (recomputing them as an edit does overlaps its recorded
  parcels). Both recordings predate `ROAD-68-C`; both are `ROAD-68-E` ignores (open question 8).
- **Grading in one pass.** `grading::road_loop_grading` probes each ray's reach once for the
  margin and the guides, and a tile emits only the rings of rays whose reach meets it. The
  generic rectangle clip and join moved from `roadbed/terrain.rs` to `terrain/rect_loops.rs`.
- **Economy perf contract: regression (fixed in part 3).** The economy work is unchanged (every work
  count equal, hourly cost equal or lower), but snapshot loads, city generation and building churn
  are slower: against `79e7a476` (A) and the D tree before `ROAD-68-G` (B), and again against
  `road` at `98c2f8cb` (A) and D rebased on it (B), the 10k load takes 3.85 s instead of 2.85 s
  and the 30k load 10.5 s instead of 7.6 s, generation 29 s instead of 27 s (10k) and 174 s
  instead of 95 s (30k), fiscal generation 40 s instead of 15 s (10k) and 158 s instead of 43 s
  (30k), and the fiscal-stress day 13.4 s instead of 4.0 s (10k) and 42 s instead of 11 s (30k).
  Reusing each patch's road margin until roads near it change did not move these numbers, so it
  was reverted. A flat profile of the fiscal-stress day puts half its time in
  `grading::road_loop_grading` (47%, with terrain sampling 4%). Part 3 found the callers with a
  call-graph profile; refined rebuilds were not one of them.
- **Product probe.** Both product-hash probes (`ProductProbe`, recorded at `79e7a476`) are
  re-recorded, every change attributed by rebuilding without the lateral change: with it
  reverted, all 93 roadbed fixtures and 20 reference junctions differ only in the span hash (the
  span outer chains now keep the road on their left); the lateral change alone changes nodes,
  layouts, spans, walkways and queries of 38 fixtures (`six_way`, `y_25`, `y_40`, `y_60`,
  `five_way`, `bend_10`/`30`/`150`/`160`, `mixed_40`, `width_bend`, `width_step_convex`,
  `width_step_curved`, `t_corner`, `t_beside_arm`, `t_beside_arm_shallow`, `through_road`,
  `path_t`, `road_end`, flat and sloped) and of the `y_25`, `y_25_branches_first` and `y_60`
  reference junctions, and never their graph or lanes.
- **Size.** `roadbed/` holds 7,611 lines (every line of `roadbed.rs` and `roadbed/**/*.rs`
  except tests and fixtures; 7,421 after the compaction, `e92359c8`, by the same count, which the
  compaction's own counter reads as 7,229): `ROAD-68-D` adds 190, the
  terrain loop entry point (`terrain.rs`, 162: chain gathering, joins at mouth corners, open-end
  errors and the provenance types) and the changed-bounds record (`system.rs`, 19). The generic
  rectangle clip and join live in `terrain/rect_loops.rs`, the owner of terrain geometry.
- **Open at part 2** (part 3 closes all but the yard test). The perf contract regression above;
  `./run.sh --test` and the rendered screenshots, not yet run on this tree. The 2 ms terrain stage (open question 22). The yard test
  `cars_and_walkers_enter_and_exit_on_compiled_yards`: building 1's samples lie in no road or
  site loop of their tile, yet the tile's CDT emits no face over them (a face dropped for crossing
  an unpreserved constraint is the next suspect); building 3's sample lies inside its own 6-vertex
  site loop, which its foundation paving does not cover. Two sub-millimetre roadbed artefacts
  remain for the CDT's 1 mm identity cell and 5 cm seam budget: a terminal cap's apex is a polar
  sample (`sin(pi)` is not zero), so on a tile line it lies 6e-16 m off it; and the span station
  rule (cadence rows from the profile range start, class rows from each knot) puts rows 3–6 mm
  apart before knots on slopes. The constructive rule is to subdivide each interval between fixed
  stations (knots, protected stations, cadence range ends) evenly at its step; it changes every
  span's rows, so it waits for its own change with the span tests.

Per-tile timing (2026-10-24, `benchmark_roadbed_terrain_tiles`, release test binaries,
`RAYON_NUM_THREADS=4 taskset -c 0,2,4,6,8,10,12,14`, both timing locks, A B B A in one session
against the first WIP binary): the sloped T over a 256 m patch plans 16 tiles and triangulates the
12 with road contributors.

| Build | Total p50 (ms) | Plan p50 (ms) | Tile CDT p50 (ms) | Slowest tile (ms) |
| --- | --- | --- | --- | --- |
| first WIP (control) | 2.924 / 2.899 | 1.385 / 1.401 | 0.335 / 0.333 | 1.200 / 1.132 |
| part 2 | 2.332 / 2.347 | 0.746 / 0.751 | 0.350 / 0.349 | 1.157 / 1.176 |

Part 2 missed the 2 ms target; part 3 meets it (below and open question 22).

Part 3 (2026-10-24, same branch, rebased on `road` at `27797b92`):

- **The perf contract regression: cause and fix.** A call-graph profile (`perf record --call-graph
  lbr`, the 10k fiscal-stress day) put the time under `mark_building_site_terrain_only_boxes` and
  the snapshot load, both through the patch ownership refresh into
  `terrain_cdt_road_tile_loops` and `road_loop_grading`, and inside the grading pass 30% of the
  whole run in `signed_area`. Three causes, three fixes, every output unchanged:
  - the grading pass computed its loop's orientation (`signed_area`, `O(V)`) once per edge, from
    both the edge rays and the vertex rays, so a loop cost `O(V^2)`; it is computed once per loop;
  - a building-site change re-graded the road loops of every tile of its patches, although a site
    moves no road and no height; it now refreshes only site ownership
    (`refresh_site_terrain_patch_ownership_for_keys`);
  - patch road margins graded one patch and one tile at a time; they grade in parallel, with an
    indexed collect and the order-free largest margin.
  Road ownership then follows the changes that move it ([ownership
  refresh](#footprint-and-terrain)): a commit's validation grades the edit's patches once, and the
  commit's later `refresh_road_locked_terrain_patch_state` refreshes only patches still pending,
  instead of every unacknowledged dirty patch (the 2.6 ms `ROAD-68-G` part 2 measured at 100k,
  open question 21, was that walk: real duplicate work, not only the harness). A terrain stroke
  marks the patches within twice the envelope cap of its brush pending and refreshes them when it
  ends; before, nothing refreshed road ownership after a stroke.
  Economy perf contract (`tools/perf_contract.py`, release test binaries, 8 workers, `taskset -c
  0,2,4,6,8,10,12,14`, both timing locks, A B B A in one session; A = `road` at `27797b92`, B =
  this tree): every work count identical, every timing within or below the control's range. The
  remaining failures are the seven work-count limits both builds fail (open question 20).

  | Metric (s, hourly ms) | A1 | B1 | B2 | A2 | part 2 B, earlier session |
  | --- | --- | --- | --- | --- | --- |
  | generation 10k / 30k | 25.05 / 90.89 | 25.34 / 89.96 | 25.03 / 89.73 | 25.42 / 90.59 | 29 / 174 |
  | fiscal generation 10k / 30k | 14.93 / 41.82 | 14.56 / 40.59 | 14.31 / 40.84 | 14.72 / 41.76 | 40 / 158 |
  | load 10k / 30k | 2.83 / 7.59 | 2.68 / 7.20 | 2.69 / 7.21 | 2.83 / 7.60 | 3.85 / 10.5 |
  | baseline day total 10k / 30k | 4.29 / 13.69 | 4.12 / 13.20 | 4.14 / 13.25 | 4.28 / 13.69 | |
  | fiscal-stress day total 10k / 30k | 3.86 / 10.86 | 3.69 / 10.49 | 3.70 / 10.45 | 3.85 / 10.88 | 13.4 / 42 |
  | hourly ms, baseline 10k / 30k | 0.85 / 4.31 | 0.86 / 4.13 | 0.87 / 4.16 | 0.83 / 4.14 | |
  | hourly ms, fiscal 10k / 30k | 6.52 / 28.30 | 6.51 / 28.53 | 6.57 / 28.34 | 6.55 / 28.63 | |
- **The 2 ms terrain stage: met** (open question 22). The orientation fix alone, bitwise
  unchanged, takes the sloped T to 1.913 / 1.885 ms (plan 0.448 / 0.421 ms), A B B A with the
  `409a0212` part 2 binary at 2.221 / 2.172 ms in the same session; the final tree, with the
  station rule's rows, measures 1.850 / 1.883 ms (table below). Options (a)–(c) of the question
  are not needed.
- **Span station rule.** Constructive, as the [span piece](#pieces-and-meshes) states: fixed
  stations, each interval divided evenly at its step. Test
  `sloped_span_rows_never_fall_millimetres_apart` (no two rows of the sloped T within 0.1 m).
- **Terminal cap apex.** The half turn's middle sample is its start turned a quarter exactly; test
  `an_axis_aligned_terminal_cap_apex_lies_on_its_axis`.
- **Product probes re-recorded,** each change attributed by a build without the other: the station
  rule alone changes the spans and queries of all 93 fixtures and of 18 of the 20 reference
  junctions (with the lanes of the 9 sloped ones), nothing else; the cap apex on top changes the
  nodes and layouts of 65 fixtures (with their walkways, lanes and queries where those follow the
  cap) and nodes, layouts, walkways, lanes and queries of all 20 reference junctions, never a
  span. Neither touches the graph.
- **Size.** `roadbed/` holds 7,588 lines by the part 2 count (7,606 at part 2 rebased on
  `27797b92`, 7,421 on `road`): part 3 removes 18 (the station rule replaces two sampling
  families with one subdivision, removes 26; the apex adds 8). `ROAD-68-D` adds 167 in all.
- **Entry points.** `terrain_cdt_window_build_inputs_with_roads`, `terrain_cdt_road_tile_loops`,
  `roadbed::terrain::terrain_loops` and `grading::road_loop_grading` keep their signatures.
- **Open at part 3: terrain pinholes (part 4 fixes the terrain's share).** The reference screenshots
  (`road_junction_reference_test.gd`, oblique views over the production terrain payloads) show
  33 isolated sky-coloured pixels through the terrain in 12 of the 20 fixtures, flat and sloped,
  and none in the pre-D set from the same harness; the part 2 tree shows 40 at the same points,
  so they predate part 3's station rule and apex. Several recur at the same screen point across
  fixtures, so a fixed tile or patch boundary or corner near the junction cracks (a T-vertex
  between tiles, between a refined tile and a contributor-free filler tile, or between the
  refined and the regular patch). Next: a mesh test that every terrain edge of the fixtures'
  patch payloads is shared by two faces, lies on a road outer chain or on the world boundary.
- **Open: the yard test** `cars_and_walkers_enter_and_exit_on_compiled_yards` (`ROAD-68-D`
  ignore). A diagnostic over its three missing samples finds two site-side cases, not the roadbed
  clip. Building 1 (patch (17, 17), tile x 64..128, z -128..-64): both samples (u = 0.1 and
  u = 0.9 of its yard) lie on sides of the building's site loop, the seam between terrain and
  foundation, to within about 0.2 mm; the terrain face along that side is in the tile and patch
  buffers, but in their patch-local f32 coordinates the sample falls just outside it, and the
  foundation's paved triangles (world f32) do not take it either. Whether that is only the test
  sampling a seam or a real sub-millimetre gap is open. Building 3 (patch (17, 15)): the yard's
  centre lies inside the building's own 6-vertex site loop, so the CDT rightly leaves no terrain
  there, but no paved foundation triangle covers it: either `foundation_mesh` is empty (it returns
  no triangles when its triangulation or partition fails) or the foundation footprint is smaller
  than the site loop, which would be a visible hole. The patch buffers also hold zero-area faces
  (three equal vertices), likely from the paving partition's f32 output of near-degenerate
  pieces.

| Build (part 3 session) | Total p50 (ms) | Plan p50 (ms) | Tile CDT p50 (ms) | Slowest tile (ms) |
| --- | --- | --- | --- | --- |
| first WIP (control) | 2.882 / 2.903 | 1.360 / 1.367 | 0.332 / 0.333 | 1.064 / 1.076 |
| part 2 (`409a0212`) | 2.202 / 2.249 | 0.741 / 0.753 | 0.321 / 0.325 | 1.105 / 1.124 |
| part 3 | 1.850 / 1.883 | 0.422 / 0.386 | 0.331 / 0.328 | 1.027 / 1.121 |

Part 4 (2026-10-24, same branch): watertight terrain.

- **Test first.** `reference_junction_terrain_is_watertight` (`nodes/sim/core/tests/terrain_seams.rs`)
  builds the 20 reference junctions as the screenshot harness does (`SimCore::commit_road` on the
  512 m world, flat and on the 2.5% slope), composes every refined patch payload, and requires
  every terrain edge, in world coordinates from the patch-local f32 vertices, to border exactly
  two faces with bitwise equal vertices, or to border the road (`band_at` 5 mm across it), a
  building site or the refined area's edge. On the part 3 tree it failed every fixture with 90
  to 416 open edges.
- **Cause 1: small faces dropped.** The CDT export skipped every face whose 3D cross product
  had a squared length of at most 10⁻⁶, a face of up to 5 cm² (`terrain_cdt_triangle_geometry`).
  The tie-in guides put triples of vertices 1.5 mm to 1 cm apart along a corner curve, and the
  slivers between them and the next sample (0.5 m long, 1.5 mm wide) fell under the cut-off:
  holes the neighbouring faces do not cover, at the screenshots' recurring points (the T's
  corner at (8.5, 8.3)). Only an exactly degenerate face is skipped now. The face slope metric
  had the same kind of cut-off (a face with a plan-area cross product of at most 10⁻⁶ counted as
  vertical, slope 10⁶); a small face is measured at its own slope now, which keeps the Kuopio
  fixture's guide slivers (slope 3 to 8) under the 256 pathology limit.
- **Cause 2: a non-conforming filler.** The regular filler over contributor-free tiles took
  every CDT vertex lying on a tile side, including those of faces inside the footprint, as
  side samples, ran every off-lattice one as a grid line across the whole patch, and densified
  a cell only where it touched a tile: T-vertices against tile sides without those vertices
  (the main road's 1 m span rows on the `z = 0` tile line reached `z = −64`) and between filler
  cells, and on slopes the filler's side heights came from the patch snapshot while the tile's
  came from the terrain system. The filler is conforming now ([watertight
  patches](#footprint-and-terrain)): window side vertices are read from the window's exported
  vertices, and a filler cell takes them bitwise on a shared side.
- **Result.** The test passes on all 20 fixtures. The screenshots (`$ARTIFACTS/screenshots-D4`)
  go from 33 pinholes to 9 (`pinholes.py`), none on the flat fixtures; the 9 lie in the road
  surface or its outer seam on the sloped fixtures (open question 24). Fresh: debug
  `cargo test --lib` 1,867 passed, 0 failed, 168 ignored (`ROAD-68-D` 1, the yard test; `-E` 5;
  `-F` 5); `./run.sh --test` exit 0 before the paving change below (Rust-only). The filler tests
  are rewritten to the conforming contract
  (`regular_terrain_filler_shares_cdt_window_side_vertices`,
  `regular_terrain_filler_conforms_across_filler_cells`,
  `regular_terrain_filler_does_not_globalize_canonical_window_lattices`).
- **Tried and reverted.** Sampling the tiles' source heights with the patch snapshot function,
  so tile and filler share one height function, made the Kuopio fixture's patch (14, 22)
  pathological at first; the cause was the slope metric's cut-off above, not the heights, but
  the change was not needed for the fixtures and stays open (question 23).
- **Yard test: building 3 fixed, seams open** (open question 23). Building 3's yard centre fell
  into a T-vertex crack on the yard's diagonal in the material partition
  (`SitePavingPartition`): a convex surface is now clipped against as one polygon, and a split
  point is computed from its segment's lower end, so faces sharing a segment split it alike.
  The seam samples of buildings 0 and 1 still fall between the world-space foundation and the
  patch-local terrain, whose seam split points two partitions compute in two frames; the
  recommended fix builds the foundation's faces into the terrain patch buffers.
- **Entry points.** `terrain_cdt_window_build_inputs_with_roads`, `terrain_cdt_road_tile_loops`,
  `roadbed::terrain::terrain_loops` and `grading::road_loop_grading` keep their signatures.
  `SimCore::build_refined_terrain_cdt_window` and
  `prepare_cached_refined_terrain_window_mesh_buffers` lose their unused boundary-step and
  window-bounds parameters, and the cached window buffers lose their four side manifests.
  `roadbed/` is unchanged (7,588 lines).

Part 5 (2026-10-25, same branch): render seams; the gate closes. Open questions 23 and 24 (a),
decided by the coordinator under the user's delegation, 2026-10-24, for review.

- **Road chunks edge-match** (question 24). Test first:
  `reference_junction_road_chunks_are_watertight` (`nodes/sim/core/tests/terrain_seams.rs`)
  publishes the 20 reference junctions' road chunks, adds each chunk's frame in f64, and
  requires every edge of the road, sidewalk, curb, curb-face and concrete layers with road on
  both sides to border exactly two faces with bitwise equal vertices. Before the fix every
  fixture failed, 72 to 138 edges, flat and sloped alike. Two causes:
  - **Zebra cut-outs.** `crosswalks::emit_triangle` cut each carriageway triangle its zebra's
    bar rectangle touched along the rectangle's lines, extended across the whole triangle;
    neighbours outside the rectangle and the curb faces kept the unsplit edge (T-vertices; at
    zero margin the rectangle's corners lie on the kerb). Now nothing is cut: an asphalt
    triangle meeting the bar rectangle (separating axes) carries the crossing's affine paint
    frame on its three vertices, and the shader draws only the bars. The margin to each kerb is
    under a bar pitch, so no extra bar fits. The layout's strips are disjoint and bounded by mesh
    rows; a debug assertion holds that at most one crossing meets a triangle. The polygon split
    helper `render/ownership/preview.rs` is gone with the cut.
  - **Chunk frames.** A triangle went whole into its centroid's chunk, and two faces sharing an
    edge across a chunk line rounded their chunk-local f32 apart (22 to 28 edges on the Y and
    bend fixtures, whose junction sits on a chunk corner). Now a triangle crossing chunk lines
    is clipped at them by `terrain/rect_clip.rs`, whose split points are functions of the edge
    alone (from its lexicographically lower end, the line coordinate exact) and whose chunk
    corners lie on the triangle's plane, so every vertex two chunks share lies on their line,
    exact in both frames.
- **One frame per seam** (question 23). A chunk's payload is local to the chunk's centre
  (`render_frame_centre`, the terrain patch's frame on the terrain-aligned grid), converted from
  the pieces' f64 positions by `render_frame_local`, the terrain export's conversion; the road
  emitters carry f64 world positions to the partition. Godot places chunk nodes, lights and
  signs at that centre (`network_tool.gd` `_road_chunk_frame_centre`). Test
  `reference_junction_road_and_terrain_share_seam_vertices`: every terrain seam vertex is a road
  chunk vertex with the same bits in the same frame, or a CDT tile-line point (open, below).
- **Site pads in the terrain** (question 23). Each site's pad is composed into the patch
  buffers (`foundation_faces_in_frame`, `partition_terrain_site_paving`; the patch inputs carry
  `site_foundations`, and the composed buffers are reused only while they are equal), so pad and
  terrain meet on one partition in one frame. `get_building_site_mesh_data_internal` uploads no
  foundation. The yard test `cars_and_walkers_enter_and_exit_on_compiled_yards` passes again.
  Its last miss, building 0 at (56.519, −111.866), fell in a 17 µm sliver between the pad side
  and a terrain vertex 17 µm off the footprint corner; the test's height query dropped faces
  under `SAMPLE_EPSILON_M` and is now an exact point-in-face test. `assert_no_terrain_over_site_pads`
  now requires every face over a pad point to be flat at the support height.
- **Open** (question 23): seam edges are shared at vertex level only. The CDT's loop clip adds a
  seam vertex where a seam crosses a tile line inside a patch, and one `y_60` terrain seam edge
  passes a road vertex 1.85 cm from its end: T-vertices on straight seam edges. The truncated
  border patch's frame and the shared filler/tile height function are open too.
- **Fresh checks** (this session): debug `cargo test --lib` (`RAYON_NUM_THREADS=4`,
  `METRUM_DEBUG=0`) 1,871 passed, 0 failed, 167 ignored (`ROAD-68-E` 5, `-F` 5; no `ROAD-68-D`
  ignore remains); `cargo clippy --all-targets` 59 warnings, the baseline set; `./run.sh --test`
  (DISPLAY and WAYLAND_DISPLAY unset) exit 0, Rust 1,871 / 0 / 167, integration 4 passed, 0 SCRIPT
  ERROR, still skipping `network_tool_chunk_renderer_test`, `road_junction_preview_test`,
  `road_preview_stream_test` (until `ROAD-68-E`) and `zoning_cells_tool_test` (until
  `ROAD-68-F`). `run.sh` and the screenshots ran on the tree before the last commit's
  single-chunk fast path in `render/road/chunks.rs`, which emits bitwise the same triangles; on
  the final tree the seam, road render, rectangle clip, zebra and building-site terrain tests
  pass (58), and clippy is the baseline set.
- **Screenshots** (`$ARTIFACTS/screenshots-D5`, `screenshots-C.sh`, final code tree): PASS, 20
  images, 0 SCRIPT ERROR; `pinholes.py` finds **0** pinholes (part 4: 9, the same script on
  `screenshots-D4`). Zebras render as before (seven bars kerb to kerb on the 6.5 m arms).
- **Terrain stage** (`benchmark_roadbed_terrain_tiles`, release test binaries,
  `RAYON_NUM_THREADS=4 taskset -c 0,2,4,6,8,10,12,14`, both timing locks; A = the first WIP
  binary, the gate's same-session control, B = part 5): the sloped T stays under its 2 ms target.
  The closing A run waited behind other agents' timing batches and is not in this record.

  | Build (part 5 session) | Total p50 (ms) | Plan p50 (ms) | Tile CDT p50 (ms) | Slowest tile (ms) |
  | --- | --- | --- | --- | --- |
  | first WIP (control) | 2.906 | 1.388 | 0.332 | 1.648 |
  | part 5 | 1.974 / 1.837 | 0.414 / 0.418 | 0.340 / 0.329 | 1.484 / 1.034 |

  Part 5 does not touch the tile plan or the CDT (the export's frame conversion is the same
  arithmetic through `render_frame_local`); pads and chunk clipping run at composition and
  publication, outside this stage.
- **Product probes** unchanged: the roadbed is untouched (`roadbed/` 7,588 lines), and
  `reference_junction_products_match_the_probe` and the fixture probes pass without
  re-recording.

Part 6 (2026-10-25, branch `road68-D6` on `road` with `ROAD-68-E` rounds 1-5): commits at city
scale. `ROAD-68-G` part 5 found the populated-city breakdown's first commit (a T split of a west
border trunk) rejected at 100k residents with `render_geometry_invalid`, and the commit's
`surface` stage at 15-69 ms median with a first sample of 34 ms to 1.7 s growing with the city.
Two causes, both fixed constructively; the patch-selection rule stays `ROAD-68-E`'s chain rule
(round 4, [patch selection](#footprint-and-terrain)).

- **The first commit inherited the city's terrain.** The roadbed records the bounds (and, since
  `ROAD-68-E` round 4, the chain segments) of every piece a compile replaces until a road edit
  takes them, and only the road edit's own terrain rebuild took them. A bulk road build (the
  populated-city fixture, the in-game benchmark city) and a load compile every piece outside a
  road edit, so the first commit afterwards stamped, graded and built every road patch of the
  city: 6,503 piece bounds and 203 of 225 patches at 100k, 2.4 s of `surface` on the bare 100k
  street grid. Fix: the world terrain refresh (`refresh_all_engineered_terrain_patch_state`,
  run by a load and now by both bulk builds) takes and drops the pending changes and bumps the
  global payload generation, since it refreshes every patch anyway. Test first:
  `the_first_commit_builds_only_its_own_terrain_as_the_city_grows` (a bulk-built trunk with and
  without five roads in other patches; the T commit must build the same patches): it built
  `(0, 1)` alone and `(0, 1), (2, 0), (2, 1)` in the city before the fix.
- **The rejection: zero-width rails from the tile clip.** With the 100k city's patches built,
  64 tiles in 46 patches failed with `conflicting_road_boundary_height`, all at tile corners. In
  each, the tile's road loop is a comb (rows every 50 m joined by a cross street west of the
  tile) of which one row crosses the tile. The axis-aligned clip joined the rows along the tile's
  west side, and the side clips cut those joins into zero-width rails from the row to the tile
  corners, at a height of 0 that no road has; a building pad touching the tile corner at
  sidewalk height (0.12) then met a "road" vertex at another height. At 100k the rows' outer
  seams lie on 64 m tile lines (rows at `-3225 + 50 i`, seam `+7`; never at the lower tiers), so
  pads touch tile corners there; elsewhere the rails stayed as degenerate constraints along tile
  sides. Fix: rail tips are removed exactly after the axis-aligned clip ([tile
  clip](#footprint-and-terrain)); loops without rails are unchanged, bit for bit. Test first:
  `a_loop_reaching_past_a_tile_side_adds_no_vertex_off_its_footprint` (the comb and the pad in
  tile `(0..64)²`): the clip returned the strip plus `(0, 0)` and `(0, 64)` at height 0, and
  canonicalisation failed with the 100k error.
- **The breakdown builds the city's terrain before the click** (harness change, test-only):
  Godot requests and uploads the refined terrain of the road patches it shows long before a
  click, so the commit finds them built; the breakdown acknowledged uploads it never built, so
  its first sample built the edit's 512 m patches (64 tiles each) cold. It now builds every road
  patch once after the city (setup, untimed), which also checks that the whole city's terrain
  builds: at 100k it does now, and it failed on the 64 tiles above before.
- **One site query per patch.** `road_terrain_tiles` asked the building index whether a site
  lies near each tile it visited within the cap of a changed piece, a 512 m chunk candidate scan
  and sort per visit, with tiles visited again for every piece reaching them (`ROAD-68-E`
  round 5: the 30k commit's warm `surface` 71.5-73.5 ms against 62-63 ms before the chain
  rule). Candidates are now deduplicated per patch, the sites overlapping their union are fetched
  once (`append_building_site_bounds_for_world_bounds`; the preview's snapshot has the same) and
  each candidate is tested against them with the same closed bounds overlap, so the stamped
  tiles are unchanged.
- **Open, not this part's family.** A clipped site loop counts as a road loop for the shared
  vertex height rule (`road_loop_edge_sources_are_building_site_only` sees the clip's unsourced
  edges), so the road-wins rule does not apply to it; making it a site loop also turns
  disagreeing pads at a shared corner into conflicts, which pass today when one pad is clipped,
  so it waits for a decision on adjacent pad heights. The Kuopio watertight check, the 72
  zero-area f32 faces at kilometre coordinates and the truncated border patch's frame centre
  (part 5) are unchanged: the 100k failure was neither.

- **Commit breakdown** (`benchmark_populated_commit_breakdown`, `ROAD-68-G`'s harness with the
  warm-up above, release library tests, eight workers, `taskset -c 0,2,4,6,8,10,12,14`, both
  timing locks, one session; A = `road` at `9f092846` plus the harness warm-up, binary
  `aac6869d…`, 05:16-05:21; B = this part, binary `6d1661b3…`, 05:48-05:53). `surface` per
  sample (the three T splits) and click to ack median, ms:

  | Tier | A `surface` | B `surface` | A click → ack | B click → ack |
  | --- | --- | --- | --- | --- |
  | 0 | 18.2 / 11.7 / 11.4 | 11.2 / 12.7 / 11.1 | 14.5 | 15.3 |
  | 2k | 90.0 / 68.0 / 9.6 | 73.2 / 65.4 / 10.3 | 74.9 | 72.7 |
  | 10k | 209 / 18.1 / 15.6 | 133 / 18.7 / 15.3 | 31.0 | 29.0 |
  | 30k | 447 / 63.5 / 53.4 | 67.3 / 63.0 / 55.4 | 80.6 | 73.3 |
  | 100k | the warm-up fails (`conflicting_road_boundary_height`, patch (1, 2)) | 71.0 / 65.3 / 11.8 | | 78.7 |

  The 100k commits are accepted, and the first sample no longer grows with the city (447 ms at
  30k in A, from the stamps A's first commit inherits; 67 ms in B). The lots and generated-cell
  digests equal A's at every tier A reaches. **Open, target missed:** with a warm cache the
  commit's terrain still takes 10-73 ms against the 3.0 ms stage budget. It follows the 512 m
  patches the edit touches, not the edit: a T beside city patches costs 55-73 ms, one in open
  terrain 10-12 ms. Per stamped patch the validation re-grades every tile's loops for the
  patch's road margin, rebuilds the stamped tiles' windows and their neighbours, and recomposes
  the whole patch, its paving partition over every site in it ([open question
  29](#road-68-open-questions)). On the stage budget's four
  workers (B only, same session, 06:07-06:13) the `surface` medians are 15.7 / 75.6 / 26.7 /
  76.8 / 74.4 ms at 0 / 2k / 10k / 30k / 100k (first samples 16.1 / 90.4 / 101 / 85.5 /
  86.5 ms). The sloped T's terrain stage (`benchmark_roadbed_terrain_tiles`, four workers, A
  and B back to back, 06:07) is unchanged: 1.846 / 1.854 ms total p50 (plan 0.434 /
  0.439, tile CDT 0.323 / 0.325), under its 2 ms target; its tiles have no rails.
- **Fresh checks.** On the final tree: `./run.sh --test` (DISPLAY and WAYLAND_DISPLAY
  unset) exit 0, Rust 1,890 passed, 0 failed, 169 ignored (`ROAD-68-E` 4, `-F` 5), integration
  4 passed, 0 SCRIPT ERROR, still skipping `zoning_cells_tool_test` (until `ROAD-68-F`) and the
  display-only rendered tests; `cargo clippy --all-targets` 59 warnings, the baseline set; the
  two new tests, `ROAD-68-E`'s `a_split_trunk_rebuilds_only_the_terrain_near_the_junction` and
  `incremental_road_terrain_equals_a_fresh_build`, D's watertight and seam-vertex tests and the
  product probes pass. Screenshots (`$ARTIFACTS/screenshots-D6`, `screenshots-C.sh`): PASS, 20
  fixtures, `pinholes.py` 0. `roadbed/` is untouched: 7,831 lines, as on `road`.

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

#### Progress (2026-10-24, branch `road68-E1`, work in progress)

Landed on the road side (part 1, round 1); Godot, terrain tiles in the preview, payload packing,
lane-local undo and the stage timings are open.

- **Install API.** `RoadbedSystem::compile_dirty` is `compile_delta` (the pieces of the dirty
  closure, built as before, unpublished) followed by `install`. A `RoadbedDelta` records the
  generation and dirty marks it was compiled from; `can_install` checks them, so an installed
  delta equals that compile bitwise. `install` returns a `RoadbedRestore` with the replaced
  pieces and node failures; `restore` swaps the same `Arc`s back, published in a new
  generation (render publication splices by build generation), and `can_restore` checks that
  nothing rebuilt them since.
- **Context.** `SimCore::publish_road_tool_view` replaces the per-command deep copies: terrain
  and water are copied only when their own generations change, the roadbed is shared in `O(1)`
  (`RoadbedSystem::snapshot`), and the graph is the commit's routing-rebuild copy (the one
  `O(city)` copy a commit makes, which the CCH job needs anyway) or the previous view's when
  nothing changed. The preview's private replica (`RoadPreviewContext`) follows each
  publication by copying the records of the owners whose pieces the roadbed rebuilt
  (`take_rebuilt_owners`, `RegionGraph::restore_undo_delta_local`, `O(records * log edges)`),
  or by one copy after a load, a clear or a terrain sync. Debug builds compare the replica with
  the published graph after every publication.
- **Preview.** `RoadPreviewContext::preview_road` compiles the commit's `RoadEditPlan` on the
  context (`compile_from`), applies its topology to the replica through
  `RoadTopologyPlan::apply_to_graph` (the commit's adoption now calls the same function), marks
  the closure through `AppliedRoadTopology::mark_pieces`, compiles a delta on an `O(1)` copy of
  the roadbed and restores the replica from the same checkpoint the undo record captures. It
  returns the plan, the delta and `RoadPreviewFingerprint` (road-tool surface, roadbed and
  source and visual terrain generations). Godot: `preview_road(points, fwd, bkw, snap)`.
- **Commit.** A click submits the latest preview of the same stroke. With a matching
  fingerprint the commit uses its plan and installs its delta when `can_install` holds after
  the adoption's marks; otherwise it plans and compiles locally (`preview_plan_reused`).
- **Undo.** The road edit's undo record keeps the install's restore; undo and staged rollback
  swap the pieces back when `can_restore` holds, and recompile the restored owners otherwise.
- **Load.** Every published bend and junction piece is checked for a profile plane, as the
  commit's `check_node` does (`RoadbedSystem::unpinned_nodes`, parallel); rejected grounded
  nodes take the question 8 repair, and any left fail the load.
- **Tests.** `core::tests::road_preview`: a current preview's pieces are the commit's own
  `Arc`s and equal a fresh commit bitwise; a stale preview's commit equals a fresh commit; undo
  restores the pre-edit `Arc`s; the replica follows commits and undo. `undoing_fourth_junction_
  mouth_restores_exact_pre_edit_surface_topology` is re-enabled and passes.

Round 2 (part 1, 2026-10-24):

- **Road payloads.** The preview returns the changed chunks as the commit publishes them, without
  the lane-derived layers of rebuilt pieces (see [Preview](#preview-commit-undo-and-load)).
  `preview_chunks_are_the_committed_chunks_without_lane_layers` checks a stroke apart from the
  trunk publishes exactly the previewed chunks, flat and sloped.
- **Godot.** `road_tool.gd` calls `preview_road` from its early process callback whenever the
  stroke or the context generation changed and shows the chunks in that frame
  (`road_preview_display.gd`, which replaces `road_junction_preview.gd`); a click submits that
  preview, which stays shown until the committed chunks are installed. Deleted: the late
  publisher, the request object and request-id plumbing (Rust and GDScript), the ribbon, the
  `RoadPreviewSnapshot` path and D3's "Road only" setting, menu and settings key.
  `network_tool_chunk_renderer_test`, `road_junction_preview_test` and `road_preview_stream_test`
  are rewritten for the synchronous path and run again in `./run.sh --test`.
- **Undo.** Lane-local and graph-local (see [Undo](#preview-commit-undo-and-load));
  `undo_rebuilds_lanes_locally_as_a_full_rebuild_would` checks it flat and sloped. Undo and road
  bulldoze offer their routing rebuild's graph copy to the view (`RoadToolPublisher::offer_graph`),
  so they copy the graph once, not twice; debug builds check the offered copy equals the live
  graph.
- **Load.** `save_and_load_reproduce_the_pieces_bitwise`: a saved city's pieces (flat and
  sloped T) load back bit for bit, with the same failed nodes.
- **Small closures inline.** `RoadbedSystem::compile_delta` builds a closure of at most 16 pieces
  (an edit's or a preview's) on the calling thread and larger ones (loads, terrain syncs) on the
  shared pool: same functions, same order, so the products are unchanged; waking the pool for a
  handful of pieces cost more than building them (timings below).
- **Size.** `roadbed/` production lines: 7,902 (7,881 after round 1): +21 for the inline build.
- **Open in part 1:** terrain tiles and payloads in the preview, the terrain halves of the
  fingerprint, the commit install and the undo record ([Preview](#preview-commit-undo-and-load));
  the Godot stage below; the context refresh and the commit's graph copy at 10k and 100k.

**Stage timings (part 1, 2026-10-24).** Native: `$TEST_BINARY
nodes::sim::core::tests::road_preview::benchmark_road_preview_stages --exact --ignored
--nocapture`, release, `RAYON_NUM_THREADS=4`, pinned to the P-cores, both timing locks, the
`ROAD-30` preview worlds (512 m, 2.5% slope): `sloped_t` sweeps a T onto a 220 m trunk and
`sloped_branch` orbits a fourth leg of the T at (-65, 0), eight poses each as
`road_preview_replay.gd` moves them, 50 measured repetitions, ms p50 / p95. A is the round-2
binary (every closure on the pool), B adds the inline small-closure build; A B B A in one
session. No earlier synchronous preview exists to control against; the budgets are absolute.

| Stage (budget) | `sloped_t` A | `sloped_t` B | `sloped_branch` A | `sloped_branch` B |
| --- | --- | --- | --- | --- |
| Edit plan (1.0) | 0.24–0.28 / 0.41–0.61 | 0.18–0.19 / 0.25–0.26 | 0.20–0.22 / 0.34–0.40 | 0.15–0.16 / 0.21–0.22 |
| Roadbed (0.5) | 0.35–0.44 / 0.60–0.84 | 0.25 / 0.32–0.35 | 0.32–0.33 / 0.66–0.69 | 0.25 / 0.31 |
| Road chunks (part of packing, 1.0) | 0.28–0.35 / 0.51–0.72 | 0.23 / 0.25–0.29 | 0.25–0.30 / 0.49–0.63 | 0.23 / 0.25–0.27 |
| Preview call | 0.91–1.08 / 1.52–2.08 | 0.66–0.67 / 0.81 | 0.79–0.84 / 1.47–1.66 | 0.64 / 0.75–0.78 |

With B every native stage is within budget at p95. The bridge's dictionary packing
(`packing_ms`) adds 0.33–0.50 ms p50 in the Godot run below, so road payload packing is about
0.6 ms of its 1.0 ms. The context refresh after one commit on these fixtures (publication
including the routing graph copy, then the context's apply by records) is 0.005–0.012 ms; at
city scale it is the graph copy (2.6 ms at 100k residents, `ROAD-68-G` part 2, historical) plus
the rebuilt owners' records, not re-measured here.

Godot (A's release library, `road_preview_stream_test.gd` headless, pinned, both locks, 96
moving frames per fixture with a 1/60 s timer between them): every moved frame previewed and
showed its own pose on all nine fixtures; the tool's `_process` took 5.6–8.6 ms p50. Inside it
the native stages took four to five times the harness's (plan 1.0–1.3, roadbed 1.1–1.8, road
chunks 1.0–2.1 ms p50), and the rest of the frame (bridge call, chunk `ArrayMesh` upload and
staging) 2.1–3.2 ms p50 against the 2.0 ms Godot budget. B's library gives the same numbers
(`_process` 5.5–8.7 ms p50, stage and install 2.1–3.0 ms), so pool dispatch is not the gap.
Headless frames idle between inputs, so cores may clock down and caches go cold; the gap is not
attributed yet. The Godot stage and the in-Godot native stages are open misses (profile with
continuously rendered frames next); the 16.7 ms frame holds on every fixture (max 9.7 ms).

Round 3 (part 1, 2026-10-25, work in progress):

- **Decision (coordinator under the user's delegation, 2026-10-24, for review): terrain in the
  preview.** A persistent per-patch terrain context published with the view and refreshed from
  a change log; one source-phase implementation for the live core and the published context;
  the commit installs the preview's entries when they match; undo swaps the replaced refined
  entries back. As built, the match is decided by the patches' full source signatures (previous
  entry by identity, assembly scope, ownership and margin, sites) instead of payload
  generations and entry identities alone: the signature is what the build reads besides the
  roads and the heightmap, which the road fingerprint and the installed delta already cover, so
  a context that missed a change cannot install a wrong patch.
- **Terrain in the preview.** Stage 3 and the terrain half of 4 (see
  [Preview](#preview-commit-undo-and-load)): `RoadTerrainInputs` (the published map, refreshed
  from `RoadTerrainInputsLog`), `road_terrain_tiles` (factored out of the commit's stamps),
  `refined_terrain_patch_source` (one source function for core requests, the commit and the
  preview), `SimulationNode::build_road_preview_terrain` and the commit's adoption in
  `validate_staged_road_terrain`. Bridge: `preview_road` returns `terrain_patches` (payload
  dictionaries with baked shading masks), `terrain_generation`, `terrain_error` and
  `terrain_ms`. Godot: `road_tool.gd` stages the terrain patches with `road_terrain_preview.gd`
  before it shows the road chunks and commits both in the same call; an accepted commit holds the
  terrain preview until the commit's own patches replace it.
- **Undo terrain.** See [Undo](#preview-commit-undo-and-load): the record keeps the replaced
  refined entries; `undo_puts_the_pre_edit_terrain_back` (flat and sloped T) checks that undo's
  refined buffers are the pre-edit ones bit for bit, as the same `Arc`s, at current generations.
- **Tests.** `preview_terrain_is_what_the_commit_installs` (flat and sloped T): the commit adopts
  the preview's entries (`preview_terrain_adopted`), the installed buffers are the preview's own
  `Arc`s at the commit's generations, and every refined patch's render buffers equal a fresh
  commit's bit for bit; `a_stale_preview_recompiles_to_a_fresh_commit` now also checks that a
  stale preview's terrain is rebuilt and equals a fresh commit's.
- **The Godot gap, attributed.** The in-Godot native stages ran 4-5 times the harness's because
  the CPU ran them at its lowest clock. `perf stat -I 250` of the headless stream (round-2 B
  library, `RAYON_NUM_THREADS=4`, pinned, both locks, nothing else running): 4.8-5.0 GHz while
  Godot loads, 0.77-0.9 GHz through every motion phase (intel_pstate, powersave governor, HWP
  `balance_performance`, 800 MHz floor, C10 idle): the headless test sleeps 16.7 ms between
  inputs and the main thread works about 1.3 ms of each, so on an otherwise idle machine the
  cores never leave the floor. The same library and configuration ran at harness speed whenever
  the clock was up: back-to-back frames, 2 ms or 5 ms intervals, a busy-wait instead of the
  timer, or another process busy on the machine (`sloped_t` over five such runs: plan 0.22-0.27,
  roadbed 0.27-0.32, chunks 0.24-0.31, packing 0.07-0.09, Godot stage and install 0.41-0.50 ms
  p50; the native harness in the same session 0.19 / 0.25 / 0.23 ms on its own `sloped_t`). Pool
  size is not the cause (1, 4 and 24 workers alike when the clock is up); 24 workers pinned to the
  8 P-cores do slow the main thread by spinning after pool work (back-to-back frames 0.6-1.0 ms
  per stage, and the native harness with 8 workers on 8 CPUs is twice as slow), so timing runs
  use 4 workers, as the budget is stated. Per-call allocation and Variant conversion are inside
  the 0.07-0.09 ms packing. No code change follows;
  `road_preview_stream_test.gd` takes `METRUM_PREVIEW_STREAM_INTERVAL_SEC` (0 drives frames
  back to back) so the stage timings are reproducible at operating clock
  ([open question 27](#road-68-open-questions)).
- **Stage timings (round 3, 2026-10-25).** `benchmark_road_preview_stages` as in part 1
  (release, 4 workers, P-cores, both timing locks), C = this round's binary, B = round 2's
  inline-build binary as the same-session control (C B; the second B and C were not run: the run
  lock went to another gate's suite), ms p50 / p95:

  | Stage (budget) | `sloped_t` C | `sloped_t` B | `sloped_branch` C | `sloped_branch` B |
  | --- | --- | --- | --- | --- |
  | Edit plan (1.0) | 0.25 / 0.29 | 0.19 / 0.27 | 0.21 / 0.24 | 0.16 / 0.22 |
  | Roadbed (0.5) | 0.34 / 0.37 | 0.25 / 0.36 | 0.34 / 0.37 | 0.25 / 0.31 |
  | Road chunks (part of packing, 1.0) | 0.26 / 0.29 | 0.23 / 0.32 | 0.26 / 0.29 | 0.23 / 0.25 |
  | Terrain patches, four workers (3.0) | 7.04 / 8.12 | — | 8.15 / 8.95 | — |
  | Preview call | 7.89 / 8.96 | 0.66 / 0.96 | 8.96 / 9.77 | 0.63 / 0.75 |

  **The terrain stage misses its 3.0 ms budget by a factor of 2.3-2.7 (open).** Each pose
  touches 11 render patches: the T splits the 220 m trunk, so the trunk's old
  and new span pieces stamp every patch within the grading halo of the whole trunk, as the
  commit's stamps do (the commit's terrain validation does the same work). The plan, roadbed and
  chunk stages rose 0.03-0.09 ms p50 against the control although their code is unchanged,
  probably because the terrain stage between poses evicts their caches (not measured). Not yet
  attributed; next: profile one pose
  (the per-patch ownership margin and source phase run serially before the parallel build), and
  stamp only the tiles whose road loops change (the split halves of a straight trunk keep their
  loops away from the node). The Godot stream with this round's library was not run.
- **Open in part 1:** the terrain stage's 3.0 ms budget (above), a Godot test of the terrain
  staging and swap on a resident terrain renderer, the roadbed p95
  per-pose scratch, the context refresh and graph copy at 10k and 100k, the load
  `unpinned_nodes` test and timing, and the rebase onto gate D.

Round 4 (part 1, 2026-10-25, work in progress; rebased on `road` `e9fee678`, gate D done):

- **Rebase.** The branch now sits on gate D's final commit; E's open questions are 25-27 (were
  23-25), after D's 23 and 24. The preview display places chunks through
  `NetworkTool._place_road_chunk_instance`, so it follows D part 5's chunk-centre frames; the
  preview's road payloads come from the commit's own emission and clip. `road_preview`, the
  product probe and D's `reference_junction_road_chunks_are_watertight` and
  `reference_junction_road_and_terrain_share_seam_vertices` pass after the rebase.
- **Terrain stage, cause.** A T splits the trunk; the trunk's far half keeps its chain vertices
  and heights bit for bit (measured: every added or removed chain vertex lies within about 40 m
  of the junction and the new stroke), but the CDT input named graph pieces and chain segment
  indices, which the split renumbers (the far half becomes another edge's span, its segments
  shift). Every window along the trunk changed fingerprint and re-triangulated (15 of 15 on the
  sloped T at x = 25), and the stamps, from the old and new pieces' whole bounds, covered the
  whole trunk (11 patches per pose).
- **Fix (exact, constructive).** (1) `road_terrain_loops` makes a tile's loops canonical in the
  footprint inside its rectangle ([source tags](#footprint-and-terrain), open question 28): the
  far windows then keep their fingerprints and are reused (11 of 15 change on the same T, all
  within the junction's halo). (2) The roadbed keeps the net multiset of changed grounded chain
  segments (`TerrainChanges`, `take_terrain_changes`), and `road_terrain_tiles` stamps only tiles
  within the cap of a changed segment, or of a changed piece when a building site lies within
  the local sample margin of the tile's window ([patch selection](#footprint-and-terrain)); the
  commit asks the allocator's site index, the preview the published patch sites. (3) The preview
  grades its patches' road margins in parallel inside one pool entry. Gate D part 6 now owns the
  patch-selection rule for commit and preview (coordinator, 2026-10-25); this round's rule stays
  until D6's replaces it.
- **Tests.** `incremental_road_terrain_equals_a_fresh_build` (flat and sloped T at x = 25, sloped
  at x = -40): after the commit every refined patch's render buffers and every window key
  (fingerprints included) equal a from-scratch build's. `a_split_trunk_rebuilds_only_the_terrain_near_the_junction`
  (locality): on a 1,024 m world the preview builds, and the adopted commit stamps, the same
  patches and tiles for a 220 m and a 660 m trunk, flat and sloped. The round-3 bitwise preview,
  commit, stale and undo terrain tests pass unchanged.
- **Commit publication (coordinator, from gate G part 4's profile).** `RoadToolShared::publish`
  swaps the context's and the shared view under their locks and frees the replaced views (with
  the simulation's previous view and a replaced replica) on the shared pool, so the last copy of
  an old graph, heightmap or watermap is no longer freed on the simulation thread. Sharing the
  preview's graph replica with the CCH job instead of the commit's graph copy is not exact as
  things stand: previews mutate the replica every pose while the job builds, so the job would
  need its own copy anyway; a persistent graph ([open question 21](#road-68-open-questions) (c))
  is the way to drop the copy. Measured with this branch's commit breakdown (release, 8 workers,
  P-cores, both locks, D2): on the 1,300-resident street grid (tier 0) the publication takes
  0.020-0.048 ms (`publish_ms`) and the commit's snapshot 0.036-0.050 ms (`snapshot_ms`) over
  three samples; the 100k tier rejects its first commit with `render_geometry_invalid` on
  `e9fee678` (gate D part 6), so the 100k numbers need gate G's harness on `a4941bbf` (next
  round). No same-session control: the freed views now drop on the pool, off the measured call.
- **Stage timings (round 4).** `benchmark_road_preview_stages` as in part 1 (release, 4 workers,
  P-cores, both timing locks), C = round 3's binary (`7089b50f`), D1 = this round before the
  rebase (`d78dc7cc`), D2 = this round on `road` `e9fee678` (`608027cb`), C D1 D2 D2 D1 C in one
  session, ms p50 / p95:

  | Stage (budget) | `sloped_t` C | D1 | D2 | `sloped_branch` C | D1 | D2 |
  | --- | --- | --- | --- | --- | --- | --- |
  | Edit plan (1.0) | 0.25 / 0.29 | 0.25 / 0.29 | 0.25 / 0.29-0.33 | 0.22 / 0.26 | 0.22 / 0.25-0.27 | 0.21-0.22 / 0.25-0.27 |
  | Roadbed (0.5) | 0.34 / 0.38-0.39 | 0.34 / 0.38-0.39 | 0.34-0.37 / 0.37-0.45 | 0.35 / 0.37-0.38 | 0.35 / 0.38-0.41 | 0.35 / 0.38-0.45 |
  | Road chunks (part of packing, 1.0) | 0.27 / 0.30-0.32 | 0.25 / 0.29-0.31 | 0.35-0.50 / 0.40-0.59 | 0.27-0.28 / 0.32-0.33 | 0.26-0.27 / 0.30-0.35 | 0.35-0.40 / 0.39-0.56 |
  | Terrain patches, four workers (3.0) | 7.07-7.09 / 8.12-8.17 | 5.56-5.57 / 6.25-6.32 | 4.50-4.91 / 5.08-6.65 | 8.30-8.33 / 9.10-9.24 | 6.25-6.35 / 7.14-7.25 | 4.82-5.48 / 5.76-7.65 |
  | Patches per pose | 11 | 8 | 8 | 11 | 9 | 9 |

  D1 against C is this round's change alone; D2 adds gate D's final terrain code (its grading
  computes a loop's orientation once) and part 5's chunk-line clipping and zebra paint frames,
  which raise the chunk stage within its budget. The first D2 run of the session is the slower
  one in each pair. **The terrain stage still misses its 3.0 ms budget (open).** A profile of what remains (rebased binary, flat):
  heightmap interpolation 7%, the tie-in widening test 6.4% and the footprint test 3.1% (linear
  in the tile's loop edges per sample, D's question 22 option (b)), grading rays 5%, the window
  ground-triangle index 2.7%, Spade and its predicates about 14%; the per-pose work is the
  junction's 11 windows plus the inputs of the stamped tiles and their neighbours.
- **Per-pose scratch.** The preview context keeps the span and node builders' buffers
  (`RoadbedScratch`) and compiles each pose's closure with them (`compile_delta_with`), as the
  [guardrail](#guardrails) asks; larger closures still build on the pool with per-worker
  scratch. The builders clear what they use, so the pieces are unchanged (probe and bitwise tests
  pass); not yet timed.
- **Size.** `roadbed/` production lines (every line of each non-test `.rs` under `roadbed/` plus
  `roadbed.rs`, without `tests/`, `tests.rs`, `fixtures.rs` and `#[cfg(test)]` items): 7,807 on
  this branch against 7,394 on `road` (gate D's final); round 4 adds 117 (7,690 before it): the
  chain-change ledger and `TerrainChanges` in `roadbed/terrain.rs`, its wiring in `system.rs`,
  the span's `grounded` flag and `RoadbedScratch` with `compile_delta_with`. Rounds 1-3 add 296 (the install API, the delta and restore, the
  inline small-closure build).
- **Open in part 1:** the terrain stage's budget (with gate D part 6's scope rule), a Godot test of
  the terrain staging on a resident terrain renderer, the rebase onto gate G parts 3-5
  (`a4941bbf`), the context refresh and graph copy at 10k and 100k (the 100k commit is rejected on `e9fee678` with
  `render_geometry_invalid`, gate D part 6), the load `unpinned_nodes` test and timing, and the
  undo record's retained heap.

Round 5 (part 1, 2026-10-25, work in progress; rebased on `road` `a4941bbf`, gate G parts 3-5):

- **Rebase.** Conflicts only in `road_edit.rs` (G's `changed_edges` of an adopted plan, now
  computed in `adopt_road_topology_plan` from the plan's edges and the live ids
  `apply_to_graph` returns) and the breakdown harness (`RoadToolShared` beside G's
  `METRUM_COMMIT_PHASES`). `schedule_cch_rebuild_of_live_graph` still returns its copy for the
  view; the preview's pool entry and `RetiredRoadToolState::retire` only spawn, so G's rule that
  no shared-pool worker waits for a pure job holds.
- **Fix: a lane-local undo left stale entrances.** Since round 2 the undo rebuilds the lane
  closure of its restored edges, which gives those lanes new ids; the buildings' entrance cache
  kept the old ones, and G's local entrance update at the next commit found them (debug check in
  `rebuild_entrances_of`; `narrow_road_marks_match_marking_the_whole_refreshed_ring`: T split,
  undo, T split again). The undo now derives the entrances that front the closure again, as the
  commit does; the dense-terrain path, which renumbers every lane, marks the cache stale for the
  next frame. Before the rebase no test committed after a lane-local undo on a built-up city.
- **Godot stream (round 4's release library `713ae3a7`, run at the end of round 4).**
  `road_preview_stream_test.gd` headless, isolated profile, pinned to the P-cores, both timing
  locks, `RAYON_NUM_THREADS=4`, 96 moving frames per fixture, nine fixtures. Every moving pose was
  previewed and shown in its own frame (864 of 864). Back to back (operating clock,
  [open question 27](#road-68-open-questions)): the tool's `_process` took 5.2-7.2 ms p50, p95
  5.7-7.9 ms, max 8.8 ms (`cross`), so every frame fits 16.7 ms; stages p50 (max): terrain
  3.48-5.13 (6.22) ms, the miss gate D part 6 now owns (`dense_t` 5.13, `cross` 4.55,
  `sloped_t` 4.05); payload packing 0.44-0.49 (0.65), within 1.0; Godot stage and install
  0.43-0.56 (0.74), within 2.0; plan 0.23-0.27, roadbed 0.28-0.37, road chunks 0.28-0.48. With
  the 1/60 s timer (idle clock, observation): `_process` 11.8-13.5 ms p50, max 21.1 ms, and
  **38 of 864 frames exceed 16.7 ms** (`wide_bend` 14, `control_curve_t` 6, `t` and `cross` 5,
  `bend` 4, `sloped_t` and `chunk_boundary_t` 2, `wide_t` and `dense_t` 0), terrain 8.1-9.8 ms
  p50 (max 15.4). Question 27's evidence that the frame holds even at the idle clock dates from
  before terrain was in the preview; it no longer holds, so the idle-clock frame is recorded as
  an open miss with the terrain stage, not as an accepted target.
- **Godot terrain staging on the real renderer.** `network_tool_chunk_renderer_test.gd`
  (`_test_road_preview_stages_terrain_on_the_real_renderer`): the road tool, given a preview
  with one road chunk and one engineered terrain payload, shows in one `_show_preview` call
  both the chunk and the payload's mesh under the resident patch of the real `terrain.gd`
  (resident mesh detached, resident data untouched); an accepted commit holds both; the
  committed chunks' install leaves the terrain preview shown; the commit's own patch, staged and
  installed through the renderer's terrain transaction, releases the held preview inside the
  install call (`patch_render_will_change`), after which the resident node shows the commit's
  mesh. So no frame shows neither.
- **Commit publication and snapshot, with gate G's breakdown.** `$TEST_BINARY
  nodes::sim::core::populated_city::routing_benchmark::benchmark_populated_commit_breakdown
  --exact --ignored --nocapture`, `METRUM_COMMIT_TIERS=0,10000,30000`, release, 8 workers as
  gate G times it, P-cores, both timing locks; C = `road` `47432fb5` (gate G part 5), E = this
  branch before the fix below (`abe1910c`), C E E C, three samples per tier (sample 0 is the
  city's first commit). Lots digests equal in every sample. Warm samples 1-2, ms:

  | Tier | `publish_ms` C | E | `snapshot_ms` C | E | click to acknowledgement C | E |
  | --- | --- | --- | --- | --- | --- | --- |
  | 0 | 0.07-0.15 | 0.03-0.06 | 0.02 | 0.03-0.05 | 13.4-17.8 | 14.6-16.3 |
  | 10k | 0.23-0.70 | 0.04-0.11 | 0.25-0.28 | 0.83-4.99 | 21.6-30.3 | 22.8-35.0 |
  | 30k | 0.49-1.66 | 0.07-0.21 | 0.72-0.77 | 1.22-9.36 | 62.7-76.4 | 66.6-91.2 |

  The publication now frees replaced views on the pool, off the commit (round 4). The
  snapshot rose: a probe in `publish_road_tool_view` put all of it in the terrain inputs'
  refresh, about 0.1-0.13 ms per noted patch (36 patches at 10k, 4.8 ms; 60 at 30k, 6.4 ms;
  4-6 patches, 0.55-0.6 ms), each re-running two building-site queries over the 512 m
  building chunks around the patch, although most notes (acknowledgements, ledger stamps, cache
  inserts) change no site; the site index preparation and the zoning grid snap measured 0. The
  city's first commit also rebuilds every patch (4.5 ms at 10k, 7.0 ms at 30k), which in the
  game the load's own publication pays. **Fix:** only patches noted for a site change
  (`note_sites`) query the sites again; the others keep their shared site snapshot and site
  ownership, and site ownership is read from the snapshot (the widest margin holds the local
  one, under the same overlap rule), so a site-noted patch queries once.
  Rebased on `road` `9f092846` (gate G part 6), B = `road` (this branch's tests only), A = the
  fix (`7b6412be`), B A A B, tiers 2k/10k/30k, same settings, lots digests equal; warm samples
  1 / 2, `snapshot_ms`: 2k B 0.54-0.55 / 0.30-0.31, A 0.06 / 0.05-0.06; 10k B 4.13-4.18 /
  0.84-0.86, A 0.29 / 0.26-0.27; 30k B 7.76-9.22 / 1.23-1.27, A 0.81-0.88 / 0.77-0.80, back to
  gate G part 5's 0.27 and 0.79 ms. Click to acknowledgement in sample 1: 10k 32.0-32.2 to
  26.9-27.0 ms, 30k 81.8-82.8 to 73.3-76.2 ms. The city's first commit still rebuilds every
  patch's inputs (A 0.68 / 3.9 / 7.4-7.6 ms), the harness's stand-in for the load's
  publication. Question 21 (c), the persistent graph: the commit's graph copy is the rest of
  the snapshot (0.33-0.40 ms at 10k, 0.94-1.07 ms at 30k, `graph_clone_ms`; 2.4-2.6 ms at
  100k before gate D, gate G part 5) beside commits of 21-27 ms at 10k and 59-76 ms at 30k,
  so it does not decide the 16.7 ms target at these tiers; 100k is measured once gate D part 6
  lets the 100k commit through.
- **The commit's surface stage on this branch (for gate D part 6).** In the same session the
  surface stage (`validate_staged_road_render`) took 71.5-73.5 / 58.9-61.9 ms on E against
  62.2-62.8 / 54.6-55.0 ms on C at 30k (samples 1 / 2), and 1,856-1,943 against 1,722-1,758 ms
  cold; at 10k they are level. Probably the commit's site test of the round-4 scope rule:
  `road_terrain_tiles` asks the allocator's site index once per visited tile of each changed
  piece, and each ask scans the 512 m building chunks; gate D part 6 owns that rule now.
- **Native stages on the rebased tree.** `benchmark_road_preview_stages` as in part 1, D2 =
  round 4's binary before the per-pose scratch (`608027cb`), E = this round (`abe1910c`), D2 E E
  D2, ms p50: roadbed `sloped_t` 0.337-0.339 against 0.331-0.333, `sloped_branch` 0.340-0.342
  against 0.327-0.329 (the per-pose scratch, a small consistent gain); plan level; road chunks
  0.338-0.340 against 0.353-0.360 and terrain `sloped_t` 4.41-4.45 against 4.67, `sloped_branch`
  4.71-4.72 against 4.88-4.90 (gate G parts 3-5 are in E and not in D2). Every native stage but
  terrain is within budget at p95.
- **Load.** `a_saved_junction_a_commit_would_reject_fails_the_load`: a saved T whose south arm
  is shorter than the profile solve's 2 m sample fails the load with its typed error, with the
  repair (every arm grounded) and without it (a bridge arm). That arm fails layout first
  (`ArmTooShort`), and so did every shorter one tried: no saved graph was found whose junction
  builds a piece without a plane, so the load's plane check (`unpinned_nodes`) mirrors the
  commit's `check_node` without a fixture that reaches it. Its cost
  (`populated_city::load_check::benchmark_load_node_check`, release, 8 workers, the economy
  performance contract's cities, medians of five alternated runs): 0.18 ms for 247 nodes at
  10k against the load's 17.3 ms piece compile, 0.29 ms for 642 nodes at 30k against 52.0 ms.
  The check is new, so the load's own compile in the same process is the reference.
- **Undo record heap.** `measure_undo_record_heap` (release, heaptrack `--record-only`, 4
  workers): the T of the locality test on a sloped 1,024 m world, the record leaked
  (`METRUM_UNDO_HEAP=forget`) against dropped. One road edit's record keeps 2.26 MB alive with
  a 220 m trunk and 2.40 MB with a 660 m trunk (leaked 2.32 / 2.46 MB against 0.06 MB), mostly
  the replaced refined terrain entries of its eight patches; the trunk's length adds the
  replaced span piece of the split trunk. With the 30-record undo stack that is about 70 MB
  in the worst case. An upper bound: it also counts what the record shares with the city.
- **Size.** `roadbed/` production lines 7,831, as on `road` `9f092846`: this round changes no
  `roadbed/` code (the round-4 count of 7,807 was before gate G parts 3-6).
- **Open in part 1:** the terrain stage's budget and the idle-clock frame (gate D part 6;
  question 27), the 100k publication and snapshot (after gate D part 6). Payloads with
  crossings or signs (question 26) are met since `ROAD-68-F` part 1.

#### Part 2 (2026-10-25, branch `road68-E2`, work in progress)

The user's decisions on open questions 1, 2, 7, 11 (c) and 16, and the coordinator's on the new
question 30 (a), on `road` `3f24cff2` (D parts 1-6, G parts 1-6, E1 rounds 1-5). Questions 8
and 21 (b), the rest of 11 (c) and question 30's mid-span meetings are open for the next round.

- **Question 16.** `plan_road_edit` (`road_edit/support.rs`) is the one preparation and plan of a
  stroke for the commit, the preview and the editor's checks: a stroke reads no carriageway of a
  piece its own edit rebuilds, meets a road it splits at that road's graph profile and joins a
  node at the node's graph height (details under the question). Cost: a stroke that reads no
  carriageway plans once; a join or crossing plans twice (stage timings below).
  `sloped_junctions_do_not_depend_on_drawing_order` runs again on the T and, with question 30
  (a), the Y fixtures and passes; `a_crossing_of_a_raised_road_meets_it_on_top` is new; the
  cross and pair fixtures stay asserted but ignored under
  [open question 30](#road-68-open-questions) (mid-span meetings).
- **Question 30 (a).** A stroke from a road end no longer re-solves the road it extends; it joins
  the end like any node. The extension's code goes (`prepare_terminal_road_extension`,
  `RoadExtensionReprofile`, the validation corridor, the editor's re-profile, about 300 lines
  outside `roadbed/`). `a_stroke_from_a_road_end_joins_it_continuously_on_a_slope` is new; the
  visible change is described under the question.
- **Question 1.** The commit tests the pieces it installs against fields and painted cells, for
  the land the edit newly covers, and rolls the staged edit back on an overlap; the field and cell
  corridor checks are gone from the commit and plan readiness, the parcel corridor check stays.
  `a_field_in_a_junction_corner_refuses_the_road` is new (fails with the corridor checks); the
  field and painted-cell readiness tests now drive the commit and check that a refused commit
  leaves the graph, undo history and roadbed as they were.
- **Questions 2 and 11 (c).** Node moves, crosswalk overrides and edge class changes try their
  graph edit on the live graph, compile the closure into an `O(1)` copy of the roadbed and put
  the graph back from a local checkpoint; a failed node or newly covered reservation refuses the
  edit with nothing else touched. Re-enabled: both `cell_zoning::movement` tests,
  `bridge_entry_points_never_use_the_global_pool` (its save now settles the routing hold) and
  `zoning_cells_tool_test` in `run.sh`. The narrow-marks digest test's two painted-grid node moves
  are refused now and checked as refusals.
- **Question 7.** Confirmed on the preview path by
  `a_far_junction_keeps_the_near_mouth_pinned_on_the_preview_path`; no code change.
- **Product probe.** Question 16 changes only `y_25/true` (graph and every product but the
  layouts): its last branch no longer pins to the first branch's span beside the node. Question 30
  (a) then changes the five sloped fixtures whose second stroke starts at a road end
  (`y_25_branches_first`, `y_25`, `y_40`, `y_60`, `bend_45`; the same products). Re-recorded with
  that attribution, each change measured on a build without the next; the flat fixtures and the
  other sloped ones are unchanged.
- **Size.** `roadbed/` production lines: 7,881 (road 7,831): +50 for the per-piece carriageway
  visitor and the installed and replaced piece accessors the reservation checks read.
- **Fresh checks (2026-10-25).** Clippy `--all-targets` on the final tree: the identical
  59-warning set (file, message) as the baseline. Targeted debug suites during the round (each
  after the change it guards): `road_edit_order` 5 passed 1 ignored (after question 30 (a));
  `road_preview` 15 passed with the benchmarks included; `road_terrain_plan` 12 passed 2
  ignored (the Kuopio pair, question 8); `cell_zoning::movement` 4 passed; the narrow-marks
  digest test passes; the global-pool child passes; `zoning` 178 passed 18 ignored and `fields`,
  `agriculture`, `editing`, `road_commit`, `undo` passed before the newly-covered rule (the full
  suite below covers the final tree).
- **Stage timings (fresh, 06:42 EET).** `benchmark_road_preview_stages` as in part 1 (release,
  4 workers, P-cores, both timing locks), C = `cc192880` (the code of `road` before E1 round 5,
  `$BENCH_BINARY` of gate G part 6) as the same-session control, E = this round's release build
  on `4b6bdb8d`; C E E C, ms p50 / p95. Edit plan: `sloped_t` C 0.236-0.238 / 0.270-0.274, E
  0.461-0.464 / 0.517-0.531; `sloped_branch` C 0.208 / 0.235-0.250, E 0.402-0.404 /
  0.451-0.466. Both fixtures join or split existing roads, so every pose plans twice; the plan
  stays within its 1.0 ms budget. Roadbed and payload stages are unchanged (0.32 / 0.31-0.35);
  terrain 4.39-4.75 (C) against 4.53-4.79 (E), the open miss gate D part 6 owns. Not measured:
  the Godot stream with this library, and whether the first pass can stop before the profile
  solve (the rebuilt pieces need the solve's dirty edges today).
- **`./run.sh --test` (fresh, 07:08-07:24 EET, tree `08c13fc3` on `road` `3f24cff2`, DISPLAY and
  WAYLAND_DISPLAY unset, run lock, shared timing lock, build slot):** exit 0. Debug lib 1,897
  passed / 0 failed / 166 ignored (379 s); integration 4 passed; every Godot script PASS with no
  SCRIPT ERROR, `zoning_cells_tool_test` included; `run.sh` skips no script any more (only the
  display-only rendered tests stay outside it). Its tmpfs profile was deleted and
  `godot/**/*.import` restored. Ignores tagged for later gates: 4 `ROAD-68-E` (questions 8 and
  30), 2 `ROAD-68-F`.
- **Not run this round:** question 21 (b)'s measurement with gate G's harness at 100k (`road`
  only accepts the 100k commit since `3f24cff2`), the Godot stream with this library, and the
  commit-time cost of the reservation checks at 100k (part of the same harness run).

#### Part 2, round 2 (2026-10-25, branch `road68-E2`, work in progress)

On `road` `359406c1` (round 1 landed as WIP), rebased onto `road` `008a3280` (`TERRAIN-07`'s
plan and `ROAD-68-F` part 1) before the post-rebase checks below. Questions 11 (c), 30
(mid-span) and 8 (Kuopio save) are done; 8's recorded replay and hillside fixture, 25 (c) and
21 (b) are open.

- **Determinism and digests (fresh).** Gate G's `road68g_commit_determinism` (release, P-cores,
  both timing locks; workers 1 / 3 / 8, late rebuild at 1 and 8 workers, save in the hold): no
  difference in any variant on the control (`road` `3f24cff2`, before E2), on round 1
  (`359406c1`, 07:34) and on this round's final code (09:08); every reference digest is
  `6252b7ba71c5a6f1`, gate G's, so no digest changed. The breakdown's lot and generated-cell
  digests equal the control's in all 24 sample pairs of round 1 (07:35-07:56) and all 12 of the
  final code (one C F pair, 09:08-09:19). The narrow-marks digest test passes.
- **Question 11 (c), the rest.** Terrain edits re-solve every junction's profiles and roll back
  on a failed node; road bulldoze is tried first; undo asserts in debug builds that its restored
  closure compiles (details under the question).
- **Question 30, mid-span meetings.** A meeting node lies on the first-drawn road's graph
  profile, through `RegionGraph::profile_height_at`, now the one function for that height (the
  stroke support's copy moved there); order independence is not attainable under the crossing
  rule (details under the question). The product probe is unchanged.
- **Question 8.** The Kuopio reference save is regenerated and re-pinned, and its load test runs
  again as `kuopio_saved_reference_loads_as_saved`; the hillside houses run again without their
  terrain step; the terrain-pipeline halves of both are ignored under `TERRAIN-07` (coordinator,
  after the user's terrain-core decision); the recorded replay stays off with its exact reason
  (details under the question).
- **Question 21 (b) and 25 (c).** Measured, not started (details under the questions).
- **Tests.** New: `a_mid_span_meeting_lies_on_the_first_drawn_road` (failed before: the sloped
  cross met 17 mm off the first road), `a_terrain_edit_beside_a_sloped_junction_leaves_no_failed_node`
  (failed before: node 2 failed after a 2 m bump). Re-enabled: the Kuopio saved reference and
  the hillside houses (both without their refined-terrain halves, now `TERRAIN-07`).
  The commit's staged-render rejection detail now names the node error.
- **Fresh checks (2026-10-25).** Clippy `--all-targets`: the identical 59-warning set (file,
  message). `./run.sh --test` (08:46-09:06 EET, DISPLAY and WAYLAND_DISPLAY unset, run lock,
  shared timing lock, build slot; tree before the `TERRAIN-07` split): exit 0; debug lib 1,900
  passed / 0 failed / 165 ignored (448 s); integration 4 passed; every Godot script PASS, no
  SCRIPT ERROR, none skipped but the display-only shader-global probe and rendered vegetation
  tests; profile deleted, imports restored. After the split (test-only): `road_terrain_plan` 14
  passed 2 ignored, `zoning_buildability` 28 passed 1 ignored, and the full debug lib suite
  1,901 passed / 0 failed / 166 ignored (559 s, 09:22-09:31). `roadbed/` production lines: 7,881 (unchanged; this round's code is in the graph,
  topology, editing and undo modules).
- **After the rebase onto `road` `008a3280` (fresh, 09:48-10:07 EET).** Debug: the product probe
  and the narrow-marks digest test pass unchanged; `road_edit_order` 7 passed 1 ignored,
  `road_terrain_plan` 14 passed 2 ignored, `zoning_buildability` 28 passed 1 ignored,
  `road_preview` 13 passed 2 ignored. Clippy `--all-targets`: the identical 59-warning set.
  `roadbed/` production lines 7,972, equal to `road`'s (gate F part 1's growth; none from E2).
  `./run.sh --test` on the rebased tree (09:49-10:07, same settings as above): exit 0; debug lib
  1,904 passed / 0 failed / 165 ignored (509 s); integration 4 passed; every Godot script PASS,
  no SCRIPT ERROR, only the display-only tests skipped; profile deleted, imports restored.
- **Not done this round:** a C F F C re-measure of the commit (stopped for the rebase; one C F
  pair only, above), questions 25 (c) and 21 (b), the recorded Kuopio replay's `MouthOffPlane`
  and the hillside fixture's parcels.

#### Part 2, round 3 (2026-10-25, branch `road68-E3`, work in progress)

On `road` `0eabfe36` (round 2 landed as work in progress). Question 8's recorded replay and
hillside fixture, question 30's order test, question 25 (c) and the new question 31 are done;
question 21 (b) is not started, so the gate's final commit waits.

- **Plan excerpt (question 8).** `kuopio_05` attempt 14's `MouthOffPlane` was the edit plan's
  local excerpt holding one ring of incidence: the solve re-derives every edge at the nodes the
  edit touches and pins each one's far end from the far node's own solve, which needs all of
  that node's arms. The excerpt now takes two rings and a plan whose pinned nodes lack an arm is
  refused (details under the question; [Heights](#heights) "Scope and cost" states the 2-ring).
  The terrain spike's `city_grid_t` on 8% failed the same way. Both new tests fail with one
  ring. The product probe, the narrow-marks digest test and every reference fixture are
  unchanged: their excerpts already held the 2-ring.
- **Terrain spike, `t_cutfill` (question 11 (c)).** Confirmed fixed by round 2's terrain-sync
  re-solve (`an_8_percent_slope_under_a_flat_t_leaves_every_node_built`, fails without it).
- **Hillside fixture (question 8).** Regenerated with the load rebuild; parcels keep their ids
  and frontage points (details under the question).
- **Question 30.** The ignored mid-span order test is replaced by the decided rule; the slope's
  order test gains end-to-end joins (details under the question).
- **Question 25 (c).** The view shares the field index and a persistent cell-reservation index
  in `O(1)`; the preview runs `ReservedOverlap::first`, the commit's own test, and the road tool
  shows the refusal before the click (details under the question).
- **Question 31 (new; decided by the coordinator under the user's delegation, for review).**
  The recorded replay's quality limits failed on three profiles beside steep junctions
  (5.08-5.76 m over the ground against 5 m): the plane was sampled 12 m out inside wider
  mouths. It now samples at the mouth station where that lies beyond 12 m; the limits hold, and
  the probe changes only in the four sloped Y fixtures, re-recorded with that attribution
  (details under the question).
- **Question 21 (b).** Not started: the preview would need the whole cell store persistent
  (details under the question).
- **Tests.** New: `kuopio_recorded_strokes_commit_with_every_node_solved` (the recorded replay's
  road half; failed before at `kuopio_05` attempt 14), `a_street_grid_t_split_on_an_8_percent_slope_commits`
  (failed before: node 15 `MouthOffPlane`), `an_8_percent_slope_under_a_flat_t_leaves_every_node_built`
  (fails without round 2's re-solve), `the_reservation_index_follows_paint_erase_and_undo`; the
  three reservation refusal tests now assert the preview's reason first (by construction it was
  empty before: the preview had no overlap test). Split: `kuopio_recorded_profiles_preserve_quality_limits`
  (runs since question 31) and `kuopio_recorded_terrain_builds_clean` (`TERRAIN-07`). Removed:
  `sloped_mid_span_meetings_do_not_depend_on_drawing_order` (question 30).
- **Size.** `roadbed/` production lines 7,972, unchanged (this round's code is in the edit plan,
  the cell store, the allocator's field index and the road tool's view and preview).
- **Preview stage timings (fresh, 11:11 EET).** `benchmark_road_preview_stages` as in part 1
  (release, 4 workers, P-cores, both timing locks), C = `road` `0eabfe36` (the same-session
  control), E = this round's release build; C E E C, ms p50 / p95:

  | Stage (budget) | `sloped_t` C | `sloped_t` E | `sloped_branch` C | `sloped_branch` E |
  | --- | --- | --- | --- | --- |
  | Edit plan (1.0) | 0.447-0.463 / 0.523-0.552 | 0.467-0.468 / 0.551-0.560 | 0.384-0.394 / 0.460-0.479 | 0.399-0.406 / 0.470-0.486 |
  | Roadbed (0.5) | 0.305-0.317 / 0.358-0.376 | 0.321-0.322 / 0.370-0.377 | 0.310-0.315 / 0.362-0.373 | 0.313-0.321 / 0.372-0.378 |
  | Road chunks (1.0) | 0.354-0.361 / 0.423-0.432 | 0.371-0.374 / 0.446-0.448 | 0.340-0.350 / 0.396-0.424 | 0.359 / 0.428-0.429 |
  | Terrain patches (3.0) | 4.39-4.42 / 4.94-5.05 | 4.39-4.41 / 4.92-4.94 | 4.65-4.78 / 5.61-5.73 | 4.58-4.68 / 5.56-5.57 |

  The plan rises 0.005-0.02 ms (the excerpt's second ring) and the roadbed stage 0.005-0.015 ms
  (the preview's reservation test, which on these fixtures finds no reservation); every stage
  but terrain (question 29) stays within budget at p95.
- **Commit breakdown (fresh, 11:12-11:33 EET).** Gate G's
  `benchmark_populated_commit_breakdown`, `METRUM_COMMIT_TIERS=0,10000,30000,100000`, release,
  8 workers, P-cores, both timing locks, C and E as above, C E E C, three samples per tier and
  run. Lot and generated-cell digests equal C's in all 24 sample pairs. Medians of the six
  samples, ms, C / E:

  | Stage | 0 | 10k | 30k | 100k |
  | --- | --- | --- | --- | --- |
  | click → ack | 14.8 / 12.7 | 27.7 / 20.9 | 72.4 / 66.1 | 80.0 / 21.3 |
  | plan | 0.33 / 0.34 | 0.37 / 0.39 | 0.41 / 0.42 | 0.41 / 0.47 |
  | roadbed and terrain (`surface`) | 11.5 / 10.1 | 18.4 / 14.7 | 62.6 / 60.0 | 66.7 / 10.5 |
  | entrances and cell lots (`buildings`) | 0.00 | 4.40 / 4.07 | 4.14 / 2.56 | 5.28 / 3.21 |
  | render publication (`mesh`) | 2.23 / 1.87 | 3.37 / 0.86 | 3.13 / 1.27 | 2.16 / 2.04 |
  | dirty edges | 4 / 3 | 4 / 3 | 4 / 3 | 4 / 3 |

  Per sample at 100k, click → ack: C 121.2 / 79.7 / 22.3 and 119.1 / 80.2 / 22.6, E 131.8 /
  21.2 / 15.4 and 129.9 / 21.3 / 15.0. In samples 1 and 2 (the edits beside the grid) the
  one-ring plan re-derived a far link the edit does not change (it lost its far pin, a
  different profile, so a dirty edge), and that link's pieces and terrain were most of the
  `surface` stage beside the painted city (100k sample 1: 66-67 ms against 10.4-10.7 ms).
  Sample 0, the city's first commit (6 dirty edges in both), is slower in E: `surface` 82-97
  against 69-83 ms at 10k, 90-138 against 69-71 at 30k, 82-85 against 73-74 at 100k; not
  attributed yet (the edges the plan now pins at both ends carry their far supports, which
  one-ring plans dropped, and so may grade more road loop; to measure). The 100k commit's third
  sample (15.0-15.4 ms) is the first under 16.7 ms on `road`; the median (21.3 ms) still misses.
- **Determinism (fresh, 11:11-11:12 EET).** Gate G's `road68g_commit_determinism` (release,
  P-cores, both timing locks; workers 1 / 3 / 8, late rebuild at 1 and 8 workers, save in the
  hold): no difference in any variant, on the control and on this round. The reference digest
  changes from `6252b7ba71c5a6f1` (control) to `da3d23b6c8b63d20`, entirely by the plan-excerpt
  fix: this round's build with only `road_edit/validation.rs` put back to one ring gives
  `6252b7ba71c5a6f1` again (11:55). The one-ring plans left profiles a fresh solve would not
  give even on flat ground (the flat street grid's edge 9 lacked its far pin; the test fails
  with one ring), so the populated city's road edits now commit different, correct profiles.
  The reservation index, the shared field index and the preview's test change no output, nor
  does question 31's plane sample (rerun on the final code at 12:06: no difference,
  `da3d23b6c8b63d20`). The preview and commit timings above were taken before question 31's
  change; it adds one comparison per plane sample, leaves the city's flat edits and the sloped
  T's probe products unchanged, and was not re-timed (the orbiting branch poses may change).
- **Clippy** `--all-targets` (fresh, on the final tree): the identical 59-warning
  set (file, message) as the baseline.
- **`./run.sh --test` (fresh, DISPLAY and WAYLAND_DISPLAY unset, run lock, shared timing lock,
  build slot):** before question 31's change (11:32-11:50 EET) exit 0, debug lib 1,908 passed /
  0 failed / 165 ignored; on the final code (12:06-12:24) exit 0, debug lib 1,909 passed / 0
  failed / 164 ignored (449 s), integration 4 passed, every Godot script PASS, no SCRIPT ERROR;
  skipped only the display-only shader-global probe and rendered vegetation tests. Profiles
  deleted, imports restored. Ignores tagged for later gates: none `ROAD-68-E`, 3 `TERRAIN-07`,
  1 `ROAD-68-F`. The rebase onto `road` `8c6f89e7` (documentation only) changes no code.
- **Not done this round:** question 21 (b), the first-sample `surface` rise above (to
  attribute), and the Godot stream with this library.

#### Part 2, round 4 (2026-10-25, branch `road68-E4`, work in progress)

Part 1 of the round: round 3 rebased onto `road` `4dc466fc` (`ROAD-68-F` part 2's code
`8b2ad9eb` and the adopted `TERRAIN-07` specification); question 21 (b) follows in part 2.

- **Conflicts.** Only the documentation and the reference probe's table conflicted. Git merged
  the code: in `junction_profiles.rs` question 31's sample distance and part 2's pass-through
  rule by profile (`edges_pass_through`) are separate lines of the same solve, so a straight
  pair of two profiles now takes its corridor plane sampled at its mouth stations; zoning's
  `piece_covers` and the field index's `overlaps_road_piece` walk the piece's query grid (part 2)
  under round 3's reservation index, and the preview still calls the commit's own
  `ReservedOverlap::first`. Ignores: four `TERRAIN-07`, none `ROAD-68-E` or `ROAD-68-F`.
- **Product probes, attributed.** Each entry of the combined tree's tables against both
  parents' recordings, with a build of the combined tree with only question 31's sample
  reverted (`R31`), whose tables equal part 2's exactly (so round 3's plan excerpt, reservation
  index and preview test change no entry on part 2's families either):

  | Probe | Entry | Products changed against `road` | Cause |
  | --- | --- | --- | --- |
  | reference | `y_25_branches_first`, `y_40`, `y_60` (sloped) | graph, nodes, spans, walkways, lanes, queries | question 31 alone (as round 3 recorded) |
  | reference | `y_25/true` | graph, nodes, spans, walkways, lanes, queries | question 31; its lanes differ from both parents: part 2's minimum run and question 31 together |
  | reference | `y_25/false` | none | part 2 alone (its lanes, as part 2 recorded) |
  | node | `width_bridge_pass_through` (flat and sloped), sloped `width_pass_through`, `band_width_step_curved` | graph, nodes, spans, walkways, lanes, queries | question 31 on part 2's new families |
  | node | sloped `band_width_step` | graph, spans, lanes, queries | question 31 on part 2's new families |
  | node | sloped `rural_y_40` | graph, nodes, spans, lanes, queries | question 31 on part 2's new families |
  | node | `path_t` (flat and sloped) | none | part 2 alone (the minimum run, as part 2 recorded) |

  No layout changes; every other entry equals both recordings. Both tables are re-recorded
  with this attribution in their comments; the narrow-marks digest test passes unchanged.
- **Determinism (fresh, 13:06 and 13:12 EET).** Gate G's `road68g_commit_determinism`
  (release, P-cores, shared timing lock; workers 1 / 3 / 8, late rebuild at 1 and 8, save in
  the hold): no difference in any variant; digest `da3d23b6c8b63d20`, as round 3. Part 2's rules
  do not change it (`ROAD-68-F`'s timing run: `6252b7ba71c5a6f1` on both its control and part 2),
  nor does question 31 here (`R31` gives `da3d23b6c8b63d20` too): the populated city is flat,
  where a junction's plane does not depend on where its arms are sampled.
- **Clippy** `--all-targets` (fresh, 13:14 and 13:52): the baseline's 59-warning set (file,
  message), none new.
- **`./run.sh --test` (fresh, `DISPLAY` and `WAYLAND_DISPLAY` unset, run lock, shared timing
  lock, build slot).** First run (13:14-13:34): debug library 1,913 passed / 0 failed / 164
  ignored, integration 4 passed, then `network_tool_chunk_renderer_test` failed once: "hover
  validation keeps its prepared profile" (`validate_road_candidate_with_snap` on a blank flat
  world, right after four valid `preview_road` calls on the same stroke), and run.sh stopped.
  The same library's script alone passed three times out of three (13:51), and a second full
  run (13:52-14:09) exits 0: debug library 1,913 / 0 / 164 (458 s), integration 4, every Godot
  script PASS, no SCRIPT ERROR; skipped only the display-only shader-global probe and the
  rendered vegetation tests. Both parents passed this script in their gate runs; the failure
  is intermittent and not reproduced, so its cause is open. The assertion now reports the
  validation's reason, so the next occurrence names it. Profiles deleted, imports restored.
- **Size.** `roadbed/` production lines 7,798, as `ROAD-68-F` part 2 (round 3 changed none).

Part 2 of the round (question 21 (b) and the first-commit attribution):

- **Persistent cell store.** `CellStore`'s maps (`frame_ids`, `blocks`, `chunks`, `frontages`,
  `curve_sources`) are `imbl` hash maps on the same fixed hasher and `frames` an `imbl` vector,
  behind the same API, so a copy of the store is `O(1)` (bar the edits' stale-chunk list) and
  shares every block until either side writes it. The road tool's view holds the store
  instead of round 3's mirror index, which is removed with its test; `ReservedOverlap::first`
  reads the store's own reserved masks in the commit and the preview. Lot sizes and the
  explicit-site index are not in the view yet: nothing reads them until the preview derives
  lots. The user decided (question 21) that the preview never shows the lots it derives; the
  preview's payload stays road chunks and terrain patches.
- **Commit sub-stages.** `surface` is split into the roadbed install (compile or preview install
  and the undo capture), the field and cell overlap test and the terrain validation
  (`surface_roadbed_ms`, `surface_overlap_ms`, `surface_terrain_ms` in `RoadEditMetrics` and
  gate G's breakdown).
- **Timing (fresh, 14:28-15:00 EET).** Release library tests, P-cores, both timing locks, no
  restic. P = `road` `4dc466fc` (before round 3), C = `road68-E4` `096df250` (part 1, the
  control), each with only the sub-stage patch; E = this part's code. Preview stages (`benchmark_road_preview_stages`, 4 workers, P C E E
  C P), ms p50 / p95:

  | Stage (budget) | `sloped_t` P | `sloped_t` C | `sloped_t` E | `sloped_branch` P | `sloped_branch` C | `sloped_branch` E |
  | --- | --- | --- | --- | --- | --- | --- |
  | Edit plan (1.0) | 0.435-0.439 / 0.553-0.554 | 0.467-0.474 / 0.546-0.549 | 0.469-0.470 / 0.526-0.531 | 0.364-0.411 / 0.473-0.482 | 0.404-0.408 / 0.456-0.460 | 0.403-0.404 / 0.455-0.461 |
  | Roadbed (0.5) | 0.298-0.306 / 0.362-0.368 | 0.322-0.326 / 0.361-0.367 | 0.325-0.326 / 0.363-0.373 | 0.310-0.323 / 0.376-0.378 | 0.324-0.326 / 0.370-0.379 | 0.318-0.322 / 0.373-0.377 |
  | Road chunks (1.0) | 0.413-0.418 / 0.483-0.501 | 0.374-0.376 / 0.444-0.446 | 0.403-0.405 / 0.457-0.468 | 0.406-0.415 / 0.481-0.493 | 0.357-0.360 / 0.415-0.432 | 0.399-0.402 / 0.456-0.459 |
  | Terrain patches (3.0) | 4.39-4.41 / 4.98-5.00 | 4.62-4.67 / 5.23 | 4.64-4.66 / 5.20-5.27 | 4.60-4.71 / 5.58-5.68 | 4.84-4.88 / 5.81-5.88 | 4.88-4.92 / 5.92-5.99 |

  With question 31's code (P to C) the plan rises up to 0.04 ms (round 3's 2-ring excerpt),
  the roadbed stage 0.01-0.02 ms and the terrain stage 0.21-0.25 ms; round 3 measured no
  terrain change before question 31's sample, so that rise follows the new planes on the
  orbiting poses (not split by a build). The road chunk stage runs no changed code; its
  0.03-0.05 ms between builds is a build-to-build difference (C against C: 0.002 ms). E
  against C: no stage beyond C against C but the road chunks. Every stage but terrain (question
  29, `TERRAIN-07`) is within budget at p95.
- **Commit breakdown (same session).** `benchmark_populated_commit_breakdown`,
  `METRUM_COMMIT_TIERS=0,10000,30000,100000`, 8 workers, P C E E C P, three samples per tier
  and run; medians of the six samples, ms, P / C / E:

  | Stage | 0 | 10k | 30k | 100k |
  | --- | --- | --- | --- | --- |
  | click → ack | 14.54 / 12.74 / 12.83 | 27.83 / 21.22 / 21.96 | 73.65 / 67.26 / 67.31 | 78.40 / 22.01 / 22.78 |
  | plan | 0.32 / 0.35 / 0.35 | 0.39 / 0.40 / 0.41 | 0.38 / 0.47 / 0.47 | 0.43 / 0.51 / 0.53 |
  | `surface` | 11.64 / 10.37 / 10.39 | 18.12 / 15.17 / 14.88 | 63.22 / 60.03 / 60.34 | 65.28 / 11.06 / 11.17 |
  | of which roadbed install | 0.56 / 0.44 / 0.43 | 0.96 / 0.62 / 0.64 | 0.84 / 0.65 / 0.65 | 0.85 / 0.72 / 0.59 |
  | of which overlap test | 0.00 / 0.00 / 0.00 | 0.04 / 0.02 / 0.03 | 0.03 / 0.01 / 0.01 | 0.04 / 0.01 / 0.02 |
  | of which terrain validation | 11.08 / 9.92 / 9.95 | 17.11 / 14.52 / 14.21 | 62.35 / 59.36 / 59.67 | 64.39 / 10.33 / 10.56 |
  | `buildings` (entrances, cell lots) | 0.00 | 4.57 / 3.90 / 4.83 | 4.16 / 3.30 / 3.38 | 4.85 / 3.24 / 3.94 |
  | `mesh` | 2.27 / 1.77 / 1.69 | 3.55 / 0.86 / 0.88 | 3.62 / 1.49 / 1.32 | 2.53 / 2.05 / 2.08 |
  | tool snapshot | 0.03 / 0.03 / 0.03 | 0.28 / 0.24 / 0.26 | 0.86 / 0.77 / 0.69 | 2.80 / 2.72 / 2.74 |

  Lot and generated-cell digests are equal across all six runs in every tier and sample;
  determinism (C and E, 14:28-14:29): no difference, `da3d23b6c8b63d20`.
- **Sample 0's `surface` rise (round 3), attributed to the sub-stage.** P against C, each run:
  at 10k the terrain validation takes 77.4 / 77.9 against 97.9 / 95.5 ms, the roadbed install
  1.4 / 1.4 against 1.3 / 1.4 and the overlap test 0.09-0.12 against 0.09; at 30k 77.4 / 68.6
  against 96.8 / 137.6 ms of terrain validation, at 100k 71.7 / 69.6 against 81.0 / 82.0, on
  open terrain 11.2 / 10.7 against 13.8 / 13.9, with the install and overlap test unchanged.
  The whole rise is the commit's legacy terrain validation (`validate_staged_road_terrain`,
  refined patches over the same 6 dirty edges), which `TERRAIN-07` replaces; per the round's
  brief it is recorded here and not pursued. Samples 1 and 2 stay far below P (round 3's
  excerpt fix).
- **The persistent store's cost, open.** In 8 of 9 tier samples both E runs take longer in
  `buildings` than both C runs, by 0.3-2.0 ms (per sample, C / E: 10k 10.7, 11.4 / 12.7, 12.7,
  4.1, 3.7 / 5.0, 4.7 and 1.1, 1.0 / 1.6, 1.3; 30k 7.9, 7.7 / 9.6, 9.3 and 1.5, 1.5 / 2.2, 2.0;
  100k 9.3, 8.6 / 10.0, 10.0, 3.2, 3.3 / 4.2, 3.7 and 2.3, 2.2 / 2.7, 2.8; 30k's second sample
  overlaps), beyond C against C (0.06-0.71 ms); the overlap test moves by
  0.01-0.03 ms and the tool snapshot not at all. An unshared regeneration
  (`benchmark_populated_cell_preparation`, 10k, A P P A on the shared timing lock beside other
  work, so an indication only) was not slower (p50 3.29, 2.17 against 2.03, 1.84 ms), which
  points at copy-on-write: the view holds the previous store, so the commit's first write to
  each block copies its map path, and a block entry carries the whole block (656 bytes), a
  chunk entry its block list. Not attributed by a build yet; the mitigations to measure are
  shared blocks (`Arc<CellBlock>`, so a path copy moves pointers and a write copies one block)
  and the same for chunk entries. Until then this part regresses the commit and is not ready
  to land.
- **Tests and checks.** Targeted debug suites on this part (13:26): zoning 129 / 0 / 12
  ignored, save 41 / 0 / 3, road preview 14 / 0 / 2, zoning buildability 28 / 0 / 1, fields
  15 / 0 / 2, editing 26 / 0, narrow-marks digest passes, both probes give part 1's products.
- **Gate checks on this part's tree (fresh, 15:01-15:43).** Clippy `--all-targets`: the
  baseline's 59-warning set, none new. `./run.sh --test` (same conditions as part 1): exit 0,
  debug library 1,912 passed / 0 failed / 164 ignored (the removed mirror-index test is the one
  fewer), integration 4, every Godot script PASS (the hover assertion included), no SCRIPT
  ERROR; skipped only the display-only shader-global probe and the rendered vegetation tests.
  No `ROAD-68-E` ignore; four `TERRAIN-07`. `roadbed/` production lines 7,798, unchanged.

#### Part 2, round 5 (2026-10-25, branch `road68-E5`, work in progress)

Round 4's part 2 on `road` `ebe4381e` (documentation only, so the rebase is empty): the
`buildings` regression first, then the first half of question 21 (b).

- **The `buildings` regression, attributed (fresh, 16:37-17:26 EET).** Gate G's breakdown
  (`benchmark_populated_commit_breakdown`, `METRUM_COMMIT_TIERS=0,10000,30000,100000`, release
  library tests, 8 workers, P-cores, both timing locks, no restic), one session, order C E A B P
  P B A E C: C = round 4's control (`096df250` with the sub-stage patch), E = round 4's part 2
  (`76b4a7e1`, persistent store), A = E with the road tool's view holding an empty store (so
  nothing shares the live store and nothing is copied on write), B = E with the store's blocks,
  chunk entries and frontage lists behind `Arc` and every store write reading first
  (`cf681907`), P = B with the lot planner below (`6f006989`). The first E run failed at
  start-up (its binary's worktree had been removed; the coordinator restored it), so E has one
  run. `buildings`, ms per sample (10k, 30k, 100k; samples 0 / 1 / 2):

  | Run | 10k | 30k | 100k |
  | --- | --- | --- | --- |
  | C | 11.77 / 3.79 / 1.06, 12.12 / 4.12 / 1.05 | 7.76 / 2.57 / 1.53, 7.88 / 3.06 / 1.45 | 8.23 / 3.11 / 2.24, 8.90 / 3.36 / 2.33 |
  | E | 12.53 / 4.70 / 1.35 | 9.47 / 3.12 / 2.00 | 10.39 / 3.72 / 2.62 |
  | A | 12.47 / 4.42 / 1.17, 12.42 / 4.31 / 1.21 | 9.49 / 3.15 / 1.85, 9.20 / 3.43 / 1.83 | 9.81 / 3.74 / 2.49, 9.58 / 3.68 / 2.57 |
  | B | 12.06 / 4.29 / 1.43, 11.90 / 4.39 / 1.14 | 9.60 / 2.77 / 1.77, 8.20 / 2.89 / 1.83 | 9.71 / 3.18 / 3.31, 9.26 / 3.35 / 2.46 |
  | P | 12.47 / 4.35 / 1.43, 11.77 / 4.25 / 1.23 | 9.23 / 3.09 / 1.80, 8.76 / 3.06 / 1.76 | 9.45 / 3.59 / 2.61, 9.84 / 3.56 / 2.41 |

  C against C: 0.01-0.67 ms. A, which shares nothing, is as slow as E: the regression is not
  copy-on-write after publication but the persistent maps themselves, on paths the commit
  takes whether or not the view holds the store. `Arc` values (B) change nothing measurable
  (they stay: a shared path now copies pointers, not 656-byte blocks), and the planner split
  (P) costs nothing beyond B. Every other stage moves within C against C; lot and generated
  digests are equal in every tier and sample of all nine runs, and gate G's determinism (B,
  P) shows no difference, `da3d23b6c8b63d20`. Preview stages (4 workers, C B P P B C): plan, roadbed and
  terrain within C against C; road chunks +0.03-0.04 ms as in round 4 (a stage that runs no
  changed code), within budget.
- **Generation's temporary index in plain maps, measured and dropped (fresh, 17:34-17:56).**
  Cell generation built its candidate index as a fresh `CellStore`, insert-heavy under `imbl`;
  R = P with that index in plain hash maps (a shared block visitor, same visit order). Same
  harness, C R R C: `buildings` per sample, C 11.31 / 3.83 / 1.00, 8.18 / 2.71 / 1.60,
  8.55 / 3.15 / 2.47 and 10.68 / 3.98 / 1.27, 8.28 / 2.42 / 1.61, 8.36 / 3.16 / 2.05; R
  13.00 / 4.72 / 1.16, 9.00 / 2.91 / 1.81, 9.27 / 3.37 / 3.02 and 12.01 / 4.46 / 1.27,
  8.37 / 2.81 / 1.79, 9.73 / 3.34 / 3.11 (10k, 30k, 100k). No gain, so the change is not
  kept; digests equal C's in every sample and determinism is `da3d23b6c8b63d20`. The cost is
  the live store's own persistent reads and writes in generation and derivation; where
  exactly is the profile the user decided on first ([question 32](#road-68-open-questions)).
- **Profile of the commit's cell-lot work (question 32's first step; fresh, 18:14-18:17, both
  locks, P-cores, 8 workers, a profile, not a timing).** C against P at 10k and 30k, `perf`
  toggled by the harness around each commit, with the commit's `economy_phases` laps
  (`METRUM_COMMIT_PHASES`). Per commit, `buildings` C / P: 12.26 / 13.60, 4.54 / 4.88,
  1.10 / 1.41 (10k) and 8.63 / 9.88, 2.68 / 3.10, 1.61 / 1.91 ms (30k). Of the first
  samples' 1.3 ms, lot generation takes 1.10 and 1.01 ms more and derivation 0.24 and 0.25;
  inside generation the extra is spread over every lap (publication 0.16 / 0.30, candidate
  indexing 0.16 / 0.26, the rest over preparation, clearance, conflicts and support), each
  the persistent maps' lookups and writes on its own path. No single cut of about 1 ms
  exists, so under the user's decision question 32 goes to (b), next round.
- **One lot planner and one installer (question 21 (b), first half).** Every lot derivation
  (the commit, the `cell lots` queue, the tests' entries) runs on `CellLotPlanner`: cell
  generation and lot claims on a cell store and lot queue it changes in place, against a parcel
  store it only reads through `PlannedParcels`, which answers reads as the store would after
  the plan's own retirements, reattachments and new lots and records each as the store call it
  stands for (`insert_new`, `set_cell_lot`, `remove_local`, `replace_geometry`).
  `ZoningSystem::install_cell_lot_plan` makes those calls in order on the store the plan read
  and advances the overlay revision as the plan did. Generation reads only authored parcels,
  which a plan never changes, so it reads the plan's base. Bitwise: lot and generated digests
  equal C's in every sample (above), determinism unchanged, the narrow-marks digest test and
  both probes pass, and a replay test (`a_planned_change_list_replays_to_the_directly_changed_store`)
  checks the plan's reads against the directly changed store and every index and revision of
  the replay. The commit plans on the live store, so the split changes no cost (P against B).
- **Gate checks on this round's tree (`6f006989` plus documentation; fresh).** Clippy
  `--all-targets` (17:55): the baseline's 59-warning set, none new. `./run.sh --test`
  (17:56-18:14, `DISPLAY` and `WAYLAND_DISPLAY` unset, run lock, shared timing lock, build
  slot): exit 0, debug library 1,913 passed / 0 failed / 164 ignored (the replay test is the
  one more), integration 4 passed, all 20 Godot scripts PASS (the hover assertion included),
  no SCRIPT ERROR; skipped only the display-only shader-global probe and the rendered
  vegetation tests. No `ROAD-68-E` or `ROAD-68-F` ignore; four `TERRAIN-07`. `roadbed/` is
  untouched (7,798 production lines, as round 4). Profile and imports cleaned up.
- **The hover-validation flake (`network_tool_chunk_renderer_test`, round 4) — open.** Not
  reproduced this round: the gate's run (`./run.sh --test`, 18:14) and a standalone run in
  the isolated profile layout (18:23) pass. The 20-run loop the brief asks for queued behind
  another gate's run lock at the handoff; its remaining runs are the next agent's to read, and
  the assertion prints the validation's reason if it recurs.
- **Question 21 (b), second half: not started.** The preview's plan needs, besides question
  32's store: a parcel view the planner can read off the simulation thread (the planner reads
  through `PlannedParcels`; its base is the live `ParcelStore`, which a view copies in
  `O(parcels)`), the lot sizes (`Arc`) and the explicit-site index in the view, the road edit's
  lot marks (`mark_cell_lots_for_road_edit`) on the plan's copies, and the commit's install of a
  plan whose base is still the live state (otherwise the commit plans itself). The test the
  round's brief asks for (commits with and without a matching preview plan, lot, generated and
  narrow-marks digests compared) comes with it.

#### Part 2, round 6 (2026-10-25, branch `road68-E6`, work in progress)

Round 5 on `road` `4710f59b` (one documentation conflict). Question 32 (b), as the user decided;
the preview half of question 21 (b) is paused by the user ([question 33](#road-68-open-questions)).

- **Plain live store, journal-synced view copy (question 32 (b)).** `CellStore<M: CellMaps>`
  (`zoning/cells/store/maps.rs`): the live store keeps plain hash maps on the fixed hasher
  (`LiveMaps`, as before round 4), the road tool's view a persistent copy (`SharedMaps`:
  `imbl` maps with values behind `Arc`, frames in an `imbl` vector), cloned in `O(1)`. Every
  store write records its key in the store's journal (blocks, chunk entries, frontage lists,
  curve sources; consecutive repeats once, sorted and deduplicated whenever the log doubles).
  Each publication copies or removes exactly the journaled entries, appends the frames
  registered since and shares the already persistent road alignments (`CellStore::follow`,
  `O(written keys * log entries)`); a clear or a whole-world invalidation copies the store
  whole, as does a publication that replaces the view. Generation, lot derivation,
  `CellLotPlanner`, the `cell lots` queue's pop and the reserved-overlap test are each one
  function generic over the kind; generation's temporary candidate index is always plain.
  Round 3's reservation mirror is removed. Debug tests compare the copy's whole state (frames,
  blocks, chunk entries, frontages, sources, alignments, counts, revision) with the live store
  at every publication, and
  `a_plan_on_the_views_persistent_copy_equals_the_live_plan_and_the_copy_follows_it` plans on
  both kinds and checks equal reports, parcel changes and digests, then the follow and a clear.
- **Commit breakdown (fresh, 19:31-19:48, both locks, P-cores, 8 workers, release library
  tests, no restic), C = `road` `4710f59b`, E = this round (`db992aa0`'s code), C E E C.**
  `buildings`, ms per sample (samples 0 / 1 / 2):

  | Run | 10k | 30k | 100k |
  | --- | --- | --- | --- |
  | C | 12.21 / 3.72 / 1.05, 11.84 / 3.79 / 0.98 | 8.16 / 2.54 / 1.51, 7.81 / 2.56 / 1.57 | 8.44 / 3.19 / 2.37, 8.38 / 3.10 / 2.20 |
  | E | 10.70 / 4.40 / 1.06, 11.08 / 4.20 / 1.00 | 7.97 / 2.51 / 1.50, 7.75 / 2.58 / 1.50 | 8.42 / 3.08 / 2.12, 8.95 / 3.23 / 2.09 |

  Lot and generated digests are equal in every tier and sample; gate G's determinism on E
  shows no difference, `da3d23b6c8b63d20`. Medians C / E: 3.75 / 4.30 (10k), 2.55 / 2.55
  (30k), 3.14 / 3.16 ms (100k). Outside C's spread: 10k sample 1 (+0.4-0.7 ms, the sample
  rounds 4 and 5 found slow with persistent maps too) and one 100k first sample (+0.5 ms);
  10k sample 0 is 0.8-1.5 ms faster. The tool snapshot (the publication) costs the follow:
  samples 1-2, C 0.23-0.25 / 0.66-0.72 / 2.53-2.73 ms, E 0.32-0.35 / 0.77-0.82 / 2.63-2.95 ms
  (10k / 30k / 100k). Each first sample replaced the view in this harness, which seeds the
  shared view without the publisher, and so took E's whole copy (+2.3 / +6.7 / +36 ms); the
  copy takes 2.4 / 7.5 / 30 ms at 10k / 30k / 100k. The commit's `surface` stage is
  8-10 ms slower at 30k and 100k in E's first two samples, all of it in the legacy terrain
  validation (E's sub-stages: roadbed 0.5-1.4 ms, overlap 0.01-0.1 ms), which this round does
  not change; the same stage moved by 7-8 ms with the crate's codegen partition before
  ([question 36](#road-68-open-questions)) and is `TERRAIN-07`'s.
- **Harness: the first view through the publisher.** (fresh, 20:26-20:48, same harness, locks and
  workers; C2 = `road` + this harness change, E2 = this round + it, as `f49f24ec`; C2 E2 E2 C2).
  The harness seeded the shared view without the publisher, so each tier's first commit
  replaced the view and carried the whole city build's cell journal; the game publishes its
  first view at start-up, and the harness now does too. `buildings`, ms per sample:

  | Run | 10k | 30k | 100k |
  | --- | --- | --- | --- |
  | C2 | 11.17 / 3.77 / 1.01, 11.48 / 3.89 / 1.05 | 7.78 / 2.68 / 1.62, 8.59 / 2.84 / 1.84 | 8.60 / 3.61 / 2.15, 8.75 / 3.67 / 2.14 |
  | E2 | 11.34 / 3.95 / 1.05, 10.82 / 3.64 / 1.02 | 8.42 / 2.57 / 1.61, 7.91 / 2.48 / 1.78 | 8.53 / 3.11 / 2.10, 8.75 / 3.12 / 2.00 |

  Medians C2 / E2: 3.83 / 3.80 (10k), 2.76 / 2.53 (30k), 3.64 / 3.11 ms (100k). Every E2
  sample lies within C2's range or below it, except one 10k second sample 0.06 ms above C2's
  higher run (C2's two runs differ by 0.12 ms there): the slow 10k second sample of rounds 4-6
  came from the harness's first commit. Lot and generated digests are equal in every tier and
  sample. `surface` is within C2's range: timing 1's 8-10 ms moved with the build's partition
  (code layout, [question 36](#road-68-open-questions); no product or work changed).
  - **The follow's cost, in the tool snapshot (samples 1-2 and the first):** C2 0.23-0.31 /
    0.67-0.79 / 2.54-2.96 ms, E2 0.30-0.42 / 0.79-0.90 (one 1.63) / 2.60-3.02 ms at 10k / 30k
    / 100k: about 0.1 ms at 10k and 30k, within C2's range at 100k. It is the copy of the
    journaled entries the decision prescribes; not yet reduced (it could overlap the
    publication's graph records and roadbed snapshot on the pool).
  - **Render publication (`mesh`, no changed code):** medians 0.94 / 1.05, 1.60 / 1.76 and
    2.21 / 2.48 ms (C2 / E2), with E2 samples up to 0.4 ms above C2's range at 30k and 100k;
    timing 1 showed the same at 10k. Unattributed; needs a profile.
  - **The view copy at 100k residents:** 175,055 entries; 16.5 MB of keys, values and lists,
    about 20.7 MB with each entry's pointer and `Arc` header (map nodes not counted). The
    resident set grew 1.9 MB while one more copy was alive (the allocator reuses freed pages,
    so this is a floor). A whole copy takes 26-31 ms; only a load or a replaced view makes one.
- **Preview stages (fresh, 19:26, 4 workers, C E E C).** Plan and terrain within C's range;
  roadbed p50 0.322 / 0.325-0.333 ms (`sloped_t`) and 0.324-0.327 / 0.329-0.331 ms
  (`sloped_branch`), payload 0.386-0.394 / 0.391-0.411 and 0.381 / 0.390-0.393 ms, C / E; the
  pose total 5.83 / 5.72-5.84 and 5.97-6.00 / 5.98-6.00 ms. The fixtures paint no cells, so
  the overlap test returns at its reservation check in both builds. A profile (21:12, both locks, P-cores, 4
  workers, the whole benchmark at 4 kHz, user cycles and instructions, C against E) puts the
  difference in inlining, not work: E calls out of line what C inlines (the chain clip
  `rect_loops::clip_path` and `roadbed::terrain` iterator closures out of `terrain_loops`,
  `emit_band` and `emit_chain_strips` out of `emit_piece_surfaces`, `rstar`'s point min/max
  helpers, `spade`'s circumference test), each moved function's samples reappearing under its
  caller in the other build; the roadbed functions' instructions differ by 1.0% and the render
  ones' by 2.0%, the calls' own overhead. The fixtures plan no lots and test no reservation, so
  this round runs no changed code in either stage: code layout under
  [question 36](#road-68-open-questions)'s rule.
- **Gate checks (fresh, on `7a8d1241`).** Clippy `--all-targets` (19:53): the baseline's
  59-warning set, none new. `./run.sh --test` (19:53-20:19, `DISPLAY` and `WAYLAND_DISPLAY`
  unset): exit 0, debug library 1,915 passed / 0 failed / 164 ignored, integration 4 passed,
  every Godot script PASS, no SCRIPT ERROR; skipped only the display-only shader-global probe
  and the rendered vegetation tests. No `ROAD-68-E` or `ROAD-68-F` ignore; four `TERRAIN-07`.
  `roadbed/` has no change against `road`.
- **The hover-validation flake, root-caused.** Round 5's 20-run loop failed 2 of 20 runs, both
  at "hover validation keeps its prepared profile" with no reason. The hover path never waits
  for the simulation lock (`try_lock_core`): while the sim thread holds it the answer is
  pending (`is_pending`, not valid, no reason), and the test read that answer as invalid. The
  road tool already asks again next frame; the test now does too (`_await_hover_validation`).
  The 20-run loop on this round's library (20:19-21:11, run.sh's isolated profile layout)
  passes 20 of 20. Closed. A hover answer can still be pending while the simulation holds its
  lock, by design: the corridor's parcel clearance reads the live core (unlike `ROAD-54`'s
  preview checks, a hover candidate cannot be evaluated ahead); a view-side parcel copy would
  remove that, and waits with question 21 (b)'s preview half.

#### Part 2, round 7 (2026-10-25, branch `road68-E7`)

Round 6 on `road` `4710f59b` (no conflict). The two commit stages round 6 left above their
controls, then a regression the round found in the city setup.

- **The cell copy follows beside the commit's graph copy.** The commit's publication now makes
  the routing rebuild's graph copy itself (`publish_committed_road_tool_view`; the view and the
  rebuild share it, as before) and runs the view's cell-copy follow beside it in one
  `rayon::join` through `thread_pools::install`, inline on the commit's pool worker; the graph
  record capture, the roadbed snapshot and any terrain or water copy run on the copy's side.
  Neither side waits for a pure job (gate G part 5's rule). A publication without a city-sized
  copy (a paint, an unchanged graph) runs both sides in turn on its own thread instead of
  entering the pool. The follow is unchanged, `O(journaled entries × log entries)`, now hidden
  behind the `O(city)` graph copy; debug tests still compare the copy's whole state with the
  live store at every publication. Undo and road bulldoze still offer their routing copy.
- **Commit breakdown, session 1 (fresh, 21:56-22:18, both locks, P-cores, 8 workers, release
  library tests, no restic; C2 = `road` + round 6's harness fix, E7 = `a16e7b17`; C2 E7 E7
  C2).** Gate G's determinism on E7: no difference, `da3d23b6c8b63d20`; lot and generated
  digests equal in every tier and sample. Tool snapshot medians C2 / E7: 0.26 / 0.25 (10k),
  0.69 / 0.70 (30k), 2.73 / 2.77 ms (100k), against round 6's +0.06-0.16 ms; every E7 sample lies
  within 0.02 ms of C2's range but one 100k second sample (+0.09 ms; C2's two runs differ by
  0.01 ms there, round 6's by 0.14). `buildings` and `mesh` medians are within 0.15 ms of C2's
  (`mesh` 0.86 / 1.01, 1.53 / 1.51, 2.31 / 2.23 ms); single samples of one E7 run lie up to
  1.0 ms (`buildings`, 10k first) and 0.3 ms (`mesh`) above C2's two runs, as do single samples of
  stages no round of gate E changed (lanes +0.46 ms and agent invalidation +0.50 ms at 100k, plan
  +0.13 ms on the bare grid): the session's noise floor for one sample.
- **Render publication profiled (22:18-22:38, both locks, P-cores, 8 workers, the commit windows
  at 30k and 100k, `perf record` 8 kHz user cycles and instructions with LBR call chains, and
  `perf stat`).** Samples whose call chain passes through `precompute_road_mesh_data` or the
  road render emission: instructions C2 1.992e8, E7 2.005e8 (+0.7%, within the sampling
  error), cycles 1.220e8 and 1.088e8 (E7 11% fewer in this run). The leaf functions differ
  only by inlining: samples move between `push_triangle_to_layer_with_crossing` and
  `push_layer_triangle`, `triangle_height_at_xz` and `LaneSurfaceQuery::height_at`,
  `emit_piece_surfaces` and `for_each_strip_triangle` / `emit_into`, the street-light chain
  closure and `for_each_crossing` / `place_road_signs`. The stage runs no changed code on equal
  inputs (equal digests), so its work is identical: code layout under
  [question 36](#road-68-open-questions). Whole windows: user instructions 12.280e9 / 12.366e9
  (+0.7%: the routing rebuild's pure job runs on the pool during the window, `flow_cut` +95 M),
  user cycles 4.811e9 / 4.730e9, page faults 33,861 / 31,425.
- **Preview stages (fresh, 21:56, 4 workers, C E7 E7 C):** plan, payload and terrain within C's
  range or below (terrain 4.39-4.42 against 4.62-4.68 ms on `sloped_t`); roadbed on
  `sloped_branch` 0.332-0.334 against 0.322-0.328 ms. Profile (22:55, both locks, P-cores, the
  whole benchmark at 4 kHz): user instructions over the run 169.70e9 (C) and 167.48e9 (E7),
  cycles 66.1e9 and 64.0e9; the roadbed functions' instructions -0.6% and cycles +3.2%, with
  `rect_loops::clip_path` called out of line from `terrain_loops` as in round 6: identical work,
  layout (question 36). No preview stage runs code this round or round 6 changed.
- **Lot plans were quadratic in their new lots (round 5's planner).** Every breakdown run of the
  round 5-7 builds took 20-30 s longer than its control's, outside the timed commits. Whole-run
  profiles of one tier (22:41 and 23:00, user cycles, LBR): 20.9 / 21.2 s at 10k, 62.9 / 65.7 s
  at 30k (C2 / E7), the difference in `PlannedParcels::overlaps_existing` (1.72e9 cycles at 30k, absent in
  C2), `rectangles_overlap_on_axes` (0.19e9 to 4.05e9) and `rectangles_overlap_geometry`
  (0.07e9 to 1.42e9). The plan's overlay tested every parcel the plan had changed on each
  overlap query and bounds visit, so a plan over a populated city's queued regions (the setup's
  preparation; in the game, a load, a large paint or a busy hourly pass) cost
  `O(new lots²)`; a commit plans few lots, which is why `buildings` never showed it. The overlay
  now indexes its changed parcels by the parcel store's 512 m chunk, under every chunk of each
  bounds they had in the plan, and each read checks the parcel's present bounds and visits it
  from its first queried chunk, as the store's own visit does; visits keep their order (the
  store's unchanged parcels, then the changed ones by id). A query costs the store's own plus
  `O(log changes)` per changed parcel in its chunks. A new test moves a planned parcel across a
  chunk line and compares visits and overlaps with the directly changed store.
- **Session 2 on the final code (fresh, 23:45-00:06, same locks, cores and workers; E8 =
  `879ffe10`; C E8 E8 C preview stages, C2 E8 E8 C2 breakdown).** Determinism: no difference,
  `da3d23b6c8b63d20`; lot and generated digests equal C2's in every tier and sample, so the
  overlay's index changes no product. A whole four-tier run takes 307.1 / 306.5 s (E8) against
  307.8 / 308.0 s (C2); session 1's E7 runs took 341 / 337 s against 310 / 309 s. Medians C2 /
  E8, ms:

  | Stage | 10k | 30k | 100k |
  | --- | --- | --- | --- |
  | `buildings` | 4.19 / 3.86 | 2.51 / 2.58 | 3.18 / 3.17 |
  | tool snapshot | 0.25 / 0.25 | 0.69 / 0.71 | 2.76 / 2.73 |
  | `mesh` | 0.96 / 1.05 | 1.40 / 1.47 | 2.29 / 2.22 |

  Single E8 samples above C2's two runs: `buildings` 30k second +0.11 ms (the other run +0.01),
  100k first +0.08-0.25 ms (C2's runs 0.52 ms apart); tool snapshot 100k first +0.20 ms in one
  run, otherwise at most +0.03; `mesh` 10k third +0.07-0.09 ms in both runs (C2's runs 0.01 ms
  apart; session 1's C2 0.80-0.84 and E7 0.85 / 1.13), one 30k third +0.13 and one 100k first
  +0.35 ms. Unchanged stages move as much (plan +0.10-0.15 ms and add +0.13 ms at 100k).
- **E8's preview payload and render code, profiled.** Preview stages (23:45): plan, roadbed and
  terrain within C's range or below (terrain 4.40 against 4.66-4.71 ms on `sloped_t`), payload
  0.421-0.434 against 0.387-0.388 ms (`sloped_t`) and 0.422 against 0.384-0.396 ms
  (`sloped_branch`), which E7 with the same render source did not show (0.382-0.387). The stage
  runs no code changed since `road` (E8 changes only lot plans, which the benchmark never
  makes). Fixed-period instruction samples (00:20, every 2 M user instructions, unthrottled;
  totals 169.5-169.6e9 for C and 163.2-163.7e9 for E8, as `perf stat` counts): the render
  functions execute 3.30 / 3.38e9 (C, two runs) and 3.32 / 3.37e9 (E8) instructions, identical
  work; E8's machine code for the same source differs (`push_triangle_to_layer_with_crossing`
  4,846 bytes in C and E7, 3,014 in E8, with `NetworkMeshData::new` called out of line), and
  the legacy terrain CDT's code moved too (E8 executes 3.5% fewer instructions over the run).
  Code layout under [question 36](#road-68-open-questions); the render publication's `mesh`
  runs the same functions, and session 1's commit-window profile of E7 showed identical work
  there.
- **E8's render publication at 10k, profiled (00:25, both locks, P-cores, 8 workers, the three
  commit windows, as session 1's profile):** samples through the render publication execute
  9.47e7 (C2) and 9.13e7 (E8) user instructions in 4.96e7 and 4.88e7 cycles; the leaf functions
  differ by inlining only (`for_each_strip_triangle`, `road_signs::place` and
  `emit_piece_surfaces` appear out of line in C2 alone). Identical work: the 10k third sample's
  `mesh` difference is code layout under question 36, as at 30k and 100k in session 1.
- **Gate checks (fresh, on `879ffe10`).** Clippy `--all-targets` (00:09): the baseline's
  59-warning set, none new (also on `a16e7b17`, 21:48 and 23:03). `./run.sh --test` (00:09-00:25,
  `DISPLAY` and `WAYLAND_DISPLAY` unset): exit 0, debug library 1,916 passed / 0 failed / 164
  ignored (round 6 1,915 plus the new overlay test), integration 4 passed, every Godot script
  PASS, no SCRIPT ERROR; skipped only the display-only shader-global probe and the rendered
  vegetation tests. Targeted debug suites on E7 and E8 (zoning, save, road preview, fields,
  editing, buildings, simulation node, both probes) pass with round 6's counts (zoning 132 with
  the new test). No `ROAD-68` ignore remains; four `TERRAIN-07`. `roadbed/` has no change
  against `road` (7,943 non-blank lines outside test modules).
- **Acceptance.** Every commit stage's median is at its control's (within 0.1 ms) and the tool
  snapshot's follow cost is gone; the samples above the control's two runs are single-run
  excursions also seen in unchanged stages, and the two stages that rose consistently without a
  code change (render publication, preview payload) do identical work in profiles, with
  products bitwise equal: code layout under question 36. Gate E lands with this round. Open:
  the preview's terrain stage budget (`TERRAIN-07`), question 21 (b)'s paused preview half
  ([question 33](#road-68-open-questions)) and the 100k commit's frame (question 21, measured
  again with `TERRAIN-07`'s terrain validation).
- **Landing on `road` (2026-10-26, branch `road68-E8`).** Rounds 4-7 rebased onto `road`
  `5f2c5fbf` (`ROAD-68-K14`'s maximum grade and Kuopio re-record, question 20's attribution,
  `TERRAIN-07-A`). Only `roadmap.md`'s status paragraph conflicted, resolved keeping both
  sides. The code merged without a conflict and needed no semantic resolution: `editing.rs`
  holds E's surface sub-stage timers beside K14's grade refusal in `try_graph_edit`, and the
  commit breakdown E's first view through the publisher and cell-copy report beside
  `TERRAIN-07-A`'s graded census and tiles. Fresh on the rebased tree: clippy `--all-targets`
  the baseline's 59-warning set, none new; `./run.sh --test` (01:00-01:18, `DISPLAY` and
  `WAYLAND_DISPLAY` unset) exit 0, debug library 1,931 passed / 0 failed / 173 ignored,
  integration 4 passed, every Godot script PASS, no SCRIPT ERROR, skipped only the shader-global
  probe and the rendered vegetation tests; both product probes and the narrow-marks test pass;
  gate G's determinism (release, 8 P-cores): no difference, `da3d23b6c8b63d20`, unchanged by
  K14's grade rule. Disabled: four `TERRAIN-07` and the two Kuopio quality tests under
  `ROAD-68-K14` (question 39). `roadbed/` has no change against `road`. No timing was re-run:
  no code conflict needed a semantic resolution.

### `ROAD-68-F` — remaining families

- footpaths: footpath-only nodes, and footpaths meeting roads ([D5](#product-decisions)): done
  early, [pre-D](#footpath-junctions-and-the-bend-crossing-fallback-pre-d)
- roads without sidewalks, and mixed widths and profiles, including the D4 taper: done in
  [part 2](#remaining-families-road-68-f-part-2-2026-10-25)
- bridges: decks through the existing bridge renderer (part 2); abutment cutouts moved to
  [`TERRAIN-07`](roadmap.md) (user, 2026-10-25)
- tunnels: road pieces inside the bore (part 2); portals, the terrain side of `ROAD-61`, moved to
  `TERRAIN-07`
- crossings: done early, in the [`ROAD-68-C` follow-up](#tighter-corners-and-junction-crosswalks-follow-up)
  (crossings and sidewalk corners from the layout; `rounded_sidewalk_corner_path_xz` removed).
  Footpath nodes have layouts, crossings and corner joins since the pre-D footpath junctions
- lane markings at junctions, street lights and road signs
- building sites, driveways and paving: moved to `TERRAIN-07`
- zoning, agriculture and vegetation overlap queries: part 2

Acceptance:

- focused fixtures for each family
- the [crossing contract](#pedestrian-junction-crossings): a zebra marking and its pedestrian
  connectors share one kerb segment
- deterministic street-light and sign placement
- the building frontage tests are re-enabled

Met (part 2 round 4, 2026-10-25): each family has its fixtures (the pre-D footpath junctions,
part 2's 48 family fixtures and its family chunks on the reference harness); the crossing
contract holds bitwise and lights and signs are placed from the pieces since part 1 (a preview
publishes its chunks bitwise; `road68g_commit_determinism` is equal across worker counts and
rebuild delays); no building frontage test is disabled (`ROAD-68-D` re-enabled the 13
`building_site_terrain` frontage tests and the yard test, and none is ignored on the round 4
tree). What moved to `TERRAIN-07` and what stays open is in the [gate table](#implementation-plan-road-68).

#### Markings, lights and signs from the layout (`ROAD-68-F` part 1, 2026-10-25)

Decision (coordinator under the user's delegation, 2026-10-25, for review): the markings part of
gate F starts now, in parallel with gate E part 2's last items, because gate E's own acceptance
(open question 26: preview payloads with crossings or signs equal the committed ones) needs it.

- **One emission for preview and commit.** `render/road/mod.rs` `emit_into` emits the pieces'
  surfaces, bridges, lane markings, zebra paint frames, street lights and signs from the graph,
  the pieces and the terrain only; the lane system is no longer an input
  (`generate_mesh_data_with_surface`, `TransitRenderer::generate_mesh_data` and the private
  `emit_chunks` lost it). The commit's publication still lifts the rebuilt pieces' lanes first,
  for agents. The preview (`generate_preview_chunks`) calls the same emission and splice, so its
  chunks are the commit's ([open question 26](#road-68-open-questions), closed).
- **One crossing record.** `RoadbedSystem::for_each_crossing(graph, node)` visits a node's
  crossings in its pieces: a node piece's layout kerbs (`NodePiece::crosswalk_kerbs`), or at a
  pass-through node the span-owned crossing's kerbs, which the span piece now keeps from its
  own mesh row on the crossing line (`SpanPiece::crosswalk_kerbs`). Lanes compute the same
  kerbs with the same functions (`crossing_kerbs` on the node plane; the span row's
  carriageway-edge vertices through `crossing_kerbs_right_first`, now shared by
  `walkways::span_walk_arm` and the span piece). `CrosswalkMarking::of_arm` orients a crossing
  as the lanes own it (from the positive lateral side's kerb, the sidewalk with the lower lane
  index when driving on the right, as before), and `pedestrian_junctions` now takes its
  ownership from the same rule (`starts_left`). `crosswalks::for_each_marking` composes the two;
  zebra paint (`prepare_crosswalks`) and crossing signs (`place_road_signs`) read it. Cost per
  node: `O(degree)` lookups in the pieces instead of a scan of the node's connector lanes.
- **Crossing contract.** `zebra_markings_and_their_connectors_share_one_kerb_segment`
  (`render/road/crosswalks.rs`): on the T, the bend, a pass-through node and every footpath
  fixture, flat and sloped, each node's painted markings equal its lanes' markings bitwise, ends
  in order, and both connectors of each walk exactly that kerb segment.
- **Street lights.** Placement stays a pure function of the pieces and the canonical chain
  (deterministic; whole-street order). A road end now measures its clearance from its layout
  furniture station, the start of the round cap (D2), which is the node: the arm runs straight
  from there to its mouth (`MIN_MOUTH_STATION_M` = 1 m, more where an attached footpath holds
  it out), so the margin from the span end is `6 m - mouth station`, at least zero. Before, the
  2 m dead-end margin ran from the span end, 3 m from the node, beside the turning cap.
  Junctions keep 6 m from their mouths, family changes at degree-two nodes and road ends without
  a node piece keep 2 m from the span end. Re-enabled:
  `a_residential_street_gets_evenly_spaced_lights_on_one_side_behind_the_kerb` (failed on `road`:
  first light at -57.0 m of the street's -60 m end, now -54.0 m, gaps 27 m).
- **The faded zebra stripes (notes for gate F).** Not z-fighting or a decal overlap: nothing
  overlaps since part 5 of gate D (the paint is a vertex frame on the asphalt, one triangle
  layer). The faded bars are the shader's studded-tyre wear (`fr_zebra_paint`, `ROAD-56`), in
  the wheel paths 0.9 m either side of each lane centre and scaled by the crossing's cosmetic
  age. Evidence: the same two bars (the third and fifth of seven, 2.25-2.75 m and 4.25-4.75 m
  from the kerb, where the inner wheel paths at 2.65 and 4.35 m fall) fade on the flat and the
  sloped Y alike; the stem's crossing (age 0.96) and the upper branch's (0.71) wear, the lower
  branch's (0.29) is crisp. Kept as designed ([open question 35](#road-68-open-questions)).
- **Building frontage tests.** All re-enabled by `ROAD-68-D` (the 13 `building_site_terrain`
  frontage tests and the yard test); none belongs to markings. The one `ROAD-68-F` test still
  disabled, the tunnel portal CDT test, belongs to the tunnel part (the node-move tests went to
  gate E part 2, which re-enabled them).
- **Timing (fresh, 2026-10-25).** Release library tests, `taskset -c 0,2,4,6,8,10,12,14`, both
  timing locks, C F F C in one session: C = `road` `3f24cff2`, F = this part on `3f24cff2`,
  built in one target directory. Native preview stages (`benchmark_road_preview_stages`, 4
  workers), payload packing p50, C / F: `sloped_t` 0.375-0.384 / 0.349-0.350 ms, `sloped_branch`
  0.366-0.367 / 0.338-0.340 ms (budget 1.0); plan, roadbed and terrain unchanged. So emitting
  zebra frames, lights and signs in the preview costs nothing measurable. Gate G's commit
  breakdown (8 workers, three samples per tier, medians): render publication (`mesh`) at 0 / 10k
  / 30k residents C 2.72-2.35 / 3.68-4.00 / 3.47-3.68 ms, F 2.14-1.92 / 3.59-3.35 / 3.27-3.22 ms
  (piece lookups replace the scans of each node's connector lanes); lots and generated digests
  equal at every tier. The commit's `surface` stage (roadbed install and terrain validation,
  which this part does not change; equal products) measured 7-8 ms slower at 30k in F (samples
  2-3: C 63.3 / 55.8 and 63.5 / 53.9 ms, F 71.3 / 61.0 and 70.2 / 62.4 ms; same in an earlier
  pair against gate E's binary of `4b6bdb8d`), not at 0 or 10k; two runs of one binary differ
  by under 2 ms. A profile of both binaries' 30k commit windows (`perf record` toggled by the
  harness, flat, `perf diff`) located it in code this part does not touch: the site paving
  R-tree query (`SitePavingPartition::partition`) recomputes each leaf's envelope (a fold over
  the region polygon) at every visit, and C inlines that fold into rstar's selection loop while
  F calls `PavingRegion::envelope` out of line (+7.2% of samples) along with rstar's point
  min/max helpers (+2.7%); the inlined loop's own share fell by 5.8%. A codegen accident of the
  crate's partitioning, not work this part adds. Left to gate D, which owns the site paving
  partition (the coordinator, 2026-10-25: D part 8 has a paving prefilter in flight and the
  exact terrain cut may be replaced): computing each region's envelope once would make the leaf
  test independent of inlining; not measured here.
- **Size.** `roadbed/` production lines 7,972 against 7,881 on `road` `359406c1` (+91): the
  crossing record (`for_each_crossing`, 49 lines with its contract), the span piece's crossing
  kerbs and `crossing_kerbs_right_first` (shared with the walkways), mostly doc comments.
- **Gate run** (fresh, 2026-10-25, the part's final sources, `DISPLAY` and `WAYLAND_DISPLAY`
  unset): `./run.sh --test` exits 0. Debug library suite 1,900 passed, 0 failed, 165 ignored;
  integration 4 passed; every Godot bridge suite passes with no SCRIPT ERROR, none skipped.
  `ROAD-68-F` ignores left: the tunnel portal boundary test, whose terrain side moves to
  [`TERRAIN-07`](roadmap.md) with bridge abutments, building sites, driveways and paving.

#### Remaining families (`ROAD-68-F` part 2, 2026-10-25)

Scope after [`TERRAIN-07`](roadmap.md) (user, 2026-10-25): this part keeps roads without
sidewalks, mixed profiles, bridge decks, tunnel road pieces and the overlap queries; abutment and
portal cutouts (the terrain side of `ROAD-61`), building sites, driveways and paving are
`TERRAIN-07`'s.

- **Fixtures (tests first).** `roadbed/node/tests/families.rs`, flat and on the 2.5% slope:
  roads without sidewalks (`rural_t`, `rural_four_way`, `rural_y_40`, `rural_bend_90`,
  `rural_bend_150`, `rural_side_road`, `rural_through_road`, `rural_wide_t`, `rural_bend_mixed`),
  mixed profiles across straight sectors (`band_step`, `band_width_step`, `band_four_way`, both
  steps again with the changing arm curving away, and the straight degree-two changes
  `band_pass_through`, `width_pass_through`, `width_bridge_pass_through`), and bridges and tunnels
  (`bridge_t`, `bridge_bend`, `bridge_pass_through`, `bridge_rural_t`, `tunnel_t`, `tunnel_bend`,
  `tunnel_pass_through`; structures climb or descend 6 m over their arm). Each fixture compiles
  without a failed node; every span is watertight; every node piece is watertight against its
  spans on shared mouth edges, on its plane bitwise, and free of slivers, stray curb faces and
  chain steps (the `ROAD-68-C` and D4 checks, now helpers); only car roads with sidewalks cross
  (D8); a structure end without a continuation has no node piece; a pass-through pair shares
  one mouth row. All 48 are product-probe entries. On the reference harness
  (`nodes::sim::core::tests::road_families`): a bridge and a tunnel as a T's side road (node
  piece) and as a road end's continuation (span mouth), made by `set_edge_class_internal`, and a
  T with its side road or all three roads without sidewalks: the published road chunks, bridge
  concrete included, are watertight by `ROAD-68-D`'s chunk check, except the structure's free-end
  row and the concrete underside's two ends, which the abutment or portal closes (`TERRAIN-07`).
- **Roads without sidewalks (D6).** Layouts, outer chains and the crossing rule hold as specified;
  no arm without sidewalks crosses. One defect: on a T of such roads the side arm's two corners
  end 6.7e-7 m apart (the arm direction's f32 noise), and with no crossing strip the mouth sat on
  the farther corner, leaving a sliver run on the other side (core and shoulder strips). Fixed by
  the minimum run ([mouth stations](#junction-layout)): every run from its reach to the mouth is
  empty or at least `s_min`. It changes the stem of the footpath-only `path_t` (the same shape)
  and a near-straight bend's arm without a crossing (167.5 degrees: corner 0.77 m out, mouth 1 m,
  now 1.77 m), which only `y_25`'s intermediate state has. No road tool authors a road without
  sidewalks (strokes always carry them); saves may. Open question 17's item on footpaths in such
  sectors stays open: one road without sidewalks withdraws the walking proof on its network
  whatever its junctions do (question 17).
- **Mixed widths and profiles.** The D4 taper on curved arms and band-only changes passes the
  taper checks on the new fixtures. A band-only change (sidewalks against shoulders) has no
  carriageway width change and takes the 12 m minimum taper; its boundaries are compared bitwise,
  which is exact because both profiles come from one function. Defect found: a straight
  degree-two node where a road with sidewalks continues into one without was a pass-through,
  whose two spans evaluate rows of 8 and 6 vertices, so the node left a hole; a straight width
  change at a bridge (the span taper skipped structures) was the same. A pass-through now needs
  one profile (`arms::edges_pass_through`, used by layouts, span ends and the profile solve, which
  takes the corridor plane for such a pair, as for a pass-through with attached footpaths); any
  other straight pair is laid out with D4 tapers and one crossing. The span model's legacy width
  taper only ever ran at those pairs, so it is removed (with its section cadence and its test,
  `pass_through_width_change_sections_taper_wider_endpoint`; `width_pass_through` covers the
  case); no other probe entry changes.
- **Bridge decks.** A bridge is a span piece and, where it meets a road at a junction or bend, an
  arm of its node piece; the existing renderer (`standard_surface/bridge.rs`) emits its concrete
  underside and piers from the span's sections, unchanged. Its ends meet grounded roads at a node
  piece (`bridge_t`, `bridge_bend`) or a shared span mouth (`bridge_pass_through`), watertight.
  Found: a bridge stroke that joins a road is rejected at the commit with `render_geometry_invalid
  roadbed_chain_open_end` (open question 19), so bridges cannot be built from a road in play until
  `TERRAIN-07` closes those chains; the renderer also leaves the deck's sides open between the
  deck edge and the slab (no fascia), visible from the side. Closed by `TERRAIN-07-B`'s fascias,
  which the slab now shares its vertices with ([`terrain.md`](terrain.md#terrain-07-b--road-skirts-in-the-piece-mesh)).
- **Tunnel road pieces.** Tunnels use the same pieces with full decks inside the bore; the legacy
  visible-range clipping does not return, since the portal and its terrain cut are `TERRAIN-07`'s.
  `cdt_promotes_high_delta_omitted_tunnel_source_to_retaining_wall` is relabelled `TERRAIN-07`.
- **Overlap queries.** One triangle walk (`PieceMesh::any_top_triangle_in_bounds`, through the
  piece's own query grid) serves every overlap query: the roadbed's own
  (`any_top_triangle_in_bounds`, `any_band_near_disc`, which fields and cells use when they are
  placed or generated), zoning's test of a piece against painted cells (`piece_covers`) and the
  field test of a piece (`FieldClearance::overlaps_road_piece`, now the fields in the piece's
  chunks against the piece's triangles in each field's bounds, `O(fields there * their cells'
  triangles)`, instead of every top triangle against the field index). So fields and cells are
  refused on a road piece, junction corners and caps included, in both directions, on the
  existing indices. Vegetation: a plant's position never lies on a road piece (its footprint's
  centre sample is a roadbed query) and trees keep 3 m from walkways by the disc query; an exact
  test of the whole sampled footprint was tried and dropped, since the canopy's 6 m footprint is
  a relief footprint and would keep trees 6 m off footpaths, against the tested 3 m rule
  (`vegetation_footpath_clearance_catches_gaps_between_footprint_samples`).
  `overlap_walk_visits_every_triangle_a_rectangle_meets` checks the walk on every family piece.
- **Probe re-records (attributed in the tables).** Node probe: the 48 family entries are new and
  `path_t` changes (the minimum run); nothing else. Reference probe: `y_25`'s lanes only (the
  lanes its 167.5 degree intermediate bend builds keep their ids in the final Y).
- **Size.** `roadbed/` production lines 7,798 against 7,972 on `road` `0eabfe36` (-174): the span
  model loses 199 lines with its taper (and `RoadProfile` its width override, 10); the minimum
  run (13), the pass-through rule (9) and the triangle walk (17) are mostly doc comments.
  Round 4 changes no `roadbed/` line (7,798 on its rebased tree): its fix is in
  `render/road/chunks.rs` (+3) and `terrain/rect_clip.rs` (+15, the second buffer).
- **Gate run** (fresh, 2026-10-25, the part's final sources, `DISPLAY` and `WAYLAND_DISPLAY`
  unset): `./run.sh --test` exits 0. Debug library suite 1,908 passed, 0 failed, 165 ignored;
  integration 4 passed; every Godot bridge suite passes with no SCRIPT ERROR, none skipped but
  the display-only shader-global probe and rendered vegetation tests. Clippy `--all-targets`:
  the baseline's 59 warnings, none new; `cargo doc`: no missing docs. `ROAD-68-F` ignores left:
  none (the tunnel portal test is `TERRAIN-07`'s).
- **Gate run, round 4** (fresh, 2026-10-25, the fix rebased onto `096df250`, the code of
  `931fbce7`; `road` moved by docs only since; `DISPLAY` and `WAYLAND_DISPLAY` unset): `./run.sh --test` exits 0. Debug library suite 1,913 passed, 0
  failed, 164 ignored; integration 4 passed; every Godot bridge suite passes with no SCRIPT
  ERROR, none skipped but the display-only shader-global probe and rendered vegetation tests.
  Clippy `--all-targets`: the baseline's 59 warnings, the same set; `cargo doc`: no missing docs.
  `ROAD-68-F` ignores left: none; no building frontage test is ignored.
- **Timing (round 3, 2026-10-25): a payload regression.** Release library tests, `taskset -c
  0,2,4,6,8,10,12,14`, both timing locks, C F F C in one session (no restic running at start or
  end): C = `road` `8c6f89e7`, F = `8b2ad9eb` (this part), two binaries built from those
  sources in one target directory (identities in `$ARTIFACTS`). Native preview stages
  (`benchmark_road_preview_stages`, 4 workers), p50 ms, C / F, samples C1 C2 against F1 F2:
  `sloped_t` plan 0.464, 0.465 / 0.470, 0.471; roadbed 0.322, 0.320 / 0.326, 0.321; terrain
  4.427, 4.385 / 4.413, 4.440; payload packing 0.351, 0.346 / 0.415, 0.408. `sloped_branch` plan
  0.407, 0.404 / 0.408, 0.405; roadbed 0.323, 0.321 / 0.324, 0.326; terrain 4.708, 4.725 /
  4.720, 4.701; payload packing 0.336, 0.335 / 0.392, 0.389. **Payload packing is 0.06 ms
  (+16-18%) slower in F, against a C-against-C spread of 0.005 ms**; plan, roadbed and terrain
  are within the spread. Within the 1.0 ms payload budget, but beyond the gate rule, so gate F
  stayed open until round 4 found the cause (below). `road68g_commit_determinism`: reference digest `6252b7ba71c5a6f1` in C and F, every
  difference null (the probe re-records of part 2 do not reach the populated fixture). Commit
  breakdown (`benchmark_populated_commit_breakdown`, tiers 0 / 10k / 30k / 100k, 8 workers,
  medians of three samples per run, C F F C): lots and generated digests equal in every run and
  tier. `commit_ms` C 15.06, 15.02 / F 14.81, 15.30 at 0; 27.78, 28.76 / 27.33, 27.36 at 10k;
  71.16, 72.84 / 73.42, 74.82 at 30k; 82.57, 81.42 / 80.91, 81.01 at 100k. The `surface` stage
  at 30k: C 61.69, 63.21 / F 63.74, 64.35 (F +1.1 to +2.7 ms over the C pair's 1.5 ms spread;
  at 10k and 100k within the spread; the same 30k surface offset as part 1's note on the site
  paving query's inlining). Other stages (plan, add, lanes, mesh, buildings, finalize, snapshot)
  within the C-against-C spread at every tier.
- **Cause of the payload regression (round 4, profiled 2026-10-25).** `perf record` (LBR call
  graphs, `cpu_core` cycles, 12 kHz, the stages benchmark at 4 workers on the P-cores, C F F C with
  round 3's binaries) puts the extra samples under `emit_piece_surfaces` (inclusive C 3,133 /
  3,111, F 3,850 / 3,859; the expected 0.06 ms x 960 previews is about 690 samples), in the
  per-triangle chunk routing whose sources part 2 did not touch (`render/road/chunks.rs`,
  `terrain/rect_clip.rs`): F compiles `NetworkMeshChunkPartition::route` out of line (3,936
  bytes; C inlines it into its only caller, `push_triangle_to_layer_with_crossing`), and
  `clip_triangle_to_rect` copies its by-value 456-byte polygon passes with four `memcpy` calls
  instead of two (libc samples under it: C 169, F 419; the unchanged function is 2,079 bytes in
  C and 2,291 in F). Part 2's added code moved rustc's codegen-unit partition of the release
  test binary, so the router's inlining and the clip's copy elision changed: the accident part 1
  met in the site paving query. The work is the same:
  a scratch digest of every payload buffer, light and sign of the stages fixtures' eight poses,
  their commits and the three preview-equals-commit fixtures flat and sloped (32 digests) is
  equal in C, F and the fix. Not the coordinator's lead: `edges_pass_through` is not on the
  payload path, and over the whole benchmark `RoadProfile::for_edge` takes fewer samples in F
  (23-36 per run) than C's per-section width-override profiles did (50-59).
- **Fix (round 4).** `route` is `#[inline]`, so its one instantiation is compiled beside its
  caller in that caller's codegen unit, and `clip_triangle_to_rect` clips between two buffers in
  place and fans the result to a closure, so no polygon is returned or copied by value: neither
  is left to the optimiser's view of the partition. No product changes: the 32 digests are equal
  before and after; the in-place clip fans the by-value passes' faces bitwise on 400,000
  generated triangle and rectangle pairs (260,531 faces, vertices on and off the lines); the
  node and reference probes, the narrow-marks digest, the chunk watertightness tests and the
  preview-equals-commit tests pass. Built on C's sources (Cx) the router is inlined and the clip
  makes no copy as well, so the fix does not depend on the partition.
- **Timing (fresh, round 4, 2026-10-25).** Release library tests, `taskset -c
  0,2,4,6,8,10,12,14`, both timing locks, one session 15:02-15:24 EET (no restic at start or
  end): C = `8c6f89e7` (round 3's binary), F' = the fix on `4dc466fc` (round 3's sources, before
  this round's rebase onto `096df250`), Cx = C's sources plus the fix; identities in `$ARTIFACTS`.
  Preview stages (4 workers), p50 ms, C1 C2 / F'1 F'2 / Cx1 Cx2: `sloped_t` payload 0.351, 0.357
  / 0.349, 0.333 / 0.343, 0.342; plan 0.461, 0.460 / 0.459, 0.464 / 0.464, 0.455; roadbed 0.316,
  0.317 / 0.320, 0.322 / 0.321, 0.316; terrain 4.447, 4.432 / 4.350, 4.301 / 4.331, 4.363.
  `sloped_branch` payload 0.334, 0.347 / 0.326, 0.325 / 0.331, 0.334; plan 0.401, 0.409 / 0.406,
  0.408 / 0.401, 0.407; roadbed 0.317, 0.324 / 0.323, 0.325 / 0.318, 0.324; terrain 4.687,
  4.743 / 4.607, 4.598 / 4.624, 4.598. Payload packing is now at or below C on both fixtures
  (C-against-C spread 0.006 and 0.013 ms). A second run of the same binaries in one
  session (15:46-15:48 EET; C and F' alternated four times, Cx twice), p50 ranges, C / F' / Cx:
  `sloped_t` plan 0.459-0.466 / 0.468-0.474 / 0.464-0.470, roadbed 0.313-0.324 / 0.324-0.331 /
  0.322-0.325, payload 0.348-0.357 / 0.337-0.356 / 0.338-0.340, terrain 4.397-4.472 /
  4.332-4.439 / 4.345-4.379; `sloped_branch` plan 0.405-0.409 / 0.406-0.419 / 0.403-0.407,
  roadbed 0.320-0.326 / 0.323-0.329 / 0.322-0.325, payload 0.334-0.348 / 0.324-0.342 /
  0.324-0.325, terrain 4.700-4.760 / 4.572-4.672 / 4.582-4.596. F''s medians differ from C's
  by at most +0.008 ms (roadbed, `sloped_t`, against C's 0.011 ms spread) and +0.004 ms (plan,
  against 0.007), so every preview stage is within C's spread or faster. Plan and roadbed sit
  1-2.5% above C's median in both sessions, not attributed: Cx, with C's plan and roadbed
  code, sits 0.003-0.005 ms above C on `sloped_t` too. On the rebased tree (the fix on
  `road` `096df250`, Fr) against `096df250` itself (R: part 2 unfixed, with `ROAD-68-E` round 4),
  in the same session (R Fr Fr R R Fr): payload `sloped_t` R 0.386-0.392 / Fr 0.337-0.348,
  `sloped_branch` 0.381-0.393 / 0.331-0.360, so the fix holds there (and the scratch digests of
  R and Fr are equal); plan and roadbed within R's range; terrain R 4.351-4.406 / Fr
  4.583-4.629 and 4.647-4.654 / 4.799-4.902, 0.23 ms (5%) above R in code both share, a
  partition accident in the legacy terrain CDT ([open question 36](#road-68-open-questions)).
  `road68g_commit_determinism`: digest `6252b7ba71c5a6f1` in C and F', every difference null.
  Commit breakdown (tiers 0 / 10k / 30k / 100k, 8 workers, medians of three samples, C1 C2 /
  F'1 F'2): lots and generated digests equal in every run, tier and sample; `commit_ms` 15.99,
  14.55 / 15.81, 14.18; 28.11, 28.98 / 27.73, 27.84; 94.55, 77.14 / 74.73, 72.37; 80.85, 79.15 /
  77.61, 78.52; `surface` 12.54, 11.52 / 11.79, 11.12; 18.65, 18.88 / 18.37, 18.19; 70.41, 66.08
  / 64.63, 63.08; 66.44, 66.75 / 64.50, 65.63. Round 3's 30k `surface` offset (F +1.1 to +2.7 ms)
  does not reproduce: F' is at or below C at every tier. The breakdown does not separate the
  roadbed install from the legacy terrain validation inside `surface` (the sub-stages come with
  `ROAD-68-E` round 4 part 2), so the offset is not attributed between them here; the site paving
  clip the validation runs is one of the two functions the fix changes. Other stages are within
  the C-against-C spread or faster, except `mesh` at 0 (C 1.96, 2.22 / F' 2.45, 2.29 ms, one
  spread above; faster than C at 10k, 30k and 100k).

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

#### Implementation (`ROAD-68-G`, 2026-10-24)

D7 as built ([pure jobs](simulation.md#pure-jobs),
[routing contract](entrance_and_exit.md#routing-after-a-topology-edit-road-68-g-d7)):

- `SimCore::commit_road` no longer builds the CCH. Once the road-tool snapshot exists it starts
  `TransitNetwork::schedule_cch_rebuild` on the snapshot's `Arc<RegionGraph>`, so the commit
  copies the graph once. The job builds the CCH and runs the connectivity log on the shared pool.
- `step_frame` holds for `CCH_ADOPTION_FRAMES` (6 here; 1 since
  [part 2](#local-commit-publication-road-68-g-part-2-2026-10-24)) frames, paused frames
  included: it returns before the flow fields, the agent tick and the clock. The seventh frame
  adopts the result, waiting for it if late, stamps a fresh build generation, and runs normally.
  The flow fields refresh there because the generation moved.
- Undo and road bulldoze schedule the same job on a copy of the live graph. Node moves, lane
  connection edits, edge class changes and border connections still built synchronously on
  Godot's main thread; part 2 converted them with the same call.
- A second edit inside the hold supersedes the job (an unstarted one never runs) and restarts
  the hold. A save installs the result and ends the hold (`save_game_settled`); load drops it.
- `RoadEditMetrics` gained `undo_ms`, `topology_ms` and `zoning_ms`, the parts of `add_ms`;
  `routing_ms` now times only the scheduling.

**Commit breakdown** (fresh, 2026-10-24). Release library tests of the control (`c59955e6`
with the harness, binary `abc892d7…`, synchronous CCH) and of this gate (`13d4ec6f…`), in one
session, pinned to the P-cores with eight workers, under the exclusive timing lock with no build
or backup running:

```bash
METRUM_COMMIT_TIERS=0,2000,10000,30000,100000 RAYON_NUM_THREADS=8 METRUM_DEBUG=0 \
  taskset -c 0,2,4,6,8,10,12,14 "$TEST_BINARY" \
  nodes::sim::core::populated_city::routing_benchmark::benchmark_populated_commit_breakdown \
  --exact --ignored --nocapture
```

Each tier is the populated city at that many residents (0: the street grid alone); each of three
samples commits a 120 m road branching off a border trunk west of the grid, a T junction that
splits the trunk. Medians of the three samples, in ms; control / gate where they differ:

| Stage | 0 | 2k | 10k | 30k | 100k |
| --- | --- | --- | --- | --- | --- |
| buildings / nodes / edges | 0 / 31 / 44 | 1,413 / 58 / 93 | 7,396 / 247 / 446 | 20,362 / 642 / 1,201 | 76,109 / 2,220 / 4,283 |
| click → ack | 20.6 / 19.1 | 64.8 / 61.3 | 256.4 / 246.0 | 689.4 / 650.6 | 2,613 / 2,391 |
| plan | 0.19 | 0.21 | 0.19 | 0.23 | 0.26 |
| undo, topology, zoning | 0.06 | 0.07 | 0.09 | 0.17 | 0.40 |
| roadbed and terrain (`surface`) | 0.33 | 0.58 | 0.51 | 0.58 | 0.63 |
| agents and lanes | 0.35 | 0.34 | 0.86 | 3.64 | 19.0 |
| entrances (`buildings`) | 0.00 | 19.8 | 21.4 | 22.8 | 29.6 |
| routing | 0.90 / 0.002 | 2.38 / 0.003 | 13.96 / 0.004 | 49.4 / 0.005 | 312.2 / 0.005 |
| render publication (`mesh`) | 18.7 | 40.9 | 218.9 | 609.6 | 2,261 / 2,335 |
| tool snapshot | 0.03 | 0.04 | 0.25 | 0.74 | 2.73 |
| road-locked terrain state | 0.00 | 0.08 | 0.83 | 0.66 | 2.55 |
| snapshot publish | 0.07 | 0.08 | 0.21 | 0.39 | 1.27 |
| CCH build alone | 0.95 | 2.50 | 15.1 | 50.0 | 311.5 |
| graph clone alone | 0.02 | 0.06 | 0.35 | 0.93 | 3.49 |

- **Routing.** The gate removes the CCH build from the commit: 312 ms at 100k becomes the
  scheduling, 5 µs. The snapshot's clones cost 2.7 ms at 100k (the terrain and water clones are
  O(1); the graph clone is 3.5 ms on its own), so reusing the snapshot's graph saved a copy.
- **The commit still misses 16.7 ms at every tier**, on stages outside this gate
  ([open question 21](#road-68-open-questions)). The render publication
  (`precompute_road_mesh_data`) grows with the network, about 0.4–0.55 ms per edge, to 2.3 s
  at 100k; the entrance rebuild costs about 20–30 ms from 1,413 buildings up; agents and lanes
  reach 19 ms at 100k. Part 2 found that this harness never acknowledged an upload, so every
  commit here republished the whole network
  ([local commit publication](#local-commit-publication-road-68-g-part-2-2026-10-24)).
- **Adoption latency.** Adoption comes six frames after the commit, 100 ms at 60 Hz. The
  build itself takes 0.95 / 2.5 / 15 / 50 / 311 ms, so at 100k the adoption frame waits about
  210 ms in the game. In the harness, whose frames take no wall time, it waited 0.5 / 2.0 / 13 /
  50 / 300 ms.
- **Backlog at adoption.** None accumulates: held frames issue no query. The adoption frame
  replans the agents whose routes the edit invalidated, as the frame after a synchronous
  rebuild did: agent tick 25–128 ms at 100k (control's next frame: 31–124 ms) against 14 ms for
  an ordinary frame, plus 57–94 ms of slot work as caches keyed on the new generation refill.
  The flow fields rebuild in 1.1 ms. The next frame is ordinary again (agent tick 9.4 ms).

**Determinism** (fresh, 2026-10-24, release binary `13d4ec6f…`):

```bash
METRUM_DEBUG=0 "$TEST_BINARY" road68g_commit_determinism --ignored --nocapture
```

The 2,000-resident populated city takes one player commit (a T branch off a border trunk), then
runs six game hours. Against the eight-worker reference, runs with one and three workers, a
rebuild delayed 400 ms at one and at eight workers, and a save taken inside the hold all give
identical hourly fingerprints (six) and identical saved tables (72). The focused tests
(`core/tests/routing_adoption.rs`) check that held frames advance nothing, a late job blocks only
the adoption frame, a paused game adopts on time, a second commit restarts the hold, a save
ends it, and the adopted CCH matches a fresh build of the live graph.

**Economy performance contract** ([`PERF-01`](economy.md#performance-contract-perf-01perf-05),
fresh, 2026-10-24). Control C is the base `c59955e6` with the gate's test-only harness
(binary `23865fac…`); G is this gate (binary `13d4ec6f…`). Both ran on the control's generated
cities (5,311 and 15,180 households): every run reported no generation, and the snapshot files'
SHA-256 sums were unchanged after each. A B B A, pinned to the P-cores with eight workers, under
the exclusive timing lock, no build or backup running; command as in the attribution below.

| Hour mean | C1 | G1 | G2 | C2 |
| --- | --- | --- | --- | --- |
| baseline 10k | 0.89 ms | 0.83 ms | 0.90 ms | 0.84 ms |
| baseline 30k | 4.14 ms | 4.27 ms | 4.12 ms | 4.26 ms |
| fiscal 10k | 6.62 ms | 6.63 ms | 6.77 ms | 6.52 ms |
| fiscal 30k | 28.30 ms | 29.42 ms | 28.52 ms | 29.39 ms |
| runner time | 110.1 s | 109.4 s | 109.6 s | 109.9 s |

Work counts are identical in all four runs. The differences lie inside the control's own spread,
so the contract is unchanged: no time limit fails, and the same seven work-count limits fail in
control and change ([open question 20](#road-68-open-questions) and the attribution below). The
contract's cities build no roads during the measured day, so no hold occurs in it.

#### Attribution of the perf-contract rise (`ROAD-68-G`)

[Open question 14](#road-68-open-questions) left the baseline-30k hour mean's rise from
3.31–3.40 ms (`SIM-02-K` part 7) to 4.09–4.44 ms on `ROAD-68` cities unattributed. On
2026-10-24 the release library tests of `d6c80f91` (A, the last code commit before `ROAD-68`,
binary `504c773d…`) and of the `ROAD-68-G` base (B, `c59955e6` with the gate's test-only commit
breakdown harness, `8f9d7f1a`, binary `23865fac…`) ran A B B A in one session, each build on
cities it generated itself, pinned to the P-cores with eight workers, under the exclusive timing
lock with no build or restic backup running:

```bash
RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14 \
  python3 tools/perf_contract.py --binary "$TEST_BINARY" --artifacts "$ARTIFACTS/<run>"   # <run>/snapshots -> that build's directory
```

| Hour mean | A1 (pre-`ROAD-68`) | B1 (base) | B2 (base) | A2 (pre-`ROAD-68`) |
| --- | --- | --- | --- | --- |
| baseline 10k | 0.88 ms | 0.81 ms | 0.87 ms | 0.88 ms |
| baseline 30k | 4.49 ms | 4.22 ms | 4.17 ms | 4.45 ms |
| fiscal 10k | 8.32 ms | 6.58 ms | 6.72 ms | 8.27 ms |
| fiscal 30k | 36.56 ms | 28.19 ms | 28.51 ms | 33.98 ms |
| runner time | 77.2 s | 109.5 s | 109.7 s | 76.5 s |

- **The rise predates `ROAD-68`.** The pre-`ROAD-68` build measures 4.45–4.49 ms at baseline
  30k in this session, more than the base's 4.17–4.22 ms. The 3.31–3.40 ms of `SIM-02-K` part 7
  came from an earlier build (`93bfaf61` and before) and boot; the change lies between that
  build and `d6c80f91` or in the session, not in `ROAD-68`.
- **Cities.** A's cities end with 5,313 and 15,179 households, B's with 5,311 and 15,180. Each
  build generated its snapshots in its first run; its second run reported no generation and
  every snapshot file's SHA-256 was unchanged after each run.
- **Work counts.** A fails five work-count limits: baseline `buyer_sites_built` (10 → 369),
  `staffing_trials` (49,289 → 231,170) and `job_search_keys` (64 → 15,448), and fiscal
  `buyer_sites_built` (716 → 2,764) and `cch_searches` (6,771 → 26,851). B fails the same five
  (20 → 396, 40,983 → 231,810, 69 → 21,371, 708 → 2,808, 5,817 → 26,657) and two more, baseline
  `supplier_searches` and `supplier_routes` ([open question 20](#road-68-open-questions)). No
  time limit fails in either.
- **Time does not follow the counts.** At baseline 30k, B does 9% more supplier routes than A
  and runs 6% faster. In the fiscal workload the counts match within 1% and B is 17–23% faster;
  that difference is code between `d6c80f91` and `c59955e6`, not work.

#### Supplier counts on `ROAD-68` cities (open question 20, 2026-10-25)

Fresh runs, no timing: release library tests of `d6c80f91` (control) and of `road` at
`4710f59b`, each with the same test-only probe (one line per supplier search with every route
quote it priced, per staffing operation, per census staffing row and per business-proof
reduction; not committed), each generating its own baseline cities, eight workers. The probe
leaves every count as it was: both builds reproduce the `ROAD-68-G` session exactly (10k
22,712 searches and 393,202 routes against 15,394 and 261,289; 30k 62,900 and 1,232,340
against 63,981 and 1,346,656).

- **No reachability or routing defect.** Both builds' cities have the same graph (247 and 642
  nodes, 446 and 1,201 edges), the same lane count (18,255 and 50,487), the same car speed
  ceiling of the straight-line prune (13.9 m/s), identical edge costs and lengths, and one car
  component holding every building. Every route quote found a route and every search found a
  supplier in both builds, so no search goes on past an unreachable candidate. All 7,396
  buildings of the 10k city keep their asset, door, edge and entrance lanes; at 30k, 20,278 of
  20,359 parcels hold the same asset, each with the same door and lanes.
- **What the roadbed changes.** The junction clips are the layout's mouth stations, 10 m at
  the grid's junctions against the legacy 5.5 m setback, so each car lane is 9 m shorter and a
  freight quote's frontage legs (door to lane end) shrink by up to 4.5 m each. In the first
  hour at 10k, quotes for the same buyer and supplier differ by −1.6 to +1.3 s (5th to 95th
  percentile, median 0.0 s), traffic included.
- **Where the searches come from.** Almost every search is one local staffing operation of the
  funding census (10k: 15,324 of 15,394; 30k: 61,550 of 63,981), and every closure pass runs
  both staffing sweeps again, about 515 searches a pass at 10k. The control city's 10k census
  takes a second closure pass in 20 of 24 hours (44 passes), the `ROAD-68` city's in 6 (30).
  A second pass follows a business-customer proof that lowered a ceiling: at 10k one firm
  (parcel 369) in 16 hours against 5, and parcel 5818 in 5 hours against none. At 30k both
  cities take two passes every hour (48), driven by some 55 firms.
- **The counts follow the city, not the build.** Each build ran on the other build's snapshot
  (loaded without the fixture revision check; the snapshot files' SHA-256 sums were unchanged):

  | Build, city | Searches 10k / 30k | Routes 10k / 30k | Closure passes |
  | --- | --- | --- | --- |
  | control, control | 22,712 / 62,900 | 393,202 / 1,232,340 | 44 / 48 |
  | `ROAD-68`, control | 22,712 / 64,415 | 394,220 / 1,310,969 | 44 / 48 |
  | control, `ROAD-68` | 15,398 / 65,510 | 260,430 / 1,417,045 | 30 / 48 |
  | `ROAD-68`, `ROAD-68` | 15,394 / 63,981 | 261,289 / 1,346,656 | 30 / 48 |

  Neither build's lanes move the 10k searches. At 30k a build on the other build's city does 2%
  more searches and 5–6% more routes, in both directions. The cities differ because each build
  grew them for twelve days with its own quote times: the 10k city's buildings are the same,
  its firms' finances are not. At 30k the `ROAD-68` city also prices more routes per search
  (21.0 against 19.6), with either build.
- **Per closure pass both cities scale within the limit.** Searches per pass go from 516 to
  1,310 (2.54×) on the control cities and from 513 to 1,333 (2.60×) on the `ROAD-68` cities;
  routes per pass from 8,936 to 25,674 (2.87×) and from 8,710 to 28,055 (3.22×), against the
  3.43× limit.

The full contract then ran once per build with the unprobed release library tests (control
`bd125930…`, `road` at `4710f59b` `5a10d2e7…`), each on its own cities, eight workers, without
the exclusive timing lock, so its times are not acceptance:

```bash
RAYON_NUM_THREADS=8 python3 tools/perf_contract.py --binary "$TEST_BINARY" --artifacts "$ARTIFACTS/<build>"
```

The control fails the five limits that predate `ROAD-68` and `road` the same five plus
`supplier_searches` and `supplier_routes`, every count as in the probed runs.

The counts therefore follow correct road topology through the economy's closure passes; there
is nothing to fix in the roadbed, the lanes or the supplier search. The recommended re-baseline
values are in [open question 20](#road-68-open-questions); the limits are unchanged until the
user decides.

#### Local commit publication (`ROAD-68-G` part 2, 2026-10-24)

**Why the publication grew with the network.** The commit breakdown never acknowledged a
road-chunk upload. `road_mesh_full_replace` starts on and stays on until Godot acknowledges a
published generation (`NetworkRenderer` → `acknowledge_network_render`), so every measured
commit republished every occupied chunk: 2.3 s at 100k residents. In the game only the first
publication after a load is whole. The harness now acknowledges the city's publication before
the first click and each commit's afterwards, as Godot does (it also records the republished
chunks, 3–6 per commit). The control then took 72–104 ms at 100k (profiled run): local, but
not inside a frame. A profile of those local commits:

- **Render publication** (22–36 ms): 67% lifting the lanes of every piece in the changed
  chunks onto the surface, although only the rebuilt pieces' lanes can move; 27% emitting every
  piece in those chunks again, although each is an immutable piece whose triangles are already
  in the published chunk.
- **Entrances** (23–38 ms): 76% regenerating the edit's dirty 512 m cell-lot regions, 16% deriving
  every building's entrance, 8% the serial `O(B)` attachment repair.
- **Agents and lanes** (17–19 ms): 80% the serial `O(A)` route-state test after reattachment.

**Changes.**

- *Lane lift.* `SimCore::road_mesh_lane_generation` records the roadbed generation of the last
  chunk publication; the next one lifts only the lanes of covered pieces built after it (the
  build generations `pieces_in_bounds` already returns). A world reset clears it, and then
  every covered piece's lanes are lifted as before. The breakdown counts live lanes a full
  surface sync would still move after every commit: none at any tier, control or change.
- *Splice.* With that generation, a changed chunk keeps the published triangles of its unchanged
  owners and emits only the rebuilt pieces (`NetworkMeshData::splice_owners`). Each mesh layer
  is written by one emission pass that visits spans, then nodes, by id, so merging the kept
  runs with the fresh ones in owner order gives the chunk a full emission builds. Street lights
  and signs, placed along whole streets, are still placed for every covered edge. Test builds
  emit every covered piece again on each spliced publication and assert the payloads equal
  (the whole debug suite passes with it).
- *Entrances.* The commit repairs attachments only for the buildings whose visit can act (a
  parallel read-only test skips buildings already on a plausible frontage of their own or of
  their parcel; the visits run in building order, as before) and derives again only the
  entrances of the buildings it visited, those the lot refresh reattached and those fronting the
  edit's lane rebuild closure. An entrance reads only its building, its edge and that edge's
  lanes, and the rebuild leaves every other edge's lanes in place. A cache already stale before
  the commit rebuilds whole; test builds assert the result equals a full derivation.
- *Agents.* The route-state test runs on the shared pool and the routes clear in agent order.
- *Adoption offset: one frame* ([pure jobs](simulation.md#pure-jobs)). The sim thread waits for a
  late build before it takes the core lock, so Godot's acknowledgements never queue behind it.
- *Editor rebuilds.* Node moves, lane connection edits, crosswalk toggles, edge class changes and
  border connections schedule the deferred rebuild (`schedule_cch_rebuild_of_live_graph`). The
  lane edits used to mark a chunk after their synchronous build, which rebuilt the CCH a second
  time at the next frame; the scheduled job's graph copy holds the edit, so that mark is gone.
  The populated-city fixture installs its border connections' rebuild before admitting
  households outside frames (`settle_cch`).

**Commit breakdown** (fresh, 2026-10-24, 19:37–19:48). Release library tests of the control
(`e92359c8` with this part's test-only harness commit `a2131e31`, binary `1e4c420d…`) and of
this part (`462dfdeb`, binary `1c799f85…`), in one session, pinned to the P-cores with eight
workers, under the exclusive timing lock with no build or backup running; command as above.
Medians of three samples, in ms; control / change where they differ:

| Stage | 0 | 2k | 10k | 30k | 100k |
| --- | --- | --- | --- | --- | --- |
| buildings / nodes / edges | 0 / 31 / 44 | 1,413 / 58 / 93 | 7,396 / 247 / 446 | 20,362 / 642 / 1,201 | 76,109 / 2,220 / 4,283 |
| click → ack | 16.1 / 8.50 | 42.0 / 28.8 | 64.2 / 33.3 | 70.6 / 34.7 | 78.2 / 41.0 |
| render publication (`mesh`) | 14.9 / 7.36 | 21.5 / 8.37 | 38.3 / 9.92 | 39.1 / 9.23 | 22.2 / 7.40 |
| entrances (`buildings`) | 0.00 | 19.2 | 22.7 / 20.4 | 24.9 / 21.0 | 29.7 / 19.9 |
| agents and lanes | 0.25 / 0.27 | 0.33 / 0.34 | 0.88 / 0.65 | 3.20 / 1.04 | 16.4 / 4.00 |
| agent invalidation | 0.00 | 0.01 | 0.07 / 0.06 | 0.26 / 0.25 | 1.19 / 1.15 |
| tool snapshot | 0.01 | 0.04 | 0.22 | 0.64 | 2.46 / 2.56 |
| road-locked terrain state | 0.00 | 0.08 / 0.09 | 0.82 / 0.83 | 0.67 / 0.66 | 2.51 / 2.62 |
| snapshot publish | 0.08 / 0.06 | 0.07 / 0.11 | 0.27 / 0.21 | 0.39 / 0.42 | 1.17 / 1.11 |
| plan, undo, topology, zoning, surface | 0.62 / 0.65 | 0.62 / 0.62 | 0.83 / 0.76 | 0.84 / 1.02 | 1.53 / 1.45 |
| republished chunks | 4 | 4 | 5 | 5 | 3 |
| held frames | 6 / 1 | 6 / 1 | 6 / 1 | 6 / 1 | 6 / 1 |
| adoption wait (harness) | 0.61 / 0.76 | 2.23 / 2.13 | 13.2 | 48.7 / 49.3 | 299 / 298 |
| CCH build alone | 0.91 / 0.93 | 2.31 / 2.35 | 14.7 / 14.1 | 51.0 / 51.7 | 298 / 296 |
| live lanes off the surface | 0 | 0 | 0 | 0 | 0 |

The first sample of each tier is the slowest (2k: 68.5 / 42.2 ms, 100k: 101.7 / 56.5 ms),
the third the fastest (100k: 67.1 / 29.6 ms); the medians above lie between.

- **Render publication now follows the edit:** 7.4–9.9 ms from the bare grid to 100k, against
  14.9–39.1 ms. What is left is lifting the commit's new lanes (twice for most of them: once as
  new lanes, again as the rebuilt pieces' lanes), emitting the rebuilt pieces, placing the
  covered edges' lights and signs, copying the changed chunks' kept runs and certifying the
  payloads, all `O(changed chunks)`.
- **Entrances** stay at about 20 ms from 1,413 buildings up, and are now flat: what remains is
  the commit's synchronous regeneration of its dirty cell-lot regions ([open question
  21](#road-68-open-questions)); the derivation and repair are a few ms of `O(B / P)` tests.
- **Agents and lanes** fall from 16.4 to 4.0 ms at 100k; the route-state test is still a pass over
  every agent, now in parallel, and the invalidation pass (1.2 ms) is serial.
- **Still growing with the city:** the tool snapshot's graph clone (2.6 ms), the road-locked
  terrain state (2.6 ms; it walks the heightmap's unacknowledged dirty render patches, and this
  harness acknowledges only road chunks, so this may be the same kind of harness artifact; terrain
  is `ROAD-68-D` part 2's), the snapshot publish (1.1 ms, dropping the previous snapshot) and the
  agent passes.
- **Adoption** comes one frame after the commit. The simulation holds for the held frame or the
  build, whichever is longer, so the hold is unchanged where the build is slower than a frame:
  about 0.3 s at 100k ([`NET-09`](roadmap.md)). The adoption frame's agent tick is as before
  (48.7 / 53.0 ms at 100k).

**Determinism** (fresh, same session, `road68g_commit_determinism`, release): against the
eight-worker reference, one and three workers, a rebuild delayed 400 ms at one and eight
workers, and a save inside the hold give identical hourly fingerprints (6) and saved tables (72),
in the control and in this part. The reference runs of control and change share one digest
(`6252b7ba71c5a6f1`, printed by the test), so the local publication, entrance rebuild, agent pass
and one-frame offset leave the game bit for bit as it was.

#### Commit cell lots (`ROAD-68-G` part 3, 2026-10-24)

[Open question 21](#road-68-open-questions) (user decision): speed up the commit's own cell-lot
regeneration first, keeping `SIM-02-H`'s rule that a commit's lots are current.

**Why one road cost about 20 ms.** Profiled with perf (LBR call stacks, the commit only, through
the breakdown's perf control FIFO) and the `economy_phases` lot and generation details, at 2k and
10k residents. Lot derivation was 1–2 % of the stage; the rest was generating cells over the
dirty envelopes, about 2 µs per candidate footprint, 4,500–17,000 footprints per commit. The
envelopes were large because the commit marked the bounding box of every dirty road plus 216 m
(146 m of lot reach, 70 m of conflict halo) and of its whole adjacency ring: for the T branch
off the trunk, the far-end arms of the split trunk (two 50 m and one 400 m grid street, dirty
because their junction was re-solved although their geometry did not change) and their ring
(another seven grid streets), mostly painted grid. Marking only the changed roads generates
10,100 / 5,300 / 2,600 footprints for the three commits at 2k instead of 13,000 / 10,200 /
4,500 (10k: 10,900 / 4,700 / 4,200 instead of 16,800 / 10,900 / 5,800). On the commit thread,
generation spent 13 % in a binary search per candidate, 20 % in SipHash and 6 % in `libm`
`round` calls.

**Changes.**

- *Marks follow the changed roads.* An adopted road plan names the dirty edges whose geometry,
  clips, endpoints or deletion changed (`FinalizedRoadGeometry::changed_edges`). Every dirty edge
  still refreshes its curb frames exactly as before; only the changed edges and the roads whose
  frames the refresh changed or that have none queue lot regions and invalidate cells
  (`ZoningSystem::mark_cell_lots_for_road_edit`). A road the edit only touched keeps its
  geometry and frames, so its cells change only near the changed roads, whose own corridors
  cover them; a road without frames takes its phase from its junctions and is still
  invalidated whole. Unplanned edits treat every dirty edge as changed
  ([`zoning.md`](zoning.md)).
- *Exact constant factors in generation.* `f64::round` lowers to a `libm` call on x86-64-v2;
  the cell geometry rounds through a bit-identical half-away helper on `trunc` (a test compares
  100,000 values and the edge cases bit for bit). The cell store and generation maps use the
  fixed fast hasher already used elsewhere (`foldhash`); no caller depends on their iteration
  order. Each candidate's footprint legality is read back by index instead of a binary search.

**Products unchanged.** The breakdown now prints, after each commit, a digest of the cell paint
and lot claims, the parcels in store order and the queued lot regions, and a second digest of
the generated cell cache (valid cells and frontages). Both equal the control's for every sample
at every tier, so the narrower invalidation regenerates nothing the whole-ring one would have
changed, and the commit's lots, parcels and cells are bit for bit the control's.

**Road-locked terrain state** was a harness artifact, as part 2 suspected: the city build leaves
every terrain patch it dirtied unacknowledged, and the commit walked them all (2.6 ms at 100k).
The breakdown now acknowledges terrain patches before each click as `acknowledge_uploads` does;
the stage then costs 0.00 ms in control and change.

**Commit breakdown** (fresh, 2026-10-24, 22:08–22:18). Release library tests of the control
(`27797b92` with this part's two test-only harness commits, `ba4bcea9` and `69936b6f`, binary
`aa06a2a0…`) and of this part (`80defe1e`, binary `f27b2cef…`), in one session, pinned to the
P-cores with eight workers, under the exclusive timing lock with no build or backup running;
command as above. Medians of three samples, in ms; control / change where they differ:

| Stage | 0 | 2k | 10k | 30k | 100k |
| --- | --- | --- | --- | --- | --- |
| buildings / nodes / edges | 0 / 31 / 44 | 1,413 / 58 / 93 | 7,396 / 247 / 446 | 20,362 / 642 / 1,201 | 76,109 / 2,220 / 4,283 |
| click → ack | 8.60 / 8.73 | 29.0 / 16.1 | 34.8 / 19.3 | 34.6 / 20.3 | 41.7 / 28.6 |
| entrances (`buildings`) | 0.00 | 19.4 / 6.40 | 21.6 / 6.39 | 20.6 / 6.55 | 20.3 / 7.84 |
| render publication (`mesh`) | 7.34 / 7.48 | 8.12 / 8.19 | 10.4 | 9.37 / 9.30 | 7.49 / 7.65 |
| agents and lanes | 0.33 / 0.26 | 0.51 / 0.35 | 0.65 / 0.66 | 1.03 / 1.01 | 4.15 / 3.82 |
| agent invalidation | 0.00 | 0.01 | 0.09 / 0.06 | 0.27 / 0.24 | 1.24 / 1.20 |
| tool snapshot | 0.01 | 0.04 / 0.03 | 0.23 / 0.22 | 0.69 / 0.65 | 2.57 / 2.55 |
| road-locked terrain state | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 |
| snapshot publish | 0.09 / 0.22 | 0.15 / 0.26 | 0.45 | 1.23 / 1.08 | 3.77 / 3.56 |
| plan, undo, topology, zoning, surface | 0.72 / 0.74 | 0.57 / 0.75 | 0.96 / 0.80 | 0.96 / 1.03 | 1.33 / 1.37 |
| republished chunks | 4 | 4 | 5 | 5 | 3 |
| held frames | 1 | 1 | 1 | 1 | 1 |
| CCH build alone | 0.92 / 1.00 | 2.24 / 2.32 | 14.9 | 51.7 / 49.4 | 300 / 298 |
| live lanes off the surface | 0 | 0 | 0 | 0 | 0 |

Per sample, click → ack at 100k is 57.0 / 41.0, 41.7 / 28.6 and 29.5 / 23.4 ms. The first
sample of each tier is the slowest in both builds; it invalidates a wider area (2k: 10,100 against
5,300 candidate footprints in the second, 2,600 in the third), not yet attributed (possibly neighbours whose curb
frames are refreshed for the first time).

- **Entrances fall from about 20 ms to 6.4–7.8 ms**, flat from 1,413 buildings up. What remains
  is generating the cells along the changed roads (the new road, both halves of the split
  trunk, each bounding box plus 216 m; 2,600–5,300 candidate footprints at 2k) and deriving their
  regions' lots. On the commit thread the generation is now dominated by its serial parts: the
  candidate priority sort (17 %), the indexing loop and footprint corners.
- **The commit still misses 16.7 ms**: 28.6 ms at 100k
  ([open question 21](#road-68-open-questions)). Besides the lots: the render publication
  (7.5–10.4 ms, flat), the agent passes at 100k (3.8 + 1.2 ms, `O(A / P)` and `O(A)`), the tool
  snapshot's graph clone (2.6 ms, `O(graph)`), and the snapshot publish, 3.6 ms at 100k.
- **Snapshot publish** grew from about 1.1 ms (part 2) to 3.6–3.8 ms at 100k in both builds,
  from the second sample on (the first is 1.2 ms). It is the harness's terrain
  acknowledgement: dropping the previous commit's snapshot now frees structure the live core no
  longer shares with it. Godot acknowledges every frame, so this is the game's cost, hidden
  before; what the drop frees is not yet attributed.

**Determinism** (fresh, same session, `road68g_commit_determinism`, release): against the
eight-worker reference, one and three workers, a rebuild delayed 400 ms at one and eight
workers, and a save inside the hold give identical hourly fingerprints (6) and saved tables
(72), in the control and in this part, and both reference runs print digest
`6252b7ba71c5a6f1`, part 2's.

**Agent passes, first step** (after the breakdown above). The lane-invalidation test and the
reattachment's choice of detached agents ran serially over every agent; both now test the agents
on the shared pool and write in agent order, as part 2 did for the route-state test (agent `i`'s
test reads and its write touches only agent `i`). At 100k, same session, exclusive lock, against
`80defe1e`: invalidation 1.25–1.30 → 0.27–0.33 ms, agents and lanes 3.91–5.96 → 3.55–5.47 ms,
click → ack 45.3 / 29.6 / 24.1 → 40.6 / 27.7 / 23.9 ms; product digests and the determinism
reference digest unchanged. The passes stay `O(A / P)`: visiting only the edit's agents needs
per-edge agent lists and an index of planned routes.

#### Commit stages, part 4 (`ROAD-68-G`, 2026-10-24)

**Narrow cell-lot marks, checked against the whole ring.** Part 3 proved its narrower marks only
on T splits, through the breakdown's digests. A digest test
(`narrow_road_marks_match_marking_the_whole_refreshed_ring`) now builds the zoned 800 m street
grid twice, every lot built, once marking as part 3 does and once marking every road the curb-
frame refresh touches (a test-only switch), and runs on both: a T split off the trunk, its undo,
the split again, a move of the branch's free end, a straight extension, a painted block's cross
street turned into a bridge and back, a crossing turned off and on, a bulldoze of a painted
block's cross street, a road beside the painted grid, a T off it, a T off the trunk beside the
painted rows, moves of a painted edge T and an interior crossing, and undo. After each edit,
with every chunk's cells generated first (so a cached chunk a narrower mark left valid is
compared with the one a wider mark regenerated), it compares the paint and lot claims, the
generated cells, the parcels in store order, the queued lot regions, the curb frames and every
building's attachment.

- **Difference found and fixed.** Undo (and rollback and border edits) leave their regions to
  the `cell lots` queue instead of deriving them at once. With part 3's marks they queued fewer
  regions than before (the undo of the interior node move: the queued-region digest differed), so
  the queue would serve other regions at other slots and the game would diverge. Every edit but
  the commit now marks its roads and the whole ring at their ends again
  (`mark_cell_lots_for_roads`); only the commit, which derives every pending region before it
  returns, keeps the narrow marks. The test passes with every product equal.
- **The test is not vacuous.** Run with part 3's marks restored for the other edits, it fails at
  the undo of the interior move (queued regions); with a commit that marks nothing, it fails at
  the first T split (generated cells). A commit that marks only one of its changed roads, or
  only the changed roads without the realigned ones, still passes: on these fixtures each
  changed road's corridor (bounding box plus 216 m) covers the others' changes at the junction.
- **The first sample's larger area** (part 3: 10,100 against 5,300 and 2,600 candidate
  footprints at 2k) is the fixture, not a stale mark: the samples split the trunk 100, 250 and
  400 m west of the painted grid, each in the piece the previous one left, so the split's east
  half ends at the grid's west junction, then at the first branch, then at the second, and its
  corridor (bounding box plus 216 m) reaches 216 m, 116 m and nothing of the painted grid.

**Render publication: each new lane lifted once.** The publication lifted the commit's new lanes
onto the roadbed (`sync_new_lane_heights`), then lifted every lane of the rebuilt pieces again,
most of them the same new lanes, against the same roadbed generation: 56 % of the publication's
samples were lane lifts. Lanes are only appended (a rebuild tombstones old ids), and only lifts
and tombstoning write lane geometry, so the lane system records from which lane on every lane
was lifted at which roadbed generation (`LaneSystem::lifted_since`); the rebuilt pieces' lift
skips those lanes while the roadbed is still at that generation, where a second lift changes no
point. Test builds lift a copy of every skipped lane and assert it unchanged. The breakdown
still counts no live lane off the surface after any commit. Same session, pinned, eight workers,
against part 3's last build (`63ee8bde`), per sample, with the new lanes then also lifted on the
pool (reverted since, below): render publication 15.2 / 8.3 / 2.5 → 8.4 / 4.4 / 0.8 ms at 2k
and 13.3 / 7.4 / 6.1 → 5.8 / 3.3 / 3.9 ms at 100k; both product digests equal in every sample.

**Deadlock in the debug suite, and why the lift stays serial.** Lifting the new lanes and
sealing the changed chunks on the shared pool deadlocked the debug suite (18 frame-stepping
scenario tests; every thread in a futex wait, no CPU). A frame adopts a routing rebuild with
`PureJob::take`, which blocks its thread while a pool worker runs the job. A pool worker waiting
in a join steals other work, here another test's frame (the suite runs many cities on one
shared pool), whose `take` then blocks that worker while the job it waits for, or the work that
job waits for, lies lower on a blocked worker's stack. The pure-job module's guarantee ("a taker
never blocks a pool worker on work queued behind it") does not cover work stolen beneath a
running job. Both steps are serial again; every other join already on those paths keeps the
same hazard, which no run has shown before. Recorded for `ROAD-68-H` and the pure-job owner:
take results outside the pool, or make a waiting taker help run the pool instead of blocking.
Fixed in [part 5](#commit-stages-part-5-road-68-g-2026-10-25): results are taken outside the
pool, and both steps are on the pool again.

**The rest of the publication.** A 100k profile of the commit thread after the lift change: the
splice copying the changed chunks' kept runs grew every layer's buffers run by run (17 % of the
commit thread in `realloc`); each spliced layer is now reserved once at its final size
(`NetworkMeshData::reserve_layer`). Street lights and signs (13 %), piece surfaces (12 %) and
the payload certificates are still produced for every covered edge, rebuilt piece and changed
chunk.

**Cell lots: generation without the lot reach.** A road edit invalidated generated cells over its
lot regions: the road's bounding box plus its candidate rows (half width, sidewalk, six cells)
plus a parcel frontage (80 m), plus the 70 m conflict and support halo. Cell generation reads a
road only through its candidate rows and its corridor; the 80 m is how far a lot can reach along
its cells, which is the lot derivation's concern. Cells are now invalidated over the rows and
the halo alone, and the lot regions keep the reach (`mark_road_cells_dirty`); the commit's
regions then regenerate about half the candidate footprints. The candidate sort compares the
priority field by field instead of building two keys per comparison (the same outcomes give the
unstable sort the same permutation), and the generation's temporary candidate index no longer
stamps chunk revisions nothing reads (`CellStore::insert_unstamped`). The digest test compares
the narrower invalidation with the wider one for every edit kind above, and the breakdown's
product digests equal the control's at every tier and sample (below).

**Agent passes.** The lane invalidation, reattachment and route-state tests probe the edit's
edge, node and lane sets once per agent, and the route-state test once per path node of every
agent on the network: 28 % of their samples were SipHash. The sets use the fixed fast hasher
(only membership is read; the reattachment breaks ties by distance and lane id), and the edit's
nodes sit behind a fixed 4096-bit filter of their ids, so almost every path node misses without
hashing (a set bit is confirmed in the set). The passes stay `O(A / P)`; visiting only the
edit's agents needs per-edge agent lists and an index of planned routes (open question 21).

**Commit breakdown** (fresh, 2026-10-25, 01:32–01:42). Release library tests of the control
(`27797b92` with part 3's two test-only harness commits, binary `aa06a2a0…`, part 3's control)
and of this part (`d607753a`, binary `2c174f71…`), in one session, pinned to the P-cores with
eight workers, under the exclusive timing lock with no build or backup running; command as
above. Medians of three samples, in ms; control / change where they differ:

| Stage | 0 | 2k | 10k | 30k | 100k |
| --- | --- | --- | --- | --- | --- |
| buildings / nodes / edges | 0 / 31 / 44 | 1,413 / 58 / 93 | 7,396 / 247 / 446 | 20,362 / 642 / 1,201 | 76,109 / 2,220 / 4,283 |
| click → ack | 8.45 / 6.45 | 29.2 / 11.9 | 35.2 / 15.4 | 34.6 / 15.1 | 41.8 / 21.8 |
| entrances (`buildings`) | 0.00 | 19.3 / 3.85 | 21.1 / 4.79 | 20.6 / 4.23 | 20.7 / 5.37 |
| render publication (`mesh`) | 7.24 / 5.53 | 8.45 / 6.34 | 11.7 / 8.15 | 9.36 / 7.05 | 7.51 / 5.49 |
| agents and lanes | 0.29 / 0.26 | 0.43 / 0.33 | 0.66 | 1.25 / 0.69 | 3.88 / 2.17 |
| agent invalidation | 0.00 | 0.01 | 0.06 | 0.27 / 0.09 | 1.21 / 0.21 |
| tool snapshot | 0.01 | 0.04 | 0.24 / 0.21 | 0.63 / 0.64 | 2.55 / 2.56 |
| snapshot publish | 0.05 / 0.04 | 0.17 / 0.13 | 0.40 / 0.46 | 1.16 | 3.58 / 3.93 |
| plan, undo, topology, zoning, surface | 0.80 / 0.59 | 0.73 / 0.80 | 0.86 / 0.81 | 1.01 / 0.81 | 1.48 / 1.36 |
| republished chunks | 4 | 4 | 5 | 5 | 3 |
| held frames | 1 | 1 | 1 | 1 | 1 |
| CCH build alone | 0.89 / 0.92 | 2.28 / 2.32 | 14.1 / 14.8 | 49.9 / 52.1 | 301 / 294 |
| live lanes off the surface | 0 | 0 | 0 | 0 | 0 |

Per sample, click → ack at 100k is 56.1 / 28.3, 41.8 / 21.8 and 30.2 / 18.3 ms; at 2k 41.6 /
24.8, 29.2 / 11.9 and 11.0 / 4.0 ms. Both product digests (paint, claims, parcels and queued
regions; the generated cell cache) equal the control's in all 15 samples. Before the parallel
lift was reverted, the same code with it measured 23.9 / 20.1 / 16.2 ms at 100k (render
publication 5.0 / 3.4 / 3.2 ms).

- **The commit halves:** 41.8 → 21.8 ms at 100k, 29.2 → 11.9 ms at 2k; cell lots 20.7 → 5.4 ms
  at 100k, agent passes 5.1 → 2.4 ms. It still misses 16.7 ms
  ([open question 21](#road-68-open-questions)): at 100k the snapshot publish and the tool
  snapshot take 6.5 ms of the 21.8 and grow with the city; the lots, the publication and the
  agent passes take the rest.
- **The first sample** (the split nearest the painted grid) is the slowest in both builds:
  28.3 ms at 100k, lots 8.8 and publication 9.8 ms of it.

**Determinism** (fresh, same session, `road68g_commit_determinism`, release): against the
eight-worker reference, one and three workers, a rebuild delayed 400 ms at one and eight
workers, and a save inside the hold give identical hourly fingerprints (6) and saved tables
(72), in the control and in this part, and both reference runs print digest
`6252b7ba71c5a6f1`, parts 2 and 3's.


#### Commit stages, part 5 (`ROAD-68-G`, 2026-10-25)

**No pool worker waits for a pure job.** The adoption frame took the routing rebuild with
`PureJob::take` inside `step_frame`'s pool entry, and the sim thread's early wait (outside the
core lock) was installed in the pool too, so a pool worker blocked on a condition variable while
another worker ran the job. A worker waiting in a join runs whatever it steals, an injected frame
among it, so the job that frame waits for, or a part of the job's own joins, can lie lower on
that worker's own stack or on another waiting worker's; neither can then finish. That is what
deadlocked the debug suite in part 4. The frame's caller now finishes the job before the frame
enters the pool (`step_frame` waits on `cch_adoption_completion` first; the sim thread waits
outside the core lock and the pool), so the adoption inside the pool finds the result ready; a
taker outside every pool runs an unstarted job in the shared pool; and a wait on a shared-pool
worker panics instead of blocking (`thread_pools::on_shared_worker`, set by the pool's start
handler). The adoption frame's `adoption_ms` still includes the wait. Owner:
[pure jobs](simulation.md#pure-jobs).

- **Regression test** `the_adoption_wait_leaves_every_pool_worker_to_the_rebuild`: the rebuild
  sleeps 100 ms, then needs every shared-pool worker (a broadcast) while the adoption frame
  runs. With the old adoption (the new test and hook on `ded1fbd4`'s frame, thread and pure-job
  code) it hit its 120 s watchdog in 3 of 3 runs; it passes in 0.2 s now. A unit test checks that
  a shared-pool worker refuses to wait for a running job
  (`a_shared_worker_never_waits_for_a_running_job`).
- **New-lane lift and chunk seal on the pool again** (each lane and each chunk is independent,
  so the output is the same): the debug suite passes with both, before and after the rebase
  (below), with no hang.
- **Determinism tests in private pools** run the whole simulation on one pool worker, whose
  stack holds nothing else, so their waits stay safe; only shared-pool workers are refused.

**Cell road overlap without rounding or margin.** The cell-site road test rounded each visited
road triangle's bounds outward to f32, compared them with the footprint's f32 bounds, queried a
footprint grown by 1 m and looked up each triangle's band kind. No f32 lies strictly between a
value and its directed rounding, so `F < round_up(v)` exactly when `F < v` and
`F > round_down(v)` when `F > v`: the same test on the f64 bounds, without rounding. Only
triangles touching the footprint's f32 bounds can pass it, so the query needs no margin, and the
kind is not read (`RoadbedSystem::any_top_triangle_points_in_bounds`). The same triangles reach
the same overlay. The test was about a quarter of cell generation's samples in part 4's
profile (2k and 100k commits) and is 7 % in this part's (100k).

**Rebase onto `ROAD-68-D`** (`e9fee678`: centre chunk frames, chunk-line clips, uncut zebras,
site pads in the terrain patch). No code conflicts; open question 21's text kept this branch's
part 4 version beside D's questions 22 to 24. Fresh on the rebased tree: the publication splice
still equals emitting every piece (its debug assertion runs on every spliced publication of the
debug suite), the narrow-marks digest test, the product probe
(`roadbed_fixture_products_match_the_probe`) and D's
`reference_junction_road_chunks_are_watertight` and
`reference_junction_road_and_terrain_share_seam_vertices` pass.

**Commit breakdown before the rebase** (fresh, 2026-10-25, 02:57–03:08). Release library tests,
one session, pinned to the P-cores, eight workers, both timing locks, no build or backup
running: part 4's control (`27797b92` with the two test-only harness commits, binary
`aa06a2a0…`) against this part on the same base (`a1861cf2`, binary `e1d32537…`). Medians of
three samples, ms, control / change:

| Stage | 0 | 2k | 10k | 30k | 100k |
| --- | --- | --- | --- | --- | --- |
| buildings / nodes / edges | 0 / 31 / 44 | 1,413 / 58 / 93 | 7,396 / 247 / 446 | 20,362 / 642 / 1,201 | 76,109 / 2,220 / 4,283 |
| click → ack | 9.91 / 4.51 | 28.7 / 8.95 | 34.7 / 12.4 | 34.6 / 12.9 | 41.6 / 19.3 |
| roadbed and terrain (`surface`) | 0.59 / 0.45 | 0.35 / 0.57 | 0.51 / 0.50 | 0.64 / 0.57 | 0.68 / 0.67 |
| agents: invalidation | 0.00 | 0.01 | 0.07 / 0.06 | 0.24 / 0.07 | 1.21 / 0.23 |
| agents and lanes | 0.30 / 0.26 | 0.32 / 0.31 | 0.70 / 0.63 | 1.02 / 0.74 | 4.00 / 2.15 |
| entrances (`buildings`) | 0.00 | 19.0 / 3.61 | 20.9 / 4.44 | 20.8 / 4.14 | 20.2 / 5.22 |
| render publication (`mesh`) | 8.53 / 3.30 | 8.31 / 3.76 | 10.9 / 5.41 | 9.28 / 4.73 | 7.56 / 3.29 |
| tool snapshot | 0.02 / 0.01 | 0.04 / 0.03 | 0.23 | 0.65 / 0.66 | 2.58 / 2.50 |
| snapshot publish | 0.06 / 0.14 | 0.20 / 0.15 | 0.21 | 1.17 / 1.02 | 3.68 / 3.58 |
| adoption latency | 1.01 / 0.70 | 0.70 / 0.89 | 2.63 / 2.71 | 15.8 / 16.2 | 185 / 191 |
| CCH build alone | 1.36 / 0.90 | 2.28 / 2.48 | 14.4 / 14.3 | 51.2 / 52.4 | 298 / 302 |
| live lanes off the surface | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 |

Per sample, click → ack at 100k is 55.1 / 23.5, 41.6 / 19.3 and 29.5 / 16.1 ms; at 2k 42.1 / 19.7,
28.7 / 9.0 and 10.9 / 2.8 ms. Both product digests equal the control's in all 15 samples. Against
part 4 (21.8 ms median at 100k) the lift and seal on the pool take the publication from 5.5 to
3.3 ms; the overlap change is within the lots' sample spread (5.4 → 5.2 ms). The median at
100k, 19.3 ms, still misses 16.7 ms by 2.6 ms; the snapshot publish and the tool snapshot,
`ROAD-68-E`'s, are about 6 ms of it.

**After the rebase the 100k commit is rejected, and the commit's terrain validation costs 15 to
69 ms** (fresh, 03:38–03:48, same method: control `d9112c47` = `e9fee678` with the two harness
commits, binary `60195992…`, against the rebased change `47432fb5`, binary `1b18d096…`). At
100k the first commit (the trunk-branch split) is rejected with `render_geometry_invalid` in the
control and in the change, so the harness stops there; the lower tiers run, with both product
digests equal in all 12 samples:

| Stage | 0 | 2k | 10k | 30k |
| --- | --- | --- | --- | --- |
| buildings / nodes / edges | 0 / 31 / 44 | 1,413 / 58 / 93 | 7,396 / 247 / 446 | 20,362 / 642 / 1,201 |
| click → ack | 22.5 / 18.8 | 93.1 / 77.4 | 49.5 / 29.7 | 101 / 76.5 |
| roadbed and terrain (`surface`) | 15.3 / 15.1 | 67.4 / 69.3 | 16.9 / 18.1 | 65.9 / 63.3 |
| agents: invalidation | 0.00 / 0.01 | 0.02 / 0.03 | 0.08 / 0.09 | 0.27 / 0.13 |
| agents and lanes | 0.30 / 0.31 | 0.55 / 0.48 | 0.74 / 0.70 | 1.30 / 0.98 |
| entrances (`buildings`) | 0.00 | 17.0 / 3.52 | 20.7 / 4.74 | 20.6 / 4.71 |
| render publication (`mesh`) | 6.40 / 2.93 | 7.44 / 3.30 | 9.46 / 4.67 | 9.29 / 4.42 |
| tool snapshot | 0.02 | 0.05 / 0.06 | 0.27 / 0.25 | 0.79 / 0.71 |
| snapshot publish | 0.10 / 0.07 | 0.20 / 0.22 | 0.55 / 0.57 | 1.64 / 1.42 |
| adoption latency | 0.76 / 0.84 | 0.85 / 0.93 | 2.62 / 2.56 | 16.6 / 16.8 |
| CCH build alone | 0.91 / 0.87 | 2.40 / 2.36 | 13.6 / 14.6 | 48.7 / 50.9 |
| live lanes off the surface | 0.00 | 0.00 | 0.00 | 0.00 |

Per sample, the first commit takes 33 / 171 / 697 / 1,744 ms at 0 / 2k / 10k / 30k (change),
nearly all in `surface` (`validate_staged_road_render`: the roadbed and the commit's terrain
validation, which `ROAD-68-D` now builds from the roadbed's outer chains). Before `ROAD-68-D`
that stage took 0.4 to 0.7 ms. Without it (click → ack minus `surface`, median per tier) the
commit takes 3.7 / 8.1 / 11.6 / 13.1 ms at 0 / 2k / 10k / 30k (control 7.2 / 25.7 / 32.6 / 34.7),
as before the rebase (4.1 / 8.4 / 11.9 / 12.3, and 18.6 at 100k): entrances 3.5–4.7 against
17–21 ms, publication 2.9–4.7 against 6.4–9.5 ms. The gate cannot be
measured at 100k on `road` until `ROAD-68-D`'s commit validation is fixed (open question 21,
reported to the coordinator).

**Determinism** (fresh, rebased, same session, `road68g_commit_determinism`, release): against
the eight-worker reference, one and three workers, a rebuild delayed 400 ms at one and eight
workers, and a save inside the hold give identical hourly fingerprints (6) and saved tables (72)
in the control and the change; both reference runs print digest `6252b7ba71c5a6f1`, parts 2 to
4's.

**Where the 100k commit goes now** (perf, LBR, 15 kHz, the three 100k commits of the
pre-rebase change binary; profiled, not acceptance). On the commit thread, per commit: cell
generation about 5 ms (its serial candidate sort, candidate copies and cell-key map inserts and
growth), the tool snapshot's graph clone 2.4 ms, road signs 1.2 ms (cloning each crossing
edge's sections with their band vectors, `crossing_sections`, and turning-radius hazards along
the touched chains), the route-state test's share 1.2 ms, the attachment repair's share 0.9 ms,
the splice 0.7 ms, piece surfaces 0.7 ms, the lane lift 0.5 ms. Across all threads the
attachment repair (`repair_road_attachments_that_may_act`, a parallel test of every building,
memory-bound on building and parcel records) has as many samples as the agents' route-state
test: both are `O(city / P)` scans.

**Checks** (fresh, rebased `47432fb5`): debug `cargo test --lib` 1,875 passed, 0 failed, 167
ignored (1,844 / 0 / 188 before the rebase, with the lift and seal on the pool); `./run.sh --test`
passes with `DISPLAY` and `WAYLAND_DISPLAY` unset, skipping `network_tool_chunk_renderer_test`,
`road_junction_preview_test` and `road_preview_stream_test` until `ROAD-68-E` and
`zoning_cells_tool_test` until `ROAD-68-F`; clippy adds no warning (the baseline's 59);
`roadbed/` holds 7,418 production lines (`e9fee678`: 7,394; the triangle query without band
kinds adds 24).

- **Next.** `ROAD-68-E` round 2's publication and graph share (6.0–6.7 ms of the median, the
  options in question 21); an index of buildings by edge, so the repair and the entrance closure
  visit only the edit's buildings, and the agent lists of option (d); road signs without section
  clones; then question 21's (b) if the first sample still misses.

#### Commit stages, part 6 (`ROAD-68-G`, 2026-10-25)

**Why the first sample is the slowest: a larger edit, not a cold start.** A 100k profile of the
pre-rebase change (perf, LBR, 15 kHz, perf enabled only during each commit, samples split per
commit by time; profiled, not acceptance) puts the first commit's excess over the third, on the
worker that runs the commit, in cell generation (+4.3 ms: two lot regions, because the first
split's east half ends at the painted grid's west junction), the reattachment lane search
(2.1 ms against none: the agents on the trunk into the grid, searched one after another), the
render publication (+2.7 ms: the grid junction's pieces, their signs and lifts) and the lane
rebuild (+1.0 ms: that junction's walkways and pedestrian connections). The tool snapshot costs
the same in all three. A repeat check rules out cold caches and lazy indices: at 30k the first
branch committed, undone and committed again costs the same each time (entrances 8.28 / 8.54 /
7.84 ms, publication 6.80 / 8.47 / 6.50 ms), while the later branches cost 4.33 and 2.61 ms in
entrances. Only `ROAD-68-D`'s commit validation is cold (1,782 ms first, 27 ms on the repeat),
which part 6 of `ROAD-68-D` owns. So the first sample is made cheaper by making those stages
cheaper, with no warm-up.

**Changes** (each leaves every output as it was; the debug suite's checks below):

- **Reattachment on the pool.** Each detached agent's best rebuilt lane reads only that agent's
  position and the rebuilt lanes, so the shared pool searches them all
  (`reattach_invalidated_lanes_for_edges`); the agents then move onto their lanes in agent order,
  as the serial loop did. `O(A / P + detached × rebuilt lanes / P)`.
- **Street lights and signs beside the piece surfaces.** Lights and signs read only the graph,
  lanes, roadbed and terrain, and signs keep clear of the lights' world-space poles, so they are
  placed (`place_street_lights`, `place_road_signs`) while the crosswalks, piece surfaces,
  bridges and markings are emitted, then pushed after them in the order the serial passes pushed
  them. One emitter (`emit_into`) now serves the whole-network mesh and the chunked publication.
- **Crossing signs without copied sections.** `crossing_sections` returned the span's sections
  with each crossing node's approach section added, cloning every section and its band vector.
  It now returns a borrowed run (`SectionRun`: an optional section before the span, the span,
  an optional one after) that `pose`, `place` and `crossing_s` walk as they walked the slice. The
  run is in station order (a zebra's inner edge lies between its node and the span's mouth;
  debug builds assert it), so its partition points equal the slice's.
- **Retained frontage lines on the pool.** Cell generation's continuity candidates (one painted
  frame holds nearly all of them, so its per-frame parallelism was serial in practice) now visit
  the frame's frontage lines on the pool and collect their candidates in line order.
- **Frame ids reused in the candidate index.** Sorted candidates come in runs of one frame, so
  the index reuses the run's frame id instead of hashing the frame again. The candidate sort
  itself is unchanged: its element type and comparator fix the unstable sort's order of equal
  keys, which decides frontage and curve-source order, so it is not replaced.
- **One pass for the repair and the fronting buildings.** The attachment repair's parallel test
  of every building now also collects the buildings fronting the lane rebuild closure
  (`repair_road_attachments_that_may_act`'s `also`). It tests their attachment before the
  repair; a building the repair moves is among its visits either way, so the set the entrances
  are derived for is the same, and one `O(B / P)` pass replaces two.

**Not done** (open, question 21): an index of buildings by edge would make the repair visit only
the edit's buildings, but the repair also visits every building that still may act from before
(a persistent residue that a local visit would have to carry), and building attachments are
written at about ten sites plus direct pushes from extraction, topology and save code, each of
which would have to keep it; at 30k the repair costs 0.2 ms and the closure test 0.05 ms. The
agents' per-edge lists and planned-route index (option (d)) are not built either.

**Commit breakdown on `road` with `ROAD-68-E1`** (fresh, 2026-10-25, 04:28–04:31). Release
library tests, one session, pinned to the P-cores, eight workers, both timing locks, no build or
backup running: control `road` = `340270e5` (binary `c2c4fd37…`), change = this part rebased
onto it (`cc192880`, binary `a3dba815…`). The 100k tier is still rejected on `road` until
`ROAD-68-D` part 6 lands, so tiers 0 to 30k; medians of three samples, ms, control / change:

| Stage | 0 | 2k | 10k | 30k |
| --- | --- | --- | --- | --- |
| buildings / nodes / edges | 0 / 31 / 44 | 1,413 / 58 / 93 | 7,396 / 247 / 446 | 20,362 / 642 / 1,201 |
| click → ack | 16.0 / 17.1 | 84.1 / 76.7 | 36.9 / 33.2 | 96.4 / 83.9 |
| roadbed and terrain (`surface`) | 12.5 / 13.0 | 75.0 / 69.6 | 20.5 / 19.4 | 73.4 / 65.5 |
| agents and lanes | 0.30 / 0.37 | 0.47 / 0.41 | 0.65 / 0.52 | 0.82 / 0.89 |
| entrances (`buildings`) | 0.00 | 3.59 / 3.19 | 4.95 / 4.36 | 4.48 / 4.32 |
| render publication (`mesh`) | 3.00 / 2.90 | 3.82 / 2.45 | 5.07 / 3.84 | 5.86 / 3.48 |
| tool snapshot | 0.04 | 0.65 / 0.54 | 4.97 / 4.27 | 8.00 / 7.82 |
| snapshot publish | 0.02 / 0.04 | 0.04 | 0.07 / 0.05 | 0.11 / 0.23 |
| live lanes off the surface | 0.00 | 0.00 | 0.00 | 0.00 |

Per sample at 2k the first commit's entrances take 14.3 / 9.7 ms and its publication 6.1 /
5.1 ms. Both product digests equal the control's in all 12 samples. The `surface` stage is
`ROAD-68-D`'s commit validation. With `ROAD-68-E1` the snapshot publish is below 0.7 ms in every
sample, but the tool snapshot rose from 0.05 / 0.27 / 0.79 ms (part 5's rebased control, 2k /
10k / 30k) to 0.65 / 4.97 / 8.00 ms in the control as well (10.8 ms in one 30k sample);
`ROAD-68-E1` owns it (reported to the coordinator).

**100k before the rebase** (same session, 04:31–04:38; evidence for this part's stages only, not
acceptance): part 5's change binary (`e1d32537…`) against the same code with this part applied
on `a1861cf2` (binary `172ab26b…`). Per sample, click → ack 23.4 / 20.5, 19.2 / 17.9 and 13.7 /
16.0 ms; without the snapshot publish, which varies with the old snapshot it frees (1.1–4.3 ms in
both, `ROAD-68-E1` changes it), the commit takes 22.3 / 19.3, 15.0 / 13.7 and 12.6 / 12.0 ms.
In the first sample the agents and lanes take 3.84 / 2.11 ms and the publication 5.40 /
3.93 ms. Both product digests equal in all three samples.

**Determinism** (fresh, same session, `road68g_commit_determinism`, release, control and change
on `road`): one and three workers, a rebuild delayed 400 ms at one and eight workers and a save
inside the hold give identical hourly fingerprints (6) and saved tables (72) against the
eight-worker reference; both print reference digest `6252b7ba71c5a6f1`, parts 2 to 5's.

**Checks** (fresh, rebased `cc192880`): debug `cargo test --lib` 1,887 passed, 0 failed, 169
ignored (it includes the narrow-marks digest test, the product probe, the publication splice's
check against emitting every piece, the crossing runs' station-order assertion and the local
entrance update's check against a full derivation); `./run.sh --test` passes with `DISPLAY` and
`WAYLAND_DISPLAY` unset, skipping only `zoning_cells_tool_test` until `ROAD-68-F`; clippy adds no
warning (the baseline's 59); `roadbed/` holds 7,831 production lines, unchanged by this part.

- **Next.** Measure the 100k tier on `road` once `ROAD-68-D` part 6 lands; then, if a sample
  still misses, question 21's (b), the tool snapshot's growth since `ROAD-68-E1`, and the
  buildings-by-edge index and agent lists above.

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
> (`roadbed::NodeWalkways`), and `rounded_sidewalk_corner_path_xz` is removed. Since
> [follow-up 2](#one-crossing-per-degree-two-node-follow-up-2), every degree-two node crosses once
> again, as the legacy rule did; a pass-through node's crossing comes from its arm's span. Otherwise
> this contract is unchanged; the legacy lane audit measurements are archived.

One authoritative crossing record drives both pedestrian routing and zebra-marking rendering:

- an arm is crossable if and only if its lane rebuild emits a `CrosswalkMarking`. Lanes take
  the [D8](#product-decisions) rule from `roadbed::arms`, which the layout and the spans read
  too: every junction arm, one arm of every degree-two node and none at a road end, adjusted by
  node overrides. At a degree-two node the crossing arm is the first by direction angle in
  `(-pi, pi]`, then edge index; an override of `true` always crosses, `false` never does, and an
  arm without one crosses only if no earlier arm did; road arms alone count, so a footpath at a
  straight road gives the node a layout with one crossing. A bend whose crossing does not fit on
  its first arm crosses on the other. Structure ends and failed layouts have none ([open
  question 13](#road-68-open-questions))
- the marking is the layout's zebra strip: its kerb points are the two carriageway-edge points on
  the strip's crossing line, at the node plane's height, bitwise the node piece's
  (`NodePiece::crosswalk_kerbs`). A pass-through node has no node piece: its strip lies on the
  arm's span from the node out to 3 m, the span keeps a section row on its crossing line, and the
  kerb points are that row's carriageway-edge vertices, bitwise (`roadbed::span`
  `pass_through_crossing`, the span model the compile builds the row with)
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
  pass a zebra and backtrack. On a pass-through node's crossing arm they end on its crossing line,
  on the walkway centre of the span's section there. At other nodes without a layout they stop
  `config::SIDEWALK_NODE_INSET` beyond the clip
- at a node without a layout, pass-through nodes included, neighbouring sidewalks join by a
  straight walk, or by the node-centred arc where that walk would cut into the asphalt of either
  arm, judged against the arm centrelines (`TRAFFIC-04`). `ROAD-68-F` gives footpath nodes layouts
- a same-side reversal uses an explicit stationary connector at the mouth instead of crossing the
  road or reattaching farther along the reverse sidewalk
- a crossing is square to the road where it is drawn (`ROAD-64`). Both kerb points, and the two
  sidewalk ends they join, lie on one section line of the arm, at the strip's crossing station.
  Before `ROAD-64` the sidewalk lanes were trimmed separately and the graph endpoint's normal was
  used, so crossings on curved arms came out skewed

Crossing availability and geometry must not be independently reconstructed by the renderer,
selection queries, or agent movement. Since `ROAD-68-F` part 1 the renderer's zebra paint and
crossing signs read the crossings from the pieces (`RoadbedSystem::for_each_crossing`: a node
piece's layout kerbs, or a pass-through crossing's span-row kerbs kept on the span piece),
oriented by the lanes' own rule (`CrosswalkMarking::of_arm`: from the kerb on the positive
lateral side, whose connector owns the marking). Lanes and pieces compute the kerbs with the
same functions, so the painted marking is the lanes' marking bitwise, ends in order, and both
directions of its connector walk exactly that segment
(`zebra_markings_and_their_connectors_share_one_kerb_segment`: T, bend, pass-through and the
footpath fixtures, flat and sloped). A preview, which builds no lanes, paints the same zebras.

Footpaths (`TransitType::Foot`, `ROAD-55`) join the same junction graph as one-mouth arms. Until
`ROAD-55` the builder skipped them, so their lanes had no junction connectors and walkers could
never plan a route onto one; footpaths had been dead ends since pedestrians moved onto lanes.

- At a node with a layout (D5), a footpath's lanes end exactly at its mouth in the layout, on its
  centreline (`NodeWalkways::footpath_end`). An attached footpath joins the corner route of the
  sector it lies in where its centreline, walked in from the mouth, first meets that route
  (`NodeWalkways::footpath_join`), and follows the route both ways to the two sidewalk ends. Its
  join therefore stays on its ribbon and the sidewalk. At a single road arm the sector is the
  cap, whose route links both sidewalks. Footpaths meeting only footpaths, or roads without
  sidewalks, connect through the node. The road arms keep their crossings (D8, road arms only).
- At a node without a layout (a straight pair of footpaths, a failed layout) the legacy rule
  stays: a footpath's lanes stop where the path leaves the paved area of the roads there, a ray at
  angle `d` to a road arm of paved half-width `w` leaving it after `w / |sin d|` along the arm
  (capped at 10°) and after `w` away from it; where only footpaths meet, each stops 1 m short of
  the node. Between two road arms the footpath joins the node-centred corner walk at its nearest
  point; at a single road arm it joins both sidewalks directly.
- A footpath's dead end keeps a stationary turn-back connector, as every sidewalk mouth has, so
  the walking planner's whole-network proof holds on networks with footpath ends.
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

Placement accepted geometry no road could have. The profile solver treated its 16% grade as a
target and relaxed each span's limit to the terrain slope under it
(`solve_standard_road_profile`), and the preview measured `max_grade` without comparing it to
anything; since `ROAD-68-K14` the hill policy below bounds it. Nothing measures horizontal radius, kinks where two edges meet at a degree-2 node, or
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
| Grade, `Standard` | > 16% | > 1:3 (33.3%) |
| Grade, `Bridge` / `Tunnel` | > 6% | > 1:3 (33.3%) |
| Grade change per horizontal metre | > 0.018 | > 0.06 |
| Horizontal radius | < v² / (127 · 0.20), v in km/h | < 1.25 × visual roadbed half-width |
| Kink at a degree-2 node | > 3° | > 10° |
| Smallest angle between junction arms | < 60° | < 25° |
| Junction degree | > 4 | > 6 |

The illegal tier allows about 20% leeway over common Western design limits, so a city may be a
little more playful than an engineer would draw it, but not mangled: grade change 0.05 became
0.06 and the arm angle 30° became 25°. Grade is the user's hill policy
([`ROAD-68` open question 37](#road-68-open-questions), 2026-10-25): 1:3, just under the world's
steepest streets (about 35%; San Francisco's steepest are 31.5%), with the stroke solve's 16%
design grade as the warning (it was 10% before; 16% was the illegal tier, never refused).

The radius is measured over a window of three roadbed half-widths from each vertex, as the
window's horizontal length over its net signed turn. A polyline corner has no radius of its own,
and the profile solver densifies centrelines to 20 cm near junctions, so a per-vertex estimate
read a gentle 17° corner as a 3 m curve. Over the window, a stretch is illegal once it turns more
than about 137°: a hairpin, not a street corner.

Edges are judged on `physical_geometry`. A profile's grade is its steepest segment's
rise over horizontal run over segments of at least 5 cm (`grade::steepest_grade`): the one
definition the validator, the placement rule, the road tool's warning and the recorded replays'
quality check share. Arm directions at a node use the first metre of each
centreline, without clips, so sub-5 cm end segments do not decide them and a smooth curve split at
a node does not read as a kink. `validate_edge` is `O(points)`, `validate_node` is
`O(degree log degree)` plus the one-metre prefix, and `audit_graph` runs both over the graph with
Rayon in a deterministic order.

Placement refuses what an edit adds (`validate::edit_violation`, called from the preview and
commit validator once the edit's local graph is finalized and before its surface compiles):
illegal horizontal radius anywhere on the drawn road's pieces (the inserted edge and every piece
split off it), and an illegal junction arm angle or degree at their endpoints unless that node was
already as bad before the edit, so a bad junction can still be extended or repaired. The road tool
shows the measured value and the limit. Grade above 1:3 is refused as `road_too_steep`
(`ROAD-68-K14`): on the drawn road's pieces, and on every other profile the edit re-derives (the
arms its node planes re-pin) where it is steeper than before, so an existing steep road can still
be extended or repaired. Node edits (moves, removals, class changes, crossing toggles) refuse the
same way in `SimCore::try_graph_edit`. Profiles are built within the rule where their
constraints allow ([Heights](#heights)), and a stroke whose pins and source-delta limit force more
is refused before its topology is planned, with the least grade they allow. A valid preview above
the design tier carries `warning_reason = "road_steep"` and `warning_limit`, which the road tool
shows in amber. Not refused yet:

- grade change, which waits for the hill policy: the profile solver still follows bumpy ground,
  so refusing it would refuse, on the Kuopio DEM fixture, ordinary roads over it (0.067 per
  metre)
- direction kinks where two roads meet end to end: a street corner the bend piece rounds

An audit of the 58 local saves found no illegal radius; illegal arm angles at 20° Y junctions in
the `simple-city` saves and the `road-snowflake` star; the degree-10 snowflake; and illegal grade
change only on the steep Kuopio hillside roads (0.06 to 0.12 per metre).

Remaining: grade change, a debug overlay and preview colouring of design warnings other than
grade.

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
mesh is built, every asphalt triangle meeting a crossing's bar rectangle carries the crossing's
affine paint frame on its vertices, uncut since `ROAD-68-D` part 5 (open question 24):
`CUSTOM0` holds metres across and along the crossing, paint age plus one from the edge hash
(zero off a crossing), and the distance from the kerb. The surface keeps its position, normal and UVs, so the paint shares the asphalt's
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
The mesh gains 16 bytes per asphalt vertex; the paint frame test is O(owner's crossings) per
asphalt triangle, with no per-triangle allocations (the cut and its extra triangles went in
`ROAD-68-D` part 5). Frame
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
  middle. Real junctions (degree three or more) keep 6 m clear beyond their mouths; a road end
  keeps its round cap as clear, 6 m from the cap's start, its layout furniture station (the node,
  from where the arm runs straight to its mouth and span; `ROAD-68-F` part 1); a family change at
  a degree-two node, and a road end without a node piece, keep 2 m from the span end. Each edge
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

- **Pedestrian crossings (E1-1, E1-2).** One sign per approach at every zebra, taken from the
  pieces' crossing (`crosswalks::for_each_marking`, the marking the zebra paint reads and the
  lanes' `CrosswalkMarking` equals; `ROAD-68-F` part 1), on the right-hand side facing approaching traffic,
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
