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

**`ROAD-68` — done (2026-10-23 to 2026-10-30); every gate closed on branch `road68-H`
(2026-10-29), on `road` and `main` since 2026-10-30.** The legacy road-surface compiler is
retired and replaced by the [constructive roadbed](#constructive-roadbed-road-68) below.
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
- the legacy compiler stays available for visual comparison at `d6c80f91`, the last commit before
  `ROAD-68-A` removed it (tagged `road-legacy` until 2026-10-30)

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
   inputs. Road queries answer from the triangles that render. The preview's terrain is the
   exception (decided by the user, 2026-10-27): while a stroke is dragged its graded terrain may
   approximate, a large pose finishing its tiles over the following frames with the previous
   pose's tiles shown until then (`terrain.md` T27 (d)); the road surface, markings and junctions
   stay exact, and commit, undo and load stay bitwise.
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

Graph clips (`start_clip` / `end_clip`), the profile solve's hard zone, crosswalk placement,
pedestrian corner routes and vehicle lane ends all read the layout: a vehicle lane ends on its arm's
mouth section line at the clip (`MouthFrame`, `walkways.rs`), and junction connectors leave and
join along that section's direction ([`TRAFFIC-12`](traffic.md#junction-connectors)). `graph/rebuild/clips.rs` and
`junction_profile_crossing_core_m` no longer compute setbacks of their own, and pedestrian lanes
compute no crossing or corner geometry of their own
([Pedestrian Junction Crossings](#pedestrian-junction-crossings)).

**Cost.** Placement validation plans every affected node on every preview pose (`check_node`)
and the roadbed compile plans every closure node, so neither builds working collections per
node: the arms are collected, the layout is rebuilt in place and the mouth stations are kept in
the thread's buffers (`NodePlanScratch` in `roadbed/node.rs`, taken for a plan as the profile
solve's are; `NodeArms::collect_into`, `NodeArms::layout_into` and `JunctionLayout::rebuild` with
a `LayoutScratch`). Once they have grown to the node, `check_node` and `terminal_plane` allocate
nothing, and `check_node` evaluates each mouth's centre on the plane without building its
section; a plan allocates only what it owns: copies of the arms and layout, and its mouth and
approach sections (`ROAD-68-K18`, [open question 37](#road-68-open-questions)).

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
  control polyline it changed: road edits (both ends of each dirty edge), the crossing toggle,
  and, since the pre-D review, node moves (the moved node and its arms' far nodes)
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

**Terrain side superseded by [`TERRAIN-07`](terrain.md#graded-terrain-surface-terrain-07) (approach B, user 2026-10-25).** Terrain is
graded per 64 m tile from the outer chains, with a closed-coverage footprint mask in the terrain
shader and road skirts. `TERRAIN-07-G` deleted the exact/CDT cut; the historical contract below is
archival and has no live implementation. The
roadbed side stays as the contract terrain consumes:

- grounded outer chains, road on the left, joined at bit-identical mouth corners, `f64` positions;
  structure spans publish none
- the pieces' top triangles and the point-in-triangle predicate `height_at` uses, which `TERRAIN-07`
  takes as the footprint's coverage (no loops, so an open end flips no region; question 19)
  - **Exact since `ROAD-68-K19` (2026-10-26; the defect `TERRAIN-07-D` round 2 found); the
    closed rule confirmed by the user, 2026-10-26.** The predicate (`geometry.rs`
    `triangle_height_at_xz`, behind `top_triangle_height_at`, `height_at`,
    `carriageway_height_at`, the lane surface query, the disc test and the legacy ground-tile
    sampler) has no tolerance and holds the closed triangle: a point is held when none of its
    exact orientations against the three edges (`robust::orient2d` on the `f64` positions) has
    the sign opposite the triangle's winding, so edges and vertices are held by every triangle
    meeting there, and a triangle without area holds nothing (its orientations are all zero on its
    line and of mixed sign off it). Coverage is "inside some closed top triangle", exactly; a thin
    triangle holds its interior. Every caller takes the union or the highest of the holding
    triangles (`height_at`, coverage, the disc test, the ground-tile sampler, the stroke support's
    per-piece reads), so none needs a point in exactly one triangle: a shared edge's heights agree
    up to rounding, and a curb line, one XZ edge under a vertical face, reads the curb's top.
    The rule depends on no direction in the world: a junction turned by quarter turns or mirrored
    gives every point the same holding triangles and heights, bit for bit. A tie rule that gives
    each point on an edge to one side by a world direction (+x, then +z), tried first, failed
    that: it put junction foot-lane points on curb lines 0.12 m down on the carriageway or kept
    them on the curb by the curb's direction. The height keeps the predicate's arithmetic
    (`orient2d` returns the same determinant unless the point lies within rounding of an edge's
    line), so a point inside keeps its height bit for bit. The predicate is one type,
    `geometry.rs` `CoverageTriangle` (`ROAD-68-K20`): the winding (twice the signed area,
    nonzero) and its reciprocal are computed once; a point's three orientations are computed
    before the one test (`ROAD-68-K23`: branching after each orientation cost more than the third
    one); `triangle_height_at_xz` builds one per call after the box test. The graded raster
    (`coverage::raise_covered`, which the portal decks share) visits only the lattice inside a
    triangle's closed box, prepares only a triangle whose box holds a lattice point (most small
    ones hold none), and ends each row's scan with its held run, which is one run as the
    triangle is convex and the predicate exact (`CoverageTriangle::for_each_held_in_row`,
    `ROAD-68-K23`). Both answer bit for bit as the predicate always did. Before, the predicate accepted
    barycentric weights down to -0.001, up to 0.1% of a triangle's height past each edge (the
    sloped T covered 3,662 of 3,822 probes just outside its edges), and dropped triangles under
    0.0005 m². Tests: `roadbed/terrain/tests.rs`
    `the_coverage_predicate_covers_no_point_outside_its_triangle` (re-enabled),
    `coverage_equals_inside_some_top_triangle_exactly` and
    `coverage_and_heights_are_the_same_in_every_quarter_turn_and_mirror`, `geometry.rs`
    `closed_triangles_hold_their_edges_in_every_orientation` and
    `a_lattice_row_holds_exactly_the_points_the_predicate_holds`, and `terrain/graded/tests.rs`
    `the_coverage_raster_equals_the_predicate_at_every_lattice_point`. The changed products and their
    attribution are recorded under [question 37](#road-68-open-questions) (`ROAD-68-K19`).
  - **Lane ends on a kerb line (`ROAD-68-K20`; the crossing on the carriageway, taken pending
    the user's review, 2026-10-26).** A crossing connector's kerb ends, the marking's kerb
    segment, lie on the curb line, where the closed predicate holds both the carriageway and the
    curb's top; built in `f32`, they land a hair on either side, so a coverage lookup gave each
    the one or the other by rounding. They stand on the crossing, on the carriageway: D8 puts the
    kerb points "at the two carriageway edges", the crossing contract at the node plane's height
    with both directions walking the marking's segment, and `node::crossing_kerbs` builds them at
    `Level::Ground` (a span-owned crossing's are its row's carriageway-edge vertices); the curb's
    top would float the walk 0.12 m over the zebra. Constructively, not by an inset (which would
    move the connector off the marking's segment, or the marking off the kerb): the lane sync
    gives a crossing connector's second and third points the height of the published crossing
    kerb at the same point, bit for bit in XZ (`RoadbedSystem::for_each_crossing`, `O(degree)`),
    and no coverage lookup; an end the pieces hold no kerb for keeps its height. No fallback and
    no predicate change. Test: `road_reference_probe`
    `crossing_kerb_ends_stand_on_the_carriageway_in_every_orientation` (every reference junction,
    flat and sloped, in each quarter turn and mirror: every stroke commits, and every kerb end
    stands at the carriageway's height 1 cm inside the crossing and over 0.1 m below the curb's
    top 1 cm outside). A first version turned every fixture and found flat
    `y_25_branches_first`, turned by three quarter turns, refused; `ROAD-68-K23` traced it to the
    validator's arm angle ([`ROAD-54`](#road-geometry-validation-road-54)), fixed it there and
    turns every fixture again.
- `take_terrain_changes`, which since `TERRAIN-07-A` also returns the changed grounded chain
  segments themselves (`TerrainChanges::segments`, endpoints with heights), and since
  `TERRAIN-07-C` round 5 the bounds of the changed pieces with a portal row
  (`TerrainChanges::portals`)
- the portal rows of a tunnel span whose mouth a grounded piece shares
  (`PieceMesh::portal_rows`, `TERRAIN-07-C`) and `for_each_portal_part_in_bounds` (rows, top
  triangles and outer-chain segments of those spans), no terrain height
- the accessors `TERRAIN-07-A` reads them through, in `roadbed/query.rs`, no new geometry:
  `grounded_seams_in_bounds` (sorted by the bits of the start then the end) and its unsorted
  visitor, `grounded_pieces_in_bounds`, `for_each_grounded_top_triangle_in_bounds` and
  `top_triangle_height_at` (the predicate itself); bridge and tunnel spans are left out of both
  seams and coverage
- road skirts, one strip per grounded outer chain in the piece mesh (`TERRAIN-07-B`, built; see
  [pieces and meshes](#pieces-and-meshes))

- wall runs: where a grounded piece meets a bridge or tunnel it publishes a wall run across the
  mouth row (`PieceMesh::wall_runs`, `TERRAIN-07-C` round 3, [T22](terrain.md#terrain-07-open-questions)
  3 (a)), and a run across a bridge's mouth is marked (`PieceMesh::abutment_runs`);
  `RoadbedSystem::wall_seams_in_bounds` gives its segments the abutment face's bottom edge below
  their ends (`WallSegment::abutment_bottom`), where the graded terrain's abutment wall starts
  (`TERRAIN-07-D` round 7)

The road side of the exact cut (patch loops, holes, open ends, source tags, the grading envelope,
seams on tile sides, watertight patches, pads in the patch frame, patch selection, tile clip,
ownership refresh, the structures' patches) was deleted with the terrain CDT core by
`TERRAIN-07-G` and is archived in
[`archive/terrain_legacy_surface_2026-10-25.md`](archive/terrain_legacy_surface_2026-10-25.md#from-roadsmd-footprint-and-terrain-the-road-side-of-the-exact-cut).
Bridges and tunnels clip no terrain mid-span; their abutments and portals are `TERRAIN-07`'s
([T14](terrain.md#terrain-07-open-questions)).

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
new surface ([local commit publication](#local-commit-publication-road-68-g-part-2-2026-10-24)). A preview
sends its chunks below the chunk since `ROAD-68-H` round 3: the rebuilt pieces' triangles and the
published runs they replace, which Godot collapses in the resident chunk
([design](#round-3-sub-chunk-preview-payloads-the-poses-main-thread-work-the-city-scan-road-68-h-2026-10-29)). The Rust to
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
render chunk the stroke changes, built by the commit's own emission and splice into the view's
published chunks and sealed, so a stroke publishes exactly the previewed chunks. Since
`ROAD-68-H` round 3 the payload is below the chunk
(`RoadRenderer::generate_preview_overlay`): per changed chunk the rebuilt pieces' triangles,
emitted as the commit emits them, and the vertex ranges of the published runs they replace, with
those runs' positions; the published chunk with those runs dropped and the overlay merged is the
committed chunk bitwise, lights and signs aside, so a stroke publishes exactly the previewed
pieces. The view holds every published chunk (a persistent map shared in `O(1)`), not the
unacknowledged upserts. Since `ROAD-68-F` part 1 that holds for strokes
with crossings and signs too ([open question 26](#road-68-open-questions), closed): the
emission (`emit_into`) reads only the graph, the pieces and the terrain, never lanes. Zebra
paint frames and crossing signs take the pieces' crossings
(`RoadbedSystem::for_each_crossing`, oriented by `CrosswalkMarking::of_arm`, the rule the
pedestrian lanes own them by), and street lights their span sections and the terminal cap's
layout station. Tested bitwise in Rust
(`preview_chunks_with_crossings_and_signs_are_the_committed_chunks`: a T and a pass-through
continuation, flat and sloped; since round 3 the composed chunks, and
`a_preview_after_an_acknowledged_publication_composes_over_every_published_chunk`) and in
`road_junction_preview_test.gd` (every fixture, every layer and `road_crossings`, as triangle
sets of the composition). Godot's preview display draws the overlays, which carry the paint, over
the resident chunks with the replaced runs collapsed; the overlays carry no lights or signs, and
the resident chunks' lights and signs stay shown until the commit installs its own. The field and cell checks of the preview's pieces and the parcel
corridor check run at the commit only ([open question 25](#road-68-open-questions)). The
fingerprint is the road-tool surface
generation, the roadbed generation and the source and visual terrain generations: the roadbed
generation and the delta's dirty marks decide whether pieces install, so no per-edge or per-node
generation was added.

Stage 3 and the terrain half of 4 (round 3): the patches and tiles whose terrain the preview's
pieces change are the commit's (`road_terrain_tiles` over the preview roadbed's
`take_terrain_changes`, the function the commit's stamps use; round 4: tiles near changed chain
segments, or near changed pieces where a site grades, so a split trunk's far half is not
rebuilt, [patch selection](archive/terrain_legacy_surface_2026-10-25.md#from-roadsmd-footprint-and-terrain-the-road-side-of-the-exact-cut)); the preview's site test reads the published
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
Correction (`TERRAIN-07-E` round 5): the preview's terrain is now the graded tiles
([`terrain.md` "Preview, commit, undo and load"](terrain.md#preview-commit-undo-and-load)): each
frame hands Godot its changed shown tiles, which the graded renderer draws from slots of its own
over the installed tiles. The exact cut's preview patches above (`build_road_preview_terrain`,
`terrain_patches`, `terrain_generation`, `terrain_error`, `terrain_ms`) and
`road_terrain_preview.gd` are retired, with the road tool view's per-patch refined entries,
ledgers and site ownership that only they read; the view keeps the patches' sites.

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
Superseded by `TERRAIN-07-E` round 1 (2026-10-26): the commit has no terrain validation, so no
terrain failure can refuse a road; it refuses only field and cell overlaps and failed node
pieces. The preview stages its graded tiles and the commit adopts those whose inputs equal its
own ([`terrain.md`](terrain.md#preview-commit-undo-and-load)); the preview still builds the
exact cut's patches for Godot's preview until `TERRAIN-07-E`'s Godot half, but no commit reads
them.

**Undo.** The undo record holds the replaced pieces (`Arc`) and terrain patch revisions; undo
restores them by swapping.

As built (`ROAD-68-E` part 1): undo swaps the replaced pieces back when nothing rebuilt them
since, restores the graph in `O(delta records * log edges)`, and rebuilds the lanes of the
restored closure as a commit does (the agents on them detach and reattach, the connection lanes
of dropped nodes retire, and the entrances fronting the closure are derived again); a test
checks the result equals a full lane rebuild. A dense terrain
restore still rebuilds every lane, as it resets every road product. The record kept the refined
terrain entries the edit's validation replaced (round 3) until `TERRAIN-07-E` round 1 removed
the validation; it keeps the graded tiles the commit replaced instead, which undo adopts where
their inputs equal the restored ones ([`terrain.md`](terrain.md#preview-commit-undo-and-load)).
A terrain record keeps the source terrain, and undo restores the source and the visual buffer
from it (`TERRAIN-07-E`, authoritative terrain undo).

**Load.** Load first checks every saved profile against the 1:3 maximum grade, with the check an
edit meets: a road steeper fails the load with `road_too_steep` ([open question
37](#road-68-open-questions)). It then compiles every piece from the graph in parallel. A junction
that fails layout or the profile solve fails the load with its typed error. A load repairs
nothing: a save loads as saved or not at all (save compatibility is not required).

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
the entrances whose inputs the edit changed. Since part 3 the edit's dirty cell-lot regions are
the corridors of the roads whose geometry or curb frames changed, not of every road the edit
touched ([commit cell lots](#commit-cell-lots-road-68-g-part-3-2026-10-24)). Since part 7
([open question 33](#road-68-open-questions) (c)) the commit no longer regenerates them: it
takes them from the queue, publishes the road, and plans them in a [pure job](simulation.md#pure-jobs)
on the inputs it left, which the next frame adopts with the attachment repair and entrance update
that follow the lots, bitwise as planning in the commit
([commit stages, part 7](#commit-stages-part-7-road-68-g-2026-10-27)).

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
| Preview payload, per pose | `O(rebuilt pieces' triangles + closure owners * layers * log runs + hidden vertices)`: the overlay, the hidden runs by binary search and their positions; Godot stages the overlay and writes only the runs whose hiding changed (`ROAD-68-H` round 3) |

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
| D9 | Preview ghost style (`ROAD-73`, todo) | A product decision for the road preview only (the user, 2026-10-29): where terrain or buildings hide the preview, it draws a translucent blue pass with a rim glow, optionally blurred; drawing it through terrain only via the stencil is a design choice left to `ROAD-73`. It is not rule 5's render masking: the committed road and the terrain stay unmasked, and the probes check the raw state. |

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
| `ROAD-68-E` | done (implemented 2026-10-25, part 2 round 7; its two open items closed by 2026-10-29) | Synchronous preview, commit adoption, undo and load: part 1 has the context, the preview's road and terrain, commit adoption, local undo (pieces, lanes, terrain) and the load check; part 2 the user's decisions on the open questions (order-independent strokes, overlap checks, node moves, pinned knots, the cell store's journal-synced view copy). The items left open then are closed: the terrain stage's budget by `TERRAIN-07` (done 2026-10-29; the user's exceptions in `terrain.md`'s budgets and `ROAD-68-H`'s acceptance), and question 21 (b)'s preview half is not needed: by question 33 (c) the preview plans lots only if the commit misses a frame, and the commit fits one since `ROAD-68-G` part 10 (`ROAD-68-H` round 6: at most 13.79 ms at 100k). The row was left at "acceptance open" until the final audit corrected it |
| `ROAD-68-F` | done (2026-10-25, part 2 round 4) | Footpaths, mixed profiles, bridges, tunnels, crossings, street furniture and overlap queries: part 1 builds zebra paint, street lights and signs from the pieces for preview and commit alike (question 26 met); part 2 the road families without terrain (roads without sidewalks, mixed profiles, bridge decks, tunnel road pieces, one overlap walk), and round 4 fixed the preview payload's codegen regression. Moved to `TERRAIN-07`: bridge abutments and tunnel portals (`TERRAIN-07-C`, with closing the grounded chains at structures, open question 19), building sites, driveways and paving (`TERRAIN-07-C`), bridge fascias (`TERRAIN-07-B`). Left open: question 17's item on footpaths in a sector without a sidewalk |
| `ROAD-68-G` | done (2026-10-29, part 10) | Commit consequences inside one frame (routing, flow fields): D7 landed; part 2 made the publication, entrances and agent pass follow the edit; part 3 made the commit's lots follow the changed roads (28.6 ms at 100k, 41.7 before); part 4 21.8 ms; part 5 fixed the pure-job deadlock, 19.3 ms median before the `ROAD-68-D` rebase; on `road` the 100k commit is rejected and its terrain validation costs 15–69 ms (`ROAD-68-D` part 6); part 7 (on `terrain07-E`) defers the commit's lots one frame (question 33 (c)), bitwise: first commits 16.0–17.3 ms at 10k–100k (control 25.8–29.7); part 8 starts the routing rebuild after the lot plan and the adoption tick a frame after the commit (question 58): adoption frames 0.3–2.3 ms, first commits 14.9–17.9 ms (three of twelve populated first samples miss), locality products bitwise from 0 to 100k remote sites but about 3 ms dearer at 100k; part 9 (rebased onto gate E) makes the adoption's attachment repair follow the edit (flat to 100k remote sites): first commits 14.4–18.2 ms (three of twelve populated first samples miss), the agent passes and graph copy open ([open questions 21, 59 and 60](#road-68-open-questions)); part 10 rebased onto `TERRAIN-07-G` (the legacy terrain state gone) and queues the commit's kept terrain tiles after its last stage: every populated first commit 9.6–13.8 ms at 10k–100k (control 21.1–27.5), every later sample at most 7.5 ms, products and agent records bitwise; questions 58 (b), 59 (b) and 60 (a) agreed by the user and carried to H with the locality growth and the fiscal contract means ([record](#commit-stages-part-10-the-gate-road-68-g-2026-10-29)) |
| `ROAD-68-H` | gate commit on `road68-H` (2026-10-29/30, rounds 6-7); on `road` 2026-10-30 after the user's review (the comparison clips: the final tip's undo hold; question 66 (a)) | Rendered acceptance and final cleanup: every target passes or is carried by the user's decisions (question 65 to `TERRAIN-13` and `RENDER-30`, T63, `PERF-12`, T65); the terrain stage's 3.01 ms catch-up frame accepted as measured by the user ([question 66](#road-68-open-questions) (a)), [acceptance](#round-6-the-users-t64-change-t65-the-runs-on-the-final-tip-road-68-h-2026-10-29) |

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
   loops take theirs from their rays ([footprint and terrain](archive/terrain_legacy_surface_2026-10-25.md#from-roadsmd-footprint-and-terrain-the-road-side-of-the-exact-cut)).
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
   **`ROAD-68-K21` (2026-10-26): the load repair is removed.** Save compatibility is not
   required (question 37, decided by the user 2026-10-26: no legacy-load path is kept), so a
   load re-clips and re-solves nothing: a save whose junction fails its piece or has no plane
   fails the load with its typed error. The repair's only test,
   `headlight_house_junction_full_load_compiles_with_terrain_earthwork`, and its pre-`ROAD-68-C`
   fixture went with it (record in question 37, "Save compatibility").
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
    - **`TERRAIN-07-C` round 4 (2026-10-26):** the road chunks close at those mouths too. A
      bridge span hangs an abutment face from its slab's end edge down to the skirt depth at
      each mouth it shares with a grounded piece ([T22](terrain.md#terrain-07-open-questions) 4
      (a), the bottom edge taken pending the user's review), so the family chunk check runs
      without its concrete-row exemption. The terrain over a tunnel past its mouth row is not
      opened yet (T22 5, detailed for round 5).
    - **`TERRAIN-07-C` round 5 (2026-10-26):** the graded terrain opens over a tunnel's deck at a
      mouth on the ground (the portal opening, [T22](terrain.md#terrain-07-open-questions) 5 (a)):
      the tunnel span marks the row it shares with a grounded piece a portal row
      (`PieceMesh::portal_rows`), and the bake cuts the ground over the deck within 10 m of it
      wherever the ground stands less than 5 m above the deck, with side walls along the
      tunnel's outer chains and no headwall where it opens. The roadbed reads no terrain height.
      With it the structure half of this question is closed in gate C; a failed node's gap still
      fails its rectangle.
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
    2026-10-25: correct topology, no defect; decided by the user, 2026-10-26: (a)).** In one
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
    with the five known failures. **Decided by the user, 2026-10-26: (a).** The runner reports
    each tier's census closure passes, and the two limits apply to the searches and routes per
    pass. Not built yet; it lands with `ROAD-68-G`'s perf-contract run.

21. **The commit's other stages exceed a frame (`ROAD-68-G` measurement) — cell lots decided
    2026-10-24 (user): profile and speed up lot regeneration first; if it still does not fit,
    `ROAD-68-E` builds the lots in the preview and the commit installs them (option (b)). The
    other stages stay open until the commit fits a frame. The commit fits since `ROAD-68-G` part
    10 (2026-10-29); the open parts are questions 59 (b) and 60 (a), agreed by the user
    (2026-10-28) and validated at `ROAD-68-H`.** With the CCH
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
    - **`ROAD-68-G` part 7 (2026-10-27, fresh, branch `road68-G7` on `terrain07-E`): question
      33 (c) built.** The commit plans its lots in a pure job adopted by the next frame, bitwise
      as planning in the commit; the background jobs start after the commit's last stage and
      the changed chunks splice on the pool. In the landing session (C = `terrain07-E`, the
      game's path, eight workers) the first commits take 15.97-17.34 ms (C 25.83-29.68) and
      every other sample 3.0-11.8 ms; only the first 100k sample (17.34, 16.86 ms) and once the
      first 30k sample (17.10 ms) miss 16.7 ms. The largest stage left there is the legacy
      refined patches' road ownership (3.6-4.2 ms), which `TERRAIN-07-D` and `-G` remove, then
      the publication (2.9-3.1), the graded stage (2.1-2.2), the tool snapshot's graph copy
      (2.4, cloned on the pool since), the agent passes and lanes (3.4-3.6)
      ([commit stages, part 7](#commit-stages-part-7-road-68-g-2026-10-27)). Target still
      missed at 100k, open.
    - **`ROAD-68-G` part 8 (2026-10-27, fresh, the same branch).** The routing rebuild starts
      after the lot plan, and the sim thread ticks a frame after a commit that deferred its lots
      (question 58), so the adoption frame takes 0.3-2.3 ms at every tier. In the landing
      session the first commits take 14.9-16.5 ms in nine of twelve populated runs and 17.0-17.9
      ms in three (10k, 30k; control 25.8-30.3), the legacy terrain state their largest stage.
      The locality fixture keeps the commit's products bitwise from 0 to 100k remote sites, but
      at 100k each sample costs about 3 ms more (graded stage, graph copy, plan and undo, agent
      passes) and the adoption's attachment repair 3-6 ms more (its whole-city test)
      ([commit stages, part 8](#commit-stages-part-8-road-68-g-2026-10-27)). Target and
      locality still open.
    - **`ROAD-68-G` part 9 (2026-10-28, fresh, branch `road68-G9` on `road` `c64786e9`).**
      Parts 7 and 8 rebased onto gate E; the adoption's attachment repair tests only the
      buildings in the 512 m chunks its edit reaches, checked against the whole city's test in
      test builds: at 100,000 remote sites 0.07-0.17 ms (2.8-8.1 ms before), flat from 0. In the
      landing session the first populated commits take 14.4-16.6 ms in nine of twelve runs and
      16.79, 16.95 (10k) and 18.17 ms (30k) in three (control 24.5-32.3), the legacy terrain
      state their largest stage. Option (d) is [open question 59](#road-68-open-questions), the
      graph copy question 60 ([commit stages, part
      9](#commit-stages-part-9-road-68-g-2026-10-28)). Target and locality still open.
    - **`ROAD-68-G` part 10 (2026-10-29, fresh, branch `road68-G10` on `road` `a0cfe039`, the
      gate).** Rebased onto `TERRAIN-07-G`, which removed the legacy terrain state; the commit
      queues its undo record's kept terrain tiles after its last stage, so their bake no longer
      takes the commit's cores. In the second landing session every populated first commit takes
      9.6-13.8 ms (control 21.1-27.5) and every later sample at most 7.5 ms: the target is met at
      every tier ([commit stages, part 10](#commit-stages-part-10-the-gate-road-68-g-2026-10-29)).
      The open parts of this question are [question 59](#road-68-open-questions) (b), the agent
      passes, in its own round after the gate, and question 60 (a), the graph copy, kept until
      it shows in a frame; both agreed by the user (2026-10-28) and validated at `ROAD-68-H`.
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
      patches](archive/terrain_legacy_surface_2026-10-25.md#from-roadsmd-footprint-and-terrain-the-road-side-of-the-exact-cut)); the reference fixtures do not need the shared height
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
      by vertex bits ([source tags](archive/terrain_legacy_surface_2026-10-25.md#from-roadsmd-footprint-and-terrain-the-road-side-of-the-exact-cut)): products depend on the footprint
      only, and the debug display's face sources (`*_face_sources`, the terrain inspector's
      summary) show run numbers instead of edge and node ids. (b) As (a), and map runs back to
      pieces in the debug export only, by looking up the current roadbed's chains at the run's
      first segment; costs a lookup per exported source, changes no product. (c) Keep graph
      labels and rebuild every tile a renumbered piece reaches (the whole trunk), which misses
      the budget for no geometric reason.
    - **Recommendation.** (a) now; (b) when the debug inspector needs edge ids again.
29. **The commit's terrain at city scale (`ROAD-68-D` part 6) — superseded by `TERRAIN-07`
    (2026-10-25; the header said "open" until the final audit).**
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
    - **Measured with `TERRAIN-07` (`TERRAIN-07-E` round 2, 2026-10-27, fresh; the commit
      breakdown's game path, `METRUM_COMMIT_PREVIEW`, eight workers, without the test-only
      checks, [`terrain.md` gate E record](terrain.md#terrain-07-e-record)).** The commit still
      misses the frame on each populated tier's first sample: 29.0-30.1 ms at 10k, 25.0-25.7 at
      30k, 27.0-27.2 at 100k, of which `buildings` (the lot regeneration) 10.2-10.7, 7.3-7.5 and
      8.0-8.2 ms; the later samples take 5.7-16.1 ms. The deferral (c) starts from those first
      commits: at 100k it would leave about 19 ms (mesh 3.5-3.9, lanes 2.0-3.1, snapshot 2.3-2.4,
      surface 2.3-2.4 ms next), so it alone does not fit the frame.
    - **Built (`ROAD-68-G` part 7, 2026-10-27, fresh).** A commit whose edit queued lot regions
      takes them and publishes its road; a pure job on the background pool plans them on the
      road tool view's cell copy, the parcel store shared copy-on-write, the published graph and
      roadbed and snapshots of the fields and explicit sites; the next frame (D7's held frame),
      or any editor path first, adopts it with the attachment repair and entrance update. The
      adopted products equal planning in the commit bitwise (a new test over a commit sequence
      with an undo while the job runs, and the breakdown's products at every tier). Landing
      session: the first commits fall from 25.8-29.7 to 16.0-17.3 ms at 10k-100k; the first
      100k sample still misses 16.7 ms by 0.2-0.6 ms
      ([commit stages, part 7](#commit-stages-part-7-road-68-g-2026-10-27)).
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
      under the user's delegation, 2026-10-25 night, confirmed by the user 2026-10-26: the user's 1:3 named no class,
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
      delegation, 2026-10-25 night; (1) and (3) confirmed by the user 2026-10-26, (2) reversed).** (1) The whole-graph terrain sync applies the
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
      the source terrain. Decided by the user, 2026-10-26: as recommended, in `TERRAIN-07-E`
      once terrain undo restores the source terrain. **Built in `TERRAIN-07-E` round 1
      (2026-10-26):** a terrain record keeps the source terrain, and undo restores the source
      and the visual buffer from it; `finish_terrain_authoring_edit` records every road's grade
      before the whole-graph sync and, when the sync leaves one steeper than 1:3 and steeper than
      before (`steepened_profile`, the node edits' check), rolls the stroke back from its undo
      record with `road_too_steep` (`O(edges)`, as the sync). Tests:
      `a_terrain_edit_that_steepens_a_road_past_one_in_three_is_refused` (a 20 m mound under a
      flat road: refused, source and visual terrain and the road restored bitwise), and
      `terrain_stroke_revalidates_cell_growth_and_undo_restores_feasibility`, whose mound now
      stands 10 m beyond the lot, away from the road (its old mound reached the road and is
      refused), and whose undo restores the lot's feasibility. (2) Old saves load as saved (reversed by the
      user, 2026-10-26, below): a save with grades over 1:3 loads
      unchanged and only new edits are refused (recommended and taken: refusing the load would
      lose the player's city over a rule it predates, and every edit beside such a road stays
      possible because placement refuses only a profile it makes steeper). (3) The first planning
      pass can refuse `road_too_steep` from a carriageway pin its own edit rebuilds before a
      second pass runs (a refused pass has no plan, so no rebuilt pieces to exclude). Fix only if
      it changes a verdict: a probe counting refused passes that read any carriageway found none in
      the debug `nodes::` (492 tests, the Kuopio replays and road tool tests included) and
      `simulation::` (1,294) suites, so it is left as is and recorded.
    - **Save compatibility (decided by the user, 2026-10-26): not required.** This reverses (2).
      A save with a road steeper than 1:3 is refused at load with `road_too_steep`, the same rule
      an edit meets, rather than loaded as saved or re-graded. No migration or legacy-load path is
      kept for older saves: an old save may fail to load. The re-recorded Kuopio reference
      (question 38) is within the rule. **Built in `ROAD-68-K21`.** The load
      (`save::network::rebuild_loaded_graph_runtime`, which `load_from_sqlite` runs before the
      lanes and agents are restored) first checks every saved profile, as saved, with the check an
      edit meets: the validator's `grade_violation` (the steepest segment, `steepest_grade`,
      against its class's tiers in `RoadGeometryLimits::default()`, illegal above
      `ROAD_GRADE_MAX`), which `validate::edit_violation` and the node edits'
      `steepened_profile` refuse with. The first edge over 1:3 in id order fails the load before
      any piece is compiled: `road surface load failed: edge N climbs at G% at S m, steeper than
      the 33% (1:3) maximum grade (road_too_steep)`. O(saved profile points), no allocation. Test:
      `a_saved_road_steeper_than_one_in_three_fails_the_load` (a road beside a flat edge 0, saved
      and loaded: at 30% it loads, at 40% the load fails naming edge 1). In the game the error
      takes the existing load failure path: `SimulationNode::load_game` logs `Load failed: …`
      with `godot_error!` and returns `false`, the input manager reports the failed path, and the
      live city stays as it was (checked headless, `ROAD-68-K21`'s gate checks below).
      Legacy-load code, listed before deleting, removed as dead under this decision:
      1. The question 8 load repair in `rebuild_loaded_graph_runtime` (re-clipped and re-solved the
         rejected grounded junctions of pre-`ROAD-68-C` saves before the node checks; about 40
         lines), with `rejected_loaded_nodes` and the `terrain` parameter only the repair read. Its
         only test, `headlight_house_junction_full_load_compiles_with_terrain_earthwork`, and that
         test's pre-`ROAD-68-C` fixture (`save/tests/data/headlight_house_junction.sql`) go with
         it: without the repair the fixture fails the load with `node 1 piece did not build
         (MouthOffPlane { arm: ArmKey { edge: 1, end: Start } })`. The
         full debug lib suite with both removals (fresh, before the test's removal): 1,951 passed,
         only that test failed, 171 ignored, so no other test reached the repair or the branches
         below.
      2. The missing-table branches of the service policy and budget history loads (and
         `sqlite_table_exists`, which only they called): both tables are in the schema since
         2026-07-25, every save writes them (the service policy's row included), and only
         `SAVE_VERSION` 107 (2026-10-22) loads, so no loadable save lacks them.

      Listed, not deleted (doubtful): the lane restore's legacy branches in `save/agents/lanes.rs`
      (connectors where a footpath arm no longer splits the road, pedestrian connectors the
      layout no longer routes, lane ids renumbered before `ROAD-65`), which also take a save whose
      lanes were rebuilt differently, such as one written while bulk commits defer the lane
      rebuild; the saved-parcel quarantine and repair in `save/world.rs`
      (`legacy_saved_parcel_quarantine`, `repair_quarantined_loaded_parcels`, with the census and
      workplace-quote paths that read the quarantine), which a road overlap may reach in a current
      save and which is not small; and two economy cases outside this gate, the ledger slots of
      bare household records (`households/admission.rs`) and saved routes older than the turn
      rules (`agents/remap.rs`). Until `TERRAIN-07-E` refuses a terrain edit that leaves a road
      over 1:3, such a city's save fails to load: [open question 48](#road-68-open-questions)
      (closed by `TERRAIN-07-E` round 1, which refuses it).
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
    - **The allocation-free node plan (`ROAD-68-K18`).** `check_node` checks every affected node
      on both planning passes of every preview pose, and did so by planning it as the roadbed
      compile does: arms collected into new vectors, a layout built with its working lists, the
      mouth stations, and every mouth and approach section, all dropped again. Heaptrack (release
      lib test binaries, the preview stages benchmark on 4 workers, 962 previewed poses;
      allocation counts, not timing), C = `road` `062575d9`, F = `road68-K18`: `check_node`
      allocates 222,520 times in C (5,784 checks, 38.5 a check, 231 a pose) and 69 in F, the
      threads' first growths. In C the node plans of all callers (9,219) allocated 350,680 times:
      the arms' records, knots, centrelines and bands with the incident and road lists 139,966,
      the layout's working lists and output vectors 129,957, the mouth sections 58,539, the
      approach sections 12,999 and the station list 9,219. The plan stage as a whole
      (`plan_road_edit`) allocates 227,693 times in F against 450,320 in C. The buffers stay with
      each thread (`NodePlanScratch` in `roadbed/node.rs`, taken for a plan as the profile
      solve's are): the arms are collected into reused vectors (`NodeArms::collect_into`, the
      incident and road lists in an `ArmsScratch`), the layout is rebuilt in place
      (`JunctionLayout::rebuild`, whose `LayoutScratch` holds the attached arms, a sector's sample
      inserts and outer samples, a boundary's emitted samples, the core loop's runs and pieces,
      and the attachments' path, ribbon, spans and reordering), the layout inputs keep one vector
      across lifetimes (std's in-place collection, `arms.rs` `recycle`), and `check_node` places
      each mouth's centre on the plane (`ArmCentre`, which the mouth sections use too) instead of
      building the sections; `terminal_plane` shares the buffers. The allocating `layout()` that
      the clip layouts and walkways still call keeps the original capacity hints, so it allocates
      no more than before (`layout_mouth_stations` 167,510 in F against 173,294 in C). The roadbed
      compile's 3,435 plans allocate 56,335 times in F (16.4 a plan):
      what a plan owns, copies of the arms (four vectors) and the layout (up to six), its mouth
      sections (the `Arc`, the row's vertices and the section's bands) and its approach
      sections. Left: `junction_rejection`'s sorted node list (one a call, 1,928) and those owned
      copies, which the compile keeps until it builds the pieces. The checks, their order and
      every product are unchanged (fresh, in a debug build of F before a formatting-only change,
      a lint attribute and the capacity hints, and again in the final code's release build): the
      roadbed fixture and reference junction product probes, the
      narrow-marks digest, the Kuopio quality tests (9 passed, 7 ignored), the watertightness
      tests (8, the reference junction's road chunks among them), the save determinism
      test, `road_preview` (15, the preview-equals-commit fixtures), `roadbed::` (60), `editing`
      (28), the clip tests (6), and the Kuopio reference re-recorded twice at `3c1dbc27…` with the
      fixture unchanged.
    - **Timing (`ROAD-68-K18`, fresh, 2026-10-26).** Release lib test binaries (`$TEST_BINARY`),
      C = `road` `062575d9` (built in its own worktree and target directory), F = `road68-K18`'s
      code before the capacity hints of the allocating `layout()` (five `reserve` calls, no
      effect on reused buffers). Both locks, `taskset -c 0,2,4,6,8,10,12,14`, no restic, `METRUM_DEBUG=0`, no
      compiler, cargo, Godot or heaptrack process at any 10 s sample of the session; 07:25-07:48:
      `benchmark_road_preview_stages` with `RAYON_NUM_THREADS=4`, C F F C twice, then
      `benchmark_populated_commit_breakdown` with `METRUM_COMMIT_TIERS=0,10000,30000,100000` and
      `RAYON_NUM_THREADS=8`, C F F C. Every payload vertex count and every breakdown digest is
      equal across the binaries.
      | Preview stage p50, ms | C | F |
      | --- | --- | --- |
      | `sloped_t` plan | 0.474-0.479 | 0.453-0.456 |
      | `sloped_branch` plan | 0.413-0.417 | 0.395-0.399 |
      | `sloped_t` roadbed | 0.321-0.325 | 0.320-0.323 |
      | `sloped_branch` roadbed | 0.325-0.328 | 0.323-0.326 |
      | `sloped_t` payload | 0.381-0.389 | 0.382-0.389 |
      | `sloped_branch` payload | 0.379-0.384 | 0.383-0.388 |
      | `sloped_t` terrain | 4.380-4.451 | 4.395-4.463 |
      | `sloped_branch` terrain | 4.686-4.769 | 4.681-4.751 |
      | `sloped_t` total | 5.533-5.628 | 5.536-5.609 |
      | `sloped_branch` total | 5.812-5.879 | 5.763-5.845 |
      The plan stage, which holds both planning passes' node checks, falls by 0.014-0.026 ms in
      every run, about what the 231 allocations and frees a pose it no longer makes cost; the
      roadbed stage, whose compile plans the closure nodes, reads 0.000-0.005 ms lower. The
      payload stage, which no `ROAD-68-K18` code reaches, reads up to 0.004 ms above C's spread
      on `sloped_branch` in two of four runs. Profiled (the preview stages under `perf record -c
      200000`, user cycles and instructions with LBR call chains, the same binaries and workers,
      C F C F, 08:10-08:11, not acceptance; samples whose chain passes
      `RoadRenderer::generate_preview_chunks`): F retires 6% fewer and 1% more instruction
      samples than C (434 / 453 against 462 / 450) and spends 5% more and 1% fewer cycle samples
      (1,007 / 915 against 955 / 928), the same work; the chunk splice
      (`NetworkMeshData::splice_owners`, `push_street_light`, `reserve_layer`) is inlined into
      `generate_preview_chunks` in F and out of line in C. Layout, question 36's case, not work
      this change adds.
      | Commit breakdown, ms (two runs each) | C | F |
      | --- | --- | --- |
      | 10k sample 0 plan | 0.463 / 0.436 | 0.418 / 0.425 |
      | 10k sample 0 surface | 84.42 / 85.03 | 84.40 / 91.60 |
      | 10k sample 0 commit | 103.73 / 104.90 | 104.16 / 112.58 |
      | 30k sample 1 plan | 0.410 / 0.382 | 0.390 / 0.367 |
      | 30k sample 1 surface | 52.57 / 51.34 | 51.86 / 52.52 |
      | 30k sample 1 commit | 58.84 / 58.05 | 58.19 / 58.62 |
      | 100k sample 0 plan | 0.521 / 0.486 | 0.489 / 0.553 |
      | 100k sample 0 surface | 81.12 / 76.94 | 78.26 / 75.97 |
      | 100k sample 0 commit | 99.90 / 96.70 | 98.60 / 96.32 |
      The commit's plan stage, which checks the edit's nodes once, reads at or below C's spread
      in 20 of 24 F samples over all tiers; the other four are single runs whose pair lies within
      it (10k samples 1 and 2 by 0.012 and 0.003 ms, 100k samples 0 and 1 by 0.032 and 0.073 ms),
      and every F plan sample of the 10k re-measure below lies at or below C's. The roadbed
      sub-stage of the surface stage, whose compile plans the closure nodes, lies within C's
      spread. The second F
      run read higher in all three 10k samples, in the legacy terrain sub-stage (10k sample 0
      surface 91.60 against C 84.42-85.03). Re-measured at 10k alone, C F F C (08:13-08:15, both
      locks, the profile analysis paused, no compiler, cargo or Godot process): sample 0 surface
      C 89.09 / 89.63, F 85.30 / 84.72; sample 1 C 14.06 / 13.92, F 13.81 / 13.94; sample 2 C
      8.86 / 8.15, F 8.77 / 10.45, so the sample 0 reading was noise, while 10k sample 2's
      terrain sub-stage reads above C's spread in two of F's four runs (9.74 and 10.45 against C's
      8.15-9.13) in code `ROAD-68-K18` does not reach. Open: that reading needs a commit-window
      profile (question 36) before it can be put down to layout; the 10k commit-window profiles
      (C F C F, 09:09-09:10) are analysed in the next item.
    - **The 10k commit-window profiles (`ROAD-68-K18`'s, analysed by `ROAD-68-K19`, 2026-10-26)
      — not layout: the windows' work differs between runs of one binary; open.** The captures
      (`perf record -c 200000`, user cycles and instructions with LBR call chains, sampled only
      inside each commit window, all threads; C = `062575d9`, F = `road68-K18` before its
      capacity hints) split into the 10k tier's three sample windows by the gaps between them.
      Instruction samples per window, in total, in the legacy terrain grading
      (`interpolate_grid_height`, `road_loop_grading`, `rect_loops`), in the terrain CDT
      (`spade`, `terrain::cdt`) and in the CCH ordering (`pathing::cch`, the previous commit's
      asynchronous rebuild, which shares the Rayon workers):
      | Run | sample 0 | sample 1 | sample 2 (window length) |
      | --- | --- | --- | --- |
      | 1 C | 8,588 / 1,145 / 1,556 / 69 | 1,854 / 204 / 316 / 84 | 1,232 / 131 / 302 / 46 (14.0 ms) |
      | 2 F | 9,393 / 1,524 / 1,549 / 244 | 2,736 / 547 / 305 / 198 | 1,728 / 225 / 304 / 261 (19.4 ms) |
      | 3 C | 9,111 / 1,201 / 1,480 / 171 | 2,238 / 223 / 319 / 174 | 860 / 33 / 177 / 45 (15.7 ms) |
      | 4 F | 8,679 / 1,151 / 1,474 / 38 | 2,200 / 372 / 351 / 106 | 1,278 / 205 / 251 / 46 (15.5 ms) |
      The legacy grading work in sample 2's window varies fourfold between C's own two runs (131
      against 33 samples) and the CDT's 1.7-fold, and the CCH rebuild reaches into the windows by
      38 to 261 samples, so the work a window holds depends on the run, not only on the binary,
      and question 36's condition (identical work) cannot be met: the reading is not recorded as
      layout. Both F runs grade more in that window (225 and 205), which two runs per binary do
      not separate from the run-to-run spread; none of the varying symbols is node-plan code.
      Open, as a property of the benchmark rather than of `ROAD-68-K18`'s change: a re-measure
      needs commit windows that hold only the commit's own work (for example the previous
      sample's asynchronous CCH and terrain jobs finished before the next sample's click) and more
      than two runs per binary; the legacy terrain sub-stage is `TERRAIN-07`'s to delete.
    - **The R-tree's traversal order (`ROAD-68-K18`, record) — decided by the user, 2026-10-26: (b).**
      `ROAD-68-K17` left the R-tree's removals because re-indexing only the changed edges would
      change the tree's shape, and with it the order in which `get_edges_near_aabb` returns edges.
      Recorded on `road` `062575d9` without changing behaviour (no number in this branch's range
      35-39 was free, so the question is recorded here). Of the collecting query's 13 production
      call sites (`get_edges_near_aabb` and `get_edges_near_point`, which wraps it), eight sort
      their result first (cell generation's neighbourhood, the stroke's snap candidates in
      `topology.rs`, the validation excerpt, the buildable frontage projection, site grading, the
      service and repaired frontages, and the building renderer's frontage) and three reduce it in
      an order-free way (the parcel road overlap's `any`, the service site overlap, whose rejection
      is the same whichever edge overlaps first and only its debug log names the edge, and the undo
      capture's hash set). Two depend on the order: the road tool's cursor snap (`interaction.rs`
      `snap_to_network`, its edge pass) keeps the first edge at the smallest snap distance, so on an
      exact tie it snaps to whichever edge the tree yields first; and the bulldozer's road pick
      (`editing.rs`) breaks ties within `f32::EPSILON` by edge id, a tolerance that makes the
      comparison intransitive, so a chain of nearly equal distances can still pick by order. The
      visiting query (`visit_edges_near_aabb`, 7 sites) has no order-dependent caller: sorted
      intervals, a boolean, a minimum, or a best candidate with an edge-id or render-id tie-break.
      Sorting the collecting query's result by edge id (experiment S: that one change on `062575d9`,
      debug, not committed) changes no tested product: every K18 product test above passes with its
      pinned products and the Kuopio reference re-records at `3c1dbc27…` twice. The full debug suite
      on S, which checks the zoning and building products the sorted callers feed (fresh,
      `ROAD-68-K19`, 2026-10-26 09:47-10:00, `ROAD-68-K18`'s S debug lib test binary, 8 test
      threads): 1,944 passed, 0 failed, 172 ignored, the counts of `ROAD-68-K18`'s own gate run on F;
      the experiment's patch is discarded. So, for every tested product, the tree's order reaches a
      product only through the two pickers' exact or near ties, which no fixture exercises. Options:
      (a) sort in `get_edges_near_aabb` (`O(K log K)` for every caller, eight of which sort again);
      (b) sort the two pickers' candidates by edge id, as the eight callers do, and document the
      collecting query's order as unspecified; (c) keep the tree's shape fixed, as today's removal
      and re-insertion of every touched edge does. Recommended: (b), landed with the gate that
      re-indexes only changed edges (`ROAD-68-K17`'s first open item); for those two callers it is
      experiment S's order, so it changes no tested product, and it also removes a save/load hazard
      the order is today: a loaded graph builds its tree in edge order, an edited graph by its edit
      history, so the two pickers can choose differently on an exact tie in the same city. Left open
      for the user's review: nothing in `ROAD-68-K18` depends on it (record only).
      **Built in `ROAD-68-K21`.** The cursor snap's edge pass (`get_closest_network_snap_xz`) and
      the bulldozer's road pick (`resolve_bulldoze_road_target`) sort their candidates by edge id,
      as the eight sorting callers do, and keep the first edge at the smallest exact distance, so
      an exact tie takes the lower edge id. The bulldozer's `f32::EPSILON` tie band is removed: it
      picked the lower id over an edge nearer by less than the band, and made the comparison
      intransitive. `get_edges_near_aabb` (and `get_edges_near_point`) document their order as
      unspecified, the tree's traversal order, and `visit_edges_near_aabb` no longer asks to keep
      it. Cost: one `sort_unstable` of the candidate list each query already returned, `O(K log K)`
      for the `K` edges within the pick radius, nothing allocated. Tests, each building the tree
      from two insertion orders: `cursor_snap_ties_take_the_lowest_edge_id_in_any_tree_order` and
      `bulldoze_road_pick_compares_exactly_and_ties_take_the_lowest_edge_id`, which also places a
      nearer edge with the higher id within `f32::EPSILON` of squared distance of a lower one
      (the band picked the farther). Products unchanged (fresh, debug, `ROAD-68-K21` before the
      legacy test's removal): the three `_probe` tests, the narrow-marks digest, the Kuopio tests
      (9 passed, 7 ignored), the save byte test, and the Kuopio reference re-recorded twice at
      `3c1dbc27…` with the fixture unchanged. Re-indexing only the changed edges (`ROAD-68-K17`'s
      first open item) no longer reaches a product through these callers; it is not built here.
      The bulldozer's building pick keeps its own `f32::EPSILON` band over site centres: its
      candidates come from the building site index, not the R-tree, outside this decision.
    - **Gate checks (`ROAD-68-K18`, fresh).** `cargo clippy --all-targets` on the final code (09:08): the
      baseline's 59 warnings, none added (the in-place collection's `filter_map` carries a
      justified `allow`, since `filter` cannot change the item's lifetime). `./run.sh --test`
      on the code before the capacity hints, with `DISPLAY` and `WAYLAND_DISPLAY` unset
      (08:30-08:49): exit 0; debug
      lib 1,944 passed, 0 failed, 172 ignored; integration 4 passed; every Godot script passes,
      no `SCRIPT ERROR`; skipped only the shader global probe and the rendered vegetation tests
      (headless). The ignored gate tests are `road`'s (`TERRAIN-07` only). `roadbed/`'s
      production lines (every line of each non-test file under `roadbed/` and `roadbed.rs`,
      without `tests/`, `tests.rs`, `fixtures.rs` and `#[cfg(test)]` items) grow from 8,026 to
      8,331 (+305): `node.rs` +132 (the plan buffers with their thread-local, `lay_out` and
      `plan` shared by `plan_node`, `check_node` and `terminal_plane`, and `ArmCentre`, the mouth
      centre that the check and the mouth sections share), `layout.rs` +100 (`LayoutScratch`,
      `rebuild` beside `build`, and the steps writing into the layout's own vectors) and
      `arms.rs` +73 (`ArmsScratch`, `collect_into` and `layout_into` beside the allocating
      `collect` and `layout`, and `recycle`). Of the 492 lines added (187 removed), 69 are
      comments and 74 the parameter lines of signatures that no longer fit on one line.
    - **Gate checks (`ROAD-68-K21`, fresh).** `cargo clippy --all-targets` on the final Rust code:
      the baseline's 59 warnings, none added. The debug lib suite before the legacy test's
      removal: 1,951 passed, only that test failed, 171 ignored. `./run.sh --test` on the final
      code with `DISPLAY` and `WAYLAND_DISPLAY` unset (ended 18:23): exit 0; debug lib 1,951
      passed, 0 failed, 171 ignored; integration 4 passed; every Godot script passes, no
      `SCRIPT ERROR`; skipped only the shader global probe and the rendered vegetation tests
      (headless). The gate-tagged ignored tests are unchanged (2 `TERRAIN-07`, 2 `TERRAIN-07-A`
      census). The load failure in Godot (headless, an isolated profile, this build's debug
      library; a one-off check, not a suite test): the Kuopio reference loads (80 nodes); a copy
      whose edge 5 has one physical point raised 10 m logs `Load failed: road surface load
      failed: edge 5 climbs at 512.1% at 48.3 m, steeper than the 33% (1:3) maximum grade
      (road_too_steep)`, `load_game` returns `false` and the live city keeps its 80 nodes. No
      timing: neither path is in a hot-path benchmark (the two pickers sort the few edges within
      their pick radius once a query; `benchmark_load_node_check` times the node check alone).
      `roadbed/` production lines: 8,815, unchanged.
    - **The exact coverage predicate (`ROAD-68-K19`, 2026-10-26).** The roadbed's
      point-in-triangle predicate is exact and closed (["Footprint and
      terrain"](#footprint-and-terrain)); the failing `ROAD-68-D` test is re-enabled. A first
      version gave each point on an edge to one side by a world direction; the coordinator's check
      of its products found junction foot-lane points on curb lines dropped 0.12 m to the
      carriageway or kept on the curb by the curb's direction, so the closed rule replaced it (no
      caller needs a point in exactly one triangle: each takes the union or the highest). Fresh
      results (debug lib test binaries of the branch): every graph, piece, layout, walkway and span
      hash of both product probes is unchanged. The attribution used two temporary builds, not
      committed: one classifies every predicate call against the old predicate (band: the old one
      held a point outside the closed triangle; thin: a triangle under 0.0005 m², which the old one
      dropped, holds one; arithmetic: both hold it with different bits), the other compares whole
      answers (lane points, probe queries, graded lattice points) under both predicates.
      - Roadbed fixture probe: `bend_10`'s queries alone change, flat and sloped. 24 calls of
        437,772 differ, all band. At one probe point per fixture the carriageway and
        carriageway-only lane queries turn `None` (2 answers each); no answer turns `Some`, and
        every answer that stays `Some` is bitwise equal.
      - Reference junction probe: the lanes of 14 fixtures (all but `four_way`, `terminal` and
        `t_mixed_4lane_through`, flat and sloped) and the queries of `t_footpath_beside_arm`
        change. Of 197,530 calls, 37,953 differ as band and 16 as thin, none by arithmetic. Over
        the 79,527 lane point evaluations of every lane sync while the 20 fixtures are built, no
        answer turns `None` or `Some`, so no lane point keeps its graph height for want of a
        surface. 968 move by more than the sync's 0.1 mm threshold, every one by 0.12 m: junction
        foot-lane points at a crossing's kerb (for example (±3.5, 8.5) on the flat T), a hair
        outside the sidewalk's closed triangles, which the band lifted to the curb's top and which
        stand on the carriageway. The others are bitwise equal or move by less than 0.1 mm, which
        the sync does not write. Queries: one answer turns `None` in each of the flat and sloped
        `t_footpath_beside_arm`, none turns `Some`, the others are bitwise equal.
      - Graded coverage (the seven graded reference fixtures, 16 tiles each at `GL0`-`GL3`,
        2,567,488 lattice points): coverage is lost at 693 lattice points outside every closed
        triangle (the defect proper: 501 on `four_way_slope`, 192 on `y_25_slope`, where lattice
        rows lie a few micrometres outside an arm's edge); the covered height changes at 915
        points whose old maximum came from a band-covered triangle; no point is gained and none
        changes on an edge or by arithmetic. On the 0.37 m off-lattice grid of `TERRAIN-07-D`'s
        mask test (3,352,048 points): 12 lost outside every triangle (one to three per fixture, its
        finding) and 21 band heights. Every change is a band point.
      - Orientation: `coverage_and_heights_are_the_same_in_every_quarter_turn_and_mirror` turns
        each fixture's compiled triangles, its lane points and its edge probes by quarter turns
        and mirrors them, exactly in `f64`, and finds the same holding triangles and highest
        height at every probe, bit for bit (the compile itself is not bitwise rotation-equivariant,
        so the junction is turned after it).
      - Zoning cells and fields do not read the predicate: their road overlap verdicts intersect
        the top triangles as polygons (`PolygonFootprint::overlaps_precise_points`), unchanged.
        The Kuopio reference save re-records twice at `3c1dbc27…` with the fixture unchanged (on
        the tie-rule build; re-run on the final code in the gate checks below), so it is not
        re-pinned.
      - One test changes with the contract: `building_site_terrain`'s frontage check sampled the
        road exactly on the frontage seam, the road's outer chain, at points interpolated in `f32`
        that lie a hair outside the exact road for about half the samples (13 tests failed there);
        it now samples the road 2.5 cm on its side of the seam, as it already sampled the yard
        2.5 cm on the other.
      - Done by `ROAD-68-K20` part 2 (["Footprint and terrain"](#footprint-and-terrain), lane ends
        on a kerb line), constructively rather than by the inset recommended here. Was open, for
        the lane builder: lane points meant to lie on a kerb line are computed in `f32`
        and land a hair on either side, so with an exact surface a foot lane's end at a crossing
        reads the curb's top or the carriageway's by rounding; the band hid it by lifting the
        carriageway side to the top. Recommendation: the lane builder places those ends a fixed
        distance inside the surface it means (the sidewalk or the carriageway), as the building
        sites' road probes already do (`BUILDING_SITE_ROAD_SAMPLE_INSET_M`); no predicate change.
    - **Gate checks (`ROAD-68-K19`).** Fresh on `5230a0bd` (debug lib
      test binary): the product tests `ROAD-68-K18` ran (save bytes, both probes with strips,
      narrow marks, Kuopio 9 passed / 7 ignored, watertightness 8, grade 55, junction blends,
      one-in-three 2, `graph::rebuild` 15, edit order 9, road families 4, save 41, `roadbed::` 65,
      `road_preview` 15, editing 28, clip tests 6) plus `terrain::graded` 20, `network::lanes` 40,
      zoning 185, agriculture 7, `building_site_terrain` 19 and vegetation 62 all pass, and the
      Kuopio reference re-records twice at `3c1dbc27…` with the fixture unchanged. `cargo clippy
      --all-targets` (12:17): the baseline's 59 warnings plus one `useless_vec` in the new
      orientation unit test, fixed after the run (re-run pending). `roadbed/`'s production lines
      (counted as for `ROAD-68-K18`) grow from 8,718 to 8,733 (+15: the closed predicate's
      comments and the shared `xz` helper in `geometry.rs`, the doc of `top_triangle_height_at`).
      Fresh on `5e9c6331`: `./run.sh --test` (13:05-13:40, `DISPLAY` and `WAYLAND_DISPLAY`
      unset) exits 0, debug lib 1,951 passed / 0 failed / 171 ignored, integration 4 passed, every
      Godot script passes with no `SCRIPT ERROR`; it skips only the shader global probe and the
      rendered vegetation tests (headless). `cargo clippy --all-targets` (`ROAD-68-K20`, 13:44):
      exactly the baseline's 59 warnings (the warning sets are equal).
    - **Timing (`ROAD-68-K19`, fresh 2026-10-26 12:43-13:05, recorded by `ROAD-68-K20`).** Release
      lib test binaries (`$TEST_BINARY`), C = `road` `446290d6` (built in its own worktree and
      target directory), F = `5230a0bd` (the tip but for a test lint and a doc comment). Both
      locks, `taskset -c 0,2,4,6,8,10,12,14`, `METRUM_DEBUG=0`, no restic and no compiler, cargo
      or Godot process at the session's start, after each breakdown run or at its end (load
      2.0-2.9); `benchmark_road_preview_stages` with `RAYON_NUM_THREADS=4`, C F F C twice, then
      `benchmark_populated_commit_breakdown` with `METRUM_COMMIT_TIERS=0,10000,30000,100000` and
      `RAYON_NUM_THREADS=8`, C F F C. Payload vertex counts are equal (9,845 and 11,706 a pose),
      and so are the generated and lot digests at every tier and sample. The graded tile digests
      are equal but at 100k samples 0 and 1 (C `78c7850b…` and `2e1b5b59…`, F `f9d27713…` and
      `7bc646e7…`, each the same in both runs of its binary). Attributed by `ROAD-68-K20` (a
      temporary release build of `5e9c6331` that classifies every predicate call against the old
      predicate; the 100k tier alone, which reproduces F's three graded digests): every call that
      differs is a band call (the old predicate held a point outside the closed triangle), none a
      thin triangle's and none by arithmetic. The graded stage made 19,860 band calls of
      3,079,714 holding calls at sample 0, 8,528 of 1,130,527 at sample 1 and 5,982 of 872,972 at
      sample 2, whose digest does not change (no lattice point's highest covering triangle there
      was a band one); the commits' own calls (lanes, queries, the exact cut) differ by band only,
      and their generated and lot digests are equal. With no thin or arithmetic difference, the
      closed predicate is the old one but at band points, so the digest changes are band points
      alone; a second run giving band points the old answer reproduces C's three graded digests.
      Each run's peak resident set is 18.0-18.2 GiB (18,899,648 and 19,055,268 kB).
      | Preview stage p50, ms | C | F |
      | --- | --- | --- |
      | `sloped_t` plan | 0.447-0.458 | 0.446-0.457 |
      | `sloped_branch` plan | 0.392-0.399 | 0.394-0.400 |
      | `sloped_t` roadbed | 0.326-0.331 | 0.308-0.322 |
      | `sloped_branch` roadbed | 0.331-0.332 | 0.321-0.325 |
      | `sloped_t` payload | 0.386-0.405 | 0.385-0.391 |
      | `sloped_branch` payload | 0.386-0.399 | 0.381-0.384 |
      | `sloped_t` terrain | 4.379-4.486 | 4.485-4.547 |
      | `sloped_branch` terrain | 4.686-4.726 | 4.737-4.820 |
      | `sloped_t` total | 5.511-5.627 | 5.616-5.688 |
      | `sloped_branch` total | 5.775-5.857 | 5.820-5.900 |
      The legacy terrain stage (the exact cut, whose ground-tile sampler reads the predicate)
      rises by 0.05-0.11 ms; the roadbed stage falls by up to 0.02 ms. Commit totals and the exact
      cut's `surface_ms` stay within the two binaries' spreads (the 30k sample 0 window is
      bimodal in both, 87-137 ms). **Open, a miss: the graded stage is slower.** Its
      `GRADED_TILES_COMMIT` `stage_ms` rises at 11 of the 12 samples, by 0.13-0.62 ms, most of
      it in the bake, which rasterizes coverage with the predicate at every lattice point of each
      triangle's box: 1.54-1.62 to 1.83-1.91 ms at tier 0 sample 0, and 2.78-2.86 to 2.95-3.48 ms
      at 100k sample 0, past the 3.0 ms terrain stage. Not profiled; the exact predicate makes
      three `orient2d` calls a point (four where it holds the point) with no early exit, against
      the old one's two determinants and its area.
    - **Rebase (`ROAD-68-K20`).** The branch's commits are squashed into one and rebased onto
      `road` `fdf6c7a1` (`TERRAIN-07-C`'s portal opening, which reads coverage through the same
      predicate: `coverage::raise_covered` and `PortalWindow::deck_at`), then onto `294fd0f1`
      (the user's decisions of 2026-10-26, documents only). Conflicts: `project.md`'s recent
      changes, and this question's `ROAD-68-K18` items beside the user's R-tree decision (both
      texts kept each time); `graded/portal.rs`'s bounds pad comment cited the old tolerance and
      now says what coverage's does. The graded attribution re-run on the rebased code (a
      temporary debug build) gives `ROAD-68-K19`'s counts exactly (693 lattice points lost
      outside every closed triangle, 915 band heights; 12 and 21 off the lattice), and extended
      to `TERRAIN-07-C`'s structure-mouth fixture (`site_tests::structure_mouth` with a tunnel,
      a bridge and a road, 36 tiles at `GL0`-`GL3`, 825,264 lattice points each) changes no
      point: grounded coverage and the portal decks are bitwise the old predicate's there.
    - **The graded stage's speed (`ROAD-68-K20`).** The miss above is fixed in the predicate,
      without changing an answer: `CoverageTriangle` (["Footprint and
      terrain"](#footprint-and-terrain)) computes the box, the winding and the area's reciprocal
      once, and leaves a point at the first edge it lies beyond (no orientation of the sign
      opposite the winding: the same set, as the orientations' exact sum is twice the area); the
      graded raster prepares each triangle once for its lattice. `roadbed/`'s production lines
      (counted as for `ROAD-68-K18`) grow from 8,800 on the rebased branch to 8,859 (+59: the type,
      its box helpers and comments). `cargo clippy --all-targets`: exactly the baseline's 59
      warnings. Products, fresh: the fixed tip's
      debug lib test binary passes both product probes as pinned, the graded and portal tests, the
      gate's other product tests and two Kuopio re-records at `3c1dbc27…`; and a release run of
      the pre-fix tip `9b696cc5` (P, 17:56-18:03, after the session) gives every generated, lot
      and graded tile digest of the commit breakdown at 0 to 100k (24 records) equal to F's.
      **Timing (`ROAD-68-K20`, fresh 2026-10-26 17:14-17:37): still a miss, open.** Release lib
      test binaries (`$TEST_BINARY`), C = `road` `294fd0f1` (built in its own worktree and target
      directory), F = `bf68b8e2` (this fix). Both locks, `taskset -c 0,2,4,6,8,10,12,14`,
      `METRUM_DEBUG=0`, no restic and no compiler, cargo or Godot process at the start, after
      each breakdown run or at the end (load 2.1-2.7 after the first run; 18.2 at the start,
      just after another agent's suite, and the last debug build's binary copy overlapped the
      first two seconds of stage run 1, a C run); the preview stages with 4 workers, C F F C
      twice, then the commit breakdown at 0, 10k, 30k and 100k with 8 workers, C F F C. Payload
      vertex counts, generated and lot digests are equal; the graded tile digests are equal but
      at 100k samples 0 and 1, where F gives `ROAD-68-K19`'s band digests (`f9d27713…`,
      `7bc646e7…`). Graded stage `stage_ms` (bake within it), C runs 1 and 4, F runs 2 and 3:
      | Tier, sample | C | F |
      | --- | --- | --- |
      | 0, 0 | 1.505, 1.464 | 1.592, 1.492 |
      | 0, 1 | 1.521, 1.614 | 1.498, 1.589 |
      | 0, 2 | 1.281, 1.469 | 1.343, 1.365 |
      | 10k, 0 | 2.522, 2.481 | 2.559, 2.650 |
      | 10k, 1 | 1.429, 1.474 | 1.718, 1.690 |
      | 10k, 2 | 1.582, 1.497 | 1.584, 1.608 |
      | 30k, 0 | 2.110, 2.264 | 2.299, 2.252 |
      | 30k, 1 | 1.340, 1.423 | 1.393, 1.378 |
      | 30k, 2 | 1.181, 1.062 | 1.188, 1.161 |
      | 100k, 0 | 2.836 (bake 2.084), 2.950 (2.302) | 3.074 (2.348), 2.754 (2.164) |
      | 100k, 1 | 1.311, 1.361 | 1.325, 1.411 |
      | 100k, 2 | 0.970, 1.105 | 0.948, 1.064 |
      Most of `ROAD-68-K19`'s rise is gone (at tier 0 sample 0 F reads within C's spread, where
      it read 0.29 ms above), but F's first 100k sample 0 run takes 3.074 ms, past the 3.0 ms
      terrain stage, and 10k sample 1 reads 0.22-0.29 ms above C in both runs. The two binaries'
      spreads at 100k sample 0 (C 0.11 ms, F 0.32 ms) do not separate the fix from the run, and
      C's own second run reaches 2.950 ms. Not accepted. Next levers, exact and bit-identical:
      a row of a closed triangle's lattice holds an interval of points (the triangle is convex
      and the predicate exact), so each row can stop at its first point past a held one; and
      profile the 10k sample 1 bake against C's. The legacy preview terrain stage, which reads
      the predicate per call through the ground-tile sampler, stays 0.03-0.08 ms above C
      (`sloped_t` 4.398-4.424 against 4.365-4.383, `sloped_branch` 4.692-4.737 against
      4.614-4.661); plan, roadbed and payload stages and the commit totals are within the
      binaries' spreads.
    - **The graded stage's speed, round 2 (`ROAD-68-K23`, fresh 2026-10-26): still a miss, open;
      stopped for the user.** Branch `road68-K19` on `road` `6c51cf00`, three commits: this
      predicate, `ROAD-68-K20` part 2 (the lane ends on their kerb's surface, ["Footprint and
      terrain"](#footprint-and-terrain)) and the arm angle fix ([`ROAD-54`](#road-geometry-validation-road-54)).
      No landing session was run: the stage-only runs below already place the fix outside
      `road`'s spread.
      - **Method.** Stage-only runs, not a session: `benchmark_populated_commit_breakdown` at
        `METRUM_COMMIT_TIERS=10000` with a temporary harness (not committed) that repeats each
        sample's graded stage 201 times (p10, p50, p90; perf counting only inside them), then
        times every selected tile's serial `GL0` bake and its coverage raster alone (median of
        51 each, summed over the tiles). Release lib test binaries (`$TEST_BINARY`), both locks,
        `taskset -c 0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`, `METRUM_DEBUG=0`, no restic.
        C = `road` `294fd0f1` with the harness, F = `ROAD-68-K20`'s tip `6f3a43d1` with it.
        Graded digests equal in every run (10k: `a7321391…`, `73ba5d43…`, `7d6cb7ff…`).
      - **What is extra at 10k sample 1: the coverage raster.** Session 19:40-19:43 (C F X):
        stage p50 C 1.285, F 1.526 ms; serial bake over the 9 tiles C 3.049, F 4.155 ms, of
        which the raster C 0.721, F 1.424 ms; samples 0 and 2 alike (raster C 1.285 and 0.734,
        F 2.399 and 1.399). The exact test at every point of a triangle's box costs about twice
        `road`'s tolerance test (three `orient2d` calls against two determinants); profiled (perf,
        LBR, inside the repeats) `raise_covered` is 22.6% of F's stage, 13.2% of C's.
      - **Exact levers tried** (each bitwise: the graded digests above, and the debug coverage,
        graded and portal tests):
        - each row's held run by binary search per edge (the run is one interval: the triangle is
          convex and the predicate exact): raster 1.77 ms at sample 1, slower than F (the
          searches' `orient2d` calls on rows of a few points);
        - the run's ends by exact steps from the edge's rounded crossing: 1.35 ms (unlocked);
        - in one binary, locked (20:20-20:23, two runs each, sample 1 raster): `road`'s test
          0.89-0.90 ms, the three orientations then one test 1.31-1.32, that with the row's
          scan ended after its run (`ROAD-68-K20`'s lever (a)) 0.98-0.99, and with each row's run
          sought from the previous row's 1.00-1.01;
        - kept: the three orientations then one test, the scan ended after its run, and no
          preparation for a triangle whose box holds no lattice point. Locked, C X X C
          (20:34-20:36): stage p50 at 10k sample 0 C 2.204 / 2.255, X 2.362 / 2.282; sample 1
          C 1.177 / 1.246, X 1.396 / 1.370; sample 2 C 1.103 / 1.095, X 1.379 / 1.350; sample 1
          raster C 0.738 / 0.736, X 0.989 / 0.971;
        - `orient2d`'s fast determinant used directly where it provably is `orient2d`'s answer
          (a per-triangle bound on its error bound over the box): no gain (raster 1.025),
          dropped.
      - **Open.** The kept form takes back part of F's rise at sample 1 (+0.12 to +0.22 ms over
        C in its session, against F's +0.24 in its own) and stays outside C's spread; 100k
        sample 0 is not re-timed. What remains is the three exact orientations at every point the scan tests,
        against `road`'s two determinants. Exact levers left: an exact row interval for wide
        boxes only, with the scan on narrow ones (rows of a few points made the searches lose);
        and, in the same stage, the sorts of the coverage inputs (`[u64; 9]` triangle bits) and
        of `[[u64; 3]; 3]` keys, about 4.9% and 4.7% of the stage in both binaries. Nothing is
        relaxed: the 3.0 ms stage and `road`'s spread stand.
      - **Checks (fresh, on the branch's final Rust).** `cargo clippy --all-targets`: the
        baseline's 59 warnings, the warning sets equal. Debug lib tests: `roadbed::geometry`,
        `roadbed::terrain` and `terrain::graded` (33 passed, 7 ignored; among them the two new
        tests, `geometry.rs` `a_lattice_row_holds_exactly_the_points_the_predicate_holds`, which
        checks the row scan and the per-point predicate against the closed rule written out from
        `orient2d` on rows and columns through every vertex and an ulp to either side, and
        `the_coverage_raster_equals_the_predicate_at_every_lattice_point` on every graded
        fixture and level); the reference and roadbed probes, the crossing orientation test over
        every fixture, `network::validate`, `road_edit_order`, `network::lanes` and the crosswalk
        tests (113 passed). `roadbed/`'s production lines (counted as for `ROAD-68-K18`): 8,815 on
        `road` `6c51cf00`, 8,909 on the branch (+94: the closed predicate and its comments,
        `CoverageTriangle` with the row scan, and the `xz` and `orientation` helpers). `./run.sh
        --test` on the rebased tip (`road` `6c51cf00`, 21:15-21:35, `DISPLAY` and
        `WAYLAND_DISPLAY` unset): exit 0; debug lib 1,958 passed, 0 failed, 173 ignored;
        integration 4 passed; every Godot script passes, no `SCRIPT ERROR`; skipped only the
        shader global probe and the rendered vegetation tests (headless).
    - **The landing session (`ROAD-68-K25`, fresh 2026-10-26 22:05-22:09): the graded stage is
      under 3.0 ms at every tier and sample; land-ready.**
      - **Decision (decided by the coordinator under the user's delegation, 2026-10-26, for
        review).** The recorded target is the 3.0 ms terrain stage per commit at every tier, 0
        to 100k. "Within `road`'s spread" was the coordinator's stricter condition in the
        `ROAD-68-K20` and `ROAD-68-K23` briefs, not a recorded target. The exact predicate is
        required by the no-tolerance rule, so the old predicate is no option. The branch is
        land-ready when every F run's graded `stage_ms` is under 3.0 ms at every tier and sample
        (the 100k first commit binding) and every product is as recorded; the cost against
        `road` is then the exact predicate's price, within budget.
      - **Method.** Release lib test binaries (`$TEST_BINARY`): C = `road` `6c51cf00` (its own
        worktree and target directory), F = this branch's tip Rust (`7c178931`). Each binary's
        saved cities were prepared before the session (`benchmark_populated_commit_prepare`, a
        heavy run, not timed: 225 and 202 s), and every populated tier of every run reads
        `loaded`. Both locks, `taskset -c 0,2,4,6,8,10,12,14`, `METRUM_DEBUG=0`, no restic and no
        compiler, cargo or Godot process at the start, after each breakdown run or at the end
        (load 10.2 at the start, just after another agent's timing run; 3.3-5.2 after the
        breakdown runs); the preview stages with 4 workers, C F F C twice, then the commit
        breakdown at 0, 10k, 30k and 100k with 8 workers, C F F C (50.1-50.6 s a run). Question
        50's whole-city checks run as before (`ROAD-68-K24`'s change).
      - **Products.** Payload vertex counts are equal (9,845 and 11,706 a pose, skirts 2,034 and
        2,448), and so are the generated and lot digests and `lanes_off_surface` at every tier and
        sample. The graded tile digests are equal (10k: `a7321391…`, `73ba5d43…`, `7d6cb7ff…`, as
        `ROAD-68-K23` recorded) but at 100k samples 0 and 1, where F gives `ROAD-68-K19`'s band
        digests (`f9d27713…`, `7bc646e7…`; C `78c7850b…`, `2e1b5b59…`). The `COMMIT_PRODUCTS`
        records differ in those two digests alone, each binary's two runs are identical, and C's
        equal `ROAD-68-K22`'s session.
      - Graded stage `stage_ms` (bake within it), C runs 1 and 4, F runs 2 and 3:
        | Tier, sample | C | F |
        | --- | --- | --- |
        | 0, 0 | 1.646, 1.465 | 1.501, 1.526 |
        | 0, 1 | 1.463, 1.497 | 1.460, 1.527 |
        | 0, 2 | 1.179, 1.263 | 1.345, 1.327 |
        | 10k, 0 | 2.479, 2.529 | 2.511, 2.569 |
        | 10k, 1 | 1.526, 1.587 | 1.591, 1.476 |
        | 10k, 2 | 1.532, 1.557 | 1.394, 1.507 |
        | 30k, 0 | 2.230, 2.337 | 2.209, 2.143 |
        | 30k, 1 | 1.348, 1.345 | 1.371, 1.109 |
        | 30k, 2 | 1.238, 1.107 | 1.191, 1.068 |
        | 100k, 0 | 3.019 (bake 2.366), 3.114 (2.526) | 2.722 (1.948), 2.628 (2.051) |
        | 100k, 1 | 1.346, 1.127 | 1.271, 1.085 |
        | 100k, 2 | 0.932, 0.925 | 1.042, 1.104 |
      - **Verdict: land-ready.** F's slowest graded stage is 2.722 ms (100k sample 0), under
        3.0 ms in all 24 F samples. The price against `road`: F's two runs overlap or undercut
        C's at 10 of 12 samples (at most 0.04 ms past C's slower run) and lie wholly above them
        at tier 0 sample 2 (+0.06 to +0.17 ms) and 100k sample 2 (+0.11 to +0.18), both under
        1.35 ms. At 100k sample 0 F reads 0.30-0.49 ms under C, whose own runs reach 3.019 and
        3.114 ms there (`road` past the 3.0 ms stage in this session; 2.836-2.950 in
        `ROAD-68-K20`'s, 2.911 in `ROAD-68-K22`'s). Not attributed; unlike `road`'s, F's raster
        prepares no triangle whose box holds no lattice point and ends each row's scan with its
        run. Preview stages: plan, roadbed and payload within the binaries' spreads; the legacy
        terrain stage (the exact cut, whose ground-tile sampler reads the predicate) `sloped_t`
        4.466-4.511 against C's 4.477-4.508, `sloped_branch` 4.765-4.783 against 4.714-4.760.
        Commit totals (`commit_ms`) are within C's spread or
        overlap it but at tier 0 sample 1 (12.106-12.361 against 11.294-11.718), tier 0 sample 2
        (7.910-7.967 against 7.135-7.239) and 10k sample 1 (20.244-20.556 against
        19.377-20.136); the rise sits in `surface_terrain_ms` (the exact cut: 9.446-9.532 against
        8.637-9.229, 5.001-5.169 against 4.750-4.865, 13.802-14.171 against 13.066-13.572), and
        at tier 0 sample 2 also in the mesh stage. The 30k sample 0 window stays bimodal (F
        106.2 and 149.0 ms, C 102.2-108.1; `ROAD-68-K19` saw 87-137 ms in both binaries).
      - **Part 2's lane sync.** The commit syncs lane heights in its mesh stage
        (`precompute_road_mesh_data`), after the lanes stage. `lanes_and_reattach_ms` overlaps or
        undercuts C's spread at tier 0, 10k sample 0 and 100k, and reads above it at 10k samples
        1 and 2 and at 30k, by at most 0.08 ms (30k sample 0: 0.824-0.884 against 0.800-0.821;
        30k sample 1: 0.474-0.478 against 0.440-0.451). `mesh_ms` overlaps or undercuts C's at 10
        of 12 samples and lies above it at tier 0 sample 2 (1.604-1.870 against 1.260-1.302) and
        30k sample 1 (1.432-1.491 against 1.287-1.350). The surface stage's roadbed sub-stage
        (`surface_roadbed_ms`) overlaps or undercuts C's but at 30k sample 1 (0.583-0.608 against
        0.546-0.553) and 100k sample 1 (0.548-0.632 against 0.479-0.501).
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
    - **Re-recorded in save format 108 (`ROAD-68-K22`, fresh).** The format keeps the pending
      arrivals ([`simulation.md`](simulation.md#save-bytes-road-68-k16)). Two re-records give the
      same bytes, `8fa19a1e…` (was `3c1dbc27…`); a dump of each differs only in
      `save_meta.version` and `household_queue_state`'s four new columns, all zero, so every saved
      control and physical point keeps its bits and storage order. The Kuopio tests (9 passed, 7
      ignored), `road_terrain_plan` (21, 6 ignored), `building_site_terrain` (19, 5 ignored) and
      the save byte test pass in debug.
39. **Junction blends beside arms cut to the source-delta limit (`ROAD-68-K14`, from question 37)
    — decided by the coordinator under the user's delegation, 2026-10-25 night, confirmed by the
    user 2026-10-26: (a); implemented in `ROAD-68-K15`.**
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
48. **A terrain edit can leave a city whose save does not load (`ROAD-68-K21`, from question 37) —
    closed by `TERRAIN-07-E` round 1 (2026-10-26): option (c) is built.**
    - **Evidence.** Since `ROAD-68-K21` a load refuses any road steeper than 1:3 (question 37,
      "Save compatibility"). The whole-graph terrain sync (`RegionGraph::sync_to_terrain`) builds a
      re-graded road at the least grade its source band allows where the ground leaves none
      within 1:3, steeper than the maximum, and refusing such a terrain edit waits for
      `TERRAIN-07-E` (question 37, decided by the user 2026-10-26). Until then a sculpt can leave
      a road over 1:3; the city still saves, and that save fails to load with `road_too_steep`. No
      fixture or recorded session does so (the re-recorded Kuopio reference has no edge over 1:3).
    - **Options.** (a) Accept the gap until `TERRAIN-07-E` refuses such terrain edits; no code.
      (b) Refuse the save too: `save_game` fails with `road_too_steep` while a road exceeds 1:3
      (the same `grade_violation` check), so no unloadable file is written, but the player cannot
      save until they rebuild a road the terrain, not they, steepened. (c) Refuse the terrain edit
      now, as `ROAD-68-K15`'s measured rollback did, which first needs terrain undo to restore
      the source terrain (`TERRAIN-07-E`'s precondition).
    - **Recommendation.** (a): the gap exists only on the `road` branch, where saves may break,
      and `TERRAIN-07-E` closes it at its cause; (b) would block saving over a road the player
      did not make that steep, and (c) is `TERRAIN-07-E` itself.
    - **Closed (`TERRAIN-07-E` round 1).** A terrain edit that would leave a road steeper than
      1:3, and steeper than before, is refused with `road_too_steep` and rolled back from its
      undo record, which now restores the source terrain (question 37, K15 (1)). Road commits, node
      edits and terrain strokes all refuse a road steeper than 1:3 now, so none of them can
      leave a city whose save fails to load.
50. **The commit breakdown runs the test build's whole-city checks (`ROAD-68-K22`) — decided by the
    user, 2026-10-26: (a), measured first.**
    - **Evidence.** The economy runners switch off the test-only checks (`contract::timing_mode`:
      the planning passes' book fingerprints and `debug::work::set_city_checks(false)`); the
      commit breakdown never did, so its city builds and every frame it steps run them. They are
      large: with them the 10k / 30k cities build in 19.9 / 57.0 s, without them the snapshot
      generation, which builds the same city and then steps it a further day twice for the
      continuation check, takes 12.8 / 33.9 s (fresh, `ROAD-68-K22`, not a timing session). Some
      run inside the frames the breakdown reports: after a commit's lane rebuild the next agent
      tick checks the lane buckets against a rebuild from every agent
      (`check_lane_buckets_match_rebuild`), O(agents), in the adoption frame whose agent time is
      reported. The checks are verification and change no product (the snapshot cities, made
      without them, give the built cities' products bitwise).
    - **Options.** (a) Switch them off in the breakdown as the economy runners do; the reported
      frame and stage times then hold no test-only work, and a session needs a control built with
      the same switch. (b) Keep them; the reported frame times stay comparable with earlier
      records and keep test-only work.
    - **Recommendation.** (a), measured first: which reported stages move, and by how much, in
      one session against a control without it. Not taken in `ROAD-68-K22`, which changes no
      timed path.
    - **Decision (the user, 2026-10-26): (a).** The breakdown switches the test-only checks off as
      the economy runners do. One session against a control without the switch records which
      reported stages move and by how much; later sessions build every binary with the switch.
    - **Implemented (`ROAD-68-K24`).** `benchmark_populated_commit_breakdown` calls
      `contract::timing_mode`, as its prepare test already did: no book fingerprints after the
      planning passes, and `debug::work::set_city_checks(false)`, which also stops the agent
      tick's lane bucket check. No second switch; every other test keeps the checks. The
      products are bitwise unchanged; the frames' agent time falls by the lane check (up to
      3.0 ms at 100k), in the frame after the adoption frame rather than in it, and no commit
      stage moves consistently. The landing session and the products are under
      [Commit breakdown without test-only checks](#commit-breakdown-without-test-only-checks-road-68-k24-2026-10-26).
55. **The populated city could not house the breakdown's default tiers (`ROAD-68-K24`) — decided
    by the coordinator under the user's delegation, 2026-10-26: (a), for review.**
    - **Evidence** (fresh, release, `ROAD-68-K24`). `PopulatedCity::build` sizes a square side
      for its residents with 8% headroom and floors the grid's width to whole 400 m blocks
      between cross streets. Where the side falls just short of a block, the floor loses more
      area than the headroom keeps: 1,300 residents get a 400 m by 700 m grid, 54% of the side's
      square, which houses 819; 1,536 house 898 and 13,000 house 12,723, while 1,537 (side
      800.2 m, a third cross street) builds. The 1,300 tier fails at the fixture's first commit
      (`3cbdc4af`, 826 housed), at `b75e03c9`, at `7425f1a9`, which set the breakdown's default
      tiers (0, 1,300, 13,000), and on `road` `bcf865a2` (819 each), so no commit was good and
      there was nothing to bisect. No rule changed: the flat fixture meets every grade rule.
      Every recorded session names its tiers (0, 10k, 30k, 100k), all of which the floored grid
      houses.
    - **Options.** (a) A tier the floored grid cannot house is built once more one block wider,
      covering its side, and fails only if that grid cannot house it either; a tier the floored
      grid houses is built exactly as before. (b) Always round the width up: every populated
      city changes, so the economy contract's fixtures and every recorded breakdown and economy
      number re-baseline. (c) Make the breakdown's defaults tiers the fixture houses (for example
      2,000 and 10,000) and leave the fixture unable to build some tiers.
    - **Recommendation.** (a): it removes the lost area at its cause without moving a city
      anyone has measured; (b) is a re-baseline for the user to choose, and (c) leaves the
      defect for the next tier anyone picks.
    - **Implemented (`ROAD-68-K24`).** `PopulatedCity::build_timed` tries the floored grid,
      then one more cross street; the breakdown's tier 0 keeps the floored 1,300 grid. The
      1,300 and 13,000 cities build on three and seven cross streets. The wider 13,000 city
      then failed the test build's lane bucket check (2,404 live lane agents patched against
      2,403 rebuilt): the fixture moved its arrival carriers in by writing their transit
      before they left the lane buckets, so a carrier a swap-remove had moved into the lane
      snapshot kept its entry. The fixture now moves them in through
      `AgentSystem::place_arrival_carrier_at_home`, which leaves the buckets first (test
      `arrival_carrier_placed_at_home_leaves_the_lane_snapshot`). Carriers in the game drive in
      and enter their homes through the tick, which keeps the buckets.
58. **The tick after a commit that deferred its lots (`ROAD-68-G` part 8) — (b), agreed by the
    user, 2026-10-28; validated at `ROAD-68-H`.**
    - **Evidence.** The sim thread runs a tick at once after its commands whenever the tick's
      deadline passed during them (`begin_due_sim_tick`). In a populated city the previous tick
      itself takes a few milliseconds, so a 15-17 ms commit always overruns the next deadline,
      and the frame that adopts the commit's lots (question 33 (c)) ran in the same loop
      iteration right behind the commit and waited for the whole plan: the breakdown's frame,
      which starts at once, is what the game ran. Part 7's record assumed the game's next tick
      comes up to a frame later; the code did not do that.
    - **Options.** (a) Keep it: the adoption frame waits for the plan, 7.5-13 ms at the first
      populated samples in part 7. (b) After a commit that deferred a plan, the sim thread
      publishes the commit's edit snapshot without a tick, as an edit between deadlines does,
      and moves the next tick to one interval after the commit, so the plan has that frame to
      finish. The tick is the routing hold's, which advances nothing: no simulated outcome
      changes, only wall-clock time (the hold, and the routing adoption after it, start up to a
      frame later). (c) Adopt at the first held frame that finds the plan done: the adoption
      frame would follow machine speed, which D7 rules out for the routing hold too.
    - **Recommendation.** (b).
    - **(b) taken pending the user's review, 2026-10-27** (reversible inside the branch: one
      call in the sim thread). Built in part 8 (`postpone_sim_tick`, test
      `a_deferred_lot_plan_moves_the_next_tick_a_frame_past_the_commit`); the breakdown measures
      the game's frame with `METRUM_COMMIT_NEXT_TICK=frame`
      ([commit stages, part 8](#commit-stages-part-8-road-68-g-2026-10-27)).
    - **Agreed by the user, 2026-10-28; validated at `ROAD-68-H`:** (b), the next tick moves one
      frame past a commit that deferred its lots. H checks nothing beyond this record
      ([carried to H](#road-68-h--acceptance-and-cleanup)).
59. **The commit's agent passes visit every agent (`ROAD-68-G` part 9, question 21 (d)) — (b),
    agreed by the user, 2026-10-28, in its own round after `ROAD-68-G` closes; validated at
    `ROAD-68-H`.**
    - **Evidence.** A commit runs three parallel scans over the whole agent store
      (`AgentSystem::invalidate_lane_ids_for_edges`, then the detached-agent search and the
      route-state test of `reattach_invalidated_lanes_for_edges`), `O(A / P)` each. In part 8's
      locality fixture (historical, 2026-10-27) the invalidation took 0.003-0.007 ms per sample
      without the background and 0.14-1.06 ms at 100,000 remote housed agents; the lanes and
      reattachment stage 0.27-0.58 against 0.49-0.95 ms.
    - **What exists.** The lane buckets (`AgentSystem::lane_buckets`) hold each live on-lane
      agent under its current lane, valid between ticks (filled after movement, patched by
      swap-removes, invalidated by edits), so they could serve the first two scans: the agents
      on the closure's lanes or on a connector into it, and the detached agents on closure
      edges. Nothing indexes the third: an agent's planned path (`current_path`) and its planned
      attach and detach lanes and nodes can cross the edit from anywhere in the city, and the
      in-building test that skips housed agents is the only filter. No list of agents outside
      buildings exists either.
    - **Options.** (a) Keep the scans: about 1 ms at 100,000 agents, growing with the city. (b)
      The lane buckets for the first two scans and an index of planned routes by node,
      maintained wherever a path is set, advanced or cleared (trip starts, replans, arrivals,
      swap-removes: the tick's own hot path, inside its parallel movement), checked in test
      builds against the full scans; the passes then visit only the edit's agents. (c) A list of
      the agents outside buildings, kept at each building exit and entry, so the scans cost
      `O(agents in transit / P)`: not local, but housed agents cost nothing.
    - **Recommendation.** (b), in its own round with the tick's frame contract (`PERF-07`)
      measured against a same-session control, since its maintenance runs in every tick. Not
      started at night (2026-10-28): it adds per-tick work to the agent tick, which the commit's
      0.1-1 ms does not justify without that measurement.
    - **Agreed by the user, 2026-10-28; validated at `ROAD-68-H`:** (b), the route index by node,
      so the commit's agent passes visit only the edit's agents, in its own round with `PERF-07`
      against a same-session control, not before `ROAD-68-G` closes; `ROAD-68-G` builds nothing
      for it. H checks the passes' cost against click → committed at 100k, then that round runs
      ([carried to H](#road-68-h--acceptance-and-cleanup)).
60. **The commit's graph copy (`ROAD-68-G` part 9) — (a), agreed by the user, 2026-10-28;
    validated at `ROAD-68-H`.**
    - **Evidence.** A commit copies the live graph once (`publish_committed_road_tool_view`,
      `RegionGraph::clone_on_pool`); the road tool's view and the routing rebuild share that
      copy, an offered copy serves undo and bulldoze, and the preview's replica follows by
      records (`ROAD-68-E`, `ROAD-68-G` part 7). The copy is `O(nodes + edges + geometry)`: the
      tool snapshot took 0.05-0.08 ms per sample without background and 0.57-0.63 ms at
      100,000 remote sites (2,583 edges, part 8's fixture, historical); the serial clone of that
      graph takes 2.3 ms.
    - **Sharing the replica is not exact.** The replica is the preview's mutable graph: each
      pose applies its plan to it, compiles and restores it, under the road tool context's lock
      on the Godot side, while the routing rebuild reads its copy for up to 0.3 s at 100k. A
      shared replica would need the rebuild to hold the lock (stalling previews) or the
      previews to copy (the same cost, moved).
    - **Options.** (a) Keep the copy (0.6 ms at 100k on the pool beside the cell follow). (b) A
      persistent graph store (nodes, edges and adjacency in structurally shared vectors, as
      the cell store's `imbl` maps since `ROAD-68-E` round 4), so the copy is `O(changed
      records)`; its readers (pathing, lanes, the agent tick, the roadbed) then pay persistent
      lookups, which cost the cell store 0.3-2.0 ms on its own hot path, so it needs its own
      same-session measurement. (c) Give the routing rebuild only the car topology and costs it
      reads instead of the whole graph: smaller, still `O(edges)`.
    - **Recommendation.** (a) until `ROAD-68-G` closes; then (b) as its own measured track if the
      copy shows in a frame. Not decided at night.
    - **Agreed by the user, 2026-10-28; validated at `ROAD-68-H`:** (a), keep the graph copy until
      `ROAD-68-G` closes; (b), a persistent graph store, becomes its own measured track if the
      copy shows in a frame. `ROAD-68-G` builds nothing for it. H checks whether the copy shows
      in a frame ([carried to H](#road-68-h--acceptance-and-cleanup)).
61. **The rendered Moving row with the simulation active (`ROAD-68-H` round 1) — (a), decided by
    the user, 2026-10-29; the row misses (question 63).**
    - **Evidence.** The [Moving target](#targets) asks for the simulation active on the populated
      fixture (at least 99% of sampled poses exact within 16.7 ms, every miss attributed). No
      rendered harness runs that case: the gameplay harness pauses the simulation for every
      matrix (`gameplay_road_benchmark.gd` sets the speed to 0 and records it in each run's
      metadata; the preview, graded drag and idle matrices set it again), and the preview matrix
      (`road_preview_replay.gd`) builds a blank 512 m world without agents. The headless
      populated drags (`benchmark_populated_graded_drags`) step the simulation between poses but
      render nothing, so by the [Method](#method) they cannot establish the row.
    - **Options.** (a) Extend the preview matrix with a populated case: load a saved populated city
      (the commit breakdown's 10k or 30k, saved for Godot), run the simulation at 1x, and drag
      `sloped_t`'s and `sloped_branch`'s pointer paths through the production road tool there,
      with the same schema-2 collector, so each miss carries the frame's simulation and render
      shares. (b) The same in the graded drag probe on Kuopio with agents spawned (no populated
      economy, so it misses the agent pass's cost). (c) Judge the row from the paused rendered
      runs plus the headless populated drags (does not meet the Method).
    - **Recommendation.** (a): the target's own fixture and collector, and the populated city the
      commit row already uses. Not built in round 1 (it is new harness work and the rendered
      session is the user's display); `ROAD-68-H` stays open on this row until it runs.
    - **(a), decided by the user, 2026-10-29; run in `ROAD-68-H` round 2:** the populated cases
      `populated_t` and `populated_branch` on the commit breakdown's 100k city at 1x; 0 of 590
      sampled poses exact within 16.7 ms (0 of 493 paused), attributed in part
      ([round 2](#round-2-the-pacing-check-the-active-rendered-row-the-carried-items-road-68-h-2026-10-29));
      [question 63](#road-68-open-questions).
62. **The rendered preview rows at the CPU's idle clock (`ROAD-68-H` round 2) — (d) and (b),
    decided by the user, 2026-10-29; built in the next round.**
    - **Evidence.** On the user's display the five preview rows put 70.4 % of sampled poses on time
      with the Method's settings, 85.2 % without the FPS cap and 90.4 % without it on Godot's
      Wayland driver, in one session; every miss is one frame late. Profiled, a pose executes the
      same 14 M main-thread and 16-17 M pool cycles between its sample and its exact frame whether
      on time or late; every late pose ran at 1.18-1.26 GHz, the on-time ones at 1.7-2.4 GHz p50.
      The P-cores' power policy (`powersave`, `balance_performance`) sets that clock from their
      recent load, and a paused preview with a frame cap is a light load. The headless stage
      runs, back to back, run at operating clock (the preview's stages 2.4-2.8 ms there against
      4-6 ms on time and 8-11 ms late on the display). This is [question 27](#road-68-open-questions)'s
      open half (the 16.7 ms frame at the idle clock), now measured on the display.
    - **Options.** (a) The [Method](#method) fixes the CPU power policy for the rendered runs (the
      `performance` governor or energy preference for the run, a machine setting that needs root),
      as question 27 (a) states the stage budgets at operating clock; the default-policy numbers
      stay recorded beside each run. (b) Keep the default policy and make a pose fit at the idle
      clock: at 1.2 GHz the main thread's 14 M cycles alone take 11.7 ms, so about a third of the
      main thread's work between sample and frame must go (question 27 (b)). (c) Keep the clock
      up while the road tool previews (question 27 (d)): power spent to hide the cost. (d) Measure
      without the FPS cap: 85-90 %, still a miss.
    - **Recommendation.** (a), with one rendered run under the `performance` policy to confirm the
      rows at operating clock (the agent cannot change the policy); not decided by the agent.
    - **(d) and (b), decided by the user, 2026-10-29 (about 14:55):** the [Method](#method) drops
      the 60 FPS cap (vsync on, no cap) and reports Godot's native Wayland driver beside
      Xwayland; the pose's main-thread work is cut until it fits at the idle clock. The targets are
      unchanged: paused, every sampled pose exact within 16.7 ms; active, at least 99 %.
      [Question 63](#road-68-open-questions)'s sub-chunk payloads are the main lever for (b).
      Nothing is built for it in round 2; the next round builds it. (On this machine the window's
      display, DP-2, refreshes at 120 Hz, so without the cap the frames pace at 8.3 ms.)
63. **The populated rendered row's misses (`ROAD-68-H` round 2) — (b), sub-chunk payloads,
    decided by the user, 2026-10-29; built in the next round.**
    - **Evidence.** 0 of 590 sampled poses within 16.7 ms with the simulation at 1x on the 100k
      city (0 of 493 paused), at full clock. (1) The preview re-emits every render chunk the
      stroke changes, whole (the [preview contract](#constructive-roadbed-road-68): the commit
      publishes exactly the previewed chunks); the game's worlds use 512 m chunks (the synthetic
      fixtures 128 m), so a pose carries 11.5 MB of road chunks (0.53-0.63 MB on the fixtures):
      the call 8.1-9.9 ms and the install 6.3-6.6 ms p50, about 15 ms before the rest of the frame.
      (2) The rest of the paused frame, 4.5 ms on the branch and 26.6 ms on the T, is the engine's
      main-thread time, not named (Arch's Godot is stripped). (3) The simulation adds 6-22 ms of it
      and 0.6-0.9 ms to the call; consistent with `RENDER-28` and `RENDER-27`, not shown.
    - **Options.** (a) Smaller road render chunks in the game's worlds (128 m, as the fixtures):
      the payload falls with the chunk's content, the city draws more chunks (the GPU matrix
      measures it). (b) Chunk payloads below the chunk: the preview sends the changed pieces'
      meshes and the display keeps the rest of each chunk, so the payload follows the edit; the
      commit's "exactly the previewed chunks" becomes exactly the previewed pieces. (c) Keep whole
      512 m chunks and accept the miss. For (2) and (3), any option: name the engine's time with
      an unmodified upstream Godot built with symbols, then judge it against the separate tracks.
    - **Recommendation.** (a) measured first (the smallest change), (b) if (a) costs the render
      frame; then rerun the row and attribute the engine's time. Not decided by the agent.
    - **(b), decided by the user, 2026-10-29 (about 14:55):** a preview pose sends only the changed
      part of a road chunk, not the whole 512 m chunk (11.5 MB a pose at 100k); it is also the main
      lever for [question 62](#road-68-open-questions) (b). Nothing is built for it in round 2; the
      next round builds it, reruns the row and attributes the engine's rest-of-frame time.
    - **Built in round 3 (2026-10-29):** the payload falls from 11.5 to 2.8 MB a pose at 100k and
      the call and install from 14-15 to 7-10 ms (gamescope, full clock); the row still misses,
      its rest of frame the graded renderer's streaming after the load (question 64).
64. **The populated row is sampled inside the graded renderer's streaming after the load
    (`ROAD-68-H` round 3) — (c), decided by the user, 2026-10-29 (about 17:35); (a) built in
    round 4, (b) is `TERRAIN-13`'s.**
    - **Evidence.** With the sub-chunk payloads the populated pose's call and install take 7-10 ms
      (from 14-15), but `populated_t`'s rest of frame stays 24-44 ms. Symbolized (Godot's debug
      file by build ID), that rest is GDScript walking dictionaries and arrays; PerfDebug names
      it: `terrain.gd`'s frame rises to 47 ms while each case runs after its load and falls under
      2 ms once the load settles, nearly all the graded tile renderer's residency pass (13-33 ms a
      frame, growing with the wanted tiles, 2,145 to 6,923) and the patch residency (6-7 ms
      against its 4 ms budget). The idle-frame matrix met the same window and, by the user's
      decision (`TERRAIN-07-E` round 7, `5972bef8`), waits for the cell set builds after a load;
      the preview harness does not wait.
    - **Options.** (a) The populated row waits, as the idle matrix does, until the graded renderer
      has settled after the load (its wanted tiles resident and no cell set build pending), so it
      measures the preview in the steady state; the streaming's own cost is reported apart. (b)
      Make the graded renderer's residency pass follow the changed tiles instead of every wanted
      tile each frame (a `TERRAIN-07` track: the same cost shows after every load and camera jump
      at 100k). (c) Both. (d) Count the window in the row as it is (a miss).
    - **Recommendation.** (c): (a) for the row, so it measures what the road tool costs, and (b) as
      its own measured track, since a 13-33 ms frame while tiles stream is a real cost. Not
      decided by the agent; the round measures the row as before (d) meanwhile.
    - **(c), decided by the user, 2026-10-29 (about 17:35).** The populated row waits until the
      graded renderer has settled after the load (its wanted tiles resident, no cell set build
      pending) and measures the preview in the steady state; the streaming window's cost is
      reported apart. Built in round 4 (`road_preview_replay.gd` `_wait_for_steady_terrain`:
      `terrain.gd` `is_render_settled()` and no cell set build pending, three frames in a row, the
      simulation paused; its frames, time and frames over 16.7 ms in each case's `steady_wait`).
      (b) is `TERRAIN-13` (the user widened it at about 17:30: the graded renderer's GDScript
      bookkeeping moves to Rust, its residency pass following the changed tiles); its own track,
      not blocking `ROAD-68`.
65. **The rendered rows' misses beyond the GDScript share (`ROAD-68-H` round 4) — (a) with (d),
    decided by the user, 2026-10-29 (about 19:15).**
    - **Evidence** (round 4, fresh, [record](#round-4-terrain-12-fixed-in-the-rust-staging-the-poses-main-thread-work-attributed-the-runs-road-68-h-2026-10-29)).
      Paused rows (before the fixes) N 84.3 %, W 89.5 % (on T61 (1) (a)'s tip N 87.6 % (3,440 of 3,926), W 88.9 % (3,519 of 3,959));
      stationary p95 up to 24 ms and stop → exact up to 17.5 ms; populated steady state (before
      the fixes) x11 A 43.3 %, P 84.3 %, Wayland A 44.6 %, P 77.8 %. The paused pose's 14.5-14.9 M main-thread cycles are engine 43-45 %,
      GDScript 26-27 %, the Rust call 21-23 %, libc 6-7 %, at 1.2 GHz when late: removing the
      GDScript time would leave an estimated 2.5-3.3 % of poses late, and with the engine's
      conversion of the GDScript-built preview meshes 1.8-2.6 %; the Rust call has no cut that
      reaches the 2.5-3.5 M cycles a late pose needs (its largest function 0.35 M). The active
      populated pose runs at 4.9 GHz; its late poses carry 12 M more cycles than its on-time ones:
      the frame's render work while the city grows (`_surface_set_data` and
      `octahedron_encode` 5 M, MultiMesh buffer sets, the building plot and construction site
      transforms 1.8 M) and GDScript; without the GDScript time an estimated 55-58 % fit.
    - **Options.** (a) Carry the moving and stationary rows whole to `TERRAIN-13`, which removes
      the GDScript share and, with Rust calling `RenderingServer` directly, the engine's
      conversion of the meshes GDScript builds, and re-measures them against the unchanged
      targets; `ROAD-68` lands with the rows open there. (b) Keep them blocking and cut more in
      H: the Rust call's serial main-thread work onto the pool or a frame earlier (a pipeline
      change to "exact in the frame that samples it"); the engine's submission (driver, render
      lists) is upstream Godot's. (c) [Question 62](#road-68-open-questions) (a): the rows at the
      operating clock (the CPU power policy). For the active row: (d) a separate track for the
      growing city's per-frame visual uploads and transforms on the main thread, as `RENDER-27`
      and `RENDER-28` are (neither covers it: camera turns and agent draw distance), or (e) count
      them in `RENDER-27`'s CPU-bound render frame.
    - **Recommendation.** (a) for the paused and stationary rows; for the active row (a) with (d).
      Not decided by the agent; no target relaxed.
    - **(a) with (d), decided by the user, 2026-10-29 (about 19:15).** The paused and stationary
      rows are carried whole to `TERRAIN-13` and re-measured there against the unchanged targets;
      the active row is carried to `TERRAIN-13` and to `RENDER-30` (new: the growing city's
      per-frame visual uploads and transforms on the main thread). `ROAD-68` lands with these
      rows open there; H still runs them on its tip and records them with their attribution,
      carried, not passed.
66. **The terrain stage's frames over 3.0 ms on H's final tip (`ROAD-68-H` round 6) — (a),
    decided by the user, 2026-10-30 (about 07:18; recorded by the coordinator 2026-10-30, the user
    asleep).**
    - **Evidence** (round 6, fresh, [record](#round-6-the-users-t64-change-t65-the-runs-on-the-final-tip-road-68-h-2026-10-29)).
      Populated graded drags, C F F C: F 2 of 161 pose frames over 3.0 ms, 10.23 ms (30k:
      selection 3.94, bake 6.29) and 4.30 ms (100k: bake 3.70), both while the previous commit's
      lot and routing adoption jobs ran on the pool; and one catch-up frame at 3.01 ms (10k, one
      3.01 ms bake, no background job). The control: 0 of 168 pose frames over, one catch-up frame
      at 3.11 ms under the same jobs. The pose's selection p50 is C 0.52-0.54, F 0.52-0.56 ms.
    - **What the user's decisions cover.** The two pose frames are the class carried to `PERF-12`
      at 21:20 (frame stages sharing Rayon workers with background commit jobs; "H lands with it
      open there"). No decision covers the 3.01 ms catch-up frame: T63 (19:55) accepted round 4's
      3.84 ms frame, one heavy one-tile bake, as measured, not the class.
    - **Options.** (a) Accept the catch-up frame as measured, as T63 did round 4's frame of the
      same class (a single tile's bake at the budget; the control shows a 3.11 ms frame in the
      same session). (b) Carry it to `TERRAIN-14` (the cheaper bake per tile, the user's order of
      2026-10-29), re-measured there against the unchanged 3.0 ms; `ROAD-68` lands with it open
      there. (c) Keep `ROAD-68-H` open until `TERRAIN-14` cuts the bake.
    - **Recommendation.** (a), consistent with T63; (b) if the user prefers not to accept
      single-frame misses. The coordinator confirms the two pose frames as `PERF-12`'s class.
      Not decided by the agent; no target relaxed; `road` is not fast-forwarded before the user
      answers.
    - **Decision (the user, 2026-10-30): (a).** The catch-up frame is accepted as measured, as T63
      accepted round 4's; the two pose frames stay `PERF-12`'s class, the 3.0 ms target unchanged.
      With the comparison clips reviewed (the user took the final tip, the undo hold kept),
      `road` takes `ROAD-68-H`.

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

Landed (see [footprint and terrain](archive/terrain_legacy_surface_2026-10-25.md#from-roadsmd-footprint-and-terrain-the-road-side-of-the-exact-cut) for the contract):

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
  ([footprint and terrain](archive/terrain_legacy_surface_2026-10-25.md#from-roadsmd-footprint-and-terrain-the-road-side-of-the-exact-cut)), with the test
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
  refresh](archive/terrain_legacy_surface_2026-10-25.md#from-roadsmd-footprint-and-terrain-the-road-side-of-the-exact-cut)): a commit's validation grades the edit's patches once, and the
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
  patches](archive/terrain_legacy_surface_2026-10-25.md#from-roadsmd-footprint-and-terrain-the-road-side-of-the-exact-cut)): window side vertices are read from the window's exported
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
(round 4, [patch selection](archive/terrain_legacy_surface_2026-10-25.md#from-roadsmd-footprint-and-terrain-the-road-side-of-the-exact-cut)).

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
  clip](archive/terrain_legacy_surface_2026-10-25.md#from-roadsmd-footprint-and-terrain-the-road-side-of-the-exact-cut)); loops without rails are unchanged, bit for bit. Test first:
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
  footprint inside its rectangle ([source tags](archive/terrain_legacy_surface_2026-10-25.md#from-roadsmd-footprint-and-terrain-the-road-side-of-the-exact-cut), open question 28): the
  far windows then keep their fingerprints and are reused (11 of 15 change on the same T, all
  within the junction's halo). (2) The roadbed keeps the net multiset of changed grounded chain
  segments (`TerrainChanges`, `take_terrain_changes`), and `road_terrain_tiles` stamps only tiles
  within the cap of a changed segment, or of a changed piece when a building site lies within
  the local sample margin of the tile's window ([patch selection](archive/terrain_legacy_surface_2026-10-25.md#from-roadsmd-footprint-and-terrain-the-road-side-of-the-exact-cut)); the
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

#### Saved cities for the commit breakdown (`ROAD-68-K22`, 2026-10-26)

A timing session stops every build and test run on the machine, and a breakdown run took about
5 minutes, nearly all of it building the cities it then times commits on for milliseconds. The
breakdown now loads each populated tier from a saved snapshot ([method](#method)).

- **Where a run's time went** (fresh, `road68-K22` before the snapshots were used, cities built
  in the run; a heavy run, not a timing session: `.run.lock`, the shared timing lock, P-cores,
  eight workers, other agents' builds running). 355 s in all, peak resident set 19.3 GB. Building
  the cities took 0.12 / 19.9 / 57.0 / 252.2 s at 0 / 10k / 30k / 100k, 93% of the run. The
  rest at 100k: two reference frames 0.42 s, the road mesh and the tool view 0.79 s, the refined
  terrain of every patch 6.22 s, the graded census 1.42 s, the wall census 0.85 s, the three
  samples with their untimed work (the frames to adoption, the clones, a CCH build, the graded
  stage) 7.28 s, the cell copy 0.05 s and the site removal 0.03 s; 24 s over all tiers.
- **The snapshots.** They reuse the economy contract's snapshot code
  ([`economy.md`](economy.md#performance-contract-perf-01perf-05), "Commit breakdown cities"): a
  third fixture, the city as `PopulatedCity::build` leaves it at the build's own clock, saved in
  the ordinary format with its revision, residents and clock, published only after the
  exact-continuation check (the next day, hour by hour: records, building churn, the agents'
  movement and schedule state). Its key adds the test binary's SHA-256 to the fixture revision,
  so two binaries share a city only when they are the same build, and the file name carries the
  key, so a control's and a candidate's cities coexist. A run that finds no matching snapshot
  generates one and says so (`generated`). Nothing in the game or in a timed path reads them.
  Tier 0, the street grid alone, builds in 0.12 s and is still built in the run.
- **What the check found.** At 100k the first snapshot failed its continuation in the first
  hour (vacant homes 3,732 against 3,641, open jobs 477 against 340): a save dropped the pending
  arrivals the operational hour had counted (1,591 households in flight at 08:00; the loaded
  game counted none until the next hour), which demand planning reads. Save format 108 keeps them
  ([`simulation.md`](simulation.md#save-bytes-road-68-k16)); the 10k and 30k cities had none in
  flight. With it the 100k city passes. The Kuopio reference is re-recorded in format 108
  (question 38).
- **Equivalence** (fresh). On `road68-K22` before the format fix (built against loaded, 0 / 10k
  / 30k; 100k could not be saved yet): every product is bitwise equal. The cell store's entry
  count is not (17,720 / 17,696 at 10k, 48,333 / 48,304 at 30k): it follows the store's edit
  history, and the store's digests are equal, so it is left out of the products (explained in
  `ROAD-68-K24`, which adds the count without empty chunk entries back). On the final
  production code (release binary F2; the final commit differs only in an ignored test's tier and
  the re-pinned fixture hash that `placements.json` carries): the session's two runs below
  print identical `COMMIT_PRODUCTS` lines at all four tiers, 100k included, and their digests
  equal `ROAD-68-K18`'s (100k sample 2: lots `4c43d000…`, generated `a7bb4ce6…`). The ignored
  release test `commit_breakdown_products_survive_the_snapshot` (2,000 residents; the default
  1,300 tier no longer houses its residents, before this branch too) passes in 34 s.
- **The gain** (fresh, F2, one session 20:36-20:42: both locks, P-cores, eight workers,
  `METRUM_DEBUG=0`, no restic, compiler, cargo or Godot process at its start). The breakdown
  with the cities built in the run (`METRUM_COMMIT_CITIES=built`) took **313.2 s**; with the
  saved cities, prepared before the session, **50.7 s** (6.2 times shorter). The cities took
  18.3 / 56.8 / 212.6 s to build and 2.3 / 4.9 / 18.8 s to load at 10k / 30k / 100k (the 10k
  figure includes hashing the binary, about 0.4 s, once a run); the untimed setup after the
  city stays about 17 s at 100k (the refined terrain of every patch 6.4 s of it). A session
  of eight stage runs and C F F C breakdowns falls from about 22 to about 4-5 minutes.
  Preparing a binary's three cities (`benchmark_populated_commit_prepare`, the exact-continuation
  check included) took 256 s outside the session (generation 12.8 / 33.9 / 208.0 s), peak
  resident set 6.0 GB; the built run's peak was 19.3 GB. The commits' stage times on a loaded
  city lie within the built city's spread (for example `surface` at 100k sample 0: built
  83.78, loaded 88.25 ms; at 30k sample 0, the legacy terrain stage's known bimodal sample,
  137.72 and 93.94 ms), but a loaded city's heap differs from a built one's, so a session
  compares binaries on cities from one source.
- **Gate checks** (fresh, rebased on `road` `602f58ee`). `cargo clippy --all-targets`: the
  baseline's 59 warnings, none added. `./run.sh --test` with `DISPLAY` and `WAYLAND_DISPLAY`
  unset (ended 21:11): exit 0; debug lib 1,951 passed, 0 failed, 173 ignored (the two new
  ignored tests, the prepare and the equivalence check, over `ROAD-68-K21`'s 171); integration
  4 passed; every Godot script passes, no `SCRIPT ERROR`; skipped only the shader global probe
  and the rendered vegetation tests (headless). The gate-tagged ignored tests are unchanged.
  `roadbed/` is untouched: 8,815 production lines. No timed path changed: the commit window,
  the stages and the frames run the same code; the breakdown reads the payload's vertex counts
  after a sample's timings.

#### Commit breakdown without test-only checks (`ROAD-68-K24`, 2026-10-26)

Question 50's decision (a), with two fixture defects the work found. The four commits change
no game code: the breakdown's harness, its fixture city and a test-only count.

- **The switch.** `benchmark_populated_commit_breakdown` calls `contract::timing_mode`, as its
  prepare test and the economy runners do: the planning passes skip their book fingerprints and
  `debug::work::set_city_checks(false)` stops the operational hour's whole-city checks and the
  agent tick's lane bucket check against a rebuild from every agent. Every other test keeps
  them.
- **The landing session** (fresh, 2026-10-27 00:42-00:46: both locks, P-cores, `METRUM_DEBUG=0`,
  no restic, compiler, cargo or Godot process at its start or after any run). Control C =
  `road68-K24~1` (every fix below, no switch; binary `d6cd84ee…`), change F = `road68-K24` (binary
  `d37954d0…`), release library tests built in one worktree, each binary's cities prepared before
  the session (generation 10.8 / 32.4 / 155.8 s for C, 10.9 / 32.2 / 159.3 s for F; peak
  resident set 5.9 / 6.0 GB). Preview stages (four workers) C F F C twice; breakdown (eight
  workers, 0 / 10k / 30k / 100k, saved cities: every populated tier `loaded`) C F F C, 49.8 /
  47.7 / 47.4 / 49.7 s a run. Medians over the three samples of each sample's two runs, ms,
  C / F:

  | Reported number | 0 | 10k | 30k | 100k |
  | --- | --- | --- | --- | --- |
  | click → ack | 11.53 / 12.07 | 19.91 / 19.70 | 58.01 / 58.04 | 20.63 / 20.45 |
  | commit | 11.49 / 12.02 | 19.88 / 19.66 | 57.96 / 58.00 | 20.58 / 20.40 |
  | roadbed and terrain (`surface`) | 9.34 / 9.89 | 14.17 / 13.92 | 51.98 / 52.41 | 10.68 / 10.46 |
  | entrances (`buildings`) | 0.00 | 3.67 / 3.64 | 2.29 / 2.31 | 3.06 / 3.07 |
  | `finalize` | 0.21 | 4.07 / 4.03 | 3.11 / 2.85 | 4.30 / 4.28 |
  | render publication (`mesh`) | 1.40 / 1.56 | 0.85 | 1.41 / 1.36 | 2.25 / 2.26 |
  | tool snapshot | 0.04 | 0.24 / 0.23 | 0.62 | 2.37 / 2.41 |
  | reference frame (agents) | 0.00 | 1.00 / 0.89 (0.52 / 0.46) | 3.22 / 2.27 (2.13 / 1.22) | 14.69 / 12.63 (8.24 / 5.41) |
  | adoption frame agents | 0.00 | 1.50 / 1.53 | 4.54 / 4.52 | 29.77 / 29.54 |
  | adoption frame minute | 0.00 | 0.40 | 7.71 / 6.92 | 81.64 / 78.11 |
  | following frame agents | 0.00 | 0.40 | 2.09 / 1.27 | 7.68 / 4.72 |

  - **What moves.** The checks' cost sits in the frames, not the commit. The agent time of every
    frame whose tick patched the lane buckets falls by the lane bucket check: the reference
    frame by 0.9 ms at 30k and 2.8 ms at 100k, the frame after the adoption frame by 0.8-0.9 ms
    at 30k and 2.7-3.0 ms at 100k in all three samples (both F runs below both C runs); at 10k
    it is under 0.1 ms. The adoption frame's agent time does not move (spreads overlap in 11 of
    12 samples; 10k sample 0 reads 0.41 ms lower): its tick rebuilds the buckets the commit
    invalidated, and the check runs only after a patch, so question 50's evidence placed it one
    frame early. The adoption frame's minute systems differ by -0.8, -0.1 and -0.2 ms per sample
    at 30k (only sample 0's spreads are apart) and by +0.9, +0.7 and -3.5 ms at 100k (sample 2's
    C spread is 79.1-84.2 ms): no consistent move. The two untimed setup frames take 0.42 / 0.03 s
    at 100k. No commit stage moves consistently: tier 0, which has no agent to check, differs by
    up to 0.6 ms (`surface`), the run-to-run noise. The first commit reads higher in F at 10k
    (107.1 / 114.1 against 102.9 / 103.5 ms, `mesh` 6.5-6.8 against 4.0-4.3) and in `surface` at
    100k sample 0 (+4.4 ms), and no later sample does; a likely cause, not verified, is that the
    checks' per-tick allocations had left freed memory the first commit reused. The preview
    stages do not change.
  - **The 100k commit against 16.7 ms** (F): 97.9 / 101.8 ms at sample 0 (the legacy terrain
    stage, about 80 ms of it), 20.9 / 19.9 at sample 1, 15.3 / 14.0 at sample 2; only sample 2
    meets the target, which stays open.
  - **Products.** The four runs print bitwise identical `COMMIT_PRODUCTS` lines at every tier,
    so the switch changes no product, and apart from the new `cell_entries` they equal
    `ROAD-68-K25`'s change (`road`'s Rust): the fixture fixes below change no city that built
    before. Later sessions build every binary with the switch, so these numbers start the new
    series; records before `ROAD-68-K24` hold the checks.
- **The 1,300 tier** (question 55). It never built: `PopulatedCity::build(1300)` houses 819
  residents on `road` and 826 at the fixture's first commit, because the grid's floored width
  keeps 54% of the side's square. A tier the floored grid cannot house is now built one block
  wider; 1,300 and 13,000 build, and every city that built before is built by the same calls.
  The wider 13,000 city then failed the lane bucket check, a stale entry the fixture left by
  writing a carrier's transit before it left the buckets; fixed in the fixture
  (`AgentSystem::place_arrival_carrier_at_home`, with a unit test).
- **The loaded cell store** (fresh, release, a temporary census of every entry, built against
  saved and loaded in one process). The only entries that differ are chunk entries that hold
  no block and no curve source and say only that their chunk was generated: 24 in the built
  2,000 city (3,501 entries against 3,477) and 26 at 10k (17,669 against 17,643), none in the
  loaded city alone, before and after two frames, with equal digests. A build's lot passes generate
  every chunk their queued regions touch, including chunks beyond any road's reach and past the
  world's edge; a load warms only the chunks near its roads (`warm_loaded_cells`) and leaves the
  rest cold, so a later request generates them, finds no cell and records the entry. These are
  rebuilt caches, not dropped state, so the save is unchanged. `COMMIT_PRODUCTS` now carries
  `cell_entries`, the store's entries less its empty chunk entries
  (`CellStore::empty_chunk_entries`, test-only), and `COMMIT_CELL_COPY` both counts.
- **Equivalence** (fresh, release F). `commit_breakdown_products_survive_the_snapshot` (2,000
  residents) passes in 28.5 s with `cell_entries` in the products (built 3,552 - 30 empty,
  loaded 3,536 - 14). The breakdown on the 10k city built in the run (`METRUM_COMMIT_CITIES=built`)
  prints the session's loaded products bitwise, `cell_entries` 17,688 (17,720 - 32 against
  17,696 - 8). `PopulatedCity::build` at 1,300 and 13,000 now passes with the test build's checks
  on (3.3 and 31.8 s).
- **Gate checks** (fresh, on the final code). `cargo clippy --all-targets`: the baseline's 59
  warnings, none added. `./run.sh --test` with `DISPLAY` and `WAYLAND_DISPLAY` unset (ended
  01:09): exit 0; debug lib 1,959 passed, 0 failed, 173 ignored (`ROAD-68-K23`'s 1,958 and the
  new lane snapshot test); integration 4 passed; every Godot script passes, no `SCRIPT ERROR`;
  skipped only the shader global probe and the rendered vegetation tests (headless). The
  gate-tagged ignored tests are unchanged (4, `TERRAIN-07`). `roadbed/` is untouched: 8,909
  production lines. The fixture source change re-keys the economy contract's snapshots once.

#### Commit stages, part 7 (`ROAD-68-G`, 2026-10-27)

Branch `road68-G7` on `terrain07-E` (`5f07b2ee`, `TERRAIN-07-E`'s Rust half: the graded terrain
path, no commit terrain validation). [Open question 33](#road-68-open-questions) (c), the user's
decision: with `TERRAIN-07`'s terrain the first commit of each populated tier still missed the
frame (25-30 ms on the game's path, `buildings` 7-11 ms of it), so lot regeneration is deferred
one frame, as D7 defers routing.

**The deferral** ([pure jobs](simulation.md#pure-jobs), [`zoning.md`](zoning.md)).

- *The commit.* At its entrance stage a commit whose edit queued lot regions takes them from the
  `cell lots` queue (`SimCore::take_commit_cell_lot_regions`) and leaves the lots, the attachment
  repair and the entrance update for later; an edit that queued none rebuilds its entrances at
  once, as before. After its last stage (the publication and the road-locked terrain state) it
  starts a pure job on the background pool, before the routing rebuild, that plans the regions
  in order through the planner the commit used (`CellLotPlanner::plan_region`, one function for
  both), on the inputs the commit left: the road tool view's journal-synced cell copy
  ([question 32](#road-68-open-questions)) cloned in `O(1)`, the parcel store shared
  copy-on-write (`SharedParcelStore`), the published graph and roadbed, the field index and a
  snapshot of the explicit cell sites (`ExplicitSiteSnapshot`: the allocator now lists them in
  its zone index, `explicit_cell_sites`, kept across swap-removes like the chunk lists, and the
  snapshot visits the chunks the allocator's own candidate query visits, with its bounds and
  overlap tests). Taking the inputs is `O(regions + explicit sites)`, under 0.01 ms in every
  sample below.
- *The adoption.* The next frame adopts the plan first, waiting for a late job before it enters
  the pool (the sim thread waits outside the core lock); that frame is D7's routing hold, which
  advances nothing. It copies the cell entries the plan wrote into the live store
  (`CellStore::adopt_plan`, journaled for the view), makes the plan's parcel calls, reattaches
  the lots' occupants, runs the commit's attachment repair and entrance update
  (`repair_commit_entrances`, with the entrance cache state read before the plan, as the commit
  read it) and publishes the road tool's view of the lots. Every editor path that reads or writes
  the lots, parcels or attachments adopts first (commits, undo, bulldoze, node, lane, class,
  crosswalk and border edits, terrain strokes, building and field placement, cell gestures,
  chunk and lot preparation, saves); loads and world resets drop the plan; the cell overlay's
  stale chunks wait for it. Debug builds assert that nothing wrote the parcel or cell store
  while a plan was pending and that every edit adopted before recording its undo.
- *Exact.* `deferred_cell_lots_equal_lots_planned_in_the_commit` (new) builds the zoned 800 m
  grid twice, one city deferring and one planning in the commit (a test switch), and after a T
  split adopted by the next frame, a T beside the painted rows undone while its job runs (each
  job sleeps 150 ms first), and three commits adopted by the next commit and by a save, compares
  the cell store's whole state and entry count, the overlay and building revisions, every
  entrance, the undo history and the narrow-marks test's products (paint and claims, generated
  cells, parcels, queued regions, curb frames, attachments). It passes; with the adoption's cell
  copy left out it fails at the first comparison. The breakdown now reads its zoning products
  after the first frame (the held frame, which adopts), and they equal the control's, which read
  them before any frame, at every tier (below).

**Two exact levers on what remains.**

- *The background jobs start last.* The lot plan and the routing rebuild are started after the
  road-locked terrain state, which is CPU-bound on every worker; started before it, they took its
  cores (both pools share the eight).
- *Changed chunks spliced on the pool.* Each changed road chunk is spliced from its own
  published and fresh meshes (`splice_chunks`), so the chunks splice in parallel and are
  collected in chunk order; the debug build's check against emitting every piece still runs on
  every spliced publication. The preview's splice is the same function.

**Landing session** (fresh, 2026-10-27 06:05:59-06:09:44, one hold of both locks, `taskset -c
0,2,4,6,8,10,12,14`, `METRUM_DEBUG=0`, no restic, compiler, cargo or Godot process at its start
or after any run; release library tests). C = `terrain07-E` `5f07b2ee` (`$TEST_BINARY` sha256
`293341d6…`), F = this part at `71161e5e` (sha256 `b2c096f9…`: the deferral, the job order and
the parallel splice); each binary's 10k, 30k and 100k cities prepared before the session, every
populated tier's `COMMIT_FIXTURE` `loaded`. C F F C: the preview stages (four workers) and the
breakdown on the game's path (`METRUM_COMMIT_PREVIEW`, eight workers, tiers 0 to 100k). The
four runs print bitwise identical `COMMIT_PRODUCTS` at every tier.

Click → ack per sample, ms, the two C runs / the two F runs:

| Tier | Sample 0 | Sample 1 | Sample 2 |
| --- | --- | --- | --- |
| 0 | 8.12, 8.20 / 7.32, 7.10 | 5.42, 5.20 / 5.31, 5.55 | 3.78, 3.66 / 4.04, 3.01 |
| 10k | 27.85, 29.68 / 15.97, 16.24 | 12.50, 12.40 / 8.56, 7.48 | 5.59, 5.26 / 4.26, 4.79 |
| 30k | 25.83, 29.04 / 17.10, 16.43 | 11.22, 10.59 / 8.10, 8.20 | 9.78, 9.45 / 7.79, 7.75 |
| 100k | 25.88, 26.70 / 17.34, 16.86 | 14.43, 14.25 / 11.30, 11.80 | 12.62, 10.97 / 7.65, 7.75 |

The first frame after the commit, which adopts the lots (F; it starts at once in the harness, so
it waits for the whole job, while the game's next tick comes up to a frame later), ms, the two
runs: 10k 13.47, 13.19 / 7.77, 4.80 / 1.59, 2.32; 30k 6.47, 17.95 / 2.04, 2.12 / 2.88, 1.91;
100k 9.00, 9.05 / 4.76, 4.48 / 2.85, 2.45 per sample. Of it, the wait for the job at sample 0:
13.15, 12.83 (10k), 5.96, 11.76 (30k), 7.57, 7.53 ms (100k); the install, reattachment, repair,
entrances and view publication 0.2-1.6 ms but once 5.90 ms (30k sample 0, second run). The
routing adoption frame that follows is unchanged (wait 59-74 ms at 100k in both builds).

- **Against 16.7 ms** (F): every sample at every tier fits except the first at 100k (17.34 and
  16.86 ms) and once at 30k (17.10 ms; the other run 16.43). The deferral took `buildings`
  (7.1-10.4 ms at the first samples, 1.1-3.9 at the others) off the commit.
- **What the first 100k sample spends** (C / F, two runs each): the road-locked terrain state
  3.64, 4.22 / 4.18, 3.61; the render publication 3.81, 3.68 / 3.08, 2.89 (the parallel splice);
  the graded terrain stage (`surface`) 2.37, 2.29 / 2.10, 2.22; the tool snapshot 2.36, 2.40 /
  2.38, 2.37; the agent invalidation 1.83, 1.73 / 1.66, 1.96 and the lanes and reattachment
  1.57, 2.03 / 1.75, 1.66; the plan, undo, topology and marks 0.87, 0.94 / 0.90, 0.91.
- **Preview stages.** The graded stage is unchanged (`sloped_t` p50 1.27 / 1.30-1.31 ms,
  `sloped_branch` 1.13 / 1.15 ms), but the payload stage rose from 0.38-0.39 to 0.46-0.47 ms in
  both fixtures and both runs: the preview's splice entered the pool from outside it for its
  few chunks. Fixed after the session (below).

**Where the first 100k commit goes** (perf, LBR, 15 kHz, enabled per commit, the deferring build
before the two levers; a profile, not acceptance). The road-locked terrain state is the legacy
refined patches' road ownership: for every pending patch it grades the road loops of every CDT
tile in the patch to find the patch's largest grading margin (`road_terrain_patch_margin_m`,
`road_loop_grading`, bilinear height samples), 420 samples on the eight workers (28 ms of CPU).
It exists for the refined patches Godot renders until `TERRAIN-07-D`'s renderer replaces them;
`TERRAIN-07-G` deletes it with the CDT. The tool snapshot is the commit's graph copy (51 of the
publication's 54 commit-thread samples), 21 of those in page faults and heap growth. The splice
took 32 commit-thread samples, the reattachment search 159 samples across the workers (19 on
the commit thread), the lots job's generation 67 samples on the background pool inside the
commit window (it then started before the terrain state).

**After the session** (two more exact levers; not in the session's F):

- *The commit's graph copy on the pool.* `RegionGraph::clone_on_pool` clones each field by its
  own `clone`, the nodes, edges and adjacency in parallel and collected in order; the struct is
  destructured whole, so a new field cannot be left out. The publication copies the graph with
  it beside the cell follow. Debug builds compare the preview replica with the published graph at
  every publication the road tool follows.
- *The preview splices off the pool.* `splice_chunks` splices in parallel only on a pool worker
  (the commit's publication); the road tool's preview, outside the pools, splices its few chunks
  where it is, as before the session.

Diagnostic after the levers (fresh, 06:32-06:41, a heavy run, not a session: `.run.lock` and
the shared timing lock, P-cores, eight workers, saved cities, no compiler running, one C and one
F run): C as above against `540d7b9d` (sha256 `40777b93…`). Click → ack per sample: tier 0
8.21 / 7.63, 5.28 / 4.72, 3.84 / 3.34; 10k 29.97 / 17.21, 16.38 / 7.68, 5.51 / 4.49; 30k
25.74 / 16.42, 10.31 / 7.87, 10.20 / 6.81; 100k 25.63 / 15.45, 15.41 / 10.10, 9.91 / 5.63 ms.
The tool snapshot at 100k falls from 2.17-2.33 to 0.52-0.71 ms; `COMMIT_PRODUCTS` equal C's
at every tier. One run each, so the 10k first sample (17.21 ms here, 15.97 and 16.24 in the
session's F) is not settled. A short stage-only run (both locks, four workers, C F F C) puts the
preview's payload stage back at the control's (0.383-0.386 ms C, 0.387-0.391 ms F, `sloped_t`
and `sloped_branch`). The next landing session (after the rebase onto `road`) measures this tip.

**Open (not relaxed).** The 16.7 ms acceptance: the first 100k and 30k samples missed in the
session by up to 0.6 ms; the levers after it bring them under in one diagnostic run, which does
not establish it. Locality (the commit's cost flat from 0 to 100k with the edited neighbourhood
held fixed) is not measured: the breakdown's branches sit beside a different block edge at each
tier, so a fixture that holds the neighbourhood fixed is needed. The adoption frame (in the
harness, which steps it at once, 9.0-13.5 ms at the first samples, once 17.95 ms) waits for the
plan; starting the routing rebuild after the plan is the next lever there. The legacy refined
road ownership stays on the commit until `TERRAIN-07` removes it.

**Checks** (fresh). Debug `cargo test --lib` on `71161e5e` and again on `540d7b9d`: 1,962 passed,
0 failed, 175 ignored (`TERRAIN-07-E` round 3's 1,961 plus the new test). `./run.sh --test` with
`DISPLAY` and `WAYLAND_DISPLAY` unset on `71161e5e`: exit 0, the library as above,
`tests/growth_scenario.rs` 4 passed, every Godot script passes with no `SCRIPT ERROR`; skipped
the shader global probe (headless renderer) and the rendered vegetation tests (Forward+
display); not re-run on the two later commits. `cargo clippy --all-targets` on `71161e5e` and
`540d7b9d`: the baseline's 59 warnings, the same set. `roadbed/` is untouched (8,946 production
lines, part 6's counting). The change adds about 950 production lines (the adoption module 236,
the planner's deferred plan 134, the store's adoption 89, the allocator's explicit sites 99, the
editors' adoption calls 66, the pool clone 50) and the test 68.

#### Commit stages, part 8 (`ROAD-68-G`, 2026-10-27)

Branch `road68-G7` on `terrain07-E` (`5f07b2ee`, unmoved), after part 7's `4c99adce`.

**What changed.**

- *The routing rebuild after the lot plan.* A commit that deferred its lots schedules the
  routing rebuild unstarted and the lot job starts it when its plan ends
  ([pure jobs](simulation.md#pure-jobs): `PureJob::unstarted`, `JobStart`); a commit that
  deferred nothing starts it at once. The plan no longer shares the background pool with the
  rebuild.
- *The adoption tick a frame after the commit* ([question 58](#road-68-open-questions), (b)
  taken pending the user's review). The sim thread used to run the adoption frame at once
  behind a 15-17 ms commit, so the frame waited for the whole plan; it now publishes the
  commit's edit snapshot without a tick and moves the next tick to one interval after the
  commit.
- *Attribution in the breakdown.* The adoption's install is split (plan install, reattachment,
  repair), every background job's span is listed (`jobs`), and `METRUM_COMMIT_NEXT_TICK=frame`
  steps the first frame one frame after the commit returned, as the sim thread now does
  ([Method](#method)).
- *The locality fixture* (`benchmark_commit_locality`, [Method](#method)): the zoned 800 m
  grid and its trunk held fixed in a 12,288 m world while 0, 1,000, 10,000 and 100,000 remote
  sites (each a parcel and a housed agent, on 480 m block streets: 2,583 edges at 100k) grow
  east of it; the breakdown's three branches are the edits.

**Landing session** (fresh, 2026-10-27 08:15:23-08:21:51, one hold of both locks, `taskset -c
0,2,4,6,8,10,12,14`, `METRUM_DEBUG=0`, no restic, compiler, cargo or Godot process at its start
or after any run; release library tests). C = `terrain07-E` `5f07b2ee` (`$TEST_BINARY` sha256
`293341d6…`, part 7's control), F = this part at `632347b3` (sha256 `d0cefe49…`); every
populated tier's `COMMIT_FIXTURE` `loaded`. Preview stages C F F C (four workers), the
breakdown C F F C on the game's path (`METRUM_COMMIT_PREVIEW`, eight workers, tiers 0 to 100k),
then F twice with `METRUM_COMMIT_NEXT_TICK=frame` and the locality fixture on F twice (eight
workers, the game's path). All eight breakdown runs print bitwise identical `COMMIT_PRODUCTS`.

Click → ack at the first sample, ms, C (two runs) / F (four runs: two at once, two a frame
later, the same commit):

| Tier | C | F |
| --- | --- | --- |
| 0 | 8.11, 8.00 | 7.47, 7.04, 8.40, 8.25 |
| 10k | 30.12, 30.28 | 16.04, 17.92, 16.17, 17.00 |
| 30k | 25.97, 25.80 | 16.52, 15.71, 16.40, 17.83 |
| 100k | 27.39, 28.32 | 14.94, 15.74, 15.72, 16.16 |

The later samples take 3.9-10.9 ms in F (C 5.5-15.1). At 10k the first sample's largest stages
are the legacy road-locked terrain state (6.37, 7.31 ms in the two F runs at once; C 8.40,
8.96) and the render publication (4.13, 4.25); at 100k the tool snapshot is 0.55 ms (C 2.35-2.45,
part 7's graph copy on the pool).

The adoption frame (the first after the commit), ms at the first samples:

- *At once* (the harness default; what the sim thread ran before question 58): 10k 9.93, 11.56;
  30k 7.31, 13.20; 100k 8.18, 8.63 (part 7: 13.47, 13.19; 6.47, 17.95; 9.00, 9.05). Of it, the
  wait for the plan 9.65, 9.84; 6.69, 6.95; 6.51, 7.25; the install 0.20-1.66, once 3.35 ms (30k,
  all but 0.07 ms of it the repair's whole-city attachment test), the view publication
  0.06-0.14. The lot job ran 11.1-11.2 ms at 10k and 7.5-8.5 ms at 30k and 100k from the
  commit's last stage; the routing rebuild then started (15 ms at 10k, 49-52 ms at 30k, about
  295 ms at 100k), and its adoption frame waits 64-86 ms at 100k (part 7: 59-74).
- *A frame later* (`METRUM_COMMIT_NEXT_TICK=frame`, the game since question 58): the plan is
  done in every sample, so the frame takes 0.34-2.33 ms at every tier and sample, the repair
  0.16-2.01 ms of it.
- **Against 16.7 ms.** With the tick a frame later no adoption frame comes near the frame
  budget. The commit itself still misses at the first sample in three of twelve populated runs:
  10k 17.92 and 17.00, 30k 17.83 (each other F first sample 14.94-16.52).
- **Preview stages** unchanged: payload p50 0.387-0.395 ms (C 0.380-0.389), graded 1.27-1.29 /
  1.13-1.14 ms (C 1.27-1.28 / 1.12-1.14), totals 6.94-7.16 ms (C 6.90-7.06).

**Locality** (the same session, F, two runs). The local products (each sample's republished
chunks 16 / 13 / 13 and graded tile digests, the neighbourhood's cell claims and generated cells
with grids, lots and roads named by geometry, its 1,264 parcels and 1,264 buildings by geometry)
are bitwise identical at 0, 1,000, 10,000 and 100,000 remote sites and between the runs. The
costs are not flat. Click → ack per sample, ms, the two runs:

| Remote sites | Sample 0 | Sample 1 | Sample 2 |
| --- | --- | --- | --- |
| 0 | 18.89, 19.22 | 12.08, 12.18 | 11.46, 11.23 |
| 1,000 | 18.39, 16.79 | 12.26, 12.21 | 12.29, 12.88 |
| 10,000 | 19.14, 19.04 | 13.49, 12.41 | 12.25, 12.27 |
| 100,000 | 21.97, 22.09 | 14.87, 14.88 | 14.58, 14.73 |

- Flat to 10,000 sites; at 100,000 each sample gains about 3 ms: the graded surface stage
  +0.6-0.9 ms, the tool snapshot's graph copy +0.5 (`O(edges)`), the plan and undo (`add`)
  +0.4-0.6, the agent passes +0.1-1.0 (`O(agents / P)`, question 21 (d)), the lanes +0.2-0.4 and
  the terrain state +0.2-0.7. The adoption's attachment repair grows from 0.04-0.13 ms (once
  1.18) to
  2.8-6.2 ms: its plausibility test visits every building (`O(buildings / P)`), walking each
  one's street to its frontage point. A first trial with 4.8 km background streets (fresh,
  heavy run, not a session) put that test at 15-17 ms at 100,000 sites with the same local
  products, so its cost follows street length too.
- The neighbourhood's own first commit misses 16.7 ms with no background (18.9-19.2 ms; render
  publication 8.7-8.9 ms, terrain state 4.3-5.3), larger than the populated cities' first
  samples: the zoned grid's lots beside the trunk.
- Setup, outside timing: the neighbourhood 0.24-0.28 s per tier; the background 0.03 / 0.06 /
  0.38 / 3.5-3.8 s; the breakdown's own setup (`COMMIT_SETUP`: refined terrain 0.14-6.3 s, graded
  census 0.03-1.65 s) as printed.

**Open (not relaxed).** The 16.7 ms commit: the first 10k and 30k samples miss in three of
twelve runs by up to 1.2 ms, the legacy terrain state (until `TERRAIN-07-D`/`-G`) their largest
stage. Locality: products local, cost not flat at 100k remote sites (about 3 ms of commit, and
the adoption's whole-city repair test); the neighbourhood's own first commit misses 16.7 ms.
The candidates are question 21 (d) for the agent passes, an attachment repair that tests only
the buildings the edit can reach, and the graph copy.

**Checks** (fresh). `./run.sh --test` with `DISPLAY` and `WAYLAND_DISPLAY` unset, on part 7's
tip `4c99adce` (in the control worktree) and on `01aa1189` (this part before the block-street
background, a change to an ignored benchmark only): exit 0 both; the library 1,962 / 0 / 175
and 1,964 passed, 0 failed, 176 ignored (the two new tests and the new benchmark);
`tests/growth_scenario.rs` 4 passed; every Godot script passes with no `SCRIPT ERROR`; skipped
the shader global probe (headless renderer) and the rendered vegetation tests (Forward+
display). `cargo clippy --all-targets` on `632347b3`: the baseline's 59 warnings, the same set.
`roadbed/` is untouched (8,946 production lines). The part adds about 85 production lines (the
pure job's unstarted start 31, the sim thread's tick 14, the adoption split 19, the commit's
job order 10, the rebuild's unstarted schedule 9) and about 400 test lines (the locality fixture
286, the cell store's named digest 49, the breakdown's mode and spans 34, two tests 33).

#### Commit stages, part 9 (`ROAD-68-G`, 2026-10-28)

Branch `road68-G9`: parts 7 and 8 rebased from the old `terrain07-E` work-in-progress commit
`5f07b2ee` onto `road` `c64786e9` (gate E's commit) with no conflict; `road68-G7` stays at
`da362880` for the hashes the earlier records cite. Part 8's tip `da362880` is `9e647b86` here,
part 7's `4c99adce` is `35ed7721`.

**What changed.**

- *The adoption's attachment repair follows the edit.* Its plausibility test visited every
  building (`O(B / P)`, 2.8-6.2 ms at 100,000 remote sites in part 8). It now tests only the
  buildings indexed in the 512 m building chunks within reach of a segment of a lane rebuild
  closure edge (`BuildingAllocator::attachment_repair_candidates`), plus the occupants of the
  lots the plan reattached: the reach runs from the largest lot's centre to its frontage, then
  as far as a repair's frontage search or a plausible attachment's tolerance, then the edge's
  frontage offset. The closure's geometry covers what its edges had before the edit (a split
  keeps both halves in it), so a building outside fronts no changed edge on an attachment that
  was plausible and finds none in its search. The cost is `O(closure segments + chunks +
  candidates log candidates)` plus the visits, independent of the city; while the chunk index
  is stale the test is the whole city's, as before. Test builds (outside timing runs) compare
  each such repair's admitted buildings with the whole city's test, which stays as that
  reference; the debug suite passes with it, and a test shows it catches a building attached
  to an edited edge far from it.
- *Question 21 (d), the agent passes:* not built, [open question 59](#road-68-open-questions).
  The lane buckets could serve two of the three scans, but nothing indexes the third (the
  planned routes), and an index would run in every tick.
- *The tool snapshot's graph copy:* not shared, [open question 60](#road-68-open-questions):
  the preview's replica is mutable per pose, so sharing it is not exact.
- *The zoned neighbourhood's render publication* (attributed, not changed). A perf profile
  (LBR, 20 kHz, eight workers; a heavy run, not a timing) of the locality fixture's
  neighbourhood alone puts 42 % of the commit thread's samples inside
  `precompute_road_mesh_data` in the new lanes' lift onto the visible surface
  (`sync_new_lane_heights`: per-lane height queries on the roadbed's triangles), 19 % in chunk
  emission (`generate_mesh_chunks_with_surface`), 19 % in the changed chunks' seal
  (`render_payload_is_valid`) and the rest in the pool's collect. Under the profiler the commit
  thread runs for about half of the stage's wall time and waits on the pool for the rest. The lift and
  the emission are already parallel and per changed lane or piece; no exact small fix was found,
  so the stage stays as it is. The neighbourhood's first edit simply changes more lanes and
  pieces than the populated cities' (16, 13 and 13 changed chunks against 2-6).

**Landing session** (fresh, 2026-10-28 02:34:13-02:40:40, one hold of both locks, `taskset -c
0,2,4,6,8,10,12,14`, `METRUM_DEBUG=0`, no restic, compiler, cargo or Godot process at its start
or after any run; release library tests). C = `road` `c64786e9` (`$TEST_BINARY` sha256
`02e168b3…`), F = this part at `1e8bd3d3` (sha256 `279ff37c…`), L = part 8 rebased (`9e647b86`,
sha256 `bd9a8074…`, the locality control); every populated tier's `COMMIT_FIXTURE` `loaded`.
Preview stages C F F C (four workers), the breakdown C F F C (`METRUM_COMMIT_PREVIEW`, eight
workers, tiers 0 to 100k), F twice with `METRUM_COMMIT_NEXT_TICK=frame`, and the locality
fixture L F F L. All eight breakdown runs print bitwise identical `COMMIT_PRODUCTS`, C's
included.

Click → ack at the first sample, ms, C (two runs) / F (four runs: two at once, two a frame
later, the same commit):

| Tier | C | F |
| --- | --- | --- |
| 0 | 7.75, 8.11 | 6.95, 7.93, 8.35, 7.97 |
| 10k | 32.26, 30.96 | 16.47, 16.79, 16.59, 16.95 |
| 30k | 26.36, 24.49 | 16.18, 18.17, 16.43, 16.48 |
| 100k | 25.22, 24.98 | 15.38, 15.41, 15.78, 14.40 |

- The later samples take 3.2-9.3 ms in F (C 3.0-20.7). At 10k and 30k the first sample's
  largest stage is still the legacy road-locked terrain state (6.3-8.1 ms in F, C 7.3-9.0),
  which `TERRAIN-07-G` removes; at 100k the tool snapshot is 0.49-0.86 ms (C 2.1-2.4).
- **Against 16.7 ms.** Three of twelve populated first samples miss: 10k 16.79 and 16.95, 30k
  18.17. Every 100k sample fits.
- **The adoption frame.** At once (the harness default) it waits 7.2-12.4 ms for the plan at
  the first samples; a frame later (the game since question 58) it takes 0.23-0.58 ms at every
  populated tier and sample (0.05-0.14 at 0), the repair 0.05-0.22 ms of it (part 8: 0.34-2.33 and 0.16-2.01, on
  `terrain07-E`, historical).
- **Preview stages** unchanged: payload p50 0.398-0.409 ms, totals 2.26-2.29 / 1.99-2.04 ms
  (C 2.23-2.28 / 1.99-2.01) on the sloped T and branch.

**Locality** (the same session, L F F L). The local products (republished chunks 16 / 13 / 13,
graded tile digests, the neighbourhood's cell claims, generated cells, 1,264 parcels and 1,264
buildings) are bitwise identical at 0, 1,000, 10,000 and 100,000 remote sites, between the
runs and between L and F. The adoption's repair, ms per sample (two runs each):

| Remote sites | L | F |
| --- | --- | --- |
| 0 | 0.04-0.92 | 0.06-0.14 |
| 1,000 | 0.08-2.36 | 0.07-0.16, once 2.25 |
| 10,000 | 0.32-0.83 | 0.06-0.17 |
| 100,000 | 2.79-8.10 | 0.07-0.17 |

- The repair is flat now. The single 2.2-2.4 ms samples at 1,000 sites, in both binaries, are
  not attributed (part 8 had one too).
- What still grows at 100,000 sites: the tool snapshot's graph copy, 0.05-0.07 to 0.48-0.56 ms
  (question 60), and the agent passes, 0.29-1.42 to 0.78-2.73 ms (question 59).
- Click → ack varied by up to 8 ms between runs of one binary at one tier in this session (F
  at 10,000 sites, second sample: 12.22 and 20.32 ms; the render publication and the terrain
  state carry most of it), so it cannot show the commit's flatness to better than that. F's
  first samples: 19.62, 17.59 ms at 0 sites; 17.41, 18.44 at 1,000; 19.06, 18.41 at 10,000;
  17.98, 20.52 at 100,000.

**Open (not relaxed).** The 16.7 ms commit: three of twelve populated first samples miss by up
to 1.5 ms, the legacy terrain state their largest stage (`TERRAIN-07-G`). Locality: the
repair follows the edit; the agent passes (question 59) and the graph copy (question 60) still
grow with the city; the neighbourhood's own first commit misses 16.7 ms (17.6-19.6 ms in F),
its render publication 7.0-8.9 ms.

**Checks** (fresh). On the rebased tip `9e647b86`, before this part's change: the debug
`cargo test --lib` 1,981 passed, 0 failed, 180 ignored; `./run.sh --test` with `DISPLAY` and
`WAYLAND_DISPLAY` unset (in the control worktree) exit 0: the library 1,981 / 0 / 180,
`tests/growth_scenario.rs` 4 passed, every Godot script passes with no `SCRIPT ERROR`; skipped
the shader global probe (headless renderer) and the rendered vegetation tests (Forward+
display). On `1e8bd3d3`: `cargo clippy --all-targets` reports the baseline's 59 warnings, the
same set; the debug `cargo test --lib` 1,983 / 0 / 180 (the two new tests); `./run.sh --test`
the same way, after the session: exit 0, the library 1,983 / 0 / 180, `tests/growth_scenario.rs`
4 passed, every Godot script passes with no `SCRIPT ERROR`, the same two skips. `roadbed/` is
untouched (gate E's count, 9,617, stands). The part adds about 100 production lines (the
candidate search 63, the split admission pass 25, the adoption's call 12) and 71 test lines.

#### Commit stages, part 10: the gate (`ROAD-68-G`, 2026-10-29)

Branch `road68-G10`: parts 1-9 (`road68-G9`, `febc30fa`) rebased from gate E (`c64786e9`) onto
`road` `a0cfe039` (`TERRAIN-07-G`'s gate), then the rebase's fixes and part 10. `road68-G9` stays
for the hashes the earlier records cite; part 9's `1e8bd3d3` is `e570573d` here, part 8's
`9e647b86` is `10715797`, part 7's `35ed7721` is `ff998211` (the full map is
`$ARTIFACTS/road68/g10/hashmap.txt`).

**The rebase.** Git reported no conflict, but two merges brought back what `TERRAIN-07-G` deleted,
and the crate did not compile until `8bfd9ebf` removed them:

- part 7 (`bb19de50`, here `3b1e587b`) moved the commit's road-locked terrain patch state after
  the tool snapshot; on a base that no longer has it, the move re-added the call
  (`refresh_road_locked_terrain_patch_state`). It is gone: the commit's last stage is the tool
  snapshot, and its background jobs start after it;
- part 8's locality fixture refreshed the deleted engineered terrain ownership after its
  background's bulk build; it now resets the graded terrain once, as `road`'s populated city does
  (`subsume_world_scale_road_changes`, `bump_global_terrain_payload_generation`).

Every other overlap merged at its meaning (the core state's pending lots beside `road`'s camera
position, the sim thread, the editors' adoption calls, undo, world resets, save and load, the
breakdown's stage list without `refined_state_ms` and with `road`'s `terrain_query_index_ms`, the
allocator's explicit-site list replacing its count, the site query, the docs).

**Part 10: the undo record's kept terrain tiles are queued last** (`10477798`, found by the first
landing session below). A commit's graded stage queued the bake of the tiles its undo record keeps
(`TERRAIN-07-E`, [`terrain.md`](terrain.md#preview-commit-undo-and-load)) on the background pool
at once. That pool has its own workers but shares the eight CPUs with the shared pool the
commit's later stages run on. In session 1 the bake ran from about 4-5 ms after the click while
the agent pass, the lanes and the publication ran; each populated first sample that missed 16.7
ms (below) had the bake running 5.6-10.9 ms beside a slower stage (the 100k agent pass 9.71 ms
against 2.7-4.0 in the other runs). The job is now made unstarted, with its inputs taken where
they were (`PureJob::unstarted`), and the commit queues it after its lot plan and routing jobs
(`SimCore::take_kept_graded_tiles_start`), as part 7 did for those. Its result depends only on
the inputs; undo never waited for it and still does not (it bakes what the job has not), and a
test that waits runs the job if nobody started it. In session 2 the bake ends 0.2-4.5 ms after
the acknowledgement at every tier, before the next frame.

**Landing sessions** (fresh, 2026-10-29, one hold of both locks each, `taskset -c
0,2,4,6,8,10,12,14`, `METRUM_DEBUG=0`, no restic, compiler, cargo or Godot process at the start or
after any run; release library tests). C = `road` `a0cfe039` with a test-only harness (the
locality fixture, `zoned_city_in_world`, the cell store's named digest and `profile_commits`'s
site-removal switch from parts 8 and 9; `$TEST_BINARY` sha256 `aec1c293…`), the same C in both.
Session 1 (10:24:21-10:35) ran F1 = `5e31ad31` (sha256 `b3689483…`, the rebased parts and the
rebase's fixes); session 2 (10:48:58-10:54:15) F2 = `10477798` (sha256 `4ce7421b…`, part 10).
Each binary's 10k, 30k and 100k cities were prepared before its session; every populated tier's
`COMMIT_FIXTURE` is `loaded`. Each session: the preview stages C F F C (four workers), the
breakdown C F F C on the game's path (`METRUM_COMMIT_PREVIEW`, eight workers, tiers 0 to 100k),
the breakdown F F with `METRUM_COMMIT_NEXT_TICK=frame` (the game's adoption frame, question 58
(b)) and the locality fixture C F F C; session 1 also the economy contract C F F C. All twelve
breakdown runs of both sessions print bitwise identical `COMMIT_PRODUCTS`, C's included.

Click → ack, ms: the first sample of each run, and the median of each run's three samples. C's
two runs; F's four (two at once, two a frame later: the same commit, timed the same way):

| Tier | C first (session 1 / 2) | F1 first (session 1) | F2 first (session 2) | C median (session 2) | F2 median |
| --- | --- | --- | --- | --- | --- |
| 0 | 7.17, 7.40 / 8.44, 7.31 | 7.04, 6.75, 7.21, 7.04 | 5.56, 6.07, 6.04, 6.24 | 3.55, 4.26 | 3.11, 4.07, 4.23, 4.45 |
| 10k | 21.33, 21.29 / 27.45, 26.16 | 12.48, 12.32, 11.56, 11.41 | 10.23, 9.63, 10.75, 9.67 | 7.94, 8.65 | 3.42, 3.33, 4.29, 3.67 |
| 30k | 19.80, 20.12 / 21.11, 24.10 | 13.38, 12.74, 12.05, **17.50** | 10.40, 10.65, 12.09, 11.67 | 8.24, 7.33 | 4.80, 4.12, 4.11, 4.38 |
| 100k | 24.65, 25.72 / 23.60, 25.42 | 15.14, **17.26**, **20.76**, 14.58 | 12.48, 12.22, 11.45, 13.79 | 11.76, 13.12 | 7.21, 7.54, 7.05, 7.04 |

F1's medians: 3.58-4.33 (0), 3.98-4.28 (10k), 4.89-5.45 (30k), 7.88-8.77 ms (100k).

- **Against 16.7 ms.** Session 1: three of twelve populated first samples miss (30k 17.50, 100k
  17.26 and 20.76), each attributed to the kept tiles' bake beside the commit's later stages
  (above): the 100k 20.76 ms sample's agent pass took 9.71 ms with the bake running 4.8-15.7 ms,
  the 17.26 ms sample's publication 5.43 ms (3.3-3.6 in the others) with the bake 5.2-10.8 ms,
  the 30k 17.50 ms sample's publication 7.43 and lanes 5.36 ms with the bake 4.2-10.7 ms.
  Session 2, after part 10: **every sample of every run at every tier fits**, the first samples
  9.63-13.79 ms at 10k-100k, every later sample at most 7.54 ms in F2 (C up to 13.12). The 100k
  agent pass is 0.26-0.83 ms (C 1.91-2.89).
- **What the first 100k sample spends** (F2, four runs; C's two in brackets): the graded and
  roadbed stage (`surface`) 4.08-4.25 (4.01, 4.10), of which `road`'s terrain query index update
  2.23-2.34 (`TERRAIN-07-F`'s T44, [carried to H](#road-68-h--acceptance-and-cleanup) item 7);
  the render publication 3.32-4.05 (3.68, 3.70); the lanes and reattachment 1.53-1.79 (2.18,
  3.60); the plan, undo, topology and marks 0.84-0.88 (0.81, 0.83); the tool snapshot's graph
  copy 0.50-0.54 (1.94, 2.50); the agent pass 0.26-0.83 (1.91, 2.89); C's lots and entrances
  (`buildings`) 7.68, 8.39 are the frame after in F. The legacy terrain state (6.3-8.1 ms of part
  9's misses, historical) is gone with `TERRAIN-07-G`.
- **The adoption frame** (the first after the commit, which adopts the lots). At once (the
  harness default) it waits 9.4-14.6 ms for the plan at the populated first samples; a frame later
  (the game since question 58) it takes 0.31-0.46 ms at every populated tier and 0.09-0.10 at 0,
  the attachment repair 0.16-0.19 ms of it.
- **Routing adoption, separately** (D7's hold): at 100k the rebuild takes 256-287 ms and
  the routing adoption comes 242-262 ms after the click, the adoption frame waiting 26-37 ms for
  it (C: 301-305, 268-278 and 54-64 ms); at 30k 41-46, 32-43 and under 0.3 ms; at 10k 12-14, 7-22
  and under 0.3 ms.
- **Preview stages** unchanged in both sessions: payload p50 0.389-0.429 ms (C 0.388-0.427),
  graded p50 1.37-1.74 / p95 1.84-2.16 ms (C 1.41-1.70 / 1.94-2.17), totals p50 2.37-2.84 ms (C
  2.46-2.81), on the sloped T and branch.

**Locality** (both sessions, C F F C). The local products (republished chunks, graded tile
digests, the neighbourhood's cell claims, generated cells, 1,264 parcels and 1,264 buildings) are
bitwise identical at 0, 1,000, 10,000 and 100,000 remote sites, between the runs, between C and F
and between the sessions. Click → ack per sample in session 2, ms (F2's two runs; C's in
brackets):

| Remote sites | Sample 0 | Sample 1 | Sample 2 |
| --- | --- | --- | --- |
| 0 | 12.35, 14.69 (30.91, 34.30) | 11.27, 10.19 | 11.21, 11.26 |
| 1,000 | 12.01, 12.45 (33.24, 31.53) | 11.33, 10.77 | 11.34, 10.77 |
| 10,000 | 14.40, 15.38 (34.64, 36.46) | 13.99, 10.73 | 9.93, 9.52 |
| 100,000 | 15.42, 16.28 (42.00, 41.35) | 12.38, **16.73** | 11.86, 11.55 |

- The neighbourhood's own first commit now fits (12.35-14.69 ms; session 1 16.73-16.80; part 9
  17.6-19.6, historical), its publication 6.3-7.2 ms (session 1 10.4-10.5, with the bake beside it).
- **Not flat at 100,000 sites:** the first sample gains 1.6-3.1 ms. Its parts (F2, sample 0, 0 →
  100,000 sites): the roadbed stage +0.8-0.9 ms, of which the terrain query index update +0.56-0.59
  (`TERRAIN-07-F`'s T44, carried to H item 7); the graded stage +0.5-0.7; the topology adoption
  +0.42-0.43 (in C too: 0.03 → 0.49); the graph copy +0.50-0.62 ([question 60](#road-68-open-questions)
  (a), agreed); the agent passes +0.05 (question 59 (b), agreed); the lanes +0.1. One second
  sample at 100,000 sites took 16.73 ms (the other run 12.38): its roadbed stage 4.34 ms against
  2.28, the terrain query index update 3.46 against 1.81 of it, and its publication 7.44 against
  6.62. The adoption's repair stays flat (0.06-0.19 ms), except five of F's 48 samples in the two
  sessions at 1.2-3.0 ms, each a third sample (at 0, 10,000 and 100,000 sites), as part 9's at
  1,000; not attributed.
  `ROAD-68-H` owns the populated locality check, so this is carried there (item 14).

**Bitwise** (fresh, not timed). `road68g_commit_determinism`
(`nodes::sim::core::populated_city::envelope`) commits a T branch on the 2,000-resident city,
which defers its lots in every run (`ROAD68G_COMMIT … lots_deferred=true`, new), and runs six game
hours: against the eight-worker reference, one, three and four workers (four added in this part),
a rebuild delayed 400 ms at one and at eight workers, and a save inside the hold give identical
hourly fingerprints (6) and saved tables (72), agent records among them: reference digest
`b41ab4b79632ebc4` on F1, F2 and C. This is the acceptance's third row.

**Economy performance contract** ([`PERF-01`](economy.md#performance-contract-perf-01perf-05)).
Parts 7-9 changed the tick path: the parcel store is shared copy-on-write (`SharedParcelStore`:
a read goes through an `Arc`, a write through `Arc::make_mut`), every frame checks for a pending
lot plan (`O(1)`), and the allocator keeps its explicit cell sites as a list. Session 1 ran the
contract C F F C on cities C generated before it (the fixture revision is the same in both: the
fixture sources are equal), `tools/perf_contract.py --binary "$TEST_BINARY"`, eight workers:

| Hour mean | C1 | F1 | F1 | C2 |
| --- | --- | --- | --- | --- |
| baseline 10k | 0.866 ms | 0.836 ms | 0.837 ms | 0.854 ms |
| baseline 30k | 4.302 ms | 4.244 ms | 4.166 ms | 4.237 ms |
| fiscal 10k | 6.090 ms | 6.242 ms | 6.162 ms | 5.975 ms |
| fiscal 30k | 26.007 ms | 26.433 ms | 26.839 ms | 25.932 ms |
| runner time | 71.3 s | 71.4 s | 71.2 s | 71.4 s |

Work counts are identical in all four runs, the households end at 5,311 and 15,180 in each, no
time limit fails, and the same seven work-count limits fail in C and F (question 20). The baseline
means lie inside C's spread. The fiscal means are 0.07-0.27 ms (1.2-4.5 %) above C's two runs at 10k
and 0.43-0.91 ms (1.6-3.5 %) at 30k, in the fiscal day's planning (473.8 and 478.2 against 461.9
and 462.8 ms a day at 30k) and census; the isolated phases do not differ. Not attributed: by
[question 36](#road-68-open-questions)'s rule it is layout only once a profile shows identical
work, and none was taken; the candidate is the parcel store's copy-on-write indirection on the
fiscal day's parcel writes. Carried to H (item 15). Part 10 changes only the commit, so session 2
did not repeat the contract.

**Decisions** ([open questions 58-60](#road-68-open-questions), the user, 2026-10-28; recorded
here, validated at `ROAD-68-H`): 58 (b), the next tick moves one frame past a commit that
deferred its lots; 59 (b), the route index by node for the commit's agent passes, in its own
round with `PERF-07` after this gate; 60 (a), the graph copy kept, (b) a persistent graph store
as its own track if the copy shows in a frame. Nothing is built for 59 or 60 here. The superseded
routing rebuild ([carried to H](#road-68-h--acceptance-and-cleanup) item 10, `terrain.md` T57
(d)): no sample of either session runs a commit beside a superseded rebuild (each sample's
rebuild is adopted before the next commit), so neither session shows that contention and nothing
is built for it here; it stays carried.

**Checks** (fresh). The debug `cargo test --lib` on `8bfd9ebf` and on `10477798`: 1,842 passed,
0 failed, 177 ignored (`TERRAIN-07-G`'s 1,837 / 0 / 176 plus this gate's tests and its locality
benchmark). `./run.sh --test` with `DISPLAY` and `WAYLAND_DISPLAY` unset on `10477798`: exit 0,
the library as above, `tests/growth_scenario.rs` 4 passed, every Godot script passes (the graded
tile renderer among them) with no `SCRIPT ERROR`; skipped as before: the shader global probe
(headless renderer) and the rendered vegetation tests (Forward+ display). `cargo clippy
--all-targets` on `5e31ad31` and on `200569e9`: 59 warnings, `TERRAIN-07-G`'s set exactly; on
`10477798` it found one more, the kept tiles' job field never read outside tests, which
`200569e9` allows as dead there as the crate does for such fields (an attribute: no code
changes, so the sessions' F2 stands for it). No compiler warning in any build (missing docs 0).
`git grep 'ignore = "ROAD-68-G'` returns nothing, and no `ROAD-68-*` ignore on a disabled test
remains (the `release ROAD-68-G …` and `ROAD-68-K22 …` ignores mark release benchmarks).
`roadbed/` is untouched: 9,239 production lines on `road` and on the tip by part 6's counter
(every non-test file, `#[cfg(test)]` items, `tests/` and `fixtures.rs` excluded; gate E's 9,617
was another counter). The round's production change is net +20 lines (`graded_stage.rs` +20;
`road_commit.rs` 0: the rebase fix removed the terrain state call and part 10 added the kept
tiles' start), plus 7 test lines and the locality fixture's reset; the whole gate, parts 1-10
on `road`, changes 37 files under `rust/src`, 1,813 lines added and 195 removed.

### `ROAD-68-H` — acceptance and cleanup

1. The [rendered runs](#targets), with the simulation paused and active, and the populated
   locality check.
2. Remove the remaining dead paths and every `ROAD-68` ignore. Update references to legacy types
   in [`terrain.md`](terrain.md) and [`earthworks.md`](earthworks.md). Record final line counts.

**Carried to H for final verification (the user, 2026-10-27).** Results too close to call at the
gate that measured them: they do not block that gate and are neither relaxed nor accepted; H
measures and judges each against its target. A gate landing later adds its own here.

1. **Accepted by the user for `ROAD-68` (2026-10-29, round 3: round 2's one install at 1.009 ms
   against the 1.0 ms share, inside the 2.0 ms stage).** Godot terrain install per pose
   ([`terrain.md`](terrain.md#budgets), the 1.0 ms share proposed within this spec's 2.0 ms Godot
   stage): 2 of 594 frames over 1.0 ms, 1.55 and 1.39 ms
   (`kuopio_12` attempts 34 and 36, four cell sets starting to draw; binding their materials is
   about 65 % of a flip), within the 2.0 ms stage. The final probe: `TERRAIN-07-E` round 7,
   `windowed-20261027T185026` ([record](terrain.md#terrain-07-e-record)).
2. Whole drag frames of 14-20 ms in that windowed probe (worst 20.23 ms, `kuopio_10` attempt 25)
   while Godot's terrain install took under 1.6 ms, with no screenshots taken: H attributes them
   against the [Moving target](#targets) (16.7 ms).
3. Streaming installs at 1.04-1.05 ms against their 1.0 ms a frame (`TERRAIN-07-D` round 11,
   [record](terrain.md#terrain-07-d-record)): named for `TERRAIN-07-G`'s acceptance, which keeps
   it; listed here for reference.
4. [`terrain.md`](terrain.md#terrain-07-open-questions) T42 (a), the preview's tiles at the
   renderer's distance rule without its hysteresis, pending the user's review: H checks it
   visually on the display.
5. T37's final matched 100k culled gap: 1.168 ms shared-index tip against 0.846 ms control,
   +0.322 ms against the unchanged +0.42 ms bar; every one of 1,101 products matches. Round 11's
   0.682 ms index-off value gathered no terrain geometry and remains invalid. T46 (b), keeping the
   LOD (exact everywhere is 2.145 ms), was agreed by the user on 2026-10-28; H judges this gap.
6. T45's far contacts keep at least the intended 0.020 m clearance within floating-point
   representation: fresh minima 0.019999988377 m at 10k and 0.019999999553 m at 100k.
7. T44 keeps the shared-owner index (T44 and T47 (a), agreed by the user on 2026-10-28): final
   matched
   culled means are 0.311/0.811 ms shared/no-index at 10k and 1.168/1.443 ms at 100k, while shared
   retains 0.436/4.462 MB; `PERF-07` snapshots are 1.227/1.569 ms. At H compare the copied index's
   108 MB/1.10 GB against shared as the user requested, and retain the corrected local mutation
   cost: the fresh 100k commit breakdown is 1.989/0.567/0.483 ms (mean 1.013, max 1.989); its
   existing path does not execute a user undo, while the small mutation fixture's undo is 0.074 ms.
8. Measure the release game's peak retained memory before and after `TERRAIN-07-G`, separating
   load/setup peak from the settled world. `TERRAIN-07-G` round 3 (fresh, RSS by phase, both
   graded, the saves loaded and proved; round 2's comparison loaded no save and is withdrawn;
   before three runs on road's library, after the tip `d7a3c338`): 100k city settled maximum
   5,734,064-5,783,156 -> 5,727,764 KiB, load peak 5,322,348-5,392,628 -> 5,339,940 KiB, setup
   1,302,404-1,337,204 -> 1,324,724 KiB; Kuopio settled maximum 1,911,036-1,924,596 -> 1,897,116
   KiB, load 1,533,508-1,542,408 -> 1,522,320, setup 1,332,520-1,343,832 -> 1,328,148 KiB. No
   increase. Method and the round 2 water reflection attribution in `terrain.md`'s Gate G record.
9. Attribute `PERF-09` separately from terrain query work: the road control already misses the
   frame guards. H records matched control/terrain snapshot and frame shares and does not charge
   that inherited regression to Gate F or relax it.
10. Stop a superseded routing rebuild (`ROAD-68-G10`, or H): a commit's rebuild runs to its end
    beside the undo's that supersedes it, and the frame's pool then waits for CPUs and allocator
    locks; on `road` too ([`terrain.md`](terrain.md#terrain-07-open-questions) T57 (d), carried by
    the user on 2026-10-29 with the populated drag's 3.79 ms first pose accepted for
    `TERRAIN-07-G`; the allocator half is `PERF-10`).
11. [Question 58](#road-68-open-questions) (b), the next tick a frame past a commit that
    deferred its lots: agreed by the user, 2026-10-28; validated at `ROAD-68-H`. H checks nothing
    beyond the record.
12. [Question 59](#road-68-open-questions) (b), the route index by node for the commit's agent
    passes: agreed by the user, 2026-10-28; validated at `ROAD-68-H`. H checks the passes' cost
    against click → committed at 100k, then runs that round with `PERF-07` against a same-session
    control.
13. [Question 60](#road-68-open-questions) (a), the commit's graph copy kept: agreed by the user,
    2026-10-28; validated at `ROAD-68-H`. H checks whether the copy shows in a frame; if it does,
    (b), a persistent graph store, is its own measured track.
14. The commit's locality (`ROAD-68-G` part 10's sessions, [record](#commit-stages-part-10-the-gate-road-68-g-2026-10-29)):
    products bitwise from 0 to 100,000 remote sites, but the first commit gains 1.6-3.1 ms at
    100,000 (the terrain query index update and the graded stage, `TERRAIN-07`'s; the topology
    adoption +0.4 ms, in the control too; the graph copy and agent passes of questions 60 and 59),
    and one second sample took 16.73 ms there (its terrain query index update 3.46 ms against
    1.81). H's populated locality check judges it.
15. The economy contract's fiscal hour means on `ROAD-68-G` part 10's first session: 1.2-4.5 %
    (10k) and 1.6-3.5 % (30k) above the same-session control, with identical work counts and no
    limit crossed, in the fiscal day's planning and census; not profiled (question 36's rule).
    The candidate is the parcel store's copy-on-write indirection (`SharedParcelStore`, part 7). H
    profiles it against a same-session control and attributes it.

#### Round 1: cleanup and the headless rows (`ROAD-68-H`, 2026-10-29)

Branch `road68-H` from `road` `8b3ce801`. Not the gate: the rendered rows run on the user's
display, and question 61 is open.

**Dead paths.** Method (fresh): the crate's top modules made private for one `cargo check --lib`,
so rustc reports every item no non-test code reaches, and the same check at `27797b92` (`road`
just before gate D); the items the later check adds are what gates D-G and `TERRAIN-07-G` left
behind (`$ARTIFACTS/road68/h1/new-dead.txt`, 14). All but one are reached by tests (accessors the
tests read, the populated fixtures). The same comparison for Godot-facing `#[func]` methods
without a GDScript caller finds none new, and for GDScript symbols without a reference finds
`render_debug.gd`'s terrain clip diagnostics. Following callers finds one more. Removed:

- `RoadbedDelta::base_generation`: no caller;
- `RoadTerrainInputs::global_generation` and `stroke_active`: read only to bump the inputs'
  revision, for the refined terrain sources that `TERRAIN-07-E` retired; nothing consumed them,
  so the revision now moves only when a patch's sites change;
- `render_debug.gd`'s `clip_stats`, `polygon_area` and `bounds_label`, whose terrain clip
  callers `TERRAIN-07-G` deleted;
- comments that still named the refined terrain, the visual buffer, the CDT and the road-locked
  terrain state, in Rust and in the Godot tests.

`git grep 'ignore = "ROAD-68'` under `rust/` returns nothing. The ignores that name a gate or a
tracked item are release benchmarks and manual diagnostics, none a disabled test
(`$ARTIFACTS/road68/h1/ignores.txt`: `SIM-02-A/I/K`, `PERF-01/02/06/07`, `ROAD-68-G` (the commit
breakdown and locality), `ROAD-68-K22`, `TERRAIN-07-E` (the populated drags)).

**`./run.sh --test` runs every rendered test.** The terrain shader tests, the debug-view shader
globals (`debug_visual_modes_test.gd`, now rendered only: its headless run in the bridge list
skipped the probe), the vegetation level and wind tests and the plot lighting tests go through
one helper, `run_rendered_godot_test`: gamescope's headless backend with `DISPLAY` and
`WAYLAND_DISPLAY` unset, `$DISPLAY` only without gamescope, and a failure when neither renders.
`xvfb-run` is gone from the suite.

**Legacy references** in [`terrain.md`](terrain.md) (the `TERRAIN-07` status, the deleted types
named as live, the runtime contract's refined and visual-buffer rules) and
[`earthworks.md`](earthworks.md) (its current-runtime sections, the roads-first status and the
building-site target still described the stitched CDT cut as live) now describe the graded
terrain; the stitched-CDT text moved to the
[archive](archive/terrain_legacy_surface_2026-10-25.md#moved-at-road-68-h-2026-10-29).

**Line counts** (production, part 6's counter): `roadbed/` 9,239 -> 9,234; `terrain/graded/`
10,169 (unchanged); `terrain/` 12,393 -> 12,391.

**The landing session** (fresh, 2026-10-29 12:03:43-12:15:40, one hold of both locks,
`taskset -c 0,2,4,6,8,10,12,14`, `METRUM_DEBUG=0`, no restic, compiler, cargo or Godot process at
the start or after any run; release library tests). C = `road` `8b3ce801` before the cleanup
(`$TEST_BINARY` sha256 `01b89e93…`), F = `3e662826`, the cleanup's code (sha256 `a880ce56…`; the
later commits change GDScript and docs), E = `road` `a0cfe039` with `ROAD-68-G` part 10's
locality harness (sha256 `aec1c293…`, the economy contract's control from before `ROAD-68-G`).
Each binary's 10k, 30k and 100k cities prepared first; every populated tier `loaded`. Runs: the
preview stages C F F C (four workers); the commit breakdown C F F C on the game's path
(`METRUM_COMMIT_PREVIEW`, eight workers, tiers 0 to 100k) and F F a frame later
(`METRUM_COMMIT_NEXT_TICK=frame`); the populated graded drags C F F C (four workers, 0 to 100k);
the commit locality C F F C; the economy contract E F F E. Logs `$ARTIFACTS/road68/h1/session/`,
`perf/`, tables `breakdown-table.txt`, `breakdown-summary.txt`, `locality-table.txt`,
`drags-table.txt`, `perf-summary.txt`.

- **Commit** (click → committed generation installed and acknowledged, every first sample):

  | Tier | C first | F first (at once) | F first (a frame later) | F later samples, max |
  | --- | --- | --- | --- | --- |
  | 0 | 7.14, 5.98 | 6.16, 6.47 | 6.36, 6.68 | 7.56 (all tiers) |
  | 10k | 9.76, 10.55 | 9.23, 8.99 | 10.75, 9.51 | |
  | 30k | 10.20, 11.61 | 11.16, 10.09 | 10.02, 11.14 | |
  | 100k | 15.26, 11.38 | 14.61, 13.73 | 13.73, 12.91 | |

  Every sample of every run at every tier fits 16.7 ms (C's later samples at most 7.24). At
  100k the commit's agent passes take 0.24-0.27 ms (question 59, item 12: under 2 % of the
  13.7-14.6 ms) and the tool snapshot with its graph copy 0.49-0.58 ms (question 60, item 13);
  the routing adoption comes 220-264 ms after the click, the adoption frame waiting 13-36 ms for
  it; a frame later the lots' adoption frame takes 0.36-0.52 ms at 100k.
- **Bitwise.** All six breakdown runs print the same `COMMIT_PRODUCTS`, C's and `ROAD-68-G`
  part 10's F2 among them, so the cleanup changed no product; all sixteen locality product sets
  are equal (C, F, every tier) and equal part 10's.
- **Preview stages** (unchanged): payload p50 0.387-0.406 ms, graded p50 1.37-1.69 / p95
  1.82-2.12 ms, totals p50 2.36-2.78 ms on the sloped T and branch, C and F alike.
- **Populated drags** (`TERRAIN-07-E`'s frame paths): pose frames at most 2.57-2.91 ms, none of
  502 over 3.0 ms; converged commits at most 1.66 ms, undos at most 1.98 ms; the fast click's
  accepted exception at most 8.38 ms (T27 (f)).
- **Superseded routing rebuilds (item 10).** At 100k every drags run has three or four pairs of
  routing rebuilds running at once (the fast click's rebuild still running when its undo's
  starts, up to three together); the 0-30k rebuilds never overlap. The pose frames timed inside
  those windows take 0.70-1.47 ms, none near its 3.0 ms stage, and no commit, undo or pose frame
  of the session misses its stage. No sample shows the contention, so nothing is built; the
  overlap itself stays measured (`drags-table.txt`).
- **Locality (item 14).** Products bitwise from 0 to 100,000 remote sites (above). First sample,
  ms (C runs; F runs): 0 sites 15.74, 13.91; 16.76, 16.14 · 1,000 13.05, 12.53; 12.80, 12.04 ·
  10,000 15.22, 15.59; 16.89, 12.92 · 100,000 18.76, 18.58; 15.71, 17.13; second and third
  samples 9.6-14.8 ms. Not flat. F's parts from 0 to 100,000 sites: the terrain query index update
  +0.45-0.57 ms and the graded stage +0.1-0.4 ms (`TERRAIN-07-F`'s T44 and `TERRAIN-07`'s), the
  rest of the roadbed stage +0.3; the topology adoption +0.42-0.45 (C the same; not questions 59
  or 60, unattributed); the tool snapshot's graph copy +0.56-0.57 (question 60); the agent passes
  +0.03-0.08 (question 59); the lanes +0.1: about 1.9-2.5 ms in all, inside a render publication
  that varies by 1-3 ms between runs (5.7-8.9 ms). A miss for the user: not one of the separate
  tracks.
- **Economy contract (item 15)**, E F F E, `tools/perf_contract.py`, eight workers: work counts
  identical in all four runs, households 5,311 and 15,180, no time limit fails, and the same seven
  work-count limits fail in E and F (question 20).

  | Hour mean | E1 | F | F | E2 |
  | --- | --- | --- | --- | --- |
  | baseline 10k | 0.830 ms | 0.836 ms | 0.832 ms | 0.838 ms |
  | baseline 30k | 4.174 ms | 4.099 ms | 4.166 ms | 4.101 ms |
  | fiscal 10k | 6.025 ms | 6.149 ms | 6.232 ms | 6.081 ms |
  | fiscal 30k | 25.758 ms | 26.750 ms | 26.917 ms | 25.949 ms |

  The baseline means lie inside E's spread. The fiscal means are 1.1-3.4 % (10k) and 3.1-4.5 %
  (30k) above E, as part 10's session 1 measured; at 30k the fiscal day's planning takes
  483.7/484.4 ms against 458.9/461.2 and its census 353.8/350.4 against 345.4/341.5. The
  isolated phases do not differ (planning at 30k E 4.84/4.39, F 4.57/4.37 ms; census E
  23.31/22.09, F 22.93/23.45).

  **Profile (item 15, fresh, not timing).** `perf record` of `perf_contract_fiscal_day` at 30k,
  E and F alternating (E F E F, eight workers, P-cores; `-D 5000` skips the 4.6 s city load, so
  the samples fall inside the fiscal day; `$ARTIFACTS/road68/h1/profile/`): 55K and 56K samples, cycles
  43.85 G against 44.15 G (+0.7 %). The same symbols lead in the same order, every one of the top
  30 within 0.37 percentage points of its share in E; no copy-on-write symbol (`Arc::make_mut`,
  a parcel store clone or drop) appears, and the parcel store's own symbols (the parcel
  iterator, `visit_in_bounds`, the edge projection) hold 0.07-0.19 % each in both. The work is
  identical by the counts and by the profile, so by [question 36](#road-68-open-questions)'s
  rule the fiscal rise is layout, not `SharedParcelStore`'s indirection on a hot path: judged
  within the contract (no limit crossed, work counts equal), recorded, not relaxed.

**The carried items in round 1** (fresh unless marked):

- 1 and 2 (the Godot terrain install per pose, the whole drag frames): the drag probe on the
  user's display (13:05:41-13:07:38, the tip's release library, 1920x1080, vsync,
  `drag-windowed-20261029T130543/summary.txt`; its two checks stale since `TERRAIN-07-G`, the
  catch-up share and the pages and variants made on demand, fixed first, `d4172ccb`). 38 of 39
  strokes: one more probe check, `pool_exhausted` on `kuopio_11` attempt 32 (a full page during a
  preview, which the renderer handles by T27 (d)'s lag since T54), stopped that case's last
  stroke; 0 holes in 13 loads, 0 instance-uniform overflows, no `SCRIPT ERROR`. Item 1: pose
  installs at most 0.985 ms, **none of 580 over the 1.0 ms share** (was 2 of 594), catch-up at
  most 0.61, commit 0.71; the fast click's install at most 1.24 (1 over 1.0) and the undo's 1.25
  ms (6 over), within the 2.0 ms Godot stage. Item 2: the pose frames' intervals p50 11.7 / p95
  19.1 ms, 17 of 608 over 20 ms, the commit frames' p95 28.0 / max 33.7 ms, with the terrain
  install at most 1.25 ms and the GPU at most 4.2 ms in every frame: not the terrain install.
  The three largest (183, 140 and 85 ms) are `kuopio_01`'s first strokes after the first load
  (first use); the rest are not attributed in this round (a frame profile on the display is
  owed), a miss for the user. 3: `TERRAIN-07-G`'s, kept there.
- 4 (T42 (a)): captures for the user's visual check (`$ARTIFACTS/road68/h1/references/t42/`,
  index): the drag probe under gamescope with the camera at 70 m and 150 m, past the `GL0` and
  `GL1` level ends inside their hysteresis, each stroke's converged preview and its settled
  commit (8 strokes each); not judged by the agent.
- **Reference renders** (fresh, `screenshots-C.sh`, the tip's release library, gamescope):
  every fixture, 197 images in close, plan, oblique and distance (200 m to 3 km) views, equal to
  `TERRAIN-07-G` round 10's final renders by that round's comparison (no pixel of any image
  differs by more than 40 levels), 0 pinholes in all 33 plan views, 0 failures
  (`$ARTIFACTS/road68/h1/references/index.txt`).

**Gate checks** (fresh). `cargo clippy --all-targets` on `5e02102d` (the Rust of every later
commit): 59 warnings, `ROAD-68-G` part 10's set exactly. Missing docs 0 (`cargo doc`; its eight
link warnings are `TERRAIN-07-G`'s). `./run.sh --test` with `DISPLAY` and `WAYLAND_DISPLAY` unset on
`82cfe7a3`: exit 0; the debug library suite 1,842 passed, 0 failed, 177 ignored;
`growth_scenario` 4 passed; every Godot suite passes and nothing is skipped: the terrain overlay,
graded cells and debug-view shader tests, the vegetation level match and wind gate tests and both
plot lighting tests ran in gamescope's headless backend, no `SCRIPT ERROR`. Later commits change
the drag probe (not in the suite) and docs. Bitwise (release F, not timed):
`road68g_commit_determinism` differences null at 1, 3 and 4 workers, a rebuild delayed at 1 and
8, and a save in the hold, digest `b41ab4b79632ebc4` (part 10's); the three graded preview =
commit = undo = load tests pass.
- 5, 6 and 9 (T37's culled gap, T45's clearance, `PERF-09` apart from terrain): not re-measured.
  They need the `SIM-02-K` prepared-snapshot run (`culled_snapshot_matches_full_pose_on_a_prepared_city`,
  which prints the culled and full means and `far_min_clearance_m`) and the `PERF-07` day at 100k,
  on a 100k `PERF-07` city of the current fixture revision (none exists: the saved one predates
  `ROAD-68-G`'s fixture change) against `TERRAIN-07-F`'s control. The latest values stand as
  historical (`TERRAIN-07-F`'s final matched session: +0.322 ms culled at 100k against the +0.42
  ms bar; 0.019999988377 / 0.019999999553 m), and the graded tiles changed after them (T48, T49),
  so round 2 measures them.
- 7 (T44, the copied index against the shared one): the copied index's 108 MB / 1.10 GB at
  10k / 100k against the shared 0.436 / 4.462 MB (historical, `TERRAIN-07-F`); the shared index's
  local update costs the 100k commit 2.29-2.34 ms of its 13.7-14.6 ms first sample in this
  session (C the same) and gains 0.45-0.57 ms with 100,000 remote sites (item 14). Judged: the
  shared index keeps the commit row inside 16.7 ms at 1/250 of the copy's memory.
- 8: superseded by `TERRAIN-07-G`'s T59 and [`TERRAIN-09`](roadmap.md) ([`terrain.md`](terrain.md#terrain-07-open-questions)).
- 10: not built; no sample shows the contention (above).
- 11 (question 58 (b)): nothing beyond the record; a frame later the lots' adoption frame takes
  0.36-0.52 ms at 100k.
- 12 (question 59 (b)): the commit's agent passes cost 0.24-0.27 ms at 100k, under 2 % of click
  → committed, and +0.03-0.08 ms at 100,000 remote sites; the route index round with `PERF-07`
  follows `ROAD-68-H`, as agreed.
- 13 (question 60 (a)): the graph copy shows in the commit's frame as the tool snapshot's
  0.49-0.58 ms at 100k (the serial clone of that graph 2.83-2.94 ms) and +0.56 ms at 100,000
  remote sites, and no frame misses because of it. Whether 0.5 ms "shows" enough to open (b),
  the persistent graph store, is the user's call; the measurement does not ask for it.
- 14: products bitwise; the first commit not flat (+1.9-2.5 ms in its parts at 100,000 remote
  sites), a miss for the user (above).
- 15: layout by the profile (above).

**`./run.sh --test` finds a stale test.** Run under gamescope for the first time,
`vegetation_level_match_test.gd` failed 25 of 108 poses (the distant level at 0.863-0.897 of the
near level at 900 m): since the weather and sky landed (`46aec2ca`) the weather globals default to
a 45 % cloud deck, and the test, which fixes its own light, did not fix the weather. With
`614dcaa0`'s vegetation shaders it passes, and under a clear sky with today's (0.896-1.077), so the
test now clears the deck ([`terrain.md`](terrain.md#the-level-match-test-clears-the-cloud-deck-2026-10-29)).

**Rendered rows** (`$ARTIFACTS/road68/h1/display-ready`; fresh, the user's display, taken by
this agent on the coordinator's word, 2026-10-29 13:02-13:04; release library of the tip, sha256
`cca536b7…`, Rust = `3e662826`'s; 1920x1080 windowed, vsync, 60 FPS cap, four workers, P-cores,
both locks, under `display-awake.sh`, a throwaway profile; two runs, 100 samples after 8
warm-ups; `preview-windowed-20261029T130204/summary.txt`). The preview matrix, the simulation
paused, ms:

| Fixture | Stationary p50 / p95 / max | Stop → exact | Moving displayed age p50 / p95 / max | Sampled poses exact within 16.7 ms | Sample → exact frame p50 / p95 / max |
| --- | --- | --- | --- | --- | --- |
| `sloped_t` | 13.29 / 20.29 / 26.53; 12.56 / 21.31 / 32.40 | 14.18; 13.37 | 13.61 / 21.81 / 156.70; 13.72 / 22.29 / 65.88 | 251 of 365; 260 of 377 | 13.5 / 21.7 / 156.7; 13.6 / 22.5 / 65.9 |
| `sloped_branch` | 14.17 / 20.70 / 25.54; 13.31 / 22.04 / 25.38 | 14.94; 7.55 | 13.60 / 20.84 / 25.28; 14.92 / 22.96 / 28.30 | 285 of 385; 236 of 362 | 13.5 / 20.8 / 25.3; 14.5 / 22.6 / 26.6 |
| `flat_cross` | 13.39 / 24.77 / 29.15; 15.05 / 25.86 / 29.78 | 24.76; 11.60 | 13.95 / 24.35 / 29.80; 14.34 / 23.51 / 66.78 | 245 of 373; 233 of 367 | 13.9 / 24.4 / 29.8; 14.1 / 23.5 / 30.2 |
| `sloped_cross` | 14.45 / 21.66 / 31.15; 14.79 / 22.29 / 30.74 | 12.30; 21.72 | 14.38 / 24.35 / 34.73; 15.27 / 26.56 / 36.23 | 237 of 369; 221 of 357 | 14.4 / 25.1 / 34.7; 15.1 / 26.8 / 36.2 |
| `dense` | 11.65 / 21.76 / 23.59; 11.15 / 20.96 / 27.57 | 22.65; 6.70 | 12.48 / 23.04 / 27.77; 13.69 / 25.50 / 30.77 | 258 of 357; 241 of 342 | 12.0 / 23.3 / 27.8; 13.1 / 25.5 / 30.8 |

(Stationary p99 needs ten samples above it; 100 samples give none.) Against `ROAD-67-A`'s legacy
baseline (stationary p50 57-92 ms, moving p50 53-120 ms) every row is four to eight times lower,
but **the Moving and Stationary targets miss**: 62-74 % of sampled poses render their exact
result within 16.7 ms, the rest one frame later (every miss is `late_exact`, none stale or
superseded), and the stationary p95 is 20-26 ms. Attribution: the frames themselves are cheap
(the viewport's CPU time p50 1.1-1.9 / p95 2.3-3.4 ms, GPU 1.6-2.0 ms in every case), but the
frame intervals run p50 16.1-17.3 / p95 24-32 ms: about half the frames are recorded over 16.7
ms and a frame often takes two refresh periods, with the scripted input arriving 6-10 ms late at
the median. The time goes to presentation pacing (vsync with the 60 FPS cap on KDE's Xwayland),
not to the roadbed, the terrain or the frame's own work; that is no `ROAD-68` or `TERRAIN-07` cost
the round could change, but it is not one of the separate tracks either, so it is a miss for the
user. The Moving row with the simulation active has no rendered harness
([question 61](#road-68-open-questions)).

**Merge risks.** Every terrain and road acceptance ran on Linux with the RX 7900 XTX under Vulkan
(and the Compatibility renderer for the shader tests); macOS is unvalidated (`./run.sh --test`
does not run the rendered tests there) and has [`TERRAIN-10`](roadmap.md) open: graded patches
drawing nothing on another developer's Mac. The commit's locality at 100,000 remote sites and
question 61 are open for the user.

#### Round 2: the pacing check, the active rendered row, the carried items (`ROAD-68-H`, 2026-10-29)

Branch `road68-H` on round 1's `9e7782cd`. Not the gate: the rendered preview rows and the
populated row miss (below) and need the user's decisions.

**The user's decisions (2026-10-29, about 13:20).**

- **Locality** (round 1: products bitwise, the first commit +1.9-2.5 ms in its parts at 100,000
  remote sites): accepted as measured for `ROAD-68`. Later tracks in the roadmap:
  [question 60](#road-68-open-questions) (b), the persistent graph store (`ROAD-71`), and the
  terrain query index update's cost on the commit (`TERRAIN-11`). The topology adoption's +0.44 ms
  is attributed below (`ROAD-72`).
- **The rendered preview's pacing:** confirmed in this round before the user decides (below,
  [question 62](#road-68-open-questions)).
- **[Question 61](#road-68-open-questions) (a):** a populated case in the rendered preview matrix
  with the simulation active, run on the display, every miss attributed, the routing adoption
  frame's agent tick at 100k included (below; the row's misses are [question 63](#road-68-open-questions)).
- **After this round's results (about 14:55):** [question 62](#road-68-open-questions) (d) and
  (b): the [Method](#method) drops the 60 FPS cap (vsync, no cap, Wayland reported beside
  Xwayland) and the pose's main-thread work is cut to fit at the idle clock;
  [question 63](#road-68-open-questions) (b): sub-chunk preview payloads. Targets unchanged; the
  next round builds both.

**Fresh and historical.** Everything in this record is fresh unless marked: the user's display
(DP-2, 3840x2160 at 120 Hz; a 1920x1080 window, vsync) on 2026-10-29 13:25-13:53, round 1's
release library of the tip (sha256 `cca536b7…`; Rust = `3e662826`'s, unchanged in this round),
four workers, P-cores, `.run.lock` with the exclusive timing lock, under `display-awake.sh`, a
throwaway Godot profile (`$ARTIFACTS/road68/h2/display-ready`). Round 1's numbers are historical
here.

**1. The pacing check.** The five preview rows of round 1 (100 samples after 8 warm-ups) in three
variants, interleaved C N W W N C in one session (13:25:59-13:32:02,
`pacing-windowed-20261029T132601/`): C the Method's settings (60 FPS cap, Godot's x11 driver
through Xwayland), the same-session control; N vsync with no FPS cap (the harness's
`METRUM_GAMEPLAY_BENCHMARK_MAX_FPS=0`, which clamped to 1 before this round); W as N with Godot's
native Wayland driver (`--display-driver wayland`). The display refreshes at 120 Hz, not 60, so N
and W pace at 8.3 ms. Sampled poses exact within 16.7 ms: **C 2,614 of 3,712 (70.4 %), N 3,335 of
3,916 (85.2 %), W 3,579 of 3,958 (90.4 %)**; every miss is `late_exact` (none stale or
superseded). Two runs a cell, ms:

| Fixture | Variant | Frame interval p50 / p95 | Sampled poses exact within 16.7 ms | Stationary p50 / p95 / max | Moving displayed age p50 / p95 / max |
| --- | --- | --- | --- | --- | --- |
| `sloped_t` | C | 16.9 / 26.2; 16.7 / 25.7 | 271 of 377; 261 of 374 | 12.7 / 21.5 / 24.9; 13.6 / 20.8 / 24.8 | 13.5 / 22.7 / 147.0; 14.1 / 23.4 / 64.3 |
| `sloped_t` | N | 8.7 / 18.8; 8.6 / 18.6 | 339 of 389; 329 of 389 | 8.2 / 16.6 / 21.8; 8.3 / 16.9 / 25.0 | 13.7 / 23.0 / 68.0; 13.8 / 23.4 / 76.2 |
| `sloped_t` | W | 8.4 / 16.1; 8.5 / 17.1 | 366 of 396; 348 of 393 | 8.3 / 13.9 / 17.6; 8.3 / 11.1 / 15.4 | 13.8 / 23.9 / 61.8; 13.7 / 23.4 / 74.7 |
| `sloped_branch` | C | 16.8 / 27.2; 17.0 / 24.5 | 244 of 371; 280 of 381 | 13.8 / 22.3 / 26.7; 13.2 / 20.7 / 22.9 | 14.6 / 22.9 / 27.6; 13.6 / 21.8 / 26.3 |
| `sloped_branch` | N | 8.6 / 19.0; 8.6 / 18.7 | 330 of 388; 338 of 394 | 8.3 / 15.8 / 22.8; 8.0 / 16.5 / 29.4 | 13.9 / 23.5 / 33.9; 13.5 / 22.6 / 29.9 |
| `sloped_branch` | W | 8.5 / 17.6; 8.4 / 16.6 | 347 of 397; 359 of 394 | 9.2 / 17.7 / 22.8; 8.2 / 12.5 / 17.2 | 13.9 / 22.9 / 27.6; 14.1 / 21.4 / 30.5 |
| `flat_cross` | C | 17.0 / 29.0; 17.0 / 27.4 | 236 of 358; 237 of 364 | 14.9 / 24.5 / 31.4; 12.8 / 19.0 / 21.0 | 14.3 / 25.5 / 33.0; 14.3 / 24.9 / 34.6 |
| `flat_cross` | N | 8.4 / 19.5; 8.6 / 19.7 | 315 of 390; 325 of 387 | 8.2 / 11.9 / 23.5; 8.2 / 15.6 / 22.7 | 13.7 / 23.0 / 65.2; 14.2 / 21.4 / 67.5 |
| `flat_cross` | W | 8.4 / 16.7; 8.6 / 17.1 | 362 of 396; 350 of 394 | 8.5 / 17.8 / 22.3; 9.3 / 16.1 / 25.6 | 14.2 / 19.1 / 66.2; 13.7 / 21.3 / 67.8 |
| `sloped_cross` | C | 16.8 / 27.3; 17.0 / 26.9 | 254 of 373; 224 of 362 | 15.2 / 23.6 / 29.3; 13.4 / 22.4 / 25.9 | 13.6 / 24.5 / 36.1; 14.9 / 25.2 / 31.2 |
| `sloped_cross` | N | 8.5 / 18.9; 9.2 / 20.5 | 339 of 396; 317 of 389 | 9.2 / 19.4 / 26.7; 11.1 / 21.4 / 28.1 | 14.0 / 21.0 / 25.8; 14.0 / 21.7 / 30.8 |
| `sloped_cross` | W | 8.5 / 17.5; 8.4 / 17.3 | 346 of 395; 357 of 395 | 8.4 / 12.2 / 18.4; 8.6 / 21.0 / 30.5 | 13.9 / 20.8 / 27.1; 14.3 / 19.5 / 28.9 |
| `dense` | C | 16.4 / 28.0; 16.7 / 27.2 | 312 of 382; 295 of 370 | 12.2 / 23.0 / 30.3; 11.9 / 21.6 / 25.9 | 11.9 / 21.7 / 29.4; 12.2 / 22.1 / 31.3 |
| `dense` | N | 8.5 / 18.0; 8.4 / 17.6 | 357 of 399; 346 of 395 | 8.4 / 18.1 / 26.5; 8.4 / 16.0 / 22.8 | 13.3 / 23.0 / 26.9; 13.2 / 20.2 / 29.3 |
| `dense` | W | 8.4 / 14.0; 8.4 / 16.1 | 381 of 400; 363 of 398 | 8.3 / 10.7 / 13.6; 8.3 / 11.5 / 12.5 | 13.6 / 18.2 / 24.6; 13.4 / 22.3 / 29.8 |

No variant meets the target, so the late poses were profiled. **Where a pose's time goes.** The
preview is synchronous: the pose is sampled in the road tool's `_process`, `preview_road` runs,
its chunks and tiles install, and the rest of the frame (the other nodes, the draw) follows to
`frame_post_draw`. In every variant every part of a late pose takes about twice an on-time pose's
(p50, C): the call 8.4-10.9 against 4.9-6.4 ms, the install 3.6-5.4 against 2.1-3.0, the rest
4.7-7.0 against 3.0-4.4. **The late frames' profile** (13:38:36-13:39:23,
`pacing-profile-windowed-20261029T133836/`, C and W on `sloped_t` and `sloped_branch` under
`perf record`, every Godot thread sampled every 500,000 reference cycles with its core cycles in
the same group, on `CLOCK_MONOTONIC_RAW`, Godot's clock; aligned with the poses by the main thread's
extension samples, 84-92 % of which fall inside the preview calls; attribution, not timing): per
pose, between its sample and its exact frame, the main thread executes **14.1-14.6 M cycles
whether on time or late**, the four pool workers 15.6-17.5 M; the on-time poses ran at a p50 clock
of 1.73-2.40 GHz (p95 2.7-4.0), **every late pose at 1.18-1.26 GHz** (p95 1.35-1.47) on both
threads. The work is the same; the clock is not. Godot's other threads take under 2 ms of any
window (other processes were not recorded). The CPU's power management (`intel_pstate`, the
`powersave` governor, `balance_performance`) sets the clock from the cores' recent load: with the
60 FPS cap they idle longer between frames than at 120 Hz without it, and the on-time poses' clock
falls from 2.27-2.40 GHz (W) to 1.73-1.86 GHz (C). At 1.2 GHz the main
thread's 14 M cycles alone take 11.7 ms. **Attribution: the frames' work runs at the P-cores' idle
clock** (question 27's open half: since terrain is in the preview the 16.7 ms frame is missed at
the idle clock); round 1's "presentation pacing" is corrected: the long frame intervals were the
slow frames, not vsync. Not a separate track: a miss for the user,
[question 62](#road-68-open-questions).

**2. The active rendered row ([question 61](#road-68-open-questions) (a)).** The preview matrix
gains `populated_t` and `populated_branch`, run only when named: they load a saved city
(`METRUM_GAMEPLAY_BENCHMARK_SAVE_PATH`), wait for the load flow's hand-over, run the simulation at
`METRUM_GAMEPLAY_PREVIEW_SPEED` (default 1x) and drag `sloped_t`'s and `sloped_branch`'s pointer
paths through the production road tool at the commit breakdown's first edit site
(`trunk_branch_starts`: 100 m west of the grid on the west border trunk, here (-3300, -325)),
mirrored to the trunk's north side (no setup roads), with the same schema-2 collector, the same
orthographic camera and the Method's 60 FPS cap; the road state must not change, agents and
buildings may. The city is the commit breakdown's 100k city (the populated fixture of the
[Targets](#targets); `road` `8b3ce801`'s snapshot, whose commit products equal the tip's, round 1),
on 512 m chunks as the game's worlds. A P A (A the simulation at 1x, P the same city paused,
13:39:23-13:45:16, `populated-windowed-20261029T133923/`):

| Case | Variant | Sampled poses exact within 16.7 ms | Frame interval p50 / p95 | Call / install / rest p50 | Stationary p50 / p95 / max |
| --- | --- | --- | --- | --- | --- |
| `populated_t` | A | 0 of 93; 0 of 98 | 66.0 / 99.0; 65.6 / 91.9 | 8.8 / 6.5 / 48.6; 8.7 / 6.5 / 46.2 | 63.3 / 91.5 / 191.5; 62.9 / 82.2 / 187.5 |
| `populated_t` | P | 0 of 164 | 46.0 / 59.3 | 8.1 / 6.3 / 26.6 | 55.2 / 60.6 / 62.3 |
| `populated_branch` | A | 0 of 199; 0 of 200 | 27.8 / 54.7; 28.0 / 57.1 | 9.8 / 6.6 / 10.5; 9.9 / 6.6 / 10.5 | 22.0 / 30.2 / 107.9; 21.9 / 29.7 / 100.6 |
| `populated_branch` | P | 0 of 329 | 20.3 / 21.7 | 8.9 / 6.3 / 4.5 | 20.4 / 21.4 / 25.0 |

**The row misses: 0 of 590 sampled poses within 16.7 ms with the simulation active (0 of 493
paused).** Every miss is `late_exact`. The simulation ran (3,985-4,322 pathfinds a case; agents
and buildings changed). Attribution (the active profile, 13:45:16-13:47:17, as above; the payload
by key from a gamescope run of P, 13:53:25):

- **The preview's own work in a 512 m chunk world** (`ROAD-68`'s): each pose re-emits whole road
  chunks: 11.48-11.53 MB of road chunk payload (graded tiles 28-37 KB) against 0.53-0.63 MB on
  the 128 m synthetic fixtures. The call takes 8.1-9.9 ms p50 (payload 3.0-3.5, packing 1.4-1.6,
  graded 1.8-2.2, roadbed 1.0-1.2, plan 0.5) and the install 6.3-6.6 ms, at 4.95 GHz (no idle
  clock here): about 15 ms of every pose's frame before anything else runs. The site's trunk edge
  (544 m) spans two 512 m chunks, one of them holding the grid's west edge. A miss for the user
  ([question 63](#road-68-open-questions)).
- **The rest of the frame, paused:** 4.5 ms on the branch, 26.6 ms on the T (the main thread's
  engine binary: 32 ms of 48 ms busy per active T pose; Arch's Godot is stripped, so perf cannot
  name the engine or GDScript functions). Unattributed.
- **The simulation's share** (A against P, same city and camera): the rest +6.0 ms (branch) and
  +20-22 ms (T), the call +0.6-0.9 ms; the pool's busy time in a pose window 45-75 ms (the agent
  tick shares the pool). Consistent with the agents' render update (`RENDER-28`) and the CPU-bound
  render frame (`RENDER-27`) but not shown to be them: unattributed by function, so it blocks.

**The routing adoption frame's agent tick** (a heavy run, not timing, 13:52:55-13:53:25: H1's F
binary, its saved 100k commit city, the breakdown on the game's path, eight workers, `perf record`
with LBR call stacks; `$ARTIFACTS/road68/h2/adoption/`): 125.2, 28.9 and 20.4 ms in samples 0-2
(round 1: 125.0, 29.5, 19.3; `following_frame_agent_ms` 4.8-5.0). **What runs:** sample 0's tick
is 38 % the movement pass's route scratch re-prepared on every pool worker for the commit's new
network revision (`prepare_movement_scratch` → `TripQueryScratch::prepare` →
`ConnectorTargets::prepare`, `O(lanes)` per scratch, nine scratches; a tenth of the window's
samples are page faults: the scratch's first growth after the load), 34 % replans of the agents
whose routes the commit invalidated (`plan_network_replan`, CCH `sweep_up`), the rest the ordinary
movement pass; samples 1 and 2: 42-49 % the scratch, 8-12 % replans, no page faults. **Why one
frame:** D7 holds every frame until the rebuilt hierarchy is adopted; the adoption frame's tick is
the first on the new revision, so it re-prepares every scratch and serves every replan that waited
for the new routing, and nothing spreads that work. **Track:** not question 59's route index (it
narrows the commit's three agent passes, 0.24-0.27 ms, not the next tick) and not `PERF-09` (a
day without edits): new, `PERF-11`. It holds the sim thread (and the core) for 125 ms, so agents
stop moving for about seven frames; no render frame waits for it, and the rendered row has no
commit.

**3. The carried items.**

- **The drag probe's health check** (round 1: `pool_exhausted` stopped `kuopio_11`'s last stroke).
  From the code: the preview path counted `pool_exhausted` whenever a tile found no room, both
  when its page was full and its block grows (T27 (d)'s lag since T54, drawn by a later pose) and
  when the page cannot grow (its cap or the budget). The renderer now counts the first as
  `preview_page_waits` and keeps `pool_exhausted` for the second, as the install path's `_exhaust`
  records it; the probe still fails on any `pool_exhausted` and records the waits. The 39 strokes
  (13:47:17-13:49:13, `drag-windowed-20261029T134717/`): **39 of 39, 0 failures**;
  `kuopio_11` attempt 32 had 2 page waits and no exhaustion.
- **1 (the Godot terrain install per pose):** pose installs at most 1.009 ms, **1 of 594 over the
  1.0 ms share** (`kuopio_07` attempt 19; round 1 0 of 580 at most 0.985); the fast click's at most
  1.233 ms (1 over), the undo's 1.509 ms (5 over), all within the 2.0 ms Godot stage. A miss of the
  share by 0.009 ms, for the user.
- **2 (the whole drag frames beyond first use):** pose frame intervals p50 11.7 / p95 18.8 ms, 71
  of 624 over 16.7 ms (18 over 20, max 136.7 on `kuopio_01`'s first stroke). The profile (the same
  method, 13:50:55-13:52:54, `drag-windowed-profile-20261029T135054/`; frames now carry their end
  time): unlike the synthetic rows, the Kuopio frames run at full clock (p50 4.6-5.0 GHz by kind); a pose frame over 16.7
  ms executes 76 M main-thread cycles against 44 M within it (15.5 against 9.7 ms busy), the extra
  in the engine binary (10.2 against 5.9 ms) and the drivers (+0.9 ms), the extension +0.9 ms
  (2.5 against 1.6), the GPU 2.9 against 2.5 ms; commits, fast clicks and undos alike. The long
  frames are the CPU-bound render frame on Kuopio, not the roadbed or the terrain: attributed to
  `RENDER-27` (its function-level split needs a Godot with symbols); the three largest (180, 146
  and 79 ms) are `kuopio_01`'s first use.
- **6 (T45's far-contact clearance):** the tip's 10k and 100k `PERF-07` cities of the current
  fixture revision, prepared by H1's F binary (24 s and 430 s, a heavy run,
  `$ARTIFACTS/road68/h2/clearance.log`), then the `SIM-02-K` prepared-snapshot pose test: every
  culled pose equals the full pose (3,301 comparisons at 10k, 1,101 at 100k), far contacts at
  59.67 / 2.30 far / near cars a frame (10k) and 252.89 / 29.00 (100k), the shared index 435,864 /
  4,462,256 bytes, all as `TERRAIN-07-F` round 13; **minimum far clearance 0.01999998837709427 m
  (10k) and 0.019999999552965164 m (100k)**, equal to round 13's. The 100k minimum is the `f32`
  nearest 0.02; the 10k minimum lies 1.1e-8 m under it. Judged: the intended clearance holds
  within floating-point rounding, as round 13 judged it.
- **5 and 9** (T37's culled gap, `PERF-09` apart from terrain): one exclusive session,
  14:29:15-15:10:04 (both locks, eight workers, P-cores, `METRUM_DEBUG=0`, no restic, compiler,
  cargo or Godot process; `$ARTIFACTS/road68/h2/session/`). C = `TERRAIN-07-F`'s control
  `c64786e9`, rebuilt as a release test binary in its own worktree (sha256 `4f0f706c…`), on its
  own `PERF-07` cities (the pre-`ROAD-68-G` fixture revision, checked current for it); F = the
  tip's Rust (H1's F, `a880ce56…`) on the cities above. Both days end on the same
  `GrowthDayRecord` (day 13, 54,405 households, 76,080 buildings).
  - **5:** the prepared-snapshot pose C F F C, culled / full means, ms: 100k C 0.816 / 5.209 and
    0.886 / 5.320, F 1.131 / 5.608 and 1.130 / 5.642, **culled gap +0.279 ms against the +0.42 ms
    bar** (round 13: +0.322); 10k C 0.210 and 0.219, F 0.310 and 0.305, +0.093 ms; every culled
    pose equals the full pose (3,301 / 1,101). Judged: passes.
  - **9:** the `PERF-07` day at 100k 1x C F F C: frame mean C 6.298 / 6.305, F 6.571 / 6.493 ms;
    render snapshot mean C 0.995 / 1.005, F 1.235 / 1.226 ms (**+0.23 ms, the terrain query
    work's share**, against round 13's +0.264 and its +0.40 bar); agent tick C 5.044 / 5.042, F
    5.082 / 5.015; slots 0.183-0.186 alike. The guards: the control misses them on its own
    (`max_unpreempted_ms` 18.95 / 18.53, p99.9 10.23 / 10.20, 716 / 712 unpreempted frames over 8
    ms), and F is there too (18.58 / 19.41, p99.9 11.31 / 11.15, 1,264 / 1,047 over 8 ms: the
    snapshot's 0.23 ms moves frames near 8 ms over it). The inherited miss is `PERF-09`'s, the
    snapshot share the terrain query's; neither is charged to `TERRAIN-07-F` nor relaxed.
- **14 (locality), the topology adoption's +0.44 ms** (round 1's logs, C and F alike:
  `topology_ms` 0.026-0.031 ms at 0 remote sites, 0.471-0.480 ms at 100,000): the split's
  `BuildingAllocator::migrate_split_frontages` scans every building for those fronting the split
  edge (`O(B)`, documented so); 100,000 remote buildings at about 4.4 ns each. By reading and
  scaling, not profiled. A later track (`ROAD-72`): find the edge's buildings through the
  allocator's existing `building_chunks` over the edge's chunks. Accepted with the locality.
- 3, 4, 7, 8, 10-13, 15: as round 1 judged them (unchanged code; historical).

**Acceptance against every target** (fresh in this round unless marked historical):

| Target | Result | Status |
| --- | --- | --- |
| Moving, simulation paused (every sampled pose exact within 16.7 ms) | 62-74 % (round 1), 70.4 % C, 85.2 % N, 90.4 % W; the late poses' same work at the idle clock | Miss; question 62 (d) and (b) decided, next round |
| Moving, simulation active (99 %, misses attributed) | 0 of 590 at 100k (0 of 493 paused): the preview's whole 512 m chunks (11.5 MB a pose); rest of frame partly unattributed | Miss; question 63 (b) decided, next round |
| Stationary (p99 and stop → exact under 16.7 ms) | p95 19.0-24.5 ms in C, 10.7-21.0 ms in W; stop → exact 8.0-27.2 ms in C, 6.5-22.9 ms in W | Miss; question 62, next round |
| Commit (click → acknowledged under 16.7 ms) | every sample, 0-100k (round 1, historical) | Pass |
| Locality (products bitwise, flat) | bitwise; +1.9-2.5 ms at 100,000 (round 1, historical) | Accepted as measured by the user |
| The terrain stage's 3.0 ms (populated drags) | pose frames at most 2.91 ms (round 1, historical) | Pass |
| Bitwise preview = commit = undo = load | (round 1, historical; no Rust change since) | Pass |

**Gate checks** (fresh, on `fdadedc9`; the later commits change docs). `cargo clippy --all-targets`:
59 warnings, the baseline's set exactly (no Rust changed, so cargo replayed round 1's checked
results). `cargo doc`: 0 missing docs (its eight link warnings are `TERRAIN-07-G`'s).
`./run.sh --test` with `DISPLAY` and `WAYLAND_DISPLAY` unset (13:57-14:15): exit 0; the debug
library suite 1,842 passed, 0 failed, 177 ignored; `growth_scenario` 4 passed; every Godot suite
passes (the graded tile renderer tests with the split counter among them), the seven rendered
tests in gamescope's headless backend, nothing skipped, no `SCRIPT ERROR`.

**Line counts:** no Rust changed in this round: `roadbed/` 9,234, `terrain/graded/` 10,169,
`terrain/` 12,391 (round 1).

**Merge risks.** Every rendered result ran on Linux with the RX 7900 XTX (Vulkan; the shader tests
in the Compatibility renderer); macOS is unvalidated and has `TERRAIN-10` open. The rebased WIP
commits between `ROAD-68-G` part 10's `3b1e587b` and `8bfd9ebf` do not compile on their own
(bisecting through them needs a skip). The rendered rows depend on the CPU's power policy
(question 62).

#### Round 3: sub-chunk preview payloads, the pose's main-thread work, the city scan (`ROAD-68-H`, 2026-10-29)

Branch `road68-H` on round 2's `fcda122e`. Built from the user's decisions on questions 62 (d)
and (b) and 63 (b).

**The user's decision (2026-10-29, during round 3):** carried item 1 is accepted for `ROAD-68`:
the single pose install of round 2's drag probe at 1.009 ms against the proposed 1.0 ms share
(1 of 594), inside the 2.0 ms Godot stage. That row carries the user's decision for the gate;
every other row still passes or carries its own decision.

**Sub-chunk preview payloads: the design** (written before the code, question 63 (b)). A pose
used to send every render chunk the stroke changes whole, as the commit publishes it (the
spliced chunk: kept owners' triangles copied from the published chunk, rebuilt pieces emitted),
and Godot hid each resident chunk and drew the replacement. In a 512 m chunk of the 100k city
that is 11.5 MB a pose, nearly all of it triangles Godot already shows.

- **Unit: the owner.** The preview sends pieces, the unit the published chunk already records
  per layer (one owner run per span or node piece, in emission order: spans by edge id, then
  node pieces by node id) and the unit the dirty closure rebuilds. Not a finer grid: cells
  would clip triangles at new lines, so the preview's vertices could not be the committed
  chunk's.
- **Payload per changed chunk.** (1) The *overlay*: the rebuilt pieces' triangles routed into
  the chunk's frame, which is the `fresh` mesh the commit's splice merges, by the same emission.
  It carries no street lights or signs: the display never showed the preview's (the resident
  ones stay until the commit's install), so placing them along every covered edge each pose is
  dropped. (2) The *hidden runs*: per layer, the vertex range of each run of the published
  chunk whose owner the preview replaces (rebuilt, or gone: exactly the runs the splice drops),
  found by binary search in the chunk's owner runs for each owner of the edit's closure. (3) The
  published positions of those ranges, which Godot restores when a run is shown again. The
  preview dictionary names the road mesh generation the ranges index.
- **The published chunks the view holds.** The ranges index the chunk Godot shows, so the road
  tool's view holds every published chunk (the committed renderer's cache, now a persistent map
  shared in `O(1)`), not the upserts not yet acknowledged it held before: after Godot
  acknowledged a publication, a later view held none of the chunks that publication did not
  change, and a preview there spliced its pieces into nothing and hid the chunk's other roads
  (a latent bug, found by reading; the round's test shows it).
- **Composition in Godot.** The resident chunk stays drawn. Each hidden run's positions are
  collapsed in its vertex buffer (`ArrayMesh.surface_update_vertex_region`; Godot 4.7 stores a
  surface's positions as their own `f32` stream ahead of the normals, 12 bytes a vertex,
  checked on the GPU in gamescope: the region update leaves normals and custom data untouched,
  and writing the saved positions back restores them), so its triangles have three equal
  vertices and draw nothing; the overlay draws the rebuilt pieces in the same chunk frame. A run
  no longer hidden gets its positions back. Every overlay mesh is filled detached first; then,
  in one call, runs collapse and restore and the overlays swap, so no frame shows a replaced
  owner beside its rebuilt piece (doubled) or neither (missing). The drawn triangles are the
  committed chunk's: the kept runs from the resident buffer, which the splice copies verbatim,
  and the rebuilt pieces from the overlay, the emission's own vertices.
- **The junction preview's chunk hiding** (the resident instance's `visible = false`) is not
  reused: hiding a whole chunk is what forced the whole chunk to be resent. The display's
  detached staging, spare instances and atomic swap are kept.
- **Generation guard.** The display composes only when Godot's installed road mesh generation
  is the one the preview's ranges index. Otherwise (a publication Godot has not installed yet:
  the next stroke of a chain in the frames before the commit's chunks install) it shows no
  preview and shows the latest one in the install call itself (`_road_mesh_installed`).
- **Commit and undo** publish as before: whole changed chunks, spliced, with lights and signs.
  The published chunk stays the one source for what is committed. Godot's committed renderer
  replaces the resident instances, and the held preview is released in the same install call:
  its overlays go, the collapsed runs of the instances the install replaced go with them, and
  those of instances it kept are restored.
- **Complexity per pose.** Emission `O(rebuilt pieces' triangles)`; the hidden runs
  `O(closure owners * layers * log runs)`; their positions `O(hidden vertices)`; packing
  `O(overlay + hidden vertices)`; Godot `O(overlay vertices)` to stage and
  `O(changed runs' vertices)` of region writes, none while the closure keeps its owners. Nothing
  per pose follows the chunk's content. The commit's publication stays
  `O(rebuilt pieces + chunk vertices)`.
- **Proof.** Rust: for every changed chunk, the published chunk with the hidden runs dropped and
  the overlay merged equals the committed chunk bitwise, layers, crossings and owner ranges
  (lights and signs are the commit's); test builds assert at every preview that the hidden runs
  are exactly the runs the full splice drops and the overlay is its fresh mesh. Godot: the
  composed resident and overlay meshes hold the committed chunk's triangles; the preview's
  pixels at converged poses equal the control's (gamescope captures). The committed payloads:
  the commit breakdown's `COMMIT_PRODUCTS` against the control.

**Built** (`f8d06a12`). `RoadRenderer::generate_preview_overlay` emits the delta's built pieces
through the commit's `emit_chunks` with an empty light and sign coverage, and finds each changed
chunk's hidden runs with `NetworkMeshData::owner_vertex_ranges` (one `partition_point` per closure
owner and layer over the chunk's owner runs); test builds compare the overlay's pieces with the
splice's fresh coverage and the hidden runs with the runs the splice drops
(`vertex_ranges_dropped`). `generate_preview_chunks` is gone. The committed renderer's cache is a
`RoadChunkMap` (`imbl::OrdMap`), which the view clones in `O(1)` with the road mesh generation
Godot records for it (`road_mesh_surface_generation`). The bridge sends each overlay in the
committed chunk format with `hidden` (flat `[layer, start, end]`) and `hidden_positions`, and the
dictionary's `road_mesh_generation`. In Godot, `_fill_road_chunk_mesh` records each layer's surface
(`road_layer_surfaces` meta) and `road_preview_display.gd` keeps the collapsed runs per resident
instance with their saved positions, applies a pose's changes after staging, and shows a preview
of a generation not installed yet from `resident_installed`. Lines: `render/` 4,317 -> 4,474;
`roadbed/` 9,234 and `terrain/graded/` 10,169 unchanged.

**Proof** (fresh, 2026-10-29):

- **Rust, debug** (`cargo test --lib`, 308 s): 1,845 passed, 0 failed, 177 ignored. The
  preview tests compare each changed chunk's composition (`compose_preview`: the view's published
  chunk without the hidden runs, the overlay merged in owner order) with the committed chunk:
  layers, crossing frames and owner runs bitwise (`preview_chunks_compose_the_committed_chunks`,
  `preview_chunks_with_crossings_and_signs_are_the_committed_chunks` (the overlays paint the
  zebras; the commit places the signs and lights), `a_widening_continuation_previews_its_committed_chunks`),
  and every preview of the suite runs the two test-build assertions.
  `a_preview_after_an_acknowledged_publication_composes_over_every_published_chunk` acknowledges
  the fixture's publication (the unacknowledged upserts empty) before the context: the view holds
  the chunks, the T hides runs of the resident through road and composes the committed chunk.
  Before, the view took the empty upserts, so the preview's splice had nothing to keep.
- **Godot** (gamescope, the round's release library): `road_junction_preview_test.gd` passes (the
  display collapses exactly the payload's runs, the hidden runs are the resident chunk's own
  positions, cancel restores every run, a chain's next stroke waits for the commit's chunks and
  shows in their install call, and each previewed chunk's composition holds the committed chunk's
  triangles, every attribute and crossing frame, as a multiset); `network_tool_chunk_renderer_test.gd`
  and `road_preview_stream_test.gd` pass.
- **Pixels.** The junction preview test's captures (`METRUM_JUNCTION_PREVIEW_CAPTURE`: before,
  the converged preview, cancelled and committed for each fixture, rendered on the GPU in
  gamescope, so the collapsed runs are drawn as Godot draws them), control (`fcda122e`'s scripts
  and tests on H1's library, the tip's Rust) against this round: **43 of 43 images identical**,
  every pixel (`$ARTIFACTS/road68/h3/captures-ctl`, `captures-new`, `compare.py`).
- **The populated pose** (the 100k commit city paused, `populated_t` and `populated_branch`,
  gamescope, MAX_FPS=0, 20 samples after 4 warm-ups; before = `fcda122e`'s scripts on H1's
  library, after = this round, 15:46-15:50; not acceptance timing, both at full clock): road
  payload of the final pose **11.53 -> 2.80 MB** (`populated_t`) and **11.48 -> 2.76 MB**
  (`populated_branch`), the whole pose's packed bytes p50 11.54 -> 2.81 MB;
  p50 ms before -> after: payload 3.03 -> 1.41 and 3.30 -> 2.10, packing 1.38 -> 0.38 and
  1.49 -> 0.50, the call 7.99 -> 5.22 and 9.02 -> 7.23, the install 6.01 -> 1.79 and
  6.05 -> 2.40 (plan, roadbed and graded unchanged: 0.5, 1.0-1.2, 1.6-2.7). What is left is the
  closure's own pieces: the split trunk's two halves (544 m) and the edges at its end nodes,
  emitted, packed and uploaded each pose. The rest of the T's frame stays 42 ms (engine time,
  below).

**ROAD-72** (`20ab7a37`). `BuildingAllocator::migrate_split_frontages` takes the buildings fronting
the split edge from `attachment_repair_candidates` over the two halves (the 512 m building chunks
within reach of their segments: the largest lot, the repair's search and the edge's frontage
offset), filtered to the split edge, in building order, so the moved rows, their undo entries and
their record are the scan's; the whole city only while the chunk index is stale. Test builds
assert the set equals the whole city's scan, so every split in the debug suite checks it;
`split_frontages_come_from_the_split_edges_chunks` (fresh and stale index) and
`split_frontages_are_checked_against_the_whole_city` (a building attached from another chunk
trips the check). `O(candidates log candidates)` against `O(B)`.

**The paused preview rows on the display** (fresh, 2026-10-29 15:51-16:09, the user's display,
DP-2 at 3840x2160, 120 Hz; the round's release library (`dca30744…`), 1920x1080 windowed, vsync,
no FPS cap, four workers, P-cores, both locks, `display-awake.sh`, a throwaway profile; N W W N
interleaved, 100 samples after 8 warm-ups; `$ARTIFACTS/road68/h3/rows-windowed-20261029T155115/`).
Sampled poses exact within 16.7 ms: **N (Xwayland) 3,040 of 3,862 (78.7 %), W (native Wayland)
3,575 of 3,954 (90.4 %)**; every miss `late_exact`. N's first run of `sloped_t` and
`sloped_branch` paced at 60 Hz (frames p50 16.8 ms; the window opened before the display ran at
120 Hz): without it N is 2,621 of 3,128 (83.8 %). Round 2 measured 85.2 % N and 90.4 % W with
the whole-chunk payload: the synthetic fixtures' 128 m chunks were already small, so the rows do
not move. Two runs a cell, ms:

| Fixture | Variant | Frame interval p50 / p95 | Exact within 16.7 ms | Stationary p50 / p95 / max | Stop → exact | Moving age p50 / p95 / max |
| --- | --- | --- | --- | --- | --- | --- |
| `sloped_t` | N | 16.8 / 24.4 (60 Hz); 8.5 / 17.9 | 206 of 358; 335 of 391 | 16.48 / 24.16 / 24.87; 8.16 / 13.24 / 18.12 | 20.08; 13.91 | 16.52 / 26.73 / 145.35; 13.59 / 22.12 / 64.49 |
| `sloped_t` | W | 8.4 / 16.6; 8.4 / 14.9 | 358 of 394; 380 of 397 | 8.31 / 11.52 / 12.52; 8.30 / 14.74 / 16.43 | 15.36; 8.35 | 13.42 / 22.86 / 69.99; 13.97 / 18.60 / 68.09 |
| `sloped_branch` | N | 16.8 / 23.1 (60 Hz); 8.5 / 17.3 | 213 of 376; 349 of 394 | 16.44 / 23.19 / 26.59; 8.06 / 12.90 / 15.55 | 25.36; 9.16 | 16.52 / 24.35 / 32.87; 13.50 / 21.21 / 26.04 |
| `sloped_branch` | W | 8.4 / 17.3; 8.4 / 14.5 | 352 of 394; 387 of 400 | 8.34 / 15.74 / 24.27; 8.26 / 10.22 / 14.32 | 9.13; 7.73 | 13.61 / 22.22 / 26.78; 13.56 / 22.39 / 24.95 |
| `flat_cross` | N | 8.7 / 20.2; 8.6 / 18.8 | 296 of 385; 328 of 392 | 8.86 / 13.70 / 15.66; 9.18 / 18.07 / 24.84 | 13.12; 12.75 | 14.31 / 20.92 / 66.25; 14.08 / 19.97 / 73.07 |
| `flat_cross` | W | 8.5 / 17.0; 8.5 / 17.8 | 354 of 395; 348 of 393 | 9.63 / 16.80 / 20.54; 8.32 / 16.82 / 21.80 | 8.41; 11.82 | 14.51 / 18.86 / 71.37; 13.86 / 20.45 / 70.44 |
| `sloped_cross` | N | 8.5 / 19.9; 8.6 / 19.1 | 314 of 392; 329 of 392 | 8.72 / 19.59 / 26.34; 10.55 / 20.60 / 22.96 | 13.82; 9.73 | 14.25 / 20.71 / 33.25; 14.23 / 20.70 / 30.72 |
| `sloped_cross` | W | 8.4 / 17.9; 8.7 / 18.3 | 342 of 395; 352 of 395 | 8.43 / 16.55 / 23.80; 9.81 / 19.95 / 26.50 | 10.45; 13.12 | 14.40 / 19.39 / 26.96; 14.16 / 20.88 / 28.27 |
| `dense` | N | 8.4 / 19.6; 8.4 / 18.4 | 327 of 389; 343 of 393 | 8.32 / 18.34 / 22.36; 8.17 / 12.92 / 23.92 | 8.51; 13.67 | 13.53 / 20.67 / 28.53; 13.36 / 22.71 / 24.56 |
| `dense` | W | 8.5 / 18.0; 8.4 / 16.6 | 340 of 394; 362 of 397 | 8.53 / 14.36 / 19.45; 8.29 / 14.79 / 22.92 | 13.37; 9.10 | 13.96 / 23.65 / 28.44; 13.83 / 19.71 / 24.75 |

One W run (`dense`, the second) hung in Godot's exit after writing its metrics and was stopped;
its numbers are complete.

**The populated rows on the display** (fresh, 16:25:31-16:37:38, as above; the 100k commit city,
`populated_t` and `populated_branch` at the commit breakdown's first edit site, A P A on Xwayland,
then on native Wayland; `populated-windowed-20261029T162531/`,
`populated-windowed-wayland-20261029T163120/`; a first attempt, 16:09-16:15, was stopped: an
exit-hang watchdog added after the rows' hang killed each run after its first case's metrics,
which are written per case; fixed to wait for every expected case). Sampled poses exact within
16.7 ms: **A (the simulation at 1x) 229 of 803 (28.5 %) on Xwayland, 213 of 790 (27.0 %) on
Wayland; P (paused) 358 of 629 (56.9 %) and 394 of 637 (61.9 %)**; round 2: 0 of 590 and 0 of
493 (60 FPS cap, whole chunks). Every miss `late_exact`. p50 ms, late poses:

| Case | Variant | Exact within 16.7 ms (Xwayland; Wayland) | Call / install / rest, late p50 (Xwayland) |
| --- | --- | --- | --- |
| `populated_t` | A | 0 of 115, 0 of 114; 0 of 115, 0 of 109 | 5.6 / 2.1 / 42.5-42.7 |
| `populated_t` | P | 35 of 233; 46 of 240 | 5.2 / 2.0 / 23.3 |
| `populated_branch` | A | 102 of 282, 127 of 292; 103 of 282, 110 of 284 | 6.3-6.5 / 2.2 / 10.9-11.8 |
| `populated_branch` | P | 323 of 396; 348 of 397 | 8.9 / 3.2 / 5.5 |

The preview's own share is now the call (5-7 ms p50) and the install (2.0-2.3 ms); the misses are
the rest of the frame: on the T the graded renderer's streaming after the load (question 64), and
with the simulation active the simulation's share (the branch's rest 11-12 ms active against
5.5 ms paused), which round 2 left unattributed by function. One Wayland run (P) hung at exit
after writing every case's metrics and was stopped by the watchdog; its numbers are complete.

**Section 2: the pose's main-thread work at the idle clock.** Not cut in this round beyond the
payload. The sub-chunk payload removes the populated pose's 11.5 MB (call and install 14-15 ms to
7-10 ms at full clock), but on the synthetic fixtures the 128 m chunks were small already: their
payload stage stays 0.40-0.43 ms p50 and the rows do not move (78.7 % N, 90.4 % W). The rows'
profile after the change (16:37:38-16:38:23, N and W on `sloped_t` and `sloped_branch` under
`perf record`, `pacing-profile-windowed-20261029T163738/`, `profile-N.txt`, `profile-W.txt`;
round 2's method, attribution not timing): between a pose's sample and its exact frame the main
thread executes **14.1-14.6 M cycles p50 on time or late, as in round 2**, the pool workers
15.5-18.0 M; the late poses run at 1.18-1.25 GHz (main busy 11.9-12.7 ms, the call 7.9-8.8 ms,
the rest of the frame 6.4-7.2 ms wall), the on-time ones at 2.2-2.3 GHz p50 (call 3.9-4.3 ms).
So the work a pose must lose to fit 16.7 ms at the idle clock is unchanged: the call's own
stages (plan, roadbed, graded, payload; 2.4-2.8 ms at operating clock headless) and the frame's
engine part (3.1-3.6 ms of the late rest in Godot). Carried to the next round (question 62 (b)).

**The landing session** (fresh, 2026-10-29 16:17:51-16:23:43, one hold of both locks,
`taskset -c 0,2,4,6,8,10,12,14`, `METRUM_DEBUG=0`, no restic, compiler, cargo or Godot process;
release library tests: C = H1's F (`$TEST_BINARY` sha256 `a880ce56…`, the Rust of `fcda122e`),
F = this round (`9f9cb9ba…`, the Rust of `20ab7a37`), each with its own saved cities, every
populated tier `loaded`; `$ARTIFACTS/road68/h3/session/`, `breakdown-table.txt`,
`locality-table.txt`, `drags-table.txt`). C F F C each:

- **Commit** (click → committed generation installed and acknowledged, first samples, ms): 0: C
  6.09, 6.17, F 6.25, 7.02; 10k: C 10.27, 9.31, F 9.58, 9.73; 30k: C 9.90, 10.23, F 10.36,
  10.33; 100k: C 14.71, 14.55, **F 12.97, 12.56**. Every sample of every run fits 16.7 ms. At
  100k the topology stage takes C 0.33-0.41, F 0.03 ms (`ROAD-72`).
- **Bitwise.** All four breakdown runs print the same `COMMIT_PRODUCTS` (the city's road payload
  vertices, every sample's republished chunks and payload vertices among them), so the committed
  and published chunk payloads are bitwise round 2's; all sixteen locality product sets are equal,
  C and F alike.
- **Locality.** The topology stage at 100,000 remote sites: C 0.475, 0.483 ms, **F 0.055, 0.056**
  (0.03-0.08 at 0 sites): `ROAD-72` makes it flat. First commits at 100,000 sites C 18.30, 20.43,
  F 15.10, 17.45 ms (0 sites: C 12.71, 14.93, F 13.67, 13.46); the remaining growth is round 1's
  accepted parts (terrain query index, graph copy).
- **Preview stages** (four workers): payload p50 0.400-0.428 ms C and F, totals p50 2.37-2.82 ms;
  `sloped_branch` sends 9,973 overlay vertices and 6,475 hidden positions a pose against 11,706
  chunk vertices; `sloped_t`'s closure is its whole 128 m chunk (9,845 both).
- **Populated drags:** pose frames at most 2.67 / 2.86 ms (C), 2.64 / 2.61 ms (F), none over the
  3.0 ms stage; fast clicks at most 3.37 (C) and 2.67 ms (F).

**Acceptance against every target** (fresh in this round unless marked):

| Target | Result | Status |
| --- | --- | --- |
| Moving, simulation paused (every sampled pose exact within 16.7 ms) | N 78.7 % (83.8 % without one run paced at 60 Hz), W 90.4 %; every miss one frame late | Miss: the pose's main-thread work at the idle clock (question 62 (b)) not cut yet |
| Moving, simulation active (at least 99 %, misses attributed) | 100k: A 28.5 % Xwayland, 27.0 % Wayland (paused 56.9 %, 61.9 %); round 2 0 % | Miss: the graded renderer's streaming after the load (question 64) and the simulation's share (unattributed by function) |
| Stationary (p99 and stop → exact under 16.7 ms) | W p95 10.2-20.0, stop → exact 7.7-15.4 ms; N p95 12.9-24.2, 8.5-25.4 ms | Miss |
| Commit (click → acknowledged under 16.7 ms) | every sample 0-100k, at most 12.97 ms (F) | Pass |
| Locality (products bitwise, flat) | bitwise; topology flat since `ROAD-72`; the first commit's accepted growth | Accepted as measured by the user (round 2) |
| The terrain stage's 3.0 ms (populated drags) | pose frames at most 2.64 ms | Pass |
| Bitwise preview = commit = undo = load | composition tests; `COMMIT_PRODUCTS` equal to C; 43 of 43 captures identical | Pass |
| Carried item 1 (one pose install at 1.009 ms against the 1.0 ms share) | round 2's probe | The user's decision: accepted (2026-10-29) |
| No frame shows a hole or stale terrain while a preview moves (`terrain.md` "What a patch draws") | the user's play, `TERRAIN-12` | **Blocker**, not fixed in round 3 |

**Carried items** beyond item 1: as round 2 judged them (5, 6 and 9 measured there; the rest as
round 1); no code of theirs changed in this round.

**Gate checks** (fresh, on `e184bec6`, the Rust of every later commit; the debug suite above ran on
`20ab7a37`'s Rust): `cargo clippy --all-targets` 59 warnings, the baseline's set exactly
(compared by file and message), every target compiling; `cargo doc` 0 missing docs (its link
warnings `TERRAIN-07-G`'s); `./run.sh --test` with `DISPLAY` and `WAYLAND_DISPLAY` unset
(16:38:30-16:56:49): exit 0, the debug library suite 1,845 passed, 0 failed, 177 ignored,
`growth_scenario` 4 passed, all 22 Godot suites pass with the seven rendered tests in
gamescope's headless backend, nothing skipped, no `SCRIPT ERROR`. Line counts as above.

**The terrain tangent warning** (found by the user in play, 2026-10-29; fixed in this round).
Godot printed "Attempting to use a shader (res://assets/materials/terrain.gdshader) that requires
tangents with a mesh that doesn't contain tangents" each time a terrain surface instance took a
material: 31-76 times a preview row run and 609-1,031 a populated run (about one a pose), none for
the `terrain_graded*` shaders. Cause (checked in gamescope with a probe, `h3/probe-tangent/`): the
mesh is the graded renderer's cell lattice (`terrain_graded_tiles.gd` `_build_meshes`, positions and
indices only), which the base cells draw with the patch's `terrain.gdshader` material; the base
variant's fragment passes `TANGENT` and `BINORMAL` to the street lamp light
(`terrain_surface.gdshaderinc`), and Godot asks the mesh for tangents. The graded variants draw the
same lattice without the warning: they assign `TANGENT` and `BINORMAL` in their paving branch before
the lamp reads them (measured; Godot's exact rule not read). **Fix (b), not (a):** the base variant
passes no tangent basis and the default normal map to the lamp, as `terrain_graded_walls.gdshader`
already does. It writes no normal map (only the graded paving does), so with the default map
(`xy` exactly 0, `z` 1, depth 1) the lamp's normal is the surface normal and the tangent terms are
multiplied by zero: the same light, by construction. (a), a +X tangent on the shared lattices,
would also need normals and would feed the base variant a tangent its light ignores, at 16 more
bytes a vertex. Test: `terrain_tangent_test.gd` (rendered, Forward+, in `run.sh`'s rendered list)
draws a tangent-less lattice with each of the four terrain shaders and reads Godot's file log: it
fails before the fix (2 warnings for `terrain.gdshader`) and passes after. The junction references
(`road_junction_reference_test.gd`, gamescope, 197 images, before and after on the round's
library): 580 warnings before, 0 after; the images differ by at most 21 levels in 93 of 197, inside
the renders' own run-to-run noise (two runs of the same build: 96 of 197 images, up to 50 levels;
the five images that differ only across the fix, by 1-16 pixels of at most 10 levels, are as
noisy), so the pixel comparison cannot prove equality beyond the noise, and the algebra above does.
**Cost of the warning:** in round 2's symbolized populated profile (about one warning a pose) the
printing shows as `String::sprintf` 0.16 ms a pose (the warning's `vformat` among its callers) and
the writes about 0.01 ms: at most about 0.2 ms a pose at full clock, perhaps 0.7 ms at the idle
clock; a matched measurement is left with the timing runs (`$ARTIFACTS/road68/h3/timing-pending`).
The preview rows and populated runs of this round still carried the warning; the run after the fix
that shows 0 such warnings in a preview row and a populated run is part of those runs.

**`TERRAIN-12`, a gate blocker** (reported by the user, 2026-10-29 about 17:00, in normal play on
`road` `8b3ce801`, a debug build): while a road preview is dragged, the terrain where the road's
previous pose lay is missing for 1-2 frames, the sky showing through a road-shaped corridor with
jagged, cell-scale edges, sometimes a thin sliver along the road's edge: tile-shaped holes, not
128 m patches. Evidence: `$ARTIFACTS/road68/bugs/dump-17-01/` (screenshots `4.png`-`7.png`, the dump
`dump-2026-10-29_17-01-40.{sqlite,png,txt}`, `godot2026-10-29T17.02.45.log`). It breaks
[`terrain.md`](terrain.md) "What a patch draws" (no frame shows a set with a listed tile missing) and
is stale geometry in a moving pose, so it blocks the gate; the rendered harness's "exact" pose
measure (the road's) and the converged-pose hole checks did not catch it. Not fixed in this round
(the coordinator's instruction). **Lead, by reading** (`terrain_graded_tiles.gd`): a pose's frame
calls `show_preview` from the road tool (`road_tool.gd` `_show_graded`, in the pose's own frame,
after `preview_road`), which first drops the tiles Rust lists in `graded_removed` through
`_drop_preview` (erase the preview entry, `_free_copies` → `_free_copy`: vacate the slots, maybe
empty and unbind the page), then shows the new tiles through `_show_preview_tile`, which ends in
`_shown_changed(key, before)` → `_cell_changed` and `_await_cells` (the cells of the patches meeting
the tile marked, rewritten in the same call by `_apply_cell_changes`). `_drop_preview` calls no
`_shown_changed`, so a dropped tile's cells are not marked and keep sampling the vacated preview
slot (blank once the page empties or the slot is reused) until another change rewrites them, which
fits "the previous pose's corridor, for a frame or two". `release_preview` (at a click or cancel)
takes the same path. The user also saw the preview road sink under the old terrain on slopes for a
while: T27 (d)'s accepted lag, not this defect.

**Merge risks.** Every rendered result ran on Linux with the RX 7900 XTX (Vulkan); the sub-chunk
display writes a resident surface's position stream in place, which relies on Godot 4.7's layout
(positions as their own `f32` stream, checked on this GPU; the display refuses a surface whose
position stride is not 12 bytes), and macOS is unvalidated with `TERRAIN-10` open. The rebased WIP
commits between `ROAD-68-G` part 10's `3b1e587b` and `8bfd9ebf` do not compile on their own. Godot's
Wayland driver hung at exit in two of this round's runs after writing its metrics (the harness's
watchdog stops it); the game itself was not seen to hang.

**Section 2: the populated pose's rest of frame, named** (fresh analysis of round 2's active
profile, `$ARTIFACTS/road68/h2/populated-windowed-profile-20261029T134516/`; not a new run).
Godot's executable is stripped; its debug file, fetched by the executable's ELF build ID
(`15425c02056a6250959df0305a18e5d0d4755b75`, debuginfod.archlinux.org, sha256 `f896470d…`, as
`TERRAIN-07-G` round 9C) and registered in perf's build-ID cache, resolves its addresses
(`$ARTIFACTS/road68/h3/frames_sym.py`: the main thread's samples between each pose's sample and
its exact frame, and after the install, by symbol). In the late `populated_t` poses (95, the
simulation at 1x) the main thread's rest of frame is 37.6 ms a pose, 33.0 of it in Godot, and
that is **the GDScript interpreter and the Variant containers it drives**:
`GDScriptFunction::call` 5.6 ms, `Variant::hash_compare` 2.5, `Dictionary::getptr` 2.2,
`Array::_ref` and `_unref` 3.6, `Dictionary` copies and releases 2.6, `Variant` references and
assignments 2.9, string reference counts 1.4, and the long tail of the same; the renderer's own
functions (draw lists, the forward renderer's lists) take under 1 ms. `populated_branch` at the
same site has 6.2 ms of rest (GDScript 0.3 ms). So the T's rest of frame is one of our scripts
walking dictionaries and arrays each frame: not the engine, not `RENDER-27`, and not the road
tool's install. **Which script** (fresh, 16:23-16:25, gamescope, the populated cases paused with
Godot's PerfDebug summaries, `METRUM_DEBUG_PERF=1`, `$ARTIFACTS/road68/h3/perfdebug-20261029T162345/`;
attribution, not timing): the terrain renderer, `terrain.gd`'s frame, rises from 8 to 47 ms a
frame while each case runs after its load and falls back to under 2 ms once it settles, almost
all of it the graded tile renderer's residency pass (`graded_residency_ms` 13.0, 24.5 and 33.0 ms
in three successive summaries, as its wanted tiles grow 2,145, 4,917 and 6,923 while the 100k
city's tiles stream in) and the patch residency (6.0-6.9 ms a frame against its 4.0 ms budget);
texture memory grows from 494 to 1,124 MB meanwhile. So the populated row is sampled inside the
graded renderer's streaming after the load, whose per-frame GDScript work grows with the tiles
wanted: `TERRAIN-07`'s renderer, not the road preview and not the engine
([question 64](#road-68-open-questions)). The window's install part before this round was Godot's `_surface_set_data`
(4.0 ms), `Vector3::octahedron_encode` (1.1 ms) and the copies in libc (3.7 ms), with `seal`
(0.8 ms) and the packed-array conversions (0.7 ms) in the extension: the 11.5 MB upload that the
sub-chunk payload removes.

#### Round 4: `TERRAIN-12` fixed in the Rust staging, the pose's main-thread work attributed, the runs (`ROAD-68-H`, 2026-10-29)

Fresh in this round unless marked; artefacts `$ARTIFACTS/road68/h4/`. **The user's decisions this
round:** question 64 (c) (about 17:35); `TERRAIN-13` widened (about 17:30: the graded tile
renderer's GDScript bookkeeping, its residency pass among it, and the other per-pose GDScript
work move to Rust; H misses caused on that GDScript side are carried there; H lands before it);
[T60](terrain.md#terrain-07-open-questions) (about 18:10: `TERRAIN-12` fixed here in the Rust
staging, rule (a) and the existing rule for invalid poses; this replaced the 17:30 carry of the
holes to `TERRAIN-13` once the cause was found outside the GDScript renderer).

**`TERRAIN-12`: reproduced, the cause, the fix.** Reproduced in isolated profiles from the
user's dump and on Kuopio with the drag probe's new swing (`METRUM_GRADED_DRAG_SWEEP`: the
stroke's end swung to each side at its length, a pose a frame; `_SWEEP_CONVERGE`: caught up
between poses, a player's pauses) and per-frame checks: the renderer's own state
(`terrain_graded_tiles.gd` `tile_draw_violations`: every resident tile of a drawing set is one
instance in the cell mesh of its base cell, installed tile or preview tile, from the slot holding
its copy in the page its set binds), no preview tile shown while the road tool draws no preview
road, and, rendered (`_HOLE_PIXELS`), the background's pixels through a magenta camera
environment looking down. Before the fix (round 3's library `dca30744`): the dump's swing showed
the background in 32 of 72 frames, `kuopio_08` attempt 22 (122 m, sloped) in 47 frames and five
invalid-pose frames, attempt 21 in 84 and five; the renderer's invariant held in every frame. Round
3's lead is refuted (`_drop_preview` calls `_shown_changed`). The cause, in the Rust staging with
the terrain's exact footprint discard: T27 (d)'s unstaged tiles showed an earlier pose's bake,
cut where that pose's road lay, until the catch-up staged them (on attempt 22: the pose's frame
and 3 catch-up frames, holes in 2-3 of them); and a pose without pieces kept the last valid pose's
tiles shown while no road was drawn. The fix (`graded_stage.rs`, T60 (a)): the renderer gets a
display apart from the adoption pool; a pose also selects the tiles its move dirtied
(`RoadbedSystem::terrain_changes_from` between the latest pose's roadbed and this one over the
owners either delta covered, then the commit's selection, testing only the displayed tiles:
selecting the whole move first cost the populated drags' poses 0.4-0.6 ms and took 1 and 3 of
84 pose frames over 3.0 ms in the session at 18:30 (C 0 and 0), so it was cut); the display keeps the pose's wanted
tiles outside that set and the frames' staged tiles; a pose without pieces, an invalid one and a
publication hand every displayed tile back. The adoption pool is unchanged, so every staged,
committed and undone tile is. Tests: the graded drag test now checks every pose's and catch-up
frame's displayed tiles against their fresh bake, and a new test swings a stroke and ends it on a
degenerate pose (every displayed tile handed back). **Why the checks missed it:** the harness's
"exact" pose measure is the road's; the hole checks counted patches drawing nothing and read
converged captures; the probe's strokes grew along one line and were all valid. The Rust tests
above now check what the renderer is handed, frame by frame, against fresh bakes; the probe's
swing fails on the old library.
**Proof** (fresh, gamescope, the same probe and scripts on the fixed library `069d2852` and
round 3's `dca30744`): the user's dump, four 125 m strokes swung 30 degrees with pauses, 0 of
416 frames with the background after the fix (before: 43 of the first stroke's 104); the 100k
populated city, the rows' T and branch swung, 0 of 282 after; the renderer's invariant and the
no-road check 0 in every frame after. Kuopio: the cases stop at their first failing stroke, so 25
of the 39 strokes ran; 16 are clean, 9 show two residuals the fix does not cover
([T61](terrain.md#terrain-07-open-questions), for the user): seam cracks at a tile seam across
the road while a pose catches up on steep strokes (1-78 sampled pixels a frame, median 2, none
converged; `kuopio_04` attempt 9: 24 frames), and one frame after an undo in which the undone
road is gone and the terrain still discards its footprint (4 strokes; there before the fix too).
`kuopio_01`, `_04` and `_05` before the fix: 49, 130 and 118 frames; after: 1, 24 and 1. So the
road-shaped holes the user saw are gone; T61 keeps `TERRAIN-12` open.

**T61, decided by the user (about 18:50): (1) (a) and (2) (a), both in H.** (1) is built: the
display is all or nothing per pose, the latest pose's whole selection once every tile of it is
staged or exactly kept, else the published tiles (`GradedPreview`: T60's exact tiles kept apart
from what the renderer draws). The graded drag test asserts it in every pose and catch-up frame
and fails without it (`1 of 6 selected tiles displayed`). The change is in what the renderer is
handed only: the adoption pool, the staging and the commit's adoption are untouched, so the
commit path is too, and the commit breakdown's and bitwise rows of this round's first session
stay valid for the tip (its second session's F, with T60's display, printed the same
`COMMIT_PRODUCTS` as the control). Proof runs (fresh, gamescope, the fixed library `f07afeb2`, the probe's swing with pauses,
`METRUM_GRADED_DRAG_CONTINUE` so every stroke runs): the dump 0 of 416 frames with the
background, the 100k populated swing 0 of 282; all 39 Kuopio strokes: the renderer's invariant
and the no-road check 0 in every frame, the seam cracks gone (no catch-up frame with a crack),
but 25 frames on 9 strokes still show the background, none of them a crack or an earlier pose's
cut: 8 undo frames (T61 (2), not built), 3 frames around a fast click on 2 strokes and 14
pose and catch-up frames on 1 stroke where the pose rebuilds an existing road's pieces (its
junction) and the published tiles, drawn until the pose's selection completes, still discard the
old pieces' footprint the pose's road no longer covers; and 11 fast clicks install no batch
frame (the probe's own expectation): a committed tile that is neither held, wanted nor shown by
the preview waits for its patch's list (`_apply_edit`), and with all or nothing a fast click
shows none. **The lag (finding for the user, the budget unchanged):** after each pause of the
swing the preview terrain is absent for 2-10 frames (Kuopio, 1,014 poses: median about 5; the
dump 3-4, the populated city 1-6), against its first tile in the pose's own frame before; in
the probe's continuous drags (a pose a frame) **no pose frame shows preview terrain (0 of 624)**:
each pose dirties more tiles (up to 18) than its frame and the catch-up stage, so the terrain
shows only when the drag pauses. [T62](terrain.md#terrain-07-open-questions) (a), decided by the
user (about 19:15): the lag and the fast click's waiting tiles accepted as T27 (d)'s; the
junction holes to fix by holding the pose's replaced road runs drawn until its selection
displays (round 5, with the three proofs T62 names). Converged poses are unchanged: the junction preview test's 43 captures on the final library are
pixel-identical to round 3's (`h4/captures-final`; its 13 fixtures' 52 graded preview tiles
bitwise). (2), the undo's hold, is not built: the undo's
graded batch comes from the edit journal through a payload worker a poll or more after the
network renderer installs the undo's road chunks, and Godot has no signal that an edit for that
generation is still in flight; the hold needs one (the journal's latest serial against the
renderer's installed one) without delaying a commit's acknowledgement. Redo and road deletion
are not yet checked for the same gap. Both are the next round's first task.

**The accepted lag** (T27 (d), not changed): after a pose, the road may sit under or over the
published terrain on a slope until its tiles are staged: on `kuopio_08` attempt 22 (122 m,
sloped) the pose's frame and 3 catch-up frames (up to 8 pending tiles; 33 ms at 120 Hz), on the
dump's 125 m strokes 3-4 frames.

**Question 62 (b): the pose's main-thread work, three ways** (fresh profiles on the user's
display, the tip's library, `perf` with Godot's debug file by build ID as in round 3,
`$ARTIFACTS/road68/h4/attrib3.py`: per sampled pose, the main thread's cycles between its sample
and its exact frame; inside the Rust preview call by its dispatch and return, outside it the
GDScript side by leaf symbol, the VM and the Variant, Dictionary, Array and string helpers it
drives, the rest the engine; libc apart, shared by both). Paused rows (`sloped_t`,
`sloped_branch`, Xwayland and Wayland, `pacing-profile-windowed-20261029T175758`): 14.5-14.9 M
cycles a pose, the late poses at 1.16-1.21 GHz: **engine 43-45 %** (6.2-6.6 M: the Vulkan driver
1.0 M, `RenderingServer::_surface_set_data` 0.78 M and `Vector3::octahedron_encode` 0.34 M
converting the preview meshes GDScript builds, the forward renderer's lists, sorts and draws),
**GDScript 26-27 %** (3.75-3.97 M, `GDScriptFunction::call` 1.2-1.3 M and its helpers), **the Rust
preview call 21-23 %** (3.13-3.34 M; no function above 0.35 M: `SurfaceTriangleGrid::from_triangles`
0.35, copies 0.45, `NetworkMeshData::seal` 0.14, the node layout 0.11), libc 6-7 %, Rust outside
the call 1 %. Estimated per pose (its sample-to-frame time less its main-thread GDScript time,
the main thread taken as the critical path): without the GDScript time 96.7-97.5 % of sampled
poses would be exact within 16.7 ms (89.9-92.0 % as profiled), and without the engine's
conversion of the GDScript-built meshes too 97.4-98.2 %. **No cut of the Rust call:** its share is
spread over the plan, the roadbed compile's serial parts, the payload and packing, the largest
function 0.35 M of 3.3 M, while a late pose needs 2.5-3.5 M fewer cycles at the idle clock. So
the paused rows' miss is the GDScript share (carried to `TERRAIN-13` by the user's decision) and
an engine and Rust share beyond it (question 65). Active populated row (100k, simulation at 1x,
x11, `populated-windowed-profile-20261029T175856`): the main thread runs at 4.9 GHz (not the idle
clock), 40-42 M cycles on time and 52-54 M late: engine 42-49 % (`_surface_set_data` 3.9 M and
`octahedron_encode` 1.2 M, the driver 2.7 M, MultiMesh buffer sets 0.5 M), GDScript 18-23 %, the
Rust call 19-26 %, libc 6-9 %, Rust outside the call up to 6 % (the simulation's building plot and
construction site transforms, 1.8 M); without the GDScript time an estimated 55-58 % of poses.
The late poses' extra 12 M cycles over the on-time ones are the frame's render work while the
city grows (surface and MultiMesh uploads, the building and construction transforms) and
GDScript; question 65.

**Section 3: the tangent fix's proof** (`63b401b9`). The 43 preview captures that proved the
payloads cannot prove it: `road_junction_preview_test.gd` draws the terrain with a plain unshaded
`StandardMaterial3D`, so they render no terrain shader, no base cell and no lamp light. Instead
(`$ARTIFACTS/road68/h4/tangent_capture.gd`, gamescope, the tip's library): the base variant
(`terrain.gdshader`) on the graded renderer's own tangent-less code-3 base-cell lattice and on a
`PlaneMesh` patch, in its own viewport (black background and ambient, no sky, clouds or
`TIME`-driven input), under one street lamp at night through the lamp globals, with the lamps on
and off, each with `63b401b9`'s parent's `terrain_surface.gdshaderinc` and with the tip's, twice:
every pair of runs bit-identical, and before = after bit for bit in all eight images; the lamp
lights 13,736 pixels of the base cells (up to 43 levels; on against off), and Godot printed the
tangent warning 4 times a run before and 0 after. The `PlaneMesh` capture shows no lamp light (the
lamp's cone misses its surface), so only the base cells, the case the fix touched, prove the
light. **Why the reference renders differ run to run:** in two runs of one build
(`h3/references-before`, `-before2`) the plan views differ only on one pixel row at a tile or
patch seam (up to 13 levels, 94-192 pixels), the colour of whichever mesh rasterises the seam's
pixels first, which follows a draw order that differs between runs; the Kuopio grass view differs
over its grass (427,000 pixels, up to 50 levels), the grass's state when captured. Neither reads
`TIME`; the preview captures avoid both (no grass, one unshaded material).

**Section 4: the runs.** The Method without the FPS cap (VSync on, `MAX_FPS=0`), the user's
display (DP-2, 3840x2160 at 120 Hz, before and after every batch), Xwayland (N) and native
Wayland (W), each step holding `.run.lock` and the exclusive timing lock, P-cores, through
`display-awake.sh` (the user allowed the display today and skipped the 18:00 backup).
- *Before the fixes* (17:41-17:58, the tip's library `dca30744`, `h4/display-ready` batch 1):
  paused rows N 84.3 % (3,277 of 3,889 sampled poses exact within 16.7 ms), W 89.5 % (3,537 of
  3,953); stationary p95 12.0-24.4 ms, stop → exact 8.0-17.5 ms; populated 100k in the steady
  state (question 64 (c)): x11 A 43.3 % (499 of 1,153), P 84.3 % (668 of 792), Wayland A
  44.6 % (526 of 1,180), P 77.8 % (609 of 783); the steady wait before `populated_t` 8.9-9.1 s,
  318-324 frames, 256-263 of them over 16.7 ms, the longest 46.5-47.8 ms (the streaming window,
  reported apart), `populated_branch` 3 frames; 0 tangent warnings in every run (round 3:
  31-1,031 a run).
- *On T61 (1) (a)'s tip* (19:09-19:28, library `f07afeb2`, `h4/display-ready` batch 4; batch
  2 on T60's library was stopped at 18:37 and is discarded, batch 3 never started): paused rows
  N 87.6 % (3,440 of 3,926), W 88.9 % (3,519 of 3,959); stationary p95 up to 21.5 ms (max 28.6), stop → exact 5.7-17.5 ms; populated 100k in the
  steady state: x11 A 42.6 % (487 of 1,143), P 82.7 % (653 of 790); Wayland A 44.2 % (511 of 1,157), P 87.8 % (696 of 793); the steady wait
  before `populated_t` 8.9-9.4 s (317-335 frames, 256-268 over 16.7 ms, the longest 46.6-51.5 ms),
  `populated_branch` 3 frames; 0 tangent warnings. The profile (N W) ran on this tip too
  (`pacing-profile-windowed-*` of 19:27). Every row is carried (question 65).
- *Timing session 1* (18:02-18:08, before the fixes; the commit and bitwise rows stand for the
  tip, the fixes not touching the commit path): C = H1's F (`a880ce56`), F = the tip's Rust
  (`9f9cb9ba`). Commit (click → acknowledged): every sample of every run under 16.7 ms, at most
  14.81 ms (F, 100k); firsts at 100k C 11.96 / 13.93, F 14.81 / 13.77 ms. `COMMIT_PRODUCTS`
  identical in all four runs; the sixteen locality product sets equal.
- *Timing session 2* (18:30-18:34, T60's display with the whole move selection): F
  (`abe995d8`) against C (`9f9cb9ba`): graded stage p50 +0.33-0.38 ms; populated drags 1 and 3 of
  84 pose frames over 3.0 ms (C 0 and 0), which the selection over the displayed tiles cut;
  `COMMIT_PRODUCTS` equal to session 1's.
- *Timing session 3* (18:42-18:47, T60's display with the move selected among the displayed
  tiles, `87bf539c`): populated drags F 0 and 0 of 84 pose frames over 3.0 ms (max 2.47, 2.99),
  the pose's selection p50 0.64-0.71 ms against C's 0.58-0.59.
- *Timing session 4* (19:04-19:09, T61 (1) (a), F = `h4/fin-release-test-bin` of the final
  Rust, C = `9f9cb9ba`, the tip before the fixes; stages and drags C F F C): preview stages'
  graded stage p50 C 1.38-1.64, F 1.48-1.84 ms, p95 C 2.01-2.10, F 2.33-2.69 ms; populated
  drags: F 1 of 168 pose frames over 3.0 ms (3.84 ms: 10k, the first sample's second pose, one
  tile's bake 2.61 ms beside a 1.23 ms selection), catch-up within 3.0 ms (max 2.80); the
  control in the same session 1 pose frame (9.27 ms, 100k) and 1 catch-up frame (4.34 ms) over;
  the pose's selection p50 C 0.55-0.60, F 0.71-0.81 ms (the move's selection about +0.2 ms).

**Acceptance against every target** (fresh in this round; the display rows on T61 (1) (a)'s
tip):

| Target | Result | Status |
| --- | --- | --- |
| Moving, simulation paused (every sampled pose exact within 16.7 ms) | N 87.6 % (3,440 of 3,926), W 88.9 % (3,519 of 3,959) | Carried whole to `TERRAIN-13` (the user: 17:30 the GDScript share, 19:15 question 65 (a) the rest) |
| Moving, simulation active (at least 99 %, misses attributed) | 100k, steady state: x11 A 42.6 % (487 of 1,143), P 82.7 % (653 of 790); Wayland A 44.2 % (511 of 1,157), P 87.8 % (696 of 793) | Carried to `TERRAIN-13` and `RENDER-30` (the user, question 65 (a) with (d), 19:15) |
| Stationary (p99 and stop → exact under 16.7 ms) | p95 up to 21.5 ms (max 28.6), stop → exact 5.7-17.5 ms | Carried whole to `TERRAIN-13` (question 65 (a), 19:15) |
| Commit (click → acknowledged under 16.7 ms) | every sample 0-100k, at most 14.81 ms (session 1) | Pass |
| Locality (products bitwise, flat) | sixteen product sets equal; topology flat (`ROAD-72`) | Accepted as measured by the user (round 2) |
| The terrain stage's 3.0 ms (pose and catch-up frames) | session 4: F 1 of 168 pose frames over (3.84 ms, a 2.61 ms one-tile bake beside the selection), the control 2 frames over in the same session; catch-up within | Miss for the user: a heavy single-tile bake (T52's class) plus the move selection's +0.2 ms p50 |
| Bitwise preview = commit = undo = load | `COMMIT_PRODUCTS` equal in sessions 1 and 2; the graded drag test (commit, undo, load equal fresh bakes) passes; tangent captures bit-identical | Pass |
| No frame shows a hole or stale terrain while a preview moves (`TERRAIN-12`) | the road-shaped holes and seam cracks gone; 25 background frames on 9 of 39 Kuopio strokes (undo frames, existing-road junction poses, fast clicks) | Open: T61 (2) and T62 (a) decided (18:50, 19:15), to build and prove in round 5 |

**Carried** (each with its decision): carried items 1 (accepted by the user, round 3) and 2-15
as rounds 1-3 judged them; the paused, stationary and active rows' GDScript share to
`TERRAIN-13` (the user, about 17:30) and, by question 65 (a) with (d) (about 19:15), the
paused and stationary rows whole and the active row to `TERRAIN-13` and `RENDER-30`;
`TERRAIN-13` also holds question 64 (b). Left for the gate: T61 (2) and T62 (a) (decided, to
build and prove), and the terrain stage's one populated pose frame over 3.0 ms (for the user).

**Gate checks** (fresh, on the final Rust): `cargo clippy --all-targets` 59 warnings, the
baseline's set exactly (file and message); `cargo doc` 0 missing docs; the graded stage tests
pass (and two fail with the display made partial again); `./run.sh --test` with `DISPLAY` and
`WAYLAND_DISPLAY` unset (19:28-19:47, the code of `11118414`): exit 0, the debug library suite
1,846 passed, 0 failed, 177 ignored, `growth_scenario` 4 passed, every Godot suite passes with the
eight rendered tests in gamescope's headless backend, nothing skipped, no `SCRIPT ERROR`. **Line counts** (production,
from `git diff --numstat`): `roadbed/` 9,234 -> 9,267 (`terrain_changes_from`), `terrain/graded/`
10,169 -> 10,180 (`filtered`, `select_road_edit_where`), `terrain/` 12,391 -> 12,402;
`graded_stage.rs` +about 120 (the display rule). **Merge risks:** as round 3 (Linux and the RX
7900 XTX only, macOS with `TERRAIN-10` open, the non-compiling rebased WIP commits between
`3b1e587b` and `8bfd9ebf`, Godot's Wayland exit hang); the preview's display rule changes what
the player sees while dragging (terrain only on a completed selection; T62). **Not the gate
commit**: T61 (2) and T62 (a) are to build and prove (the user's order for round 5: T61 (2),
T62 (a), the 0-background probe proof, the display rows on the final tip, docs, gate checks).

#### Round 5: the undo hold, the junction hold, the runs on the final tip (`ROAD-68-H`, 2026-10-29)

Fresh in this round unless marked; artefacts `$ARTIFACTS/road68/h5/`. **The user's decisions
recorded this round:** session 4's terrain stage, F 1 of 168 populated pose frames over 3.0 ms
(3.84 ms: a 2.61 ms one-tile bake beside a 1.23 ms selection), accepted as measured (about
19:55; [T63](terrain.md#terrain-07-open-questions)), in T52's and T58's class, heavy single-tile
bakes to the new `TERRAIN-14`; it covers that measurement only. From the user's discussion of the
evening, three roadmap rows: `TERRAIN-14` (a cheaper bake first, then asynchronous staging),
`ROAD-73` (a ghost style for the hidden preview, [D9](#product-decisions)) and `UI-09` (the main
menu's scene change inside its dialog handlers).

**T61 (2): the undo hold, built.** Rust keeps the graded edit journal's latest serial in a shared
atomic (`get_graded_edit_serial`, no core lock) and hands the edits over in serial order (one
edit selection on a payload worker at a time); the graded renderer reports the serial whose tiles
draw (`installed_edit_serial`). A road chunk install made while the journal is ahead of the
renderer keeps the chunks it replaces drawn and its own hidden until the renderer has installed
the edits up to that serial; `NetworkRenderer` swaps them in the edit batch's frame (after the
terrain's process). Nothing waits in Rust: the commit's acknowledgement and the generation's
install are unchanged (the commit row below). The simulation has no redo; road deletion shares
the install path and the hold (covered by the code, no probe step deletes a road). Details in
[T61](terrain.md#terrain-07-open-questions).

**T62 (a): the junction hold, built; it doubles and z-fights.** Rust marks every pose and
catch-up frame with `graded_displayed` (the display holds the pose's whole selection); until
then the road tool keeps the pose's replaced published runs drawn beside its overlay. The
junction frames' background is gone, but where the held runs and the new pieces overlap (the
junction) both draw at the same heights, triangulated differently: the two asphalts alternate in
saw-tooth patches, the new zebras are partly covered and the old curb runs across the new
branch's mouth (`h5/zfight-compare.png`, the populated T at 40 m, a held catch-up frame beside
the same pose displayed). **The exact-pose measure** (the pose's road payload installed in its
frame) still holds as defined, since the overlay is installed in the pose's frame, but such a
frame also draws the replaced runs, which the measure does not see: 1,724 of Kuopio's 6,039
probe frames and 116 of the populated swing's 282 drew held runs. Back to the user as
[T64](terrain.md#terrain-07-open-questions) (1), not worked around.

**The 0-background proof** (gamescope, the round 5 library `h5/lib`, the probe's swing with
pauses and every per-frame check, `METRUM_GRADED_DRAG_CONTINUE`): the user's dump 0 of 416
frames; the 100k populated swing 0 of 282; Kuopio's 39 strokes 1 of 6,039 frames (`kuopio_06`
attempt 17, the fast click's batch frame, a 4-pixel slit, the same frame as round 4: a newly
graded committed tile waiting for its list beside an installed one; [T64](terrain.md#terrain-07-open-questions)
(2)); the renderer's invariant and the no-road check 0 in every frame; 26 frames drew with chunks
held, none with the background. **Fails without:** the same library with both holds switched off
in the frozen scripts (`h5/mut/`) on round 4's failing cases: 22 frames on 6 strokes (the undo
frames of `kuopio_01` 1 and 2, `_03` 8, `_05` 12, `_07` 19 and 20, 406-8,188 sampled pixels; 17
junction frames on `kuopio_03` 8). The 14 fast clicks without a batch frame are T62's accepted
lag (the probe's own expectation still reports them).

**The junction preview test** asserted the collapsed runs in the pose's own frame; with T62 (a)
the runs collapse once the pose's selection displays, so the test now runs the pose's catch-up
frames first (`_converge_preview`) and checks that the runs stay drawn until then. Its 43
captures on the final library, so converged, are pixel-identical to round 4's (`h5/captures-final2`;
the 13 fixtures' 52 graded preview tiles bitwise); without the convergence 11 preview captures
showed the held runs (`h5/captures-final`, the test's old frame).

**Timing session** (20:26-20:33, `h5/session/`, the user's all-clear; C = round 4's final Rust
`h4/fin-release-test-bin`, F = this round's `h5/fin-release-test-bin`; stages, breakdown, drags
and locality C F F C; the journal path changed, so the breakdown and bitwise rows ran; every
breakdown city `loaded`):
- Preview stages, graded stage p50 C 1.77-1.78 / 1.50-1.54, F 1.79-1.80 / 1.52-1.55 ms
  (`sloped_t` / `sloped_branch`); total p50 C 2.83-2.84 / 2.44-2.54, F 2.86-2.87 / 2.52-2.57 ms.
- Commit, click → acknowledged, every sample 0-100k: C at most 13.05 ms, F at most 12.01 ms; the
  100k first samples C 13.05 / 10.86, F 12.01 / 10.72 ms. `COMMIT_PRODUCTS` identical in all
  four runs; the locality products identical in all four runs.
- Populated graded drags: F 1 of 161 parsed pose frames over 3.0 ms (run 3: 9.67 ms, 100k sample
  1 pose 3, its selection 8.60 ms while the previous commit's lot adoption and routing adoption
  ran on the pool, carried item 10's and T57 (d)'s contention); catch-up within 3.0 ms (max
  2.91); C 0 of 168 (max 2.55); the pose's selection p50 C 0.70, F 0.67-0.68 ms. One of run 2's
  twelve rows printed a malformed line (interleaved output) and is not counted. **A miss on the
  final tip, larger than T63's accepted 3.84 ms, for the user** (not accepted by T63).

**The display batch** (20:32-20:50, the final library `h5/lib`, `h5/display-ready`; the Method
without the FPS cap, DP-2 3840x2160 at 120 Hz before and after, each step holding `.run.lock`
and the exclusive timing lock, through `display-awake.sh`; all carried by question 65 (a) with
(d), not passed): paused rows N 82.9 % (3,238 of 3,907 sampled poses exact within 16.7 ms), W
87.3 % (3,446 of 3,947); stationary p95 up to 25.31 ms (max 30.08), stop → exact 7.56-22.25 ms;
populated 100k in the steady state: x11 A 50.3 % (591 of 1,175), P 84.1 % (659 of 784), Wayland
A 43.6 % (505 of 1,159), P 83.1 % (660 of 794); 0 tangent warnings; the profile (N W) ran. The
paused N row is lower than round 4's 87.6 % on T61 (1) (a)'s tip (W 88.9 -> 87.3 %); not
attributed this round (the row is carried whole to `TERRAIN-13`); the Wayland populated step's
first run ended by the watchdog after its metrics (Godot's Wayland exit hang, as in round 4).

**Acceptance against every target** (the final tip; fresh in this round unless marked):

| Target | Result | Status |
| --- | --- | --- |
| Moving, simulation paused (every sampled pose exact within 16.7 ms) | N 82.9 % (3,238 of 3,907), W 87.3 % (3,446 of 3,947) | Carried whole to `TERRAIN-13` (the user: 17:30 the GDScript share, 19:15 question 65 (a) the rest) |
| Moving, simulation active (at least 99 %, misses attributed) | 100k, steady state: x11 A 50.3 % (591 of 1,175), P 84.1 % (659 of 784); Wayland A 43.6 % (505 of 1,159), P 83.1 % (660 of 794) | Carried to `TERRAIN-13` and `RENDER-30` (question 65 (a) with (d), 19:15) |
| Stationary (p99 and stop → exact under 16.7 ms) | p95 up to 25.31 ms (max 30.08), stop → exact 7.56-22.25 ms | Carried whole to `TERRAIN-13` (question 65 (a), 19:15) |
| Commit (click → acknowledged under 16.7 ms) | every sample 0-100k, F at most 12.01 ms (C 13.05) | Pass |
| Locality (products bitwise, flat) | the locality products identical in all four runs; topology flat (`ROAD-72`) | Accepted as measured by the user (round 2); products pass |
| The terrain stage's 3.0 ms (pose and catch-up frames) | populated drags F 1 of 161 pose frames over, 9.67 ms (a selection under the previous commit's adoption jobs); catch-up within (max 2.91); the control 0 of 168. Round 4's 3.84 ms frame accepted as measured (T63, 19:55) | Miss for the user (larger than T63's accepted frame; carried item 10's and T57 (d)'s contention) |
| Bitwise preview = commit = undo = load | `COMMIT_PRODUCTS` identical in all four runs; the debug suite passes (the graded drag test checks `display_complete` against the selection in every frame) | Pass |
| No frame shows a hole or stale terrain while a preview moves (`TERRAIN-12`) | dump 0 of 416, 100k swing 0 of 282, Kuopio 1 of 6,039 (a fast click's batch frame, 4 sampled pixels); the undo's and the junction's frames gone; but T62 (a)'s held runs double and z-fight with the new pieces | Open for the user: [T64](terrain.md#terrain-07-open-questions) (1) and (2) |

**Carried** (each with its decision): carried items 1 (accepted, round 3) and 2-15 as rounds 1-3
judged them; the paused and stationary rows whole to `TERRAIN-13`, the active row to
`TERRAIN-13` and `RENDER-30` (question 65 (a) with (d)); `TERRAIN-13` also holds question 64 (b);
round 4's 3.84 ms terrain stage frame accepted as measured (T63). Open for the user: T64 (the
held runs' z-fighting and doubling; the fast click's crack) and the 9.67 ms drag pose frame.

**Gate checks** (fresh, on the final code): `cargo clippy --all-targets` 59 warnings, the
baseline's set exactly (diff 0); `cargo doc` 0 missing docs; `./run.sh --test` with `DISPLAY` and
`WAYLAND_DISPLAY` unset (20:50-21:09): exit 0, the debug library suite 1,846 passed, 0 failed,
177 ignored, `growth_scenario` 4 passed, every Godot suite passes (23, the rendered ones in
gamescope's headless backend), nothing skipped, no `SCRIPT ERROR`. **Line counts** (production, from `git
diff --numstat` against round 4's tip): `roadbed/` unchanged (9,267, round 4's count),
`terrain/graded/` unchanged; Rust +about 40 (`graded_edits.rs` +12, `graded_tiles_api.rs` +19,
`graded_stage.rs` +7, `preview.rs` +6, `simulation_node.rs` +4); GDScript +about 120
(`network_tool.gd` +63, the chunk hold; `road_tool.gd` +28; `road_preview_display.gd` +20;
`terrain_graded_tiles.gd` +14; the probe +14). **Merge risks:** Linux and the RX 7900 XTX only,
macOS with `TERRAIN-10` open (the graded patches draw nothing there; unvalidated); the
non-compiling rebased WIP commits between `3b1e587b` and `8bfd9ebf` (G10's rebase; bisect skips
them); Godot's Wayland exit hang; a chunk install held for a graded edit waits as long as that
edit's batch (a batch that never installs would keep the old road drawn); every accepted and
carried miss above. **Not the gate commit:** T64 and the 9.67 ms frame wait for the user.

#### Round 6: the user's T64 change, T65, the runs on the final tip (`ROAD-68-H`, 2026-10-29)

Fresh in this round unless marked; artefacts `$ARTIFACTS/road68/h6/`. **The user's decisions
recorded this round** ([T64](terrain.md#terrain-07-open-questions)): about 21:20, (1) (a) the
stencil-tested held mesh with (b) as the fallback, (2) (a) the fast click's batch waiting for its
new tiles' lists, and the 9.67 ms populated drag pose frame not accepted but carried to the new
`PERF-12` (frame-critical stages share the Rayon workers with background commit jobs; a separate
owned pool for frame work or yielding background jobs, after `ROAD-68`); about 21:45, before
either (a) landed, **(1) (b), the junction hold dropped; (2) carried, not built; T61 (2), the undo
hold, kept**, with a new root-cause question, [T65](terrain.md#terrain-07-open-questions): the
hand-over fixes T60-T64 are small fixes for one coupling (the preview road updates in its frame,
the graded terrain lags by T27 (d), and the terrain discards its own baked road footprint), so
the junction frames and the fast click's crack are carried there, not accepted. Both (a)s had
been built on the branch (a stencil prototype whose rendered test showed no doubled pixel over a
coplanar or nearer held quad in either submission order, and the batch wait) and were discarded
unmerged. The user also asked for comparison clips of the remaining frame classes
(`h6/clips/`, below).

**T62 (a) removed.** The road tool no longer holds a pose's replaced runs: `road_preview_display.gd`
`hold_runs` and `_targets`, the road tool's `_graded_displayed`, and what fed them in Rust
(`GradedPreview::display_complete`, the pose's and catch-up frame's `graded_displayed`, and the
graded drag test's check of it) are gone; the probe's `runs_held` row and its held captures go
with them. The preview is round 4's united junction again: the junction preview test's 43
captures are pixel-identical to round 4's (`h6/captures-final` against `h4/captures-final`; the 13
fixtures' 52 graded preview tiles bitwise), and the test is round 4's again apart from T61 (2)'s
recorder stub. The probe gains `METRUM_GRADED_DRAG_CLIP` (every frame saved, for the clips).

**The probe on the final tip** (gamescope, the final library `h6/lib`, the swing with pauses and
every per-frame check, `METRUM_GRADED_DRAG_CONTINUE`): the user's dump 0 of 416 frames with the
background; the 100k populated swing 0 of 282; Kuopio's 39 strokes 17 of 6,038 frames on two
strokes, both carried to T65: `kuopio_03` attempt 8, 16 frames (the junction rebuilt by a pose:
3 sweep, 6 catch-up and 2 pose frames at 167 sampled pixels while the pose's selection has not
displayed, and the fast click's commit and wait frames at 22, the preview held through the
commit's hand-over over the same gap; round 4's mutation count was 17) and `kuopio_06` attempt
17, 1 frame (the fast click's batch frame, the 4-pixel crack, as in rounds 4 and 5). **The undo
frames 0** (T61 (2); 27 frames drew with chunks held); the renderer's invariant, the patch holes
and the no-road check 0 in every frame. The 14 fast clicks without a batch frame are T62's
accepted lag (the probe's own expectation still reports them).

**The comparison clips** (`h6/clips/`, the user's request of 21:35, revised at 21:45; gamescope,
every frame, isolated profiles): the Kuopio junction stroke (`kuopio_03` attempt 8), an undo
(`kuopio_01` attempt 1) and the fast click (`kuopio_06` attempt 17), each with (1) no holds (round
5's scripts with both holds off in the frozen scripts, the round 5 library) beside (2) the final
tip, at real speed and then at about 8 fps; `h6/clips/README.txt` says what each shows.

**Timing session** (22:22-22:29, `h6/session/`, `timing-queue/h6.req`; C = round 5's final Rust
`h5/fin-release-test-bin`, F = this round's `h6/fin-release-test-bin`, whose only Rust change is
T62 (a)'s feed removed; F's cities prepared first; stages, breakdown, drags and locality C F F C;
every breakdown city `loaded`):
- Preview stages, graded stage p50 C 1.75-1.84 / 1.56-1.61, F 1.79-1.85 / 1.53-1.59 ms
  (`sloped_t` / `sloped_branch`); total p50 C 2.81-2.94 / 2.63-2.69, F 2.85-2.94 / 2.56-2.72 ms.
  One F line was interleaved with a log line and read by hand.
- Commit, click → acknowledged, every sample 0-100k: C at most 14.05 ms, F at most 13.79 ms; the
  100k first samples C 10.53 / 14.05, F 13.79 / 13.34 ms. `COMMIT_PRODUCTS` identical in all four
  runs; the locality products identical in all four runs.
- Populated graded drags: F run 2 0 of 77 pose frames over 3.0 ms, one catch-up frame at 3.01 ms
  (10k, a 3.01 ms bake, no background job); F run 3 2 of 84 pose frames over: 10.23 ms (30k,
  sample 0, pose 5: selection 3.94 ms, bake 6.29 ms) and 4.30 ms (100k, sample 2, pose 2: bake
  3.70 ms), both while the previous commit's lot adoption and routing adoption jobs ran on the
  pool; C run 1 0 of 84 pose frames over, one catch-up frame at 3.11 ms under the same jobs; C run
  4 0. The pose's selection p50 C 0.52-0.54, F 0.52-0.56 ms. One of F run 2's rows printed a
  malformed line (interleaved output) and is not counted. **A miss on the final tip for the
  user**, not accepted: the frame class the user carried to `PERF-12` at 21:20 (pool contention
  with background adoption jobs) together with heavy one-tile bakes (T63's class, `TERRAIN-14`);
  the change from round 5 removes one boolean from the pose's dictionary and cannot reach either
  stage, and the control showed the same class in a catch-up frame. The catch-up frame was later
  accepted as measured by the user ([question 66](#road-68-open-questions) (a), 2026-10-30).

**The display batch** (22:29-22:47, the final library `h6/lib`, `h6/display-ready`; the Method
without the FPS cap, DP-2 3840x2160 at 120 Hz before and after, each step holding `.run.lock` and
the exclusive timing lock, through `display-awake.sh`; carried by question 65 (a) with (d), not
passed): paused rows N 87.0 % (3,423 of 3,934 sampled poses exact within 16.7 ms; the two N runs
86.2 % and 87.8 %), W 90.1 % (3,565 of 3,958); stationary p95 up to 24.92 ms (max 38.20), stop →
exact 9.03-23.47 ms; populated 100k in the steady state: x11 A 46.4 % (543 of 1,169), P 82.5 % (649
of 787), Wayland A 43.9 % (508 of 1,156), P 85.2 % (674 of 791); 0 tangent warnings; the profile
(N W) ran (the Wayland step ended by the watchdog after its metrics, Godot's Wayland exit hang).
**Round 4's control row in the same batch** (6c174e0d's scripts, round 4's library `h4/lib`, one
paused Xwayland run): N 88.6 % (1,742 of 1,966). Round 5's 82.9 % against round 4's 87.6 % is so
attributed to the junction hold (its held runs and its per-frame GDScript, both removed this round,
not separable now): without it the final tip's N runs are 86.2 % and 87.8 %, round 4's code 88.6 %
beside them, a gap within the final tip's own run-to-run spread (1.6 points).

**Acceptance against every target** (the final tip; fresh in this round unless marked):

| Target | Result | Status |
| --- | --- | --- |
| Moving, simulation paused (every sampled pose exact within 16.7 ms) | N 87.0 % (3,423 of 3,934), W 90.1 % (3,565 of 3,958); round 4's code N 88.6 % in the same batch | Carried whole to `TERRAIN-13` (the user: 17:30 the GDScript share, 19:15 question 65 (a) the rest) |
| Moving, simulation active (at least 99 %, misses attributed) | 100k, steady state: x11 A 46.4 % (543 of 1,169), P 82.5 % (649 of 787); Wayland A 43.9 % (508 of 1,156), P 85.2 % (674 of 791) | Carried to `TERRAIN-13` and `RENDER-30` (question 65 (a) with (d), 19:15) |
| Stationary (p99 and stop → exact under 16.7 ms) | p95 up to 24.92 ms (max 38.20), stop → exact 9.03-23.47 ms | Carried whole to `TERRAIN-13` (question 65 (a), 19:15) |
| Commit (click → acknowledged under 16.7 ms) | every sample 0-100k, F at most 13.79 ms (C 14.05) | Pass |
| Locality (products bitwise, flat) | the locality products identical in all four runs; topology flat (`ROAD-72`) | Accepted as measured by the user (round 2); products pass |
| The terrain stage's 3.0 ms (pose and catch-up frames) | populated drags F 2 of 161 pose frames over (10.23 ms at 30k: selection 3.94, bake 6.29; 4.30 ms at 100k: bake 3.70), both under the previous commit's lot and routing adoption jobs, and 1 catch-up frame at 3.01 ms; the control 0 of 168 pose frames, 1 catch-up frame at 3.11 ms under the same jobs. Round 4's 3.84 ms frame accepted as measured (T63, 19:55) | Miss on the final tip for the user, attributed to the class carried to `PERF-12` (the user, 21:20: background jobs delaying a frame's stage) with heavy one-tile bakes (`TERRAIN-14`); the pose frames not accepted by the agent; the catch-up frame accepted as measured by the user ([question 66](#road-68-open-questions) (a), 2026-10-30) |
| Bitwise preview = commit = undo = load | `COMMIT_PRODUCTS` identical in all four runs; the debug suite passes; the 43 junction captures and 52 graded preview tiles bitwise against round 4 | Pass |
| No frame shows a hole or stale terrain while a preview moves (`TERRAIN-12`) | dump 0 of 416, 100k swing 0 of 282, the undo frames 0; Kuopio 17 of 6,038 frames on two strokes: 16 junction frames (`kuopio_03` attempt 8) and the fast click's 4-pixel crack (`kuopio_06` attempt 17); the renderer's invariant and the no-road check 0 | `TERRAIN-12` done for the hole classes fixed; the junction frames and the crack carried to [T65](terrain.md#terrain-07-open-questions) (the user, 21:45), not accepted. Round 7 (fresh, the same frames): plants on a committed road up to 41 frames after its click, the vegetation's own lag, not the undo hold; carried to T65 (the coordinator's rule; the user took the final tip's undo hold from the comparison clips, 2026-10-30) |

**Carried** (each with its decision): carried items 1 (accepted, round 3) and 2-15 as rounds 1-3
judged them; the paused and stationary rows whole to `TERRAIN-13`, the active row to
`TERRAIN-13` and `RENDER-30` (question 65 (a) with (d)); round 4's 3.84 ms terrain stage frame
accepted as measured (T63); the 9.67 ms frame class to `PERF-12` (the user, 21:20); the junction
frames and the fast click's crack to T65 (the user, 21:45), owned by `TERRAIN-12` and
`TERRAIN-13`.

**Gate checks** (fresh, on the final code): `cargo clippy --all-targets` 59 warnings, the
baseline's set exactly (diff 0); `cargo doc` 0 missing docs; `./run.sh --test` with `DISPLAY` and
`WAYLAND_DISPLAY` unset (22:47-23:07): exit 0, the debug library suite 1,846 passed, 0 failed,
177 ignored, `growth_scenario` 4 passed, every Godot suite passes (23, the rendered ones in
gamescope's headless backend), nothing skipped, no `SCRIPT ERROR`. **Line counts** (production,
`git diff --numstat` against round 5's tip): `roadbed/` unchanged (9,267), `terrain/graded/`
unchanged; Rust production -13 (`graded_stage.rs` -7, `preview.rs` -6 net), its test -6;
GDScript -30 (`road_preview_display.gd` -19 net, `road_tool.gd` -7, the probe -4 net with the clip
capture);
the junction test -14. **Merge risks:** Linux and the RX 7900 XTX only, macOS with `TERRAIN-10`
open (the graded patches draw nothing there; unvalidated); the non-compiling rebased WIP commits
between `3b1e587b` and `8bfd9ebf` (G10's rebase; bisect skips them); Godot's Wayland exit hang; a
chunk install held for a graded edit waits as long as that edit's batch (a batch that never
installs would keep the old road drawn); the junction frames and the fast click's crack (T65);
the drag frames under background jobs (`PERF-12`); every accepted and carried miss above. No
stencil state ships (the prototype was discarded), so no platform risk from it.

#### Round 7: plants on the road after the undo, the vegetation's own lag (`ROAD-68-H`, 2026-10-29)

Fresh in this round unless marked; artefacts `$ARTIFACTS/road68/h7/`; the final library is round
6's (`h6/lib`, no Rust change since). **The finding** (the coordinator, from round 6's clip
`k03a8`): the final-tip panel draws a tall pine and small plants on the road where the no-holds
panel draws none, in the undo's frame and in the converged commit's last frames.

**The check.** The drag probe's `METRUM_GRADED_DRAG_VEGETATION=<n>` (check-only, off by default)
adds a vegetation row to every recorded frame over the stroke's reach:
- the plants drawn on the drawn road: a drawn vegetation instance whose trunk stands inside a
  visible road chunk's triangles, held chunks included, the preview's overlays counted apart;
- the drawn plants against Rust's placement for the current state (`get_decorative_tree_patch`
  as the upload asks it): `extra` drawn and not placed, `missing` placed and not drawn;
- the stale and queued patches and the vegetation's upload queue.

After the converged click it records n more frames, and the last must equal Rust's placement with
no plant on the road, so a stroke whose vegetation never catches up fails. The per-frame counts
catch what the clip showed: with the undo hold switched off they count 32 plants on the converged
commit's road for 10 frames.

**Cause: the vegetation's own lag, not the hold** ([T65](terrain.md#terrain-07-open-questions),
the third layer). Rust's placement is current from the edit (`publish_network_surface` installs
the roadbed; a commit bumps the vegetation generations, `invalidate_vegetation_over`), and the
renderer's staleness sweep sees a commit the next frame, an undo when its terrain patches commit
with its chunks. But every vegetation patch of the touched
terrain patches queues at once (118-125 on `kuopio_03` attempt 8), a patch that already draws
behind every one that draws nothing (`vegetation.gd` `_queue_patch`, unchanged since 2026-10-17),
and the upload loop stops past 2 ms (`UPLOAD_BUDGET_USEC`): the queue took 38 frames to drain after
that stroke's converged commit, the stroke's own patches anywhere in it. The hold only decides
which road draws in its own frames.

| `kuopio_03` attempt 8, every frame captured | Frames with plants on the committed road | Converged after the converged click |
| --- | --- | --- |
| Final tip (the undo hold) | 1 (the fast click's wait, 50 plants) | 17 frames |
| Final tip, the hold switched off (`h7/mut`) | 13 (the converged commit's road 10 frames, 32 plants) | 20 frames |
| Round 6's no-holds variant (round 5's scripts and library) | 2 (the fast click's wait) | 19 frames |

Which variant shows the plants on the converged road depends on whether its vegetation followed
the undo before the commit (the hold-off run did, as round 6's final-tip run had), not on the hold.
**Kuopio's 39 strokes** (the game's frame pace, no capture; the final tip / the hold switched off):

| | Final tip | Hold off |
| --- | --- | --- |
| Frames with plants on a committed drawn road | 612 of 15,310 (5 held) | 602 of 15,310 |
| After the converged click: strokes, frames, the last from the click | 27, 535, at most 41 | 27, 524, at most 41 |
| The most plants on the road in one frame | 214 | 214 |
| Vegetation equal to Rust's placement after the click | at most 42 frames (median 17), all 39 | at most 42 (median 17), all 39 |
| The 240th frame after the click | none on the road, nothing extra or missing, all 39 | the same |

**Decision** (the coordinator's rule, the user's 21:45 direction, night delegation; for the user's
review): not caused by the hold and transient, so recorded under T65 with these counts; no code
change, the undo hold stays (T61 (2)). **Reviewed by the user, 2026-10-30:** from the comparison
clips they took the final tip (the right-hand panel, the undo hold), so the hold stays. Plants under a preview road are the same layer (the
vegetation follows only committed roads; 48-50 plants under the swing's poses on that stroke).

**The probe on the final tip** (gamescope, the final library, the swing with pauses and every
per-frame check, `METRUM_GRADED_DRAG_CONTINUE`; the vegetation check off): the user's dump 0 of
416 frames with the background, the 100k populated swing 0 of 282, Kuopio's 39 strokes 17 of
6,038, round 6's two classes carried to T65: `kuopio_03` attempt 8, 16 junction frames (3 sweep,
9 catch-up and 2 pose frames at 167 sampled pixels; the fast click's commit and wait frames at
22); `kuopio_06` attempt 17, the fast click's 4-pixel crack. The undo frames 0 (25 frames drew
with chunks held); the renderer's invariant, the patch holes and the no-road check 0 in every
frame; the 14 fast clicks without a batch frame are T62's accepted lag, as before.

**The junction captures** (gamescope, the final library): the junction preview test's 43 captures
pixel-identical to round 4's (`h4/captures-final`), its 13 fixtures' 52 graded preview tiles
bitwise; the test passes.

**The clips, regenerated** (`h7/clips/`, `README.txt` says what each shows): `k01a1` and `k03a8`
with their zoom, round 6's no-holds variant (round 5's scripts and library) beside the final tip,
every frame of the same probe runs as the table above, the first 60 frames after the converged
click added; and `k03a8-veg-zoom`, the final tip with the hold switched off beside the final tip,
the run that shows round 6's plants on the converged road (frames 92-101).

**Gate checks** (fresh): no production code changed; the probe's GDScript is compiled with the
main scene, so `./run.sh --test` ran with `DISPLAY` and `WAYLAND_DISPLAY` unset (00:15-00:32): exit
0, the debug library suite 1,846 passed, 0 failed, 177 ignored, `growth_scenario` 4 passed, every
Godot suite passes (the same set as round 6's run), nothing skipped, no `SCRIPT ERROR`. No Rust changed,
so Clippy, `cargo doc` and a timing session were not run (round 6's results stand for the same
Rust). **Line counts**: `roadbed/` and `terrain/graded/` unchanged; the probe +187 net (the
vegetation check); production code 0.

#### The final audit: the economy contract on the gate tip (`ROAD-68-H`, 2026-10-30)

Round 1's contract ran on round 1's build; rounds 3-6 changed Rust since, so the coordinator
re-ran it on the gate tip's release binary (the clean tree's build, the same artefact as round 6's
`$TEST_BINARY`) against round 1's control E (`road` `a0cfe039` with `ROAD-68-G` part 10's
harness), on the same cities: E F F E, `tools/perf_contract.py`, eight workers, P-cores,
00:41-00:46, the machine idle.

| Hour mean | E1 | F | F | E2 |
| --- | --- | --- | --- | --- |
| baseline 10k | 0.850 ms | 0.855 ms | 0.822 ms | 0.872 ms |
| baseline 30k | 4.250 ms | 4.138 ms | 4.322 ms | 4.321 ms |
| fiscal 10k | 6.091 ms | 6.140 ms | 6.052 ms | 6.083 ms |
| fiscal 30k | 26.829 ms | 26.040 ms | 26.456 ms | 25.727 ms |
| runner time | 71.7 s | 76.5 s | 76.3 s | 71.2 s |

Work counts identical in all four runs, households 5,311 and 15,180, no time limit fails, the
same seven work-count limits fail in E and F (question 20). Every F hour mean lies inside E's
spread, round 1's fiscal rise included (item 15: layout, by round 1's profile).

**The runner time** is new: every test's snapshot load is slower in F (30k 4.94-4.96 s against
E's 4.54-4.58, 10k 1.85 against 1.67-1.69), where round 1's F loaded as E did (4.54 s). The tick
is not affected. A profile of the 30k load (locating, not timing; round 1's F `3e662826` against
the gate tip) puts the whole difference in the save's reservation check
(`validate_reservations` -> `ZoningSystem::cells_overlap_road_points_world` ->
`FrameGeometry::rectangle_corners`, 6,424 -> 7,297 of 11,839 / 12,628 samples), whose source
(zoning, save, the roadbed's mesh, query and geometry) is unchanged since round 1; the gate tip
executes 2.6 % fewer instructions (69.24 G -> 67.43 G) in 7.9 % more cycles (25.15 G -> 27.14
G), with `rectangle_corners` now inlined into its caller. Identical work by the source and the
counts, so by [question 36](#road-68-open-questions)'s rule it is layout, recorded, not relaxed.
`ROAD-68`'s acceptance has no load-time target; reported for the user, not accepted by the
agent. The check is 55 % of a 30k load, so its hot loop is the place to look when load time is
next measured.

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
  attributes every miss. A stale or superseded result counts as a miss. The pose's terrain is
  exempt (the user, 2026-10-27): it may lag the pose's road by the frames T27 (d)'s catch-up
  takes, each frame's terrain work within its 3.0 ms stage, and must converge to the exact tiles
  (bitwise the commit's) once it has caught up; the Stationary target's exact frame likewise
  means the road's.
- **Stationary.** Input-to-frame p99 under 16.7 ms, and stop to exact frame under 16.7 ms.
- **Commit.** Click → committed generation acknowledged in under 16.7 ms, with no frame that shows
  neither the preview nor the committed road.
- **Locality.** With the edited neighbourhood held fixed, grow remote buildings, parcels, agents
  and roads from 0 to 100k. Local products stay bitwise identical and per-pose timings stay flat.
  Context refresh is reported separately.
- **Separate tracks.** A miss attributed to the CPU-bound render frame (`RENDER-27`, for example
  vegetation patch installs), to agent draw distance (`RENDER-28`) or to a region seen for the
  first time on a camera turn (`CODE-12`) is reported with its attribution and does not block a
  `ROAD-68` or `TERRAIN-07` gate (the user, 2026-10-28). A miss without such an attribution still
  blocks.

### Method

Reuse the schema-2 deadline collector, the gameplay road benchmark and the native fixtures.

- Run matched, unprofiled release processes with no overlapping builds or benchmarks, pinned to
  P-cores.
- Record hardware, settings, worker counts, extension hash and exact commands, and keep captures
  under `$ARTIFACTS`.
- `frame_post_draw` is a rendered boundary, not proof of physical presentation. Headless timings
  cannot establish rendered acceptance.
- Vsync on, no FPS cap (`METRUM_GAMEPLAY_BENCHMARK_MAX_FPS=0`; the user, 2026-10-29,
  [question 62](#road-68-open-questions) (d)). Report Godot's native Wayland driver
  (`--display-driver wayland`) beside Xwayland (the x11 driver). Rows before 2026-10-29 ran with
  the 60 FPS cap.

```bash
RUSTC_WRAPPER= cargo build --release --manifest-path rust/Cargo.toml
RAYON_NUM_THREADS=4 METRUM_DEBUG=0 METRUM_DEBUG_PERF=0 \
METRUM_GAMEPLAY_BENCHMARK_MATRIX=preview \
METRUM_GAMEPLAY_BENCHMARK_CASES=sloped_t,sloped_branch \
METRUM_GAMEPLAY_BENCHMARK_REPETITIONS=100 \
METRUM_GAMEPLAY_BENCHMARK_WARMUP_REPETITIONS=8 \
METRUM_GAMEPLAY_BENCHMARK_MAX_FPS=0 \
METRUM_GAMEPLAY_BENCHMARK_OUTPUT_DIR="$ARTIFACTS" \
METRUM_GAMEPLAY_BENCHMARK_RUN_ID=<fresh-run-id> \
./run.sh --benchmark-gameplay-roads
```

For the cross fixtures, set the cases to `flat_cross,sloped_cross` and use a fresh run ID. For
the Wayland report, run Godot with `--display-driver wayland` (as `$ARTIFACTS/road68/h2`'s
`display-pacing.sh` variant W does). The populated row: `populated_t,populated_branch` with
`METRUM_GAMEPLAY_BENCHMARK_SAVE_PATH` set to the commit breakdown's saved 100k city (its
simulation at `METRUM_GAMEPLAY_PREVIEW_SPEED`, default 1x).

**Commit breakdown** (`benchmark_populated_commit_breakdown`, [`ROAD-68-G`](#road-68-g--commit-consequences-inside-one-frame);
saved cities since [`ROAD-68-K22`](#saved-cities-for-the-commit-breakdown-road-68-k22-2026-10-26)).
Each populated tier loads its city from a snapshot keyed by the binary. Prepare every binary's
cities before the session, as a heavy run (`.run.lock` and the shared timing lock, not the
exclusive one), then time the breakdown under both locks:

```bash
# Once per binary, before the session: build, check and save the cities (about 4.5 min).
METRUM_PERF_CACHE="$ARTIFACTS/snapshots" METRUM_COMMIT_TIERS=10000,30000,100000 \
  RAYON_NUM_THREADS=8 METRUM_DEBUG=0 taskset -c 0,2,4,6,8,10,12,14 "$TEST_BINARY" \
  nodes::sim::core::populated_city::routing_benchmark::benchmark_populated_commit_prepare \
  --exact --ignored --nocapture
# In the session:
METRUM_PERF_CACHE="$ARTIFACTS/snapshots" METRUM_COMMIT_TIERS=0,10000,30000,100000 \
  RAYON_NUM_THREADS=8 METRUM_DEBUG=0 taskset -c 0,2,4,6,8,10,12,14 "$TEST_BINARY" \
  nodes::sim::core::populated_city::routing_benchmark::benchmark_populated_commit_breakdown \
  --exact --ignored --nocapture
```

- `COMMIT_FIXTURE` names each tier's source: `loaded`; `generated`, when no snapshot matched
  the binary's key and the run built, checked and saved one first (minutes at 100k, so prepare
  first); or `built`, for tier 0 and with `METRUM_COMMIT_CITIES=built`, which builds every city
  in the run as before `ROAD-68-K22`. `COMMIT_SETUP` times the untimed setup's phases.
- Since `ROAD-68-K24` the breakdown, like its prepare test, runs without the test build's
  checks (`contract::timing_mode`, question 50); compare it only with binaries that have it.
- `COMMIT_PRODUCTS` lists what two runs must share bitwise: the city's counts and road payload
  vertices, the cell store's entries without its empty chunk entries (`ROAD-68-K24`), the
  graded census and walls, each sample's lots and generated-cell digests,
  republished chunks, payload vertices, graded tile digest and keys, and the site removal's
  digest. `diff <(grep ^COMMIT_PRODUCTS a.log) <(grep ^COMMIT_PRODUCTS b.log)` compares two
  runs. Binaries from before `ROAD-68-K22` build their cities in the run and print the same
  digests in `COMMIT_BREAKDOWN` and `GRADED_TILES_COMMIT` only.
- A snapshot lives as long as its binary is in use; delete `commit_city_*` files with the
  binaries they were made for (about 280 MB a binary).
- Since `ROAD-68-G` part 8 the first frame after each commit (the one that adopts its lots)
  starts at once, the default (`METRUM_COMMIT_NEXT_TICK=once`), or one frame (16.667 ms) after
  the commit returned (`METRUM_COMMIT_NEXT_TICK=frame`), as the sim thread steps it since
  [question 58](#road-68-open-questions). `COMMIT_BREAKDOWN` splits the adoption's install
  (`lots_plan_install_ms`, `lots_reattach_ms`, `lots_repair_ms`) and lists every background job
  that ran in the sample (`jobs`: spawn site, start and end in ms since the click; the lots job
  is the first `lot_adoption.rs` one, the routing rebuild the second, or `road_commit.rs`
  when no lots were deferred).

**Commit locality** (`benchmark_commit_locality`, `ROAD-68-G` part 8). The zoned 800 m grid
with every lot built and its trunk to the border, in a fixed 12,288 m world, is the edited
neighbourhood; the breakdown's three branches off its trunk are the edits
(`profile_commits`, with its `COMMIT_BREAKDOWN` lines). East of it, beyond every lot region
and terrain patch the branches touch, each tier adds straight two-lane 480 m block streets fronted
by one-cell sites, each with a parcel and one housed agent (`METRUM_LOCALITY_TIERS`, default 0, 1,000,
10,000 and 100,000 sites). Each tier builds afresh outside timing (`LOCALITY_SETUP`, then the
breakdown's `COMMIT_SETUP`). The test asserts that `LOCALITY_PRODUCTS` equal tier 0's: each
sample's republished chunks and graded tiles, and the neighbourhood's cell claims, generated
cells, parcels and buildings, compared by geometry because everything the commits create takes
ids after the background's.

```bash
METRUM_COMMIT_PREVIEW=1 RAYON_NUM_THREADS=8 METRUM_DEBUG=0 \
  taskset -c 0,2,4,6,8,10,12,14 "$TEST_BINARY" \
  nodes::sim::core::populated_city::commit_locality::benchmark_commit_locality \
  --exact --ignored --nocapture
```

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
  segment stored by the marking, and after the lane surface sync their kerb ends keep the
  published kerbs' heights, on the carriageway (`ROAD-68-K20`, ["Footprint and
  terrain"](#footprint-and-terrain))
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
a node does not read as a kink. The angle between two arms (the junction's gaps between adjacent
arms, counter-clockwise in an exact order, and a degree-two node's kink) comes from their
directions' cross and dot products in `f64`, where each product of the `f32` components is exact,
so it is the same in every quarter turn and mirror of a junction, bit for bit (`ROAD-68-K23`).
Before, each arm's own `atan2` and their difference rounded differently in each orientation: the
reference 25° Y, exactly at the illegal limit, measured 25.000002° to 25.000023° in six of its
eight orientations and under 25° in the other two, which were refused (on `road` too); it now
measures 25.0 in all of them. The angle is compared in `f32`, as every validator measurement is.
`validate_edge` is `O(points)`, `validate_node` is
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
