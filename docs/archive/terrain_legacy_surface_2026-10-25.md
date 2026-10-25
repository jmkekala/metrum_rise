# Terrain — legacy terrain surface (archived 2026-10-25)

> **Archived, reference only.** These sections were moved verbatim out of
> [`terrain.md`](../terrain.md) and [`earthworks.md`](../earthworks.md) on 2026-10-25, when the
> user decided to rewrite the terrain surface core as `TERRAIN-07` with approach B (the base
> heightmap plus fine grading and footprint-distance tiles, an exact seam mask and road skirts)
> and declared the existing terrain surface code legacy. They describe that code: the source /
> visual buffer pair, the source-versus-visible query family, engineered ground as a visual
> derivation, the refined terrain patches built by a constrained Delaunay triangulation per 64 m
> tile (caches, composition, publication and payload rules), building pads in the patch buffers,
> the stitched-CDT seam contract and its verifications. The code still runs on `road` until
> `TERRAIN-07-G` deletes it, so these rules describe it until then; new work follows the live
> specification in [`terrain.md`, "Graded terrain surface (`TERRAIN-07`)"](../terrain.md#graded-terrain-surface-terrain-07).
> Do not update this file as live planning. Its measurements are evidence for the builds they
> name, not for the replacement.
>
> Mixed sections were split: their parts about water, base patch residency and streaming, and the
> Godot bridge stayed in `terrain.md`. Each block below names where it came from. Relative link
> targets were adjusted to this folder; the text is unchanged.

## From `terrain.md`

### Native terrain winding correction

*Moved from "Plot grass and frontage materials — RENDER-13".*

The initial plane-based checks missed the actual exported mesh orientation. Internal CDT
and regular terrain buffers use upward cross products; Godot front faces use clockwise
winding viewed from above. The exporter copied internal indices unchanged. With two-sided
rendering Godot therefore flipped the shading normals on the visible upper faces, leaving
frontage and the graded paving around the Food Processing Plant dark. Foundation/yard
meshes already used the correct winding, explaining the exact boundary. The materials
were selected correctly.

`godot_terrain_indices` reverses the second and third index of each face during the existing
`PackedInt32Array` export. Cached geometry, winding used by terrain queries, normal sums,
face order, material tags, positions and retaining-wall output remain unchanged. Cost remains
O(exported indices), only on payload publication, with no extra pass or intermediate buffer.
The iterator supplies an exact lower size hint, preserving PackedArray's single preallocation.

Fresh native verification on the rebuilt release: the extended material regression requests
an actual 80 m blank-world road preview, checks all exported faces, and renders one exported
flat face as paving against the yard reference. The old library fails with **904/904**
backward faces and daytime/nighttime RGB-vector differences **0.635 / 0.045**. The rebuilt
library passes with **0/904** backward faces and differences **0.000 / 0.0039** (tolerance
0.012). Existing material/light checks also pass. Rust winding test: 1 passed; CDT bridge
suite: 57 passed, 1 ignored; building-site terrain suite: 12 passed, 4 ignored. Release build
succeeds and is deployed; restart the game to load it. New library SHA-256:
`54ecc2f8dad9d5f3cd48134242883023dfa2d4cfdc1dc61ce1481c2a2650f108`.

Matched unprofiled iterator-copy microbenchmark: Rust 1.98.1, `rustc -O`, one thread,
preallocated destination, exact current helper extracted from source, old forward copy versus
new clockwise copy, two alternating pairs, 10 warm-up and 60 measured batches of 100 copies.
For 12,288 indices: old **0.928 / 1.032 µs**, corrected **4.001 / 1.281 µs**;
98,304: **5.448 / 5.431 → 10.005 / 9.983 µs**;
999,999: **132.457 / 140.237 → 147.808 / 150.013 µs**.
This isolates index iteration/copying; it does not measure full Godot upload or city frames.

Commands: `cargo test --manifest-path rust/Cargo.toml --lib
exported_terrain_faces_are_front_facing_from_above`, `cargo test --manifest-path rust/Cargo.toml
--lib nodes::simulation_node::tests::cdt`, `cargo test --manifest-path rust/Cargo.toml --lib
building_site_terrain`, `cargo build --manifest-path rust/Cargo.toml --release`; use the
material-test command above without benchmark flags for the native rendered check.

### 3. Terrain Uses Dual Sparse Buffers

*Moved from "Implemented Runtime Contract".*

The live `TerrainSystem` owns two sparse chunk-backed grids:

- `source terrain`: authoritative sculpted terrain
- `visual terrain`: derived terrain after engineered-ground earthworks

Current deterministic rules:

- untouched sparse terrain cells are implicit and read back as the configured base elevation
- setting terrain directly writes both source and visual buffers
- sculpting writes both source and visual buffers
- `reset_visuals_from_source()` discards the current visual terrain and clones the authoritative
  source terrain into it
- save/load persists the authoritative source terrain only
- renderer uploads use the visual terrain
- procedural hillshade is a render-only derivation generated from the uploaded visual terrain
  heightmap
- procedural hillshade must never become authored world data or save-game data
- terrain sculpting, DEM import, world load, and road-earthwork visual refreshes must all update
  hillshade automatically because it derives from the same visual terrain upload

### 4. Surface Queries Distinguish Source Terrain From Visible World Surface

*Moved from "Implemented Runtime Contract".*

Terrain-only height queries are authoritative against the source terrain surface, not the
engineered-ground-derived visual terrain. Separate world-surface queries read the current
engineered-ground client surface first and fall back to visual terrain only when no client-owned
surface owns the queried location.

Current deterministic rules:

- `get_height(x, y)` reads source terrain
- `sample_height_world(x, z)` reads source terrain
- `intersect_terrain()` and terrain height queries use source terrain interpolation in world space
- `get_world_surface_height()` returns visible client-owned surface height when an engineered-ground
  client owns the queried XZ location, otherwise visual terrain height
- `intersect_world_surface()` raycasts the visible client-owned surface first and falls back to
  visual terrain when no client-owned triangle is hit
- engineered-ground earthworks are a visual derivation, not an edit to source terrain

Editor interaction rule:

- authored-ground editing tools must use the terrain-only query family
- visible-surface placement, inspection, and selection tools must use the visible-world query
  family
- terrain-authoring tools must not implicitly move already placed engineered-ground client surfaces

This preserves the rule that engineered-ground placement must not feed back into the terrain that
grade and slope calculations treat as authored ground.

### 8. Engineered Ground Is A Chunk-Local Visual Derivation Step

*Moved from "Implemented Runtime Contract".*

Shared engineered-ground semantics now live in [`earthworks.md`](../earthworks.md). This document owns
the terrain-storage side of that boundary: source terrain stays authored ground, visual terrain is
the derived buffer, and only touched chunks are reset and restamped.

Current deterministic sequence:

1. compile or refresh the affected engineered-ground client surface inputs
2. prepare structural support writes for the touched visual terrain chunks
3. in canonical chunk order, reset each touched chunk from source terrain and apply its support
   writes before advancing to the next chunk (shared borders preserve this ordering)
4. leave untouched visual chunks and all source terrain chunks unchanged
5. rebuild dependent caches against the updated client state plus visual terrain

Current runtime client state:

- roads are the first live engineered-ground client
- grounded roads no longer stamp ordinary `Standard` footprints or margins into visual terrain;
  road-touched terrain patches receive stitched mesh topology from Rust
- grounded road-owned asphalt, shoulder / curb, and sidewalk footprints also provide exact clip
  polygons to terrain and water render patches so neither terrain nor water remains a visible carrier
  under the committed road footprint
- grounded roads use Rust-generated stitched terrain patch topology from the clipped footprint edge
  to nearby terrain, so ordinary `Standard` roads do not need a visible closure strip
- terrain-only queries still read source terrain, while visible-world queries use the client-owned
  surface first, structural local earthwork geometry second, and visual terrain third; ordinary
  grounded seams are terrain topology, not a separate road-owned query surface
- flat building/yard pads and graded edge paving use the same Rust-side stitched terrain patch model for local site
  tie-ins; future engineered-ground clients should extend [`earthworks.md`](../earthworks.md) instead
  of inventing a separate terrain-flattening path

Current deterministic editor rule:

- terrain authoring edits source terrain first
- after the source edit, touched engineered-ground clients rebuild derived terrain outputs; ordinary
  grounded roads regenerate CDT terrain patch meshes while structural clients may restamp visual
  terrain
- terrain brushes do not directly sculpt roadbeds, flat pads, or future local earthwork geometry

Remaining limitation:

- the fixed Kuopio workload passes 39 placements / 158 profile checks, shared-side and coverage
  regressions, and saved-reference settlement; broader authored-map coverage remains ongoing,
  not a claim of watertightness for arbitrary inputs (see `ROAD-24` in [`roads.md`](../roads.md))
- terrain density alone is no longer the target fix for road / terrain gaps
- the live road-touched seam path is the Spade CDT patch builder described in
  [`roads.md`](../roads.md):
  road footprint loops become hard constraints, terrain faces inside those loops are omitted, and
  road seam constraint edges are preserved exactly
- current grounded-road terrain editing must keep placed `Standard` road geometry fixed and rebuild
  derived terrain outputs around the committed roadbed instead of silently resynchronizing the road
  to later source-terrain edits

Authoritative rule:

- engineered-ground earthworks change the visual terrain only
- source terrain remains the authored ground surface

### 5. Chunk-Local Terrain / Water Rendering Is Now Live (refined-terrain rules)

*From "Implemented Editor / Rendering Slices", section 5. The patch residency rules before
these bullets, the Godot staging transaction between them and the water and bridge rules after
them stayed in `terrain.md`.*

Deterministic terrain-render rules (refined terrain):

- refined terrain divides each render patch into fixed world-aligned bounded CDT core tiles clipped
  to the parent patch; every tile uses exact indexed contributor and required grading-influence
  coverage rather than an assumed fixed-neighbor halo
- tile identity is separate from a full local content fingerprint covering clipped contours and
  provenance, local terrain samples, grading guides/constraints, render step, core bounds, and
  contract revision; unchanged fingerprints reuse immutable compiled geometry and per-tile render
  buffers from the last accepted generation
- each cached tile also retains its road-input clipping: ordered source loops shared across tiles
  and halo contributor loops/manifests. Canonicalization clips ownership to the core while retaining
  the uncut halo for grading shared sides. Reuse compares full geometry/provenance
  (including float bits), exact core bounds, and freshly sampled corner heights in
  `O(local contributor vertices + source edges)` without allocating on a hit. Halo clipping is
  skipped only after this match. Terrain samples, adaptive margins, road/site grading
  guides, and final input fingerprints are still rebuilt; source-generation equality alone cannot
  authorize reuse of visual-terrain inputs. Retention is bounded by cached tiles, with no history
  chain or independent global geometry cache.
- an edit plans `old_coverage union new_coverage` plus deterministic seam-dependency tiles; removed
  contributors omit their old refined coverage so regular terrain fills it again, while unrelated
  tile fingerprints and compiled meshes remain unchanged
- seam invalidation remains cardinal, but only tiles with current road/site contributors enter the
  CDT builder; contributor-free neighbors return to the regular filler, which takes the exact
  exported side vertices of each adjacent window, and contributor coverage/guide generation share
  one adaptive grading-margin probe result
- contributor margin/guide generation and tile-local clipping, sampling, and fingerprint generation
  run as ordered Rayon jobs; deterministic serial aggregation retains canonical tile and manifest
  ordering, keeping total work `O(contributors + windows)` while reducing its wall-clock critical path
- only changed tiles build through Rayon; completed tile results are sorted canonically before patch
  composition so scheduling order cannot change topology or shared seam identity
- road grading guides provide sampling positions, not independent parent-edge height authority.
  They sample the common visual terrain; interior samples inside the required tie-in envelope
  yield to the retained road seam. Shared-side samples remain, graded once against uncut halo
  contributors, so adjacent CDT tiles cannot bridge different elevations. Building pads and
  structural retaining walls retain their own authority. Canonical boundary-cell welding and
  strict core containment prevent sub-millimetre outside samples from expanding the CDT domain.
  Snap guide/source samples within the existing 1 mm side tolerance before testing containment:
  both adjacent tiles must retain the same side point and height, even when its original position
  falls just inside only one tile. More distant halo samples remain excluded.
  Guide points on road constraints are noded with the retained seam's interpolated height.
  Clipping also preserves exact rectangle intersections when a nearby interior road vertex
  occupies the same canonical identity cell. Deduplication prefers the vertex on the exact
  tile side, including the closing pair and reversed contours; it does not move arbitrary
  road vertices onto a boundary. Discarding that intersection previously produced a nearly
  vertical terrain face beside a flat orthogonal road block. The focused clipping regression
  and native four-road preview/commit fixture cover this case; see the
  [zoning reference verification](../zoning.md#native-road-transactions-and-rendered-references).
  Segment incidence tolerances are distances, never fractions of arbitrarily long segments.
  Noded-edge cleanup deduplicates canonical vertex IDs, not nearby segment parameters:
  even sub-millimetre intersections in distinct identity cells must remain on both incident
  boundaries. Otherwise a road/site corner can leave crossing constraints and suppress an
  entire render patch after loading. CDT contract revision `13` includes that noding fix,
  imported-bounds building support, symmetric shared-side sample welding and editor-consistent
  building-part yaw, invalidating earlier oversized/misrotated pads and one-sided apron products;
  Rust and Godot enforce the same revision. Invalid geometry still blocks publication, and both
  `terrain` and `road` debugging expose the renderer's rejection reason.
- render frames (`ROAD-68-D` part 5): a patch's vertices are local to its frame centre
  (`TerrainPatchSnapshot::frame_centre`, `render_frame_centre`), converted from `f64` world
  metres once by `render_frame_local`. The road render chunk over a patch uses the same centre
  and the same conversion, and each building site's flat pad is composed into the patch buffers
  (footprint triangulated in `f64`, clipped to the patch by `terrain/rect_clip.rs`, partitioned
  with the terrain faces by the site paving partition), so terrain, pads and roads give a shared
  seam vertex the same bits ([`roads.md`](../roads.md#footprint-and-terrain), open question 23).
  Godot places terrain patches and road chunks at those centres.
- CDT export retains world XZ in `f64` while calculating face normals, slope/pathology metrics,
  patch-local positions, and UV deltas; it converts only the resulting local values to Godot's
  `f32` buffers. Converting kilometre-scale world coordinates first can collapse a valid
  sub-millimetre road/site separation, inflate its measured slope, and falsely suppress the whole
  atomic patch. The saved benchmark-city regression covers that case at patch `(27, 0)`.
- successful tile output caches vertices, normals, UVs, offset-ready indices and pre-normalized
  local normal-sum magnitudes; reuse therefore skips triangle conversion and triangle-index normal
  reconstruction for unchanged tiles, and composition reads each window's side vertices from its
  exported vertices (`O(window vertices)`, as the copy already is)
- the regular filler is conforming (`ROAD-68-D` part 4, [`roads.md`](../roads.md#footprint-and-terrain)):
  filler cells are bounded by patch and window lines only; a cell side shared with a window takes
  that window's exported side vertices bitwise, every other cell side the render-step boundary
  lattice, and one strip per side joins it to the cell's regular grid. No side vertex runs on as
  a grid line into other cells, so there is neither a T-vertex nor a Cartesian filler-grid
  expansion. Only exactly
  degenerate faces are dropped from CDT and filler output, and face slopes are measured without
  absolute cut-offs (a small face is not vertical)
- atomic composition still copies the complete bounded render-patch mesh because Godot owns one
  surface per patch; concatenation, regular filler, duplicate-normal reconciliation, and upload
  remain `O(patch output vertices + indices)`, while contributor queries, CDT input assembly,
  triangulation, and tile conversion are proportional to changed plus deterministic seam tiles and
  never revisit every connected road in the patch
- refined patch publication is atomic for the exact requested generation; one failed tile, a failed
  road clip query, or missing road ownership on a road-owned patch suppresses the complete new
  payload, stale jobs cannot publish, and the immutable last accepted generation remains visible
  and available for reuse until a complete replacement is accepted
- road insertion also validates the production terrain payloads before accepting simulation state.
  The simulation thread borrows the staged world, rebuilds only affected patch/tile products, and
  stores successful buffers in the existing cache. A failure rolls back graph and split dependents
  before lane, entrance, parcel-maintenance, routing, or treasury changes. GPU staging remains a
  separate generation check; it is no longer the first place a terrain-CDT failure can reject a road.
- Default `ROAD-28` road-only previews display canonical road/junction geometry over unchanged terrain.
  They sample terrain for road heights but do not construct stamps, cutouts, grading meshes or
  paired terrain uploads, even while idle. A matching click shares the road solve and completes
  terrain under the simulation command before atomic validation/publication. Preview readiness
  is independent of terrain renderer residency; final placement keeps the clipping contract above.
  `ROAD-29` adds opt-in full road/terrain preview: compile the same local terrain products on
  the worker, stage both draw batches atomically, and restore original meshes on cancel,
  mode change or patch invalidation. This changes only temporary rendering; source samples,
  resident payload caches and acknowledgments remain unchanged. Matching clicks share the
  completed terrain plan after dependency revalidation.
- a statusless non-engineered payload may use the regular heightmap `PlaneMesh`. An engineered
  payload is renderable only when its current-contract final status is `ok` and its clipped baked
  buffers are structurally valid. Any omitted-pathological-face count rejects the complete patch
  in both Rust readiness and Godot staging: removing a bad face is not a coverage repair.
  `empty`, `failed`, `conflicted`, still-`pathological`, unknown,
  wrong-contract, or malformed engineered output keeps the previous mesh and may not fall back to
  raw terrain.
- a road-locked patch selected through the grading-ray safety pad expands its clip-source query by
  that same render-step / terrain-cell pad; bridge-to-ground transitions cannot mark a neighboring
  patch as road-owned while querying just short of the responsible road seam
- a road loop discovered only through that padded query contributes to a CDT tile only when its
  exact grading influence overlaps positive-area tile core coverage after parent-patch clipping;
  margin-only neighboring loops must not materialize phantom refined coverage
- engineered ownership travels with the Rust payload, and Rust must reject raw-heightmap payload
  requests for known road- or building-site-owned patches even if the renderer's patch-membership
  lookup is stale

Deterministic density gate (last item):

- the accepted road / terrain seam fix is not a density move; it is the Spade CDT terrain-patch
  hardcut in [`roads.md`](../roads.md)

#### Deterministic transition rules

*Moved from the same section 5.*

Deterministic transition rules:

- future engineered-ground closed local earthwork / tie-in geometry is required whether the
  implementation extends the current terrain runtime or rewrites it
- any terrain-runtime rewrite must still preserve:
  - authoritative source terrain
  - a derived far-field terrain surface outside engineered-ground tie-in boundaries
  - chunk-local invalidation and rebuild boundaries
  - the split terrain / water render-upload path
  - Rust-generated stitched terrain topology anywhere grounded `Standard` road top surfaces own the
    visible surface; the clip boundary is the compiled road-piece outer loop, and shader discard,
    alpha masking, internal road-band clipping, or Godot-side polygon clipping must not be the
    ordinary road seam carrier
  - terrain clipping must be derived from the compiled road-piece outer loop in Rust; asphalt /
    sidewalk render triangles are not reused as terrain ownership triangles
  - span terrain-query footprints are closed in 64 m section-station runs using existing owned
    regions and handoff sources. Internal caps cancel in the patch union; full render/earthwork
    outlines remain unchanged. This bounds exported source loops before the micrometre i32 overlay,
    fixing the 2,790 m `ROAD-14` overflow without reducing precision or relocating the fixture.
    Construction adds one linear boundary pass and O(regions) source storage, not a new spatial index.
  - road-piece and terrain-patch ownership cleanup uses `i_overlay` before triangulation; boolean
    union / difference / hole handling must produce non-overlapping asphalt, sidewalk, and terrain
    regions before Spade receives constraints
  - target road-touched patch generation uses Spade's Rust-side
    `ConstrainedDelaunayTriangulation` with a deterministic `try_bulk_load_cdt` input made from the
    terrain patch rectangle, road-owned footprint constraint loops, and deterministic
    source-terrain sample points outside those footprints
  - conflicting constraints are reported through CDT debug counters and skipped; they are treated
    as geometry bugs to fix at the road-piece source, not as a reason to panic the backend or fall
    back to legacy clipping
  - constraint incidence uses the micrometre road-contour resolution, not the millimetre
    identity grid as an overlap buffer. Distinct near-parallel approach boundaries must not be
    fabricated into overlapping constraints with conflicting heights. Real overlaps and height
    conflicts retain their existing checks; this does not widen height tolerances. The existing
    patch-local noding complexity is unchanged.
  - Spade CDT faces whose centroids are inside road-owned footprints are omitted; all emitted
    terrain triangles must preserve road seam constraint edges and must not cross road footprint
    loops
  - `ghx_constrained_delaunay` is not a terrain backend or fallback in this spec; Spade is the
    production hard-cut target because it gives the project a documented constrained triangulation
    API with exact geometric predicates
  - `robust` is not part of this path for now; standalone exact-predicate code is not needed unless
    a future measured gap remains after `i_overlay` boolean cleanup and Spade CDT
  - current road-touched patch emission uses the Spade CDT path directly; the old subtractive
    triangle cutter, visible seam strip, and conservative cell-triangle ownership rule are no
    longer live fallbacks
  - terrain render suppression for structural local earthwork geometry must remain bounded to true
    geometry overlap rather than acting as a substitute for missing tie-in faces; road-edge terrain
    topology must still be geometrically correct if terrain-side suppression is turned off
  - visible-world query precedence over client-owned top surfaces and closed local earthwork
    geometry
- post-placement terrain edits should rebuild earthworks around already placed client surfaces
  instead of resynchronizing those client surfaces to edited terrain

#### Road/site corner noding verification (2026-09-11)

`road_site_intersections_near_pad_corners_keep_distinct_vertices` uses an authored 32 m tile
with a sidewalk boundary crossing just inside two flat-pad corners. It fails with the old
split cleanup (two rejected constraints), and passes with identity-only deduplication. It
also checks complete exterior area, no missing constraints, level output, and exact geometry
agreement after contributor reordering. This fix keeps the existing tile-local complexity:
`O(K log K)` sorting and `O(K)` cleanup per edge with K split events, removing the extra
deduplication buffer. No city-wide query or per-agent work is introduced.

Fresh verification: `cargo test --release --lib` passes 1,656 tests (9 intentionally ignored);
headless `road_junction_preview_test.gd` and `network_tool_chunk_renderer_test.gd` pass.

Targeted unprofiled measurement uses the same fixture plus 1,089 regular source samples:
24 batches of 50 builds, four warmups, input cloning outside the timed compiler. Command:
`RAYON_NUM_THREADS=24 cargo test --release --lib benchmark_road_site_cdt_noding -- --ignored --nocapture`.
The production release test reports **0.4477 ms/tile**, 763 retained vertices, 1,369 faces,
and zero invalid/missing constraints. Test-binary SHA-256:
`614a167a46dc9fc3bf2c8ef5ca6820d0a156afd194126af9181733074e9cafcc`.

Three matched isolated CDT-module process pairs (`rustc 1.98.1`, `--edition=2024 --test -O`,
identical release Spade/R-tree/overlay dependencies, debug logging disabled, no concurrent
builds/tests) measure old/new medians **0.4816/0.4611**, **0.4828/0.4607**, and
**0.4842/0.4607 ms/tile**. The middle pair reverses process order. Each build runs the serial
tile kernel; outer Rayon scheduling and whole-patch upload are outside this measurement. The
old output has one invalid and three missing constraints, so these are correction-cost
measurements, not an equivalent-output speedup or a city-scale FPS claim. Matched binary
SHA-256 prefixes: `9c810e195d67e7d5` (before), `3952a9f86230267ee` (after).

#### Shared-side yard seam verification (2026-09-11)

A commercial apron exposed a 0.276727 m vertical T-junction across a 64 m tile side. A grading
sample less than 1 mm inside one tile snapped onto that side, while the neighbor rejected it
before snapping and retained an unsplit edge at a different height. Canonicalization now welds
samples first and enforces strict containment afterwards. This adds no allocations, spatial
queries or city-wide work: constant work per local source/guide sample, with existing CDT costs.
Flat pads and authored assets are unchanged; Rust/Godot contract revision 12 invalidates old meshes.

`adjacent_tiles_share_near_boundary_grading_samples_from_either_side` failed before the fix and
passes afterwards. Its 64 cases cover both axes, positive/negative large-world coordinates, both
offset directions and source/guide inputs. It compares referenced side vertices and heights;
the containment regression also proves that farther halo samples stay excluded.

Fresh release verification passes **1,659 Rust tests (9 ignored)**, including the 61 CDT tests,
plus headless `road_junction_preview_test.gd` and `network_tool_chunk_renderer_test.gd`. The
read-only loaded scene publishes all eight requested patches; its repaired point is referenced
by triangles on both sides at exactly the same height. A local rendered-triangle incidence audit
changes from one >1 cm disagreement to zero (maximum residual under 5 mm with 1 mm XZ incidence
tolerance). The neighboring frontage still passes 99 probes, maximum paired difference 6.5 mm.
Neither assets, the diagnostic save nor the checked-in terrain fixture were modified. Full-suite
command from repository root: `RAYON_NUM_THREADS=24
$TEST_BINARY --test-threads=8`.

Matched, unprofiled release timing (`rustc 1.98.1`, Cargo release profile, debug flags unset,
`RAYON_NUM_THREADS=24`, no concurrent builds/tests) reuses `benchmark_road_site_cdt_noding` above.
Three before/after process pairs give **0.4516/0.4463**, **0.4493/0.4477**, and
**0.4483/0.4477 ms/tile**; the middle pair reverses order. All outputs retain 763 vertices,
1,369 faces and zero invalid/missing constraints. No kernel regression is apparent in this fixture;
these small differences are not a general speedup claim.

The existing `populated_paved_road_plan_scaling` also passes one matched process pair, 100 measured
edits plus three warmups per population, with fixed local sites and identical local products at
every background size within each build. Worker p50 milliseconds, before/after:

| Remote buildings | Remote roads | Agents | Before / after |
| ---: | ---: | ---: | ---: |
| 0 | 0 | 24 | 21.45 / 23.50 |
| 1,000 | 4 | 6,024 | 21.55 / 22.85 |
| 10,000 | 40 | 60,024 | 22.39 / 22.60 |
| 100,000 | 391 | 600,024 | 22.76 / 22.54 |

Current readiness p50 stays below 0.020 ms; one-time snapshot cost is separate (0.007–3.312 ms).
This verifies locality, not an end-to-end speedup: one pair cannot attribute the small-map timing
variation. Commands for each retained release test binary use `RAYON_NUM_THREADS=24 <binary>
<benchmark_name> --ignored --nocapture --test-threads=1`. Deployed library SHA-256:
`8e8daecd8676bfe2a6d914531fc59b0587be111a45429ed2fa47ff3431e4e832`.

## From `earthworks.md`

*The constrained-Delaunay parts of "Geometry Decision", replaced in `earthworks.md` by
`TERRAIN-07` pointers. Its sections 1, 5 and 6 and the rest of 2, 3 and 7 stay live there.*

*Section 2, "Roads-First Rewrite Uses A Separate Piece/Profile Carrier Plus Stitched Terrain", replaced bullets:*

- ordinary grounded `Standard` roads use the road-owned top surface inside the footprint and
  Rust-stitched terrain topology outside it; they must not render a visible road-owned closure
  strip, carpet, skirt, or second support mesh as the seam fix
- terrain owns the far-field ground outside the deterministic road-owned footprint and receives
  exact road-footprint constraints for the hardcut
- sloped ordinary-road cases must keep asphalt, curb / shoulder, and sidewalk as road-owned top
  surfaces while the clipped terrain mesh terminates at their exact outer footprint

*Section 3, "Required seam acceptance checks", replaced checks:*

- a sloped road must keep asphalt, shoulder / curb, and sidewalks as the visible top surface while
  clipped terrain topology terminates exactly at the road-owned seam
- clipped terrain topology must leave an actual road-shaped hole under the full asphalt, shoulder /
  curb, and sidewalk footprint; there must be no terrain mesh there for z-fighting to occur
- clipped water topology must leave a conservative road-shaped hole where visible water overlaps a
  grounded road-owned footprint; road placement over water must not leave a hidden lake plane or
  transparent fragment wedges between the road mesh and its owned support surface

### 4. The First Required Variant Is Rust-Stitched Terrain Topology

The first required rewrite variant is intentionally hard-cut: grounded road seams are generated as
terrain topology in Rust, not as a visible seal drawn by the road renderer.

Required first variant:

- the first shipped rewrite variant must clip terrain patches directly to the road-owned footprint
  boundary for grounded `Standard` roads; the clip carrier is the compiled piece-owned
  `outer_boundary_loops`, not the internal asphalt / sidewalk band polygons
- any internal triangles derived from those outer loops are implementation detail only and must be
  rejected if their sampled interior leaves the source loop; terrain must never be cut by a
  triangulation wedge that extends outside the road-owned footprint
- the terrain patch payload must include baked mesh vertices, normals, and UVs when the patch
  intersects a grounded road-owned footprint
- Godot must upload that baked `ArrayMesh` directly and must not run `Geometry2D` terrain clipping,
  shader discard, alpha masking, or an ordinary road-side closure-strip fallback
- boundary vertices generated by the terrain patch mesh must take their height from the intersected
  road / sidewalk edge and all non-boundary terrain vertices must sample visual terrain
- road-locked terrain patch selection is bounded to each road-owned footprint plus that footprint's
  required grade-limited tie-in envelope; one steep road must not force unrelated roads or whole-map
  terrain patches into clipped-terrain rebuilds
- cut versus fill for structural variants is determined from the support-surface anchors relative
  to the sampled tie-in boundary, not from ad hoc widening of stamped terrain
- the first shipped rewrite variant does not require retaining walls, cliff faces, or other
  special-case vertical structures
- the terrain mesh itself must be open below the road footprint and must terminate at the exact
  road-owned seam boundary
- terrain suppression is allowed only as topology omission from the clipped terrain mesh; fragment
  discard, alpha masking, or post-shader hiding is not an accepted grounded-road solution
- future retaining or wall variants may replace structural local earthwork faces when deterministic
  thresholds say terrain topology alone is not the intended visual treatment, but ordinary grounded
  roads still use the stitched terrain topology as their seam carrier

*Section 7, "Required cache and query contract", replaced bullets:*

- the terrain renderer must not continue drawing visual terrain in texels already owned by a
  client-owned top surface or client-owned local earthwork geometry
- terrain omission must be expressed in the terrain mesh topology itself for grounded `Standard`
  roads; shader discard or alpha masking must not be used to hide missing clipped topology
