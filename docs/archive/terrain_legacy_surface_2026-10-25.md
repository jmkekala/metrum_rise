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
> On 2026-10-28 `TERRAIN-07-G` deleted that code and moved the rest of its contract here: the
> legacy seam contract left in `earthworks.md` and the road side of the exact cut in `roads.md`,
> "Footprint and terrain" ([that section](#moved-at-terrain-07-g-2026-10-28)). On 2026-10-29
> `ROAD-68-H` moved the parts of `earthworks.md` that still described that runtime as current
> ([the last section](#moved-at-road-68-h-2026-10-29)).
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

## Moved at `TERRAIN-07-G` (2026-10-28)

*Moved when `TERRAIN-07-G` deleted the exact/CDT cut: the legacy seam contract left in
`earthworks.md` and the road side of the exact cut in `roads.md`, "Footprint and terrain". The
text is unchanged but for relative link targets; it describes deleted code.*

### From `earthworks.md`, section 5: the legacy seam contract

*Section 5, "Earthworks Derive From The Client Support Surface", tie-in carrier bullets:*

- the seam between the road-owned top surface and surrounding terrain must be covered by a
  deterministic tie-in carrier:
  - Rust generates the road-touched terrain patch topology as an explicit local mesh
  - that mesh omits terrain below the road-owned footprint and inserts road-boundary vertices at
    the exact road / sidewalk outer edge
  - vertices created on that boundary reuse the road-owned edge heights, not resampled terrain
    heights, so the terrain and road share the same seam coordinates
- ordinary grounded roads must not render a visible closure strip, carpet, or second support mesh;
  the terrain patch mesh itself is the only ordinary seam carrier outside the road-owned footprint

*Section 5, the seam contract:*

The seam contract below is the legacy exact cut's: it describes the code on `road` until
[`TERRAIN-07`](../terrain.md#graded-terrain-surface-terrain-07) replaces the stitched CDT carrier with the graded terrain's exact mask and
road skirts and its last gate moves this text to the archive. New work follows `TERRAIN-07`.

Deterministic seam contract:

- the seam is the shared boundary between the client-owned footprint and the far-field terrain
- for roads, the footprint includes asphalt, shoulder / curb, and sidewalk bands; the seam starts at
  the exact outer sidewalk edge, or at the exact outer shoulder edge when a profile has no sidewalk
- no-sidewalk `Standard` road profiles still expose an explicit curb / shoulder band, so terrain
  clipping receives a real road-owned outer seam instead of a zero-width or asphalt-only fallback
- every seam segment must be backed on both sides by visible carriers before terrain below the
  footprint is suppressed:
  1. the client-owned top surface covers the footprint side of the seam
  2. terrain topology is clipped / triangulated so its inner edge exactly matches the client-owned
     seam vertices
- terrain masking, terrain alpha, terrain discard, or footprint suppression is not a seam carrier;
  those tools are valid only after one of the visible carriers above already covers the boundary
- grounded `Standard` roads use one seam carrier: Rust-generated terrain patch topology clipped to
  the road-owned footprint
- the clipped terrain inner edge must reuse the same coordinates and heights as the road-owned
  outer sidewalk / shoulder boundary; it must not resample, offset, snap, simplify, or widen that
  edge into a visible strip
- clipped terrain patches must receive exact road footprint loops from the same `Span`,
  `Terminal`, `Bend`, and `JunctionN` pieces that render asphalt, shoulder / curb, and sidewalk
- Rust must triangulate from those footprint loops and must insert deterministic seam triangles
  that use the loop as a hard inner constraint; terrain clipping must not reuse asphalt / sidewalk
  render triangles, because those triangles are not the ownership contract and can over-cut concave
  road footprints
- the road-touched patch mesh must emit terrain-owned seam faces from each road-owned outer-loop
  segment to source terrain outside the footprint before any terrain below the footprint is omitted
- the seam faces are part of the terrain patch mesh, use terrain material, and are not a second
  road support mesh, visual carpet, or closure strip owned by the road renderer
- grounded `Standard` seam faces use explicit Rust-generated grade-limited guide samples outside
  the final road-owned footprint, keyed before ordinary source terrain samples, so the CDT tie-in
  is an authored topology input rather than a render-side repair
- those guide samples are generated by `RoadSurfaceSystem` from the final unioned roadbed loops,
  not by Godot, terrain rendering, or a per-piece post-process
- guide rail constraints may be emitted only when the final footprint set is a single clean non-hole
  convex loop; concave junction mouths and multi-loop footprints remain sample-only to prevent
  grading rails from crossing the final road-owned footprint
- building-site loops may share the terrain-CDT patch carrier, but they are not roadbed loops and
  must not emit `RoadSurfaceSystem` roadbed grading-envelope samples or constraints; site grading is
  owned by the site / engineered-ground path and must remain sample-only around the hard support
  footprint
- ordinary `Standard` span and node footprint seam sources must not be promoted into retaining-wall
  topology; wall output is reserved for explicit structural bridge / tunnel / future retaining
  sources with preserved provenance
- terrain cell triangles may still provide far-field terrain outside the road footprint, but they
  are not allowed to be the only seam carrier because their grid edges rarely coincide with the
  road-owned outer sidewalk / shoulder edge
- target CDT contract:
  - road-piece polygon ownership is resolved with `i_overlay` before triangulation; Spade receives
    already-owned asphalt, sidewalk, and terrain regions rather than overlapping material hints
  - road-touched terrain patches are generated with Spade's Rust-side
    `ConstrainedDelaunayTriangulation`
  - `i_overlay` is the chosen Rust-side polygon boolean backend for road / terrain ownership
    cleanup; it owns union, intersection, difference, hole handling, and overlap removal before
    CDT input is built
  - Spade is the chosen CDT backend; `ghx_constrained_delaunay` is not part of this spec, is not a
    fallback, and may only be reconsidered through a new explicit benchmarked spec change
  - road node contour construction, grade evaluation, spatial lookup, and validation follow the
    accepted geometry-backend responsibilities in [`roads.md`](../roads.md);
    earthworks must not depend on hand-rolled road offset, boundary recovery, or sampled-height
    repair paths to produce a valid seam
  - road / earthwork seam math should use the same internal `glam` vector representation and
    explicit quantized keys as the road arrangement builder; Godot vectors are render / bridge
    payloads, not the authoritative seam identity
  - `robust` is not part of the accepted implementation path for now; exact-predicate needs should
    first be handled by `i_overlay` and Spade, and any standalone predicate dependency requires a
    narrow future spec change
  - the production path uses deterministic `try_bulk_load_cdt` inputs; conflicting constraints are
    counted and skipped rather than allowed to panic the simulation thread, and Spade refinement
    helpers are not used until they have a pinned deterministic contract for this project
  - the terrain patch rectangle is the outer constrained contour
  - every grounded road-owned outer footprint loop inside or crossing the patch is inserted as a
    hard constrained contour
  - road footprint constraints that cross patch edges are pre-split before CDT input; constraint
    segments must only meet at shared endpoints and must not cross through each other
  - road-piece seam vertices and constraint edges must appear exactly in the generated terrain
    mesh; no post-process snapping, widening, shader discard, or Godot-side clipping may create
    the seam
  - source-terrain sample points may be inserted as Steiner / interior points only outside road
    footprints, and their insertion order must be canonical and deterministic
  - roadbed grading-envelope guide samples are inserted before ordinary source-terrain samples and
    share the same deterministic point keying, so near-road tie-ins are bounded without moving the
    road seam
  - guide constraints are a local convex-footprint aid, not a footprint repair path; if the final
    loop set is concave, contains holes, or contains multiple non-hole loops, the grading envelope
    must rely on guide samples without adding rail constraints
  - building-site apron guides are always soft guide samples; the site support footprint is the hard
    CDT boundary, and apron guides must not add rail constraints across roads or neighboring sites
  - Spade CDT faces are classified after triangulation against the final road-owned footprint;
    ordinary non-seam faces may use centroid ownership, but faces carrying a road constraint edge
    classify the exact seam side so narrow concave tie-ins are not lost to centroid-only ownership
  - emitted terrain triangles must preserve the CDT constraint edges at the road seam and must not
    cross a road-owned footprint loop
  - CDT triangulation failures are hard errors in debug output and must not fall back to cell
    subtraction, seam carpets, closure strips, water, or shader masks
  - all local CDT windows in one refined render-patch generation are one atomic replacement; if any
    window fails, the entire generation is non-renderable and the renderer keeps the last valid
    clipped patch rather than filling the failed window from raw terrain
  - a building-site loop cannot satisfy road clipping: a patch already owned by grounded roads must
    retain nonempty authoritative road sources and road loops on every refined generation
  - the exporter may omit individual pathological ordinary terrain faces before final validation,
    record the omitted count, and keep the remaining clipped baked patch active only when its final
    status is `ok`. This containment is not a seam-closure fallback and remains visible in logs. If
    the final output is still `pathological`, the complete engineered generation is non-renderable
    and the renderer retains the previous terrain/road pair; raw terrain never replaces it
  - `Terminal`, `Bend`, and `JunctionN` visual node pieces resolve asphalt / sidewalk /
    outer-footprint ownership through `i_overlay` before Spade triangulation, so sharp-angle
    sidewalks shrink or split instead of overlapping asphalt
- clipped terrain topology must insert road-boundary vertices into the terrain mesh in Rust rather
  than approximating the seam from terrain-cell centers, a texture mask, or a Godot-side polygon
  clipping fallback
- no `Bend` or `JunctionN` clip boundary may be synthesized from one anonymous annulus, one global
  outer loop, or a terrain-cell mask after the fact
- a valid seam is closed independent of triangle winding or backface-culling behavior; clipped
  terrain may render double-sided, but the renderer must not reveal world background, holes, or
  source terrain through the road / sidewalk / tie-in boundary
- water is not a valid fallback carrier under the road footprint; water render patches that overlap
  grounded road-owned asphalt, shoulder / curb, or sidewalk must receive the same footprint clip
  polygons and omit any touched water cells instead of triangulating partial transparent fragments

*Section 5, "For roads, that means", replaced bullets:*

- grounded `Standard` roads replace the near-road visible terrain locally:
  - asphalt and sidewalk render as the visible terrain replacement inside the road-owned footprint
  - the Rust terrain patch mesh emits no terrain under the road-owned footprint
  - the seam to far-field terrain is formed by terrain triangles whose inner boundary is the
    road-owned outer sidewalk / shoulder edge
  - grade-limited guide samples around the footprint keep ordinary tie-in triangles local and
    deterministic without wall teeth beside the road
  - terrain suppression is allowed only as a consequence of the clipped terrain topology
  - the runtime must not render a separate visible cut / fill support mesh or ordinary closure strip
    below or beside grounded asphalt or sidewalk

### From `roads.md`, "Footprint and terrain": the road side of the exact cut

Everything else in this section (patch loops, holes, open ends, source tags, the grading envelope,
seams on tile sides, watertight patches, pads in the patch frame, patch selection, tile clip,
ownership refresh) is the road side of the exact cut. It describes the code on `road` until
`TERRAIN-07-G` deletes it; new terrain work follows `TERRAIN-07`.

The terrain CDT core in `rust/src/simulation/terrain/cdt/` stays. It is generic: patch-clipped
loops, source samples and tie-in guides in; stitched patch mesh out. Its contract in
[`terrain.md`](../terrain.md) and [`earthworks.md`](../earthworks.md) is unchanged:

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
  [T22](../terrain.md#terrain-07-open-questions) 3 (a)): the row's vertices from the end corner of
  one chain to the start corner of the next, a curb pair once at its lower vertex. The loops join
  through it like a chain, and its segments are structure sources with the retaining-wall policy
  (`ChainSegment::wall`), so a structure built from a road commits (open question 19). A run
  across a bridge's mouth is marked (`PieceMesh::abutment_runs`), and
  `RoadbedSystem::wall_seams_in_bounds` gives its segments the bridge's abutment face's bottom
  edge below their ends (`WallSegment::abutment_bottom`), where the graded terrain's abutment
  wall starts (`TERRAIN-07-D` round 7, the abutment band).
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
  ([open question 28](../roads.md#road-68-open-questions)).
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
  [T14](../terrain.md#terrain-07-open-questions)). The grounded chains that end at a structure close
  across its mouth with a wall run (open ends above), so the legacy terrain builds those patches
  and the edit commits (`TERRAIN-07-C` round 3, open question 19).

## Moved at `ROAD-68-H` (2026-10-29)

*Moved when `ROAD-68-H` cleaned up after `TERRAIN-07-G`: the parts of `earthworks.md` that still
described the deleted stitched-CDT runtime as current. The text is unchanged; it describes
deleted code.*

### From `earthworks.md`: Section 7, "Invalidation Must Stay Chunk-Local", the road click-completion plan

Road click-completion plans retain local structural stamp writes, road/site CDT tiles and joined
patch buffers. Road-only preview compilation reads terrain and indexed road contributors while
retaining the solved local geometry. It builds no structural terrain stamps, cutout patches or
terrain mesh products in the default road-only mode (`ROAD-28`). Optional full preview
(`ROAD-29`) compiles these same local products off-lock during hover, captures indexed site
inputs and temporarily substitutes draw meshes. A matching click revalidates and shares that
immutable terrain plan. Otherwise, after click, a local copy-on-write overlay applies source resets
and stamp writes in the
same interleaved sorted-chunk order as commit. Planned bordered textures, CDT boundary/background
samples, road grading and building-site grading all read that final visual result. Explicit chunk
coverage preserves reset-to-default samples; untouched chunks still read the pinned resident grid.
Work/storage is bounded by touched storage chunks and stamp samples, not total map size. Commit
checks exact road/site/source/visual dependencies, coverage, patch snapshots and ownership before
adopting the entire planned mesh batch. Structural stamps separately require matching source/grid
dependencies and ordered support triangles. Complete patch/tile/seam buffers retain Arc identity;
only payload generation metadata changes. A different query margin must select the identical
road/site contributors. Mismatches roll back, rather than replacing ready geometry through fresh
tile or patch assembly. Plan-less subsystem callers retain the cold compiler.
Ordinary grounded roads are CDT-only, not height-stamped. After click, the simulation command
captures local building-site footprints/support heights and paving regions using the prepared
allocator index, then compiles grading against planned roads plus unchanged resident owners. Neither capture nor status checking may
rebuild the city index; unprepared inputs remain provisional/pending. Exact local site comparisons
detect added/removed footprints and height/ID/material changes. Planned building grading, coverage and final
quality validation govern full placement readiness, with exact post-topology checks at adoption.
Hover displays the canonical road/junction scene over unchanged terrain; terrain integration is
completed before atomic placement using the production exporters/builders. Planned ownership can restore regular terrain where a cutout
disappears. Ownership is resolved before CDT assembly using the same cached per-loop grading
calculation as live patches, combining retained owners with planned replacements. Superseded
owners contribute only to the dirty envelope, not final ownership. Padded query hits cannot claim
ordinary patches; missing clipping inputs cannot release genuinely owned patches. Captured sites
use the live footprint-overlap rule and grading margin. Patch clip queries then use the resulting
production margin. Work remains bounded to affected patches and indexed local owners: O(local
query/ordering work + unique local loops × affected patches + uncached grading samples), with
cached grading evaluated once per loop and patch compilation parallelized through Rayon.
Structural ownership/clip discovery now consumes the final ordered resets/writes before CDT
assembly. Resident per-loop caches depend on source and visual-only revisions; changed overlays
use a local cache without replacing live entries. Unchanged overlays retain resident cache reuse.
Site dependencies are captured using each patch's final clip-query margin, not a fixed margin.
Visual-only changes stale the road candidate. Preview display has no terrain-residency requirement
and leaves all terrain resources, samples, payload caches and acknowledgments unchanged during
motion, idle and cancellation. Occasional clipping/z-fighting and exposed old cutouts are accepted
until placement. Road splits/attachment repair preserve authored site pose and support height;
the click-captured site set is checked again after topology adoption. Complete commit plans publish
their existing terrain buffers directly after road, visual-sample, coverage and ownership checks,
without a second CDT assembly. Failure restores exact local visual samples as well as graph/split
references. Road-preview readiness and complete placement readiness are separate; see `ROAD-28`
and the adoption contract in `roads.md`.

The local compiler frontier includes every edge dirtied by final profile solving and every affected
node, not only the inserted stroke. This same frontier drives preview products and commit's old/new
earthwork coverage. Grading guides sample a common terrain field; retained road seams own their
contact heights. Shared tile sides use the uncut local grading halo and keep canonical boundary
samples. Out-of-core samples, false beyond-endpoint constraint intersections and independently
heighted guide slivers are rejected or corrected at input construction, never concealed by dropping
terrain faces. A nonzero omitted-face count prevents readiness and publication. Contributor/window
validation authorizes buffer composition only; final acceptance also requires present, valid render
buffers for every clipped patch. Cold compilation, cache insertion and planned adoption share that
gate; an ordinary loop-free patch does not require a clipped mesh.

### From `earthworks.md`: Current Runtime, section 3, "Terrain Density Is A Blend-Quality Input, Not The Seam Carrier", opening paragraph

The current runtime no longer relies on the visual terrain heightfield to carry the ordinary
grounded-road seam. Road-touched terrain patches are stitched in Rust, and terrain density now
affects far-field blend quality and patch cost rather than correctness under the road footprint.

### From `earthworks.md`: Current Runtime, section 5, "Current Terrain Runtime Is A Compatible Base, Not The Final Visual Carrier"

### 5. Current Terrain Runtime Is A Compatible Base, Not The Final Visual Carrier

The current terrain runtime is sufficient for the road-first stitched terrain cut:

- it keeps separate authoritative `source terrain` and derived `visual terrain`
- it supports chunk-local reset and restamp of touched visual regions
- it accepts road-touched patch mesh ownership from Rust for ordinary grounded roads and still
  accepts structural earthwork inputs for explicit bridge / tunnel / retaining cases

But it is not sufficient as the final near-footprint visual carrier because the current visual
terrain remains a single-height field.

Current deterministic conclusion:

- a terrain-runtime rewrite is allowed if needed, but it is not by itself an earthworks solution
- do not expect the current visual terrain heightfield, even at a denser cell size, to represent
  all near-footprint cut / fill detail by itself
- ordinary grounded-road near-footprint ownership is now the client-owned top surface plus stitched
  terrain patch topology; structural cases may still use client-owned local earthwork geometry

### From `earthworks.md`: "Roads-First Rewrite Status", the stitched-CDT status and hardcut rules

The roads-first rewrite is live for ordinary grounded-road terrain ownership. The piece / profile
carrier owns the road footprint, and Rust now generates the road-touched terrain patch topology that
cuts the terrain to the outer sidewalk / shoulder edge.

For the roads-first rewrite, the following are deterministic and implemented:

- the current corridor-sheet prototype is retired as the target solution
- the logical graph and the visible road carrier are split: graph owns connectivity, while the
  visible road system owns deterministic geometry pieces
- the minimum required visual piece set is `Span`, `Bend`, `Terminal`, and `JunctionN`
- roads compile explicit top-surface ownership and exact footprint constraints near the terrain seam
- tie-in boundaries are specified to use dense local sampling at a maximum `2 m` longitudinal
  spacing
- committed clients stay fixed under later terrain-authoring edits
- local road ownership uses chunk-local rebuild boundaries and one explicit visible-world query
  order
- road-touched terrain patches are currently generated in Rust as baked terrain `ArrayMesh`
  payloads whose boundary vertices reuse the road / sidewalk seam height
- ordinary grounded-road tie-ins now include grade-limited guide rings generated from final
  road-owned footprint loops inside `RoadSurfaceSystem`; guide rails are constrained only for a
  single clean convex loop, while concave junction mouths and multi-loop footprints stay sample-only,
  and ordinary `Standard` / node footprint sources never emit retaining-wall mesh
- the Spade CDT terrain-patch hardcut in [`roads.md`](../roads.md) is now the live
  road-touched terrain patch path; the provisional seam-strip / cell-triangle hybrid has been
  removed rather than polished further

The following are current hardcut implementation rules:

- grounded `Standard` roads send grouped road footprint loops into road-touched terrain patches
  and those patches return baked mesh vertices / normals / UVs generated by Rust
- the Godot terrain renderer no longer performs terrain-road polygon clipping; it only uploads the
  baked mesh or the normal rectangular terrain mesh
- grounded `Standard` roads do not render an ordinary visible closure strip, seam carpet, or second
  support mesh; the CDT terrain patch mesh is the seam carrier
- `Terminal`, `Bend`, and `JunctionN` node pieces must compile final asphalt, curb / shoulder, and
  sidewalk regions as band-owned surfaces whose mouth seams, material seams, and outer footprint
  edges survive through `i_overlay` ownership cleanup and local Spade CDT triangulation
- bend / junction full-roadbed closure carriers are legacy debt, not the target rendered ownership
  path; explicit curb / shoulder and sidewalk carriers must claim their seams and heights, and
  remaining non-road residuals must be rejected or debug-counted instead of filled by a generic
  closure carrier
- node-piece terrain clips and local earthwork / skirt roots must be extracted from the canonical
  final band-owned top mesh. If the final owned-region outline introduces boundary vertices, those
  vertices must be inserted into the canonical node arrangement and rendered top mesh before CDT
  input is built. A point that is merely covered by an existing top-surface triangle is not enough
  for export; non-explicit boundary vertices and sampled boundary heights are geometry errors. A
  node path must not reconstruct a later outer boundary loop that contains vertices outside the
  canonical rendered node top-surface coverage.
- road / earthwork seam math shares the road-surface `RoadVec2` / `RoadVec3` representation and
  explicit quantized keys through the owning Rust stages; Godot vectors remain bridge, upload, and
  debug payloads rather than authoritative seam identity.
- visible water patches now use depth-owned local topology instead of full-patch planes; road-touched
  water meshes receive the same grouped road footprint loops after a network edit and suppress
  cells that touch the outer-loop-minus-hole road-owned area, so water is no longer allowed to
  render under grounded road-owned asphalt, shoulder / curb, or sidewalk
- clipped patch topology is validated against flat, diagonal, sloped, bend, terminal,
  `JunctionN`, production authored DEM road cases, and a compact baked Kuopio imported-DEM
  fixture, including steep ordinary tie-ins without wall teeth, raised ridge / valley terminals
  and bends, steep multiway junctions, convex constrained grading-envelope rails, concave
  junction-footprint sample-only grading envelopes, edit-order-stable emitted terrain-CDT topology,
  bridge-midspan, and tunnel-portal structural stamping
- imported DEM JunctionN clipping now reports and removes road-owned internal chords from the
  terrain seam constraint set only when both sides of the exact constraint classify as road-owned
  against the final footprint; exposed seam constraints remain hard CDT constraints with source
  provenance
- terrain suppression / masking is not accepted as the live seam solution; road-shaped terrain holes
  must continue to be produced by terrain mesh topology
- no current `ROAD-01` blocker remains for real-world DEM validation; any future terrain closure
  variant beyond the structural retaining-wall path should be tracked as a new explicit earthworks
  item

That means the building-site target below extends the same clipped-topology ownership model to
area clients; it is not a substitute for the road-to-terrain boundary that already ships.
