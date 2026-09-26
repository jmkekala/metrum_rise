# Terrain / World Terrain Spec

## Plot grass and frontage materials — RENDER-13

Plot grass previously retained the older blue-green palette and fixed 3.0 texture chroma gain.
Terrain had already moved to an olive/yellow-green palette and 1.15 gain. Graded frontage
sampled the paving albedo but then applied grass ambient emission and custom lighting; it
also omitted the paving normal and roughness textures. Authored plot meshes supplied no
UV/tangent basis despite using normal maps.

`ground_grass.gdshaderinc` now owns the palette, shared uniform defaults, grass sampling,
variation and hillshade functions. The duplicate plot implementation and repeated material
factory/terrain-renderer tuning constants are removed. Flat plot grass matches open flat
terrain at the same world position; terrain still owns rock, shore, forest and overlay masks.
`ground_paving.gdshaderinc` owns world-space albedo/normal/roughness sampling and tangent
orientation and the Lambert light function for road tops, yards and tagged frontage.
`paving_surface.gdshaderinc` owns the complete shared road/yard surface program. The road
and site shader files are four-line wrappers that retain their distinct culling modes.
`WorldMaterials` binds the same cached
paving textures and tone to terrain. Paving uses matte Lambert lighting, environment ambient,
real shadow attenuation and SSIL, without the grass emission floor or grass desaturation.
Road tops and site paving disable direct specular highlights, while sharing the same
rough sky reflection response (`PAVING_SPECULAR = 0.5`). Face tags only select
material; terrain ownership, simulation state and installed assets are unchanged.
The render export winding correction below preserves positions and canonical geometry.

Runtime bounds: O(visible fragments), fixed texture sample count, no new draw calls, mesh
splits, city scans or per-frame CPU allocations. Paved terrain skips the grass/relief colouring
branch. Four additional cached texture bindings are applied per terrain material setup;
shared grass defaults eliminate repeated tuning writes.

Initial shader-only validation (2026-09-26; before the export correction below): `plot_material_lighting_test.gd` passes day/moon/darkness,
asphalt/concrete frontage-to-yard matching, sidewalk matching, rotated mesh orientation,
shadows and local lamps. Mean sampled RGB-vector difference is zero for flat grass and
at most 0.000006 for frontage/yard paving (tolerance 0.012). Window-spill regression passes.
The existing chunk-renderer test passes when run from `/tmp/metrum-plot-light/renderer_only.gd`
with only the native road-cursor acknowledgment test omitted; the full command timed out
on that native contract using the deployed library. The terrain overlay test passes overlay
intensity, borders and near-floor checks but fails its far-canopy luminance assertion; the
unchanged baseline shader also fails that same assertion. Neither broader suite is claimed
as a full pass.

Matched unprofiled GPU measurements: upstream Godot 4.7.2 release, Forward+, RX 7900 XTX
(RADV), 1920×1080 SubViewport, AgX/SSIL/glow from the shared environment, one 40 m plane,
directional light plus ambient, vsync off, `RAYON_NUM_THREADS=4`. Two alternating before/after
pairs per material, 60 warm-up and 120 measured frames each; median viewport GPU time:

| Surface | Before (ms) | After (ms) |
|---|---|---|
| Plot grass | 0.558 / 0.557 | 0.559 / 0.560 |
| Tagged terrain paving | 0.627 / 0.633 | 0.559 / 0.563 |
| Open terrain grass | 0.621 / 0.620 | 0.625 / 0.629 |
| Yard paving | 0.558 / 0.561 | 0.558 / 0.559 |
| Road sidewalk | 0.560 / 0.562 | 0.560 / 0.562 |

These are isolated material measurements, not whole-city frame timings. Baseline shader
snapshots are from `345d59284f13ae81d1b984c727778c66ec17c8bc`; release library SHA-256
`5368fe8e87315e028fad2437e87a26119a0b7b1b7a63ec7b7bf592b8415dbe89` was used for these shader-only measurements.
Artifacts: `/tmp/metrum-plot-light/{benchmark-final.log,spill.log,overlays.log,overlay-baseline.log,renderer-only.log,chunks.log,identities.txt}`.
The baseline directory holds `terrain.before`, `site_ground.before`, `site_surface.before`
and `road.before`, each containing the corresponding shader from the baseline commit.

```bash
XDG_DATA_HOME=/tmp/metrum-plot-light RAYON_NUM_THREADS=4 godot --path godot --script res://tests/plot_material_lighting_test.gd -- --asset-editor --benchmark-plots --baseline-dir=/tmp/metrum-plot-light
XDG_DATA_HOME=/tmp/metrum-plot-light RAYON_NUM_THREADS=4 godot --path godot --script res://tests/window_spill_test.gd -- --asset-editor
XDG_DATA_HOME=/tmp/metrum-plot-light RAYON_NUM_THREADS=4 godot --path godot --script res://tests/terrain_overlay_shader_test.gd -- --asset-editor
XDG_DATA_HOME=/tmp/metrum-plot-light RAYON_NUM_THREADS=4 godot --headless --path godot --script res://tests/network_tool_chunk_renderer_test.gd -- --asset-editor
```

### Native terrain winding correction

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

Commands: `cargo test --manifest-path rust/Cargo.toml --lib exported_terrain_faces_are_front_facing_from_above`,
`cargo test --manifest-path rust/Cargo.toml --lib nodes::simulation_node::tests::cdt`,
`cargo test --manifest-path rust/Cargo.toml --lib building_site_terrain`,
`cargo build --manifest-path rust/Cargo.toml --release`; use the material-test command above
without benchmark flags for the native rendered check. Benchmark reproduction:
`rustc -O /tmp/metrum-plot-light/index_bench.rs -o /tmp/metrum-plot-light/index_bench`, then
`/tmp/metrum-plot-light/index_bench`. Evidence under `/tmp/metrum-plot-light/`:
`native-render-before.log`, `native-render-after.log`, `winding-rust.log`,
`winding-cdt-tests.log`, `winding-site-tests.log`, `winding-build.log`,
`index-benchmark.log`, `index-benchmark-build.txt`, `winding-identities.txt`.

### Morning sky reflection parity

After correcting triangle winding, the low-sun sky still exposed a smaller seam. Tagged
terrain paving explicitly set `SPECULAR = 0.0`, while yards/sidewalks retained Godot's
0.5 dielectric response. Disabling direct specular highlights does not remove environment
reflections, so coloured sky light differed across the boundary, especially in shadow.
`ground_paving.gdshaderinc` now owns `PAVING_SPECULAR = 0.5`, explicitly used by all three
shader paths. No material duplication, new sampling, mesh work or CPU updates are added;
fragment cost and allocation bounds are unchanged.

Fresh verification: `plot_dawn_lighting_test.gd` uses the actual `SceneLighting` node,
incrementally updated sky, an angled camera, an occluder and the shared AgX/SSIL environment.
It checks asphalt/concrete and sidewalk parity at 05:00, 06:00, 07:00, 08:00 and noon,
plus indirect-only lighting at each hour. It passes with mean sampled RGB-vector difference
at most **0.000071** (threshold 0.002); the previous shader reached **0.02391** at 06:00.
The native export/material day/night/local-light regression also passes on this shader.
The prior checks used a constant background, so they could not catch sky reflectance drift.

Matched unprofiled release Godot 4.7.2 / Forward+ / RX 7900 XTX (RADV), 1920×1080,
06:00 live sky, directional shadow, `RAYON_NUM_THREADS=4`, vsync off: two alternating
before/after pairs, 60 warm-up and 120 measured frames each. Median viewport GPU time:
**0.730 / 0.734 ms before → 0.731 / 0.733 ms after**. This is an isolated material fixture.
The Rust library remains `54ecc2f8dad9d5f3cd48134242883023dfa2d4cfdc1dc61ce1481c2a2650f108`.
Baseline `terrain.before` is the immediately preceding working-tree shader with the
`SPECULAR = 0.0` paving assignment. Evidence in `/tmp/metrum-dawn/`:
`specular-probe.log`, `regression-benchmark.log`, `material-regression.log`, `identities.txt`.

```bash
XDG_DATA_HOME=/tmp/metrum-dawn RAYON_NUM_THREADS=4 godot --path godot --script res://tests/plot_dawn_lighting_test.gd -- --asset-editor --benchmark-dawn --baseline-shader=/tmp/metrum-dawn/terrain.before
XDG_DATA_HOME=/tmp/metrum-dawn RAYON_NUM_THREADS=4 godot --path godot --script res://tests/plot_material_lighting_test.gd -- --asset-editor
```

Final consolidation removes the dormant lane-stripe calculation from the road shader:
committed road colours have green zero and actual paint comes from the marking mesh.
The road and yard shader bodies now share `paving_surface.gdshaderinc`; their direct lighting
and the terrain paving branch both call `paving_diffuse`. Shared material factory helpers own
texture/scale/tone setup, with one asphalt-04 configuration for yards and sidewalks. There
is no legacy implementation or fallback behind the wrappers; only face culling differs.

Fresh final checks: real morning-sky/indirect-light parity, native paving/material tests and
road-marking tests pass; the isolated renderer suite (excluding the previously documented
native acknowledgment test) also passes. Matched final morning GPU medians using the same
settings above: terrain **0.733 / 0.730 → 0.729 / 0.732 ms** against the original zero-specular
shader; road **0.781 / 0.785 → 0.782 / 0.786 ms**, yard **0.779 / 0.779 → 0.779 / 0.780 ms**
against the respective immediately pre-consolidation shaders (already at 0.5 reflectance).
No additional draw calls, texture fetches or steady-frame CPU work. Artifact directory:
`/tmp/metrum-dawn/`, files `unified-dawn.log`, `road-unified-benchmark.log`,
`site-unified-benchmark.log`, `unified-materials.log`, `unified-markings.log`,
`unified-renderer.log`, `unified-identities.txt`. The saved `unified_bench.gd` reuses the dawn
fixture and selects the road or yard for measurement through `METRUM_PAVING_BENCH=road`
or `site`, with `--baseline-shader` pointing to `road-refactor.before` or
`site-refactor.before`; all other command flags above remain the same.

## Window light reception — RENDER-10

Terrain and site ground now receive screen-space indirect lighting from visible emissive
windows. Their existing ambient floors remain controlled by the day-cycle globals:
zero-valued `IRRADIANCE`/`RADIANCE` overrides replace `ambient_light_disabled`, preventing
duplicate environment ambient without suppressing SSIL. No terrain mesh, texture upload or
spatial indexing changes are involved. The shared environment configuration, visibility
limitations, rendered regressions and matched GPU measurements are owned by
[the window spill contract](asset_editor.md#window-light-spill--render-10).

## Crown cohesion candidate — 2026-09-19

`RENDER-06`, visually accepted by the user on 2026-09-19 after in-game testing. Near tree cards now interpolate an outward
normal at each vertex relative to the crown centre. Solid foliage cores use the same
normal field; wood retains geometric normals. Independent face/card brightness variation
is replaced by its former mean (cards `1.105`, cores `1.04975`). Instance tint remains.
This first slice deliberately leaves silhouette consolidation and the atlas for a separate
visual comparison: vertex positions, triangle indices, UVs and wind weights match the
baseline catalogue exactly, as do instance and draw-batch counts.

The card shader also undoes the engine's backface normal reversal. Authored normals describe
the foliage volume, so turning a card over must not invert its lighting. This applies to
the shared understory card material as well; understory geometry and authored colours stay
unchanged. The reversal was verified in the [installed Godot source](https://github.com/godotengine/godot/blob/a13da4feb/servers/rendering/renderer_rd/shaders/forward_clustered/scene_forward_clustered.glsl#L1264).
Restoring the previous shader in a negative-control run fails the new rendered winding test.

Correct lighting changes the average near radiance, so distant material multipliers are now
`0.72 / 0.90` (conifer/broadleaf). The existing 900 m GPU fixture measures distant/near
luminance at `0.999 / 0.994`, within the unchanged 10% tolerance. The wind regression measures
`0.4408` moved at 55 m and `0.0000` at 300 m. The headless appearance test passes, including
finite unit foliage normals, smooth lighting and uniform foliage albedo per surface across
all 24 near variants. These checks validate rendering contracts, not player preference.

Normal construction adds O(vertices) work once at catalogue startup. The fragment correction
is O(1), with no new texture lookup, vertex attribute, surface or simulation work. Matched
unprofiled E13 measurements and captures are recorded under `/tmp/metrum-crown-cohesion/`.
The user confirmed a clear improvement in the forest appearance.

Reproduction (from repository root):

```bash
godot --headless --path godot --script res://tests/vegetation_appearance_test.gd
godot --path godot --audio-driver Dummy --resolution 640x480 --script res://tests/vegetation_level_match_test.gd
godot --path godot --audio-driver Dummy --resolution 640x480 --script res://tests/vegetation_wind_gate_test.gd
METRUM_GPU_PROBE_EXPERIMENT=E13 METRUM_GPU_PROBE_OUTPUT=/tmp/metrum-crown-cohesion/after/gpu-final godot --path godot --windowed --resolution 1280x720 --script res://tests/local_gpu_probe.gd
```

The probe requires a fresh output directory. Baseline: `2c278b01`; candidate: the working
tree patch above that commit. Both use the identical release GDExtension (`590d8b2ac614d989…`),
Godot `4.7.1`, GTX 1060 3GB, and Kuopio `kuopio_324km2_10m` (`07ae5ff39f9c905e…`).
Rayon/engine worker settings are inherited defaults (`RAYON_NUM_THREADS` unset). Each E13
trial warms for four seconds and samples for eight, pairing scatter off/on at four yaws
and at 1.5x render scale. This prices the shipped default population, not a dense painted stand.

Fresh scatter GPU cost (`full - off`, p50 milliseconds):

| View | Before | Candidate | Change |
|---|---:|---:|---:|
| yaw 000 | 6.846 | 6.800 | -0.046 |
| yaw 090 | 6.353 | 6.282 | -0.071 |
| yaw 180 | 5.874 | 5.822 | -0.052 |
| yaw 270 | 5.906 | 5.877 | -0.029 |
| yaw 180, 1.5x | 5.495 | 5.544 | +0.049 |

No measurable rendering slowdown in this comparison; these small differences do not establish
a speedup. Draw calls, primitives and resident terrain patch counts match in all ten trials;
candidate vegetation-pending frames are zero. Final data: `before/gpu/results.json` and
`after/gpu-final/results.json` beneath the artifact directory. `after/gpu/` is an intermediate
normal-only experiment, not validation of the final patch. A separate 100 stems/ha synthetic
low-sun capture is a visual aid only, not a performance or biome acceptance test.
At 220 m with an 8-degree sun, that diagnostic has a broadleaf distant/near luminance
ratio of `1.161`; the calibrated 900 m daylight fixture does not establish a match at every
sun angle or density. This remains a measured lighting limitation despite the accepted overall improvement.
Single headless catalogue-build observations were `109.7 / 113.3 ms` before/after; these
are startup observations, not a statistically established timing difference.

## Local vegetation experiment (V01)

Vegetation adds cosmetic trees to loaded worlds.
This is a local proof of concept, not persistent vegetation or a simulation resource.
Rust generates deterministic 16 m candidate cells and rejects water, steep ground,
and sampled road/building footprints. The renderer reuses terrain residency and uploads
at most one patch per frame. Two opaque tree species each have three geometry levels.
Trees have no collisions. Near trees and bushes sway in a vertex-shader wind; nothing
else moves. The initial visibility limit is 4500 m.
Shadow casting is off by default and available for the near geometry only.

Placement is derived from the terrain surface. The terrain renderer stamps each committed
patch payload with the surface generation it was built from, and that generation advances
only for the patches an edit dirtied. Each tree patch records the generation it was built
against and regenerates when the terrain renderer commits a newer one, so a road, building,
or terrain edit clears the trees it covered without a world reload. The check is
`O(resident patches)` dictionary lookups per frame with no distance work. The replacement
patch enters the scene tree before the previous patch is freed.

Geometry levels switch at a hard distance boundary with no crossfade. A visibility fade
applies to a whole `MultiMeshInstance3D`, and one instance carries a whole terrain patch, so
a fade band made every plant in a `510 m` patch translucent together and moved the patch into
the transparent pass. Two invariants follow from the same fact and set the bands below: a band
must exceed the patch diagonal (`721 m`), and a range must exceed the patch half-diagonal
(`361 m`). Trees more than about 2 km away are narrower than three pixels at 720p and still
alias.

Distant crowns, bushes and rocks are each one surface of revolution built from a radius
profile, with a per-ring and per-segment radius perturbation, flat shading, end caps, and a
root flare on each trunk. The near conifer and broadleaf levels instead carry a two-level
branch skeleton: a tapering leader, sixteen or twelve four-sided primary limbs placed on a spiral,
two smaller children on each limb, and an opaque foliage tuft at every child tip. Branch count
and recursion depth are fixed, so catalogue construction stays linear in emitted vertices.
Triangles are emitted clockwise from the front, which is what Godot treats as the front face;
the lathe walks rings in increasing angle, so `_tri` reverses the order. Emitted the other way
every surface is inside out and back-face culling makes a small dome render as a hollow ring.
Per-face vertex colours carry the crown shading. A MultiMesh instance colour multiplies
that vertex colour rather than replacing it, so a per-placement tint costs no shader and
keeps the face shade: the near band applies one, derived from disjoint bits of the
appearance seed, spanning `0.94` to `1.06` in value with opposing red/blue shifts of
`±3.5%`. The mid and far levels carry no instance colours, because a tree there is a few
pixels wide and the tint averages to one across a patch.

The mesh catalogue is indexed by species, then variant, then near-to-far level. Conifer and
broadleaf have 12 variants, bush and rock 6. Variants differ in crown proportions,
widest-point height, taper, trunk dimensions and raggedness, all derived from the variant
index, and stay within their species: no variant is more than `1.3x` the height of the
smallest in its species. The 84 mesh levels share four materials: near tree and bush
bark/foliage geometry uses one wind `ShaderMaterial`, near tree and bush cards use one
scissor wind `ShaderMaterial`, distant crowns share one scissor/backlight `ShaderMaterial`,
and rocks keep the original `StandardMaterial3D`. Distant crowns remain static. Only the near
level selects a variant; the mid and far levels use variant zero and the whole species
population, so distance does not multiply draw calls. Those two levels also share one
instance per species: they draw the same transforms with a different mesh, so crossing the
mid boundary swaps a mesh on the buffer already uploaded instead of rebuilding the patch.
`TREE_MID_M` therefore selects a mesh per patch and is not a visibility band, which leaves
`TREE_NEAR_M` as the only boundary the band-width invariant applies to. A near patch has 38
MultiMesh nodes, 36 of them in the near band; a patch beyond the near band has two.

Patch upload is the cost that governs this, because a patch uploads in a single frame and a
frame at 60 fps is `16.7 ms`. What a patch costs depends on how far it is. Past the near band
it builds one distant instance per canopy species over the whole species population, with no
variant split, no tints and no understory; inside the band it builds all 38 nodes. The near
test is conservative by the patch half-diagonal, because the patch is one instance.

On a fixed headless fixture (near: 4096 placements as 512 conifer, 512 broadleaf, 2048 bush,
1024 rock; far: the 1024 canopy placements the same patch is sent without an understory;
median of eight after warmup) a near patch measures `18.0 ms` and a far patch `2.9 ms`,
against `20.5 ms` and `6.1 ms` when every patch built every level. Residency reaches 4500 m,
so most resident patches are far ones. A near patch is still over one frame and is not an
accepted cost.

Three implementation facts set those numbers, all measured rather than assumed. Node count
is close to free on the upload side: collapsing the 40 nodes back to 8 changed nothing, and
4 variants per species measured the same as 12. It is the placement work behind the nodes
that costs, which is why the fix skips levels rather than merging them. Interpreted
per-placement work is expensive: routing every placement through a nested untyped `Array`
rather than appending to a local typed one cost `3.8 ms` per patch. And a GDScript call in
that path costs more than what it usually wraps -- writing the four `_jitter` calls out
inline saved `2.6 ms` per patch, where replacing the four `hash()` calls inside them with
bit extraction saved only `0.8 ms` more. That is why the variant index and the tint read
bits of the existing seed, and why the jitters sit inline in `_instance_transform`.

The remaining near-patch cost is the placement loop. Every species rescans the whole payload
to filter by species, so a four-species patch walks its placements four times, and
`_instance_transform` builds three `Basis` values for each one. Grouping the payload by
species on the Rust side would make that scan a single pass.

Bushes and rocks come from a denser 8 m grid that Rust generates only when the renderer asks,
which it does for patches within 800 m, because that layer draws over a much shorter range
than the canopy.

The GPU budget now has a fresh capture, on the same GTX 1060 3GB the superseded 4% and 22%
figures came from. `E13` sweeps an eye-level 300 m horizon view through four compass yaws,
each with the scatter off and on. Four matched runs are recorded, one per near-crown design:
the lathe domes, the branch skeletons that replaced them, the skeletons with alpha-scissor
cards, and the current wind shader. Every scatter-off trial agrees across all four to within
`0.15 ms`, with identical draw calls and primitives, so the scatter columns are comparable.
The last two runs are on a newer GPU driver than the first two, and that control is what shows
the driver did not move the baseline.

| yaw | off | lathe | scatter | branched | scatter | cards | scatter | wind | scatter |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 000 | `4.2 ms` | `7.18 ms` | `+2.97` (41%) | `8.55 ms` | `+4.22` (49%) | `8.96 ms` | `+4.77` (53%) | `8.66 ms` | `+4.35` (50%) |
| 090 | `4.7 ms` | `6.70 ms` | `+2.04` (30%) | `8.00 ms` | `+3.35` (42%) | `8.51 ms` | `+3.85` (45%) | `8.24 ms` | `+3.57` (43%) |
| 180 | `5.0 ms` | `7.70 ms` | `+2.75` (36%) | `9.94 ms` | `+4.99` (50%) | `10.57 ms` | `+5.62` (53%) | `10.08 ms` | `+5.13` (51%) |
| 270 | `4.8 ms` | `7.77 ms` | `+2.99` (38%) | `9.84 ms` | `+5.04` (51%) | `10.48 ms` | `+5.72` (55%) | `9.88 ms` | `+5.12` (52%) |
| 180 at 1.5x | `8.3 ms` | `10.22 ms` | `+1.93` (19%) | `12.73 ms` | `+4.52` (36%) | `13.51 ms` | `+5.22` (39%) | `13.05 ms` | `+4.77` (37%) |

Three things follow. The scatter now costs about half the frame from a ground-level view,
which is more than the retired 22% figure claimed. It is not fill-bound: at 1.5x render scale
everything else nearly doubles while the scatter's own cost barely moves in any run, so it is
priced in vertices and draw calls. Geometry added to the near meshes is therefore paid for at
every resolution, and rendering smaller will not buy it back.

The two near-crown changes price differently, and the difference is the point. Branching moved
primitives from `4.34 M` to `7.01 M` at yaw 180 with draw calls unchanged, and the cost rose by
about the same ratio: paid in vertices. Adding cards moved primitives only to `7.30 M`, a 4%
rise, but added `407` draw calls, because a second surface is a second draw per near instance
and there are up to 24 canopy near instances per patch. It bought a visibly fuller crown for
`+0.6 ms`, against `+2.2 ms` for the branch skeleton underneath it. Cut-outs are the cheaper
way to add apparent foliage, which is the whole reason they exist.

Near-level triangle counts went `211` to `565` to `645` for a conifer, and `184` to `525` to
`525` for a broadleaf, the last of which is flat because shrinking the opaque tufts paid for
its cards exactly. Wind changes no count: vertex positions, normals, UVs, indices, colour RGB
and mesh bounds are all byte-identical to the cards build, and only vertex colour alpha moved.
Catalogue construction went from `31 ms` to `51 ms` to `68 ms` including one-time texture
generation, all once at startup; a matched pair across the wind change measures `64.4 ms`
against `63.4 ms`, so authoring the weights costs nothing.

The cards are three crossed quads per branch tip on a second surface of the same `ArrayMesh`,
so they share the instance transforms already uploaded and add no instances. They use a shared
`512x512` baked 2x2 atlas (256 pixels per cell), with generic broadleaf at top left,
birch at top right, and two conifer sprays on the bottom. Broadleaf cards select their
appearance's cell; conifers select the bottom column from `seed & 1`, in O(1) work per
cluster. The shader keeps alpha scissor
at `0.4`, which keeps them in the opaque pass with depth write intact. Blending
here would repeat the crossfade mistake that cost 17-28% of frame time. Their normals point
away from the crown centre rather than along the quad, because a quad lit by its own facing
reads as a flat plate rather than as foliage.

The atlas is authored by `tools/bake_foliage_atlas.py` using an orthographic Blender
CPU render. Bake with `blender --background --python tools/bake_foliage_atlas.py --
godot/assets/textures/vegetation/foliage_atlas.png`. The PNG is the reviewable base
image and has a generated `.png.import`; the material loads the accompanying RGBA8
DDS, with its generated `.dds.uid`, because PNG cannot carry authored mipmaps. The DDS
costs 1,398,100 bytes of pixel data including mips, shared by the catalogue. No shader,
surface, placement, LOD range, or sway-weight change accompanies the atlas. The birch
and crown geometry changes below have their own measured vertex budget.
RGB is white at every texel, the exact flood extension of the white opaque mask;
vertex RGB continues to supply the foliage palette.

Mip alpha is rescaled offline independently per cell against its level-0 coverage.
The closest achievable coverage is selected after 8-bit quantization; correction uses
the uncorrected box-filter pyramid, avoiding compounded rescaling. At one pixel per
cell, none of the four target coverages can be represented: the closest result is zero.
The two terminal levels therefore disappear; this is an explicit subpixel limitation,
not a claim of exact preservation at every size. At those levels atlas cells also
cannot remain isolated under bilinear filtering. GPU appearance at these transitions
remains unverified. Measured coverage above `0.4`, in percent:

| Atlas mip size | Uncorrected | Corrected |
|---|---:|---:|
| 512x512 | 19.1074 | 19.1074 |
| 256x256 | 19.5267 | 19.1238 |
| 128x128 | 20.5994 | 19.0918 |
| 64x64 | 20.9961 | 19.1162 |
| 32x32 | 21.3867 | 19.0430 |
| 16x16 | 21.4844 | 19.1406 |
| 8x8 | 20.3125 | 20.3125 |
| 4x4 | 0.0000 | 25.0000 |
| 2x2 | 0.0000 | 0.0000 |
| 1x1 | 0.0000 | 0.0000 |

The committed `foliage_atlas.coverage.json` includes the measurements for each cell.

The atlas costs fill, and the cause is not the texture. Measured against the palette build
on one binary, the scatter's own cost rose `0.12`, `0.17`, `0.43` and `0.11 ms` across the
four yaws and `0.72 ms` at `1.5x` render scale, with draw calls and primitives byte-identical
in every trial. Halving the atlas to `512` cut its pixel data from `5.33 MB` to `1.33 MB` and
`4 MB` of measured video memory with it, and changed the frame cost by less than the
scatter-off control's own drift between runs. Texture bandwidth is therefore not what this
costs, and the atlas is `512` for video memory and repository size rather than for speed.

What it costs is occlusion. The retired procedural mask passed `34.30%` of its texels above
the `0.4` threshold; the baked cells pass `15.85%` to `21.66%`, between `0.46x` and `0.63x`
as many fragments. Honest leaf and needle silhouettes are mostly holes, so the cards hide
less of what stands behind them and that geometry is drawn instead. That is a fill cost, which
is why it grows with render scale while the rest of the scatter's price does not. It is the
standing price of the silhouette, not a defect to tune away: filling the cells back in would
return the blob the atlas replaced.
The appearance regression checks that Godot retains every authored DDS byte and all
a mip chain complete to 1x1, and checks both rows and seed parities without weakening shader or sway
assertions. A separate before/after snapshot of every surface's arrays, excluding UVs,
is byte-identical, including indices, positions, normals and vertex colour alpha.

Fresh CPU validation on 2026-09-12 used base `0ff8230d` and this atlas change, Godot
4.7.1 headless with the same existing GDExtension library and default worker settings.
No GPU benchmark or windowed game was run. Five sequential fresh processes per build
timed `Species.build_meshes()` with `Time.get_ticks_usec()`, including first-use texture
and material creation but excluding script parsing. Before: 61.753, 67.198, 62.101,
68.695, 65.026 ms; after: 56.773, 56.928, 56.153, 56.124, 56.281 ms. Medians are
65.026 and 56.281 ms (13.45% lower). The appearance test's `FIXTURE` actually reports
patch upload, not catalogue construction: its maximum is 19.358 ms before and
18.565 ms after. Those single upload maxima are not a GPU or performance acceptance.

Commands were `godot --path godot --headless --log-file <writable-log> --script
res://tests/vegetation_appearance_test.gd` and the corresponding
`vegetation_invalidation_test.gd`, plus `godot --path godot --headless --log-file
<writable-log> --import`. Full output, the temporary catalogue/snapshot harnesses,
and all samples are in `/tmp/foliage-atlas-validation/`. Final appearance output:
`PASS vegetation appearance, shared material, LOD buckets, positions and empty density`;
`grep -c "SCRIPT ERROR"` is 0. Invalidation exits 0 with 0 `SCRIPT ERROR` lines; that
harness contains no PASS print. The initial default-log run crashed on a sandbox-denied
user-log write; matched runs use writable logs. Import generated the companions despite
sandbox-denied editor settings and debug socket operations.

Two Blender 5.2.1 LTS bakes (fixed seed, 32 Cycles samples, one CPU thread) produce
PNG SHA-256 `612472fef60b135ae51678b110538b12f5d4b884425773fb365142d6440a7dca`
on both runs; the DDS files also match, SHA-256
`cd8c63752bbfbf5bcef3ea74294edc0e16c600f15ef363a32bd9e709871e315e`.
Blender wrote all artifacts but its PulseAudio shutdown stalled inside the sandbox;
artifact reproducibility is verified independently of that shutdown.

Wind is a vertex displacement and nothing else. Both wind shaders declare
`world_vertex_coords` and add a world-space offset directly, which avoids an inverse model
matrix per vertex; they share one `.gdshaderinc` so the bark surface and the card surface of
the same tree can never be displaced by different functions and tear apart. The per-plant
phase comes from `MODEL_MATRIX[3].xz`, the instance origin, so no per-instance data is added:
the placement loop already documents that one instance colour array costs an upload call and
four floats per instance, and a second such array for a cosmetic effect is not worth that.
Projecting that origin onto the wind direction also makes a gust travel across the landscape
instead of every tree moving in lockstep. How much a vertex moves is authored into vertex
colour alpha at mesh build time: `0` at the root flare, `0.16` up the trunk, `0.70` at primary
limb tips, `1` at the child tips, the foliage cores and the cards, and `0` to `0.8` up a bush.
The fragment stage reads alpha as a sway weight and never as opacity; the card shader takes
its alpha from the mask alone. Neither shader disables ambient or goes unshaded, so the day
cycle still reaches vegetation through the scene's key light exactly as it did through
`StandardMaterial3D`.

The foliage palette is calibrated against the ground rather than authored by eye. The mean
linear albedo of `grass002_2k_albedo.jpg`, after the terrain shader's `0.90` strength, is
`(0.037, 0.065, 0.016)`: hue `94.4` degrees, saturation `0.756`. Foliage sat at hue `104` to
`130` degrees and saturation `0.54` to `0.58`, which is why it read blue-green against a
yellow-green ground. Conifer moved to `112` degrees, broadleaf and bush to `100`, all three
to saturation `0.66` to `0.68`, each preserving its original luminance exactly. Foliage was
never darker than the ground; at `2.0x` to `2.7x` its luminance it is brighter, and the
blackness in low sun was a shading result rather than an albedo one.

That shading is what `BACKLIGHT` fixes: foliage transmits light instead of blocking it, so a
backlit crown no longer goes black at the hour the scatter dominates the frame. Bark must not
transmit, and one material carries both, so the term is gated on the palette's green bias,
`smoothstep(0.0, 0.05, albedo.g - albedo.r)`. That is `0` for every bark and rock colour and
`1` for every foliage colour, and it costs no vertex attribute, which matters on a scatter
priced in vertices. The gate depends on the palette keeping green above red on foliage only.

The bush was the last near plant without cards, and a closed seven-sided lathe dome reads as a
boulder rather than undergrowth. It now builds like a tree: a coarser core, five segments by
four rings instead of seven by five, under four card clusters climbing the dome. Card sway
weights are `0.2`, `0.4`, `0.6` and `0.8`, each matching the core weight directly beneath it so
the two cannot tear apart, and every one of those values lands exactly on an 8-bit vertex
colour boundary, which is what makes them assertable at all.

Palette, backlight and bush cards were measured as one matched pair on the merged tree, with
an identical binary and world on both sides; the four-column table above predates that merge
and is not comparable to it. Isolating the scatter as full minus off, its cost moved `+0.06`,
`+0.01`, `+0.02` and `-0.01 ms` across the four yaws and `-0.23 ms` at `1.5x` render scale,
against a scatter-off control that itself drifted up to `0.29 ms` between the two runs. Draw
calls rose by `42` to `60`, the bush card surface, while primitives fell by `58,566` to
`83,910`: the coarser core more than paid for the clusters on top of it. Better silhouette,
fewer triangles, cost unchanged within the noise floor.

Distant crowns and rocks stay rigid on purpose. They are the overwhelming majority of
instances, a tree at `800 m` is a few pixels wide, and its sway would be below a pixel, so
displacing them would be the largest cost in the change and would buy nothing visible.

The wind run is the first change in this experiment that made the frame faster. Draw calls and
primitives are identical to the cards run in every trial, and GPU time falls by `0.27` to
`0.60 ms`, because the two lean shaders replace `StandardMaterial3D`'s general-purpose one on
every near plant. The vertex cost of three sine terms is smaller than what the uber-shader
charged for features these meshes never used. Motion is verified the same way the cost is: two
captures of the same camera at different wall-clock times are pixel-identical with the scatter
off, and differ over 6% of the frame with it on.

Anti-aliasing was measured rather than assumed, and the obvious lever lost. The project had
none of the three modes enabled. `4x` MSAA costs `1.87` to `3.14 ms` across the four yaws and
`4.88 ms` at `1.5x` render scale; `2x` costs `0.95` to `1.71 ms` and `2.60 ms`. FXAA costs
between `-0.14` and `+0.23 ms`, inside the scatter-off control's own drift. On a crop of the
mid-distance tree line, mean pixel-to-pixel difference falls from `31.01` with no filter to
`26.13` under `4x` MSAA and `20.30` under FXAA: FXAA removes more than twice the speckle for
roughly a tenth of the cost, so it is the mode the project enables.

The reason MSAA loses is that it answers the wrong question. It antialiases geometry
silhouette edges and shades once per pixel, while this noise is alpha-scissor card cut-outs,
which it cannot touch without `alpha_to_coverage`, and foliage finer than one pixel, which it
never touches. Note also that MSAA costs the scatter-off control `2.0` to `2.9 ms` on its own:
the vegetation is vertex-bound but the frame is not, and multisampling is a whole-frame price.
FXAA is a partial mitigation, not a fix. Detail below a pixel can only be resolved by
averaging it down, which geometry cannot do and a mipmapped texture can, so impostors for the
mid and far levels remain the actual answer to distance noise.

Three pop-in symptoms share one cause and are tracked as `RENDER-02`: every range is
evaluated once per patch, so half a square kilometre of vegetation switches as a unit, near
trees and all vegetation shadows arrive together at `TREE_NEAR_M`, and branched crowns are
drawn to `800 m` where their detail aliases. `TREE_NEAR_M` cannot be shortened while the LOD
unit is the patch, because a band narrower than the patch diagonal would draw distant crowns
for plants at the camera's feet. Per-plant alpha hashing, which stays in the opaque pass
unlike the removed `VISIBILITY_RANGE_FADE_SELF`, is the fix that also unpins the band.

The September birch/crown revision below addresses the exposed conifer trunk and the
monochrome palette. Its visual result and remaining patch-switch brightness need a later
GPU inspection; the historical GPU sweeps above do not validate this revision.

The same sweep run against the build before the distance gating gives draw calls identical to
the digit and GPU times inside noise. Skipping the near band on a distant patch is an upload
and memory win only: Godot was already culling those instances by their visibility range, and
a culled `MultiMeshInstance3D` issues no draw call. The gain is that the patch never builds
them, not that the GPU stops drawing them.
Canopy density is still roughly one tree per 330 m2 against about one per 10 m2 in a real
stand, and closing that gap needs a canopy representation for mid and far distance rather
than more instances.

### The crowns lost half their ground cover at the LOD switch (2026-09-13)

The player reported that distant forest reads lighter than near forest, and that a stand gains
weight with each click the camera moves in. Top-down, one zoom click changed a closed canopy
into scattered trees on bright grass.

Darkening the ground is not the answer and was already rejected with measurements; see "The
forest floor was shade baked into albedo, and it had to go". The crowns had to do the work,
and they were not there to do it.

**The distant crown was built at half the near crown's width.** `_branched_tree` treats its
`width` as a base: a branch reaches `width * _crown_reach(t) * [0.90, 1.08]`, and foliage cards
hang past each branch tip. `_conifer` and `_broadleaf` took the same `lerpf` expression and used
it as the lathe radius directly, so the distant crown was built at the branch base while the
near crown ended roughly twice as far out.

| | near crown | mid | far |
|---|---:|---:|---:|
| conifer crown width | `9.68 m` | `4.70 m` | `4.68 m` |
| broadleaf crown width | `9.18 m` | `5.78 m` | `7.29 m` |

**The instrument could not see it.** `coverage` is covered pixels over the crown's own bounding
box, and `raster()` scales every level to one apparent height. Both of a crown's size terms are
divided out, so two levels can agree on coverage while one hides half the ground the other does.
The four transitions recorded at `47b51f64` as `+0.76`, `-4.76`, `+1.58` and `-1.85` percentage
points were all measured through that blind spot. They were correct and they were not evidence
of what the player was looking at.

`footprint` is now recorded beside `coverage`: the metre box a crown fills, at true relative
scale, so it is comparable across levels. `coverage` says how solid a crown is, `footprint` how
much sky it takes. Both are in `summary.csv` and `sweep.csv`, and `transitions.csv` reports
footprint against the NEAR crown rather than against the previous level, because two small steps
that agree with each other still open the ground up if both sit below it.

| footprint, against the near crown | before | after |
|---|---:|---:|
| conifer near to mid | `52.8%` | `101.7%` |
| conifer mid to far | `47.1%` | `93.0%` |
| broadleaf near to mid | `47.2%` | `103.9%` |
| broadleaf mid to far | `57.6%` | `88.4%` |

**The distant crown is now measured from the near one.** `_crown_envelope` returns the mean
foliage extent of a near mesh as radius, top and base, averaged over all twelve variants, and
the distant lathe is built to it. This is the same method the distant crown colour already uses,
and it keeps the two in step when the near crown changes. Four things followed:

- The extent is axis-aligned, not radial. A lathe ring puts vertices on both axes, so its radius
  becomes the half width of its box; a radial reach is up to `sqrt(2)` larger and oversized the
  hull by 10 to 16 percent when it was used.
- The base matters as much as the top. An authored `crown_base` hung the distant conifer 3.5 m
  below where a pine carries its foliage, which measured `130%` of the near footprint once the
  width was right.
- The per-variant radius and height spread is gone. The scatter draws variant zero's distant
  mesh for the whole species, so a spread produced no variety and only moved that one crown off
  the envelope, by up to 8 percent in each direction.
- The birch narrowing on the mid level is gone. The envelope already contains it, because eight
  of the twelve near variants are birch. Narrowing again also split the two distant levels,
  which stand for the same mixed stand: mid measured `91%` of the near crown and far `111%`.

Two smaller corrections finished the broadleaf. Its `widest` ring is snapped to a ring the
profile actually samples: the far level has three rings at `t = 0, 1/3, 2/3, 1`, an authored
`0.38-0.49` fell between two of them, and the widest point of the crown was never built. And
`DISTANT_COVERAGE` for broadleaf moved from `0.70` to `0.82`, because a near broadleaf carries
cards that stand out of the crown in depth as well as across it and a surface of revolution has
no depth to spare: its projected extent is already its width.

**Cost.** No triangle was added anywhere, at any level, for either species: the per-variant
counts are identical before and after, and the appearance test still pins them exactly at 28 and
32. Distant vertex counts moved only where two end caps stopped sharing vertices, and the
per-species maximum is unchanged at 77 and 92. The per-variant vertex array in the appearance
test was replaced by a per-species ceiling, which is tighter than what it replaced: it held 81
for conifer against a measured 77. That array pinned which variants happened to share vertices
between their caps, which is not what a distant mesh costs. No instance, draw call, material,
LOD range or atlas changed.

`vegetation_appearance_test`, `vegetation_edit_test`, `vegetation_invalidation_test` and
`vegetation_land_cover_test` each exit `0` with no `SCRIPT ERROR` and no `push_error`.
`tools/test_vegetation_lod_measure.py` runs 3 tests OK.

**Measured on the GPU, and it is free.** Two `E13` captures of `local_gpu_probe.gd` on a
GTX 1060 3GB at `1280x720`, Kuopio `kuopio_324km2_10m`, one binary (`af04b1ed`) and one world
for both, differing only in `tree_species.gd`. `E13` pairs vegetation off against full at four
yaws inside one process, so the scatter's own cost is the difference between a pair and thermal
drift cannot be read as a result. Resident patch counts and prewarm queues matched exactly,
trial for trial, across the two runs.

| scatter GPU cost, `full` minus `off`, p50 | before | after | change |
|---|---:|---:|---:|
| yaw 000 | `6.840 ms` | `6.927 ms` | `+0.087` |
| yaw 090 | `5.759 ms` | `5.983 ms` | `+0.224` |
| yaw 180 | `6.017 ms` | `5.969 ms` | `-0.048` |
| yaw 270 | `5.785 ms` | `5.753 ms` | `-0.032` |
| yaw 180 at 1.5x render scale | `5.923 ms` | `5.825 ms` | `-0.098` |

Three of the five decreased, the mean change is `+0.027 ms` against a `5.8-6.9 ms` scatter, and
the spread between `p50` and `p95` inside a single trial is about `0.25 ms`. The change is below
the noise floor. The `1.5x` render scale trial is the one that matters most, because wider crowns
add shaded pixels and that trial is the fill-bound case; it went down.

Two views gained `0.131%` and `0.015%` primitives and exactly one draw call each; the other three
gained neither. That is one more patch surviving frustum culling, because a wider crown gives its
`MultiMeshInstance3D` a larger bounding box. Video memory is identical to `0.1 MB` in every trial.

**Measured on screen.** In the same captures, against the `off` trial as a control, whose pixels
are identical between the two builds:

| distant ridge, yaw 000 | bare ground | with forest | darkening |
|---|---:|---:|---:|
| before | `151.45` | `134.59` | `16.87` |
| after | `151.45` | `127.56` | `23.90` |

The distant canopy darkens the ground it stands on by `42%` more than it did, which is the
symptom the player reported. The same comparison at yaw 090 moves `18.65` to `20.00`.

Still open: `land_cover.rs` sizes the forest-floor disc from `CROWN_RADII_M = [3.0, 3.5]`, which
was matched to the narrow distant crown rather than to the near one it is supposed to shade.

### Dense deliberate planting — VEG-05 (2026-09-13)

The brush now fills a world-aligned **4 m lattice (625 points/ha)** with deterministic
jitter of at most **0.2 m per axis**, independent of the saved canopy spacing and stand
acceptance. The generator's candidate remains an additional point for the existing
restore contract. At the shipped 16 m canopy spacing this adds up to 39 points/ha,
so unobstructed planting approaches 664 plants/ha including those extra positions.
Existing generated plants stay in place. Water, steep ground and built surfaces still
reject every authored point through `placement_clear`; the generator and its constants
are unchanged.

Fine lattice indices never enter the edit store: each world position is mapped through
`cell_at` using the canopy spacing. Only the generator-candidate pass can lift a tombstone,
and only for the generated species. Repainting an individual cleared generated tree as
its original species still prunes the edit; a dense disc also authors the extra lattice
plants, so it intentionally retains those additions. Painting another species leaves
the tombstone in place. Exact-position checks make overlapping stamps idempotent.

Plant stamps reject radii above **256 m** before planning. The tool clamps the planting
radius and preview to that limit, including when switching from removal; removal retains
its 1024 m limit. A stamp plans at most **129² + 65² = 20,866 slots** at the supported
8 m minimum canopy spacing (17,730 at the shipped 16 m spacing), before disc and footprint
rejection. This bounds both work and transient plan storage without cutting holes in a
large requested disc. Larger areas require multiple deliberate stamps.

The two streams use indexed Rayon planning and a serialized commit, with no allocation
inside a candidate body and no new spatial index. For K candidate slots, A authored plants
in the owning cell and H local surface hits, planning costs O(K × (A + log N + H));
commit costs expected O(K). Remote vegetation edits are never scanned. Persistent vectors
allocate only when committing additions, as before.

Fresh verification on this change:

- `cd rust && cargo test --lib vegetation`: 27 passed, 0 failed, 2 ignored, exit 0.
- `cd rust && cargo test`: 1688 passed, 0 failed, 12 ignored; doc-tests 0 passed,
  0 failed; exit 0. The known intermittent road failure did not occur in this run.
- `cd rust && cargo doc --no-deps 2>&1 | grep "warning\[missing_docs\]" | wc -l`:
  output `0`; cargo doc itself exited 0 (grep exits 1 because there are no matches).
- `cd rust && cargo build`: exit 0. The resulting debug library was copied to
  `godot/bin/libmetrum_rise.so` after replacing the shared-checkout symlink with a real file.
- An additional release-profile vegetation run: 27 passed, 0 failed, 2 ignored.

All four bridge commands ran from this worktree's `godot/` directory as
`godot --headless --script res://tests/<name>.gd`, using Godot 4.7.1:

| Script | Exit | SCRIPT ERROR lines | push_error lines | PASS lines |
|---|---:|---:|---:|---:|
| `vegetation_edit_test` | 0 | 0 | 0 | 1 |
| `vegetation_appearance_test` | 0 | 0 | 0 | 1 |
| `vegetation_invalidation_test` | 0 | 0 | 0 | 0 (by design) |
| `vegetation_land_cover_test` | 0 | 0 | 0 | 1 |

The tests cover fine-grid density inside existing forest, canopy-cell storage and removal,
1/4-worker determinism, independent spacing, the stamp limit, flooded footprints, save/load,
and both unchanged species-aware tombstone regressions. Headless checks do not establish
on-screen appearance or GPU cost.

Matched unprofiled release stroke measurements use the existing flat vegetation fixture,
default generator and seed, a 64 m radius at the origin, species 0, and four Rayon workers.
Setup and disc removal occur outside the measured interval; each measured call includes
planning and serialized commit. Criterion uses 20 samples, 1 s warmup and 3 s measurement.
Hardware: Intel i5-3350P, four cores; Rust 1.93.1. Baseline is `9415b2e8` plus only the
benchmark harness; changed API SHA-256 is
`49bbadf6f6964d888a375e32de3d1c258a589bff0a5c28cba4e008927f0b09d8`.

| Build | Plants added per stamp | Wall time estimate | Criterion interval |
|---|---:|---:|---:|
| Before | 50 | 112.24 us | 99.797–131.98 us |
| VEG-05, idle rerun | 862 | 709.83 us | 654.14–797.39 us |

The stroke adds 17.24 times as many plants at 6.32 times the wall time. A preliminary
changed-build run overlapped compilation and measured 1.9277 ms; it is not the acceptance
measurement. Neither accepted timing overlapped this task's compilation or Godot runs.
These CPU timings do not establish the cost of rendering dense vegetation across a world.

From the worktree root, the matched benchmark command is:

```bash
CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_INCREMENTAL=0 RAYON_NUM_THREADS=4 cargo test --manifest-path rust/Cargo.toml --release vegetation_brush_benchmark -- --ignored --nocapture
```

The standard Rust verification set used `CARGO_PROFILE_TEST_DEBUG=0`,
`CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=2` and
`RAYON_NUM_THREADS=4`, retaining the test profile's optimization and assertions. Temporary
files and Godot data/cache/config paths were directed into the worktree's `validation/`.
Those runs happened in a delegated worktree that has since been removed, so their raw logs
and Criterion artifacts are not retained; the figures above are what the run reported. The
merge was verified again on `dev`, and those results are recorded below.

### The paint brush regrew the forest it was painting over (2026-09-13)

Clearing an area and then brushing `+ Rock` over the same ground put the trees back. The bush
brush did the same. Painting the untouched ground next to it worked.

`paint_at` addressed a cleared cell by its removal tombstone, and treated clearing that
tombstone as the whole edit:

```rust
if restore { core.vegetation_edits.set_removed(cell, false); }
else       { core.vegetation_edits.add(cell, plant); }
```

The `restore` branch never reads `plant`, so the species the player selected was discarded on
every cell the player had previously cleared. Clearing a tombstone does not plant anything: it
lets the generator's own candidate grow back, and the generator only ever places conifer or
broadleaf on the canopy grid. The brush therefore regrew the exact stand the player had just
removed, and only over the ground they had removed it from, which is why brushing clean ground
behaved correctly.

The brush now plants the species it was given. The tombstone is cleared only when the
generated species and the painted species already agree; the restored plant is then byte
identical to an authored one, so this stays a storage saving with no observable difference.
For every other species the tombstone stays and the painted species is authored over it. A
cell whose generated candidate a later surface edit has hidden takes that same path, so the
brush fills it instead of counting a restore that puts nothing on the ground.

The test that covered this asserted the defect. `vegetation_repaint_restores_generated_tree_and_prunes_tombstone`
painted species `3`, a rock, and required the generated tree to come back and the edit store to
empty. It passed because the helper it drew its species from returned a hardcoded `0` rather
than the species the generator picked, so the assertion could not see which species had grown.
`first_candidate` now reports the generated species, and a second test paints a rock over a
cleared tree and checks that one rock stands there and no tree does.

`cargo test` is 1685 pass, 0 fail. The four Godot vegetation scripts exit `0` with no
`SCRIPT ERROR`. `road_preview_stream_test` fails here, before and independently of this change,
with a varying number of `junction must update repeatedly before the pointer stops` errors
across runs; it touches no vegetation and is left alone.

Brush density is a separate defect and is not addressed here; see `VEG-05`.

### Vegetation LOD reference measurements — RENDER-02 (2026-09-13)

**Partly superseded:** every coverage and transition figure below is measured with both of a
crown's size terms divided out, and the crowns differed in size by a factor of two. See "The
crowns lost half their ground cover at the LOD switch" above. Kept for the colour and silhouette
measurements, which stand, and for the atlas work.

This is a partial material correction, with GPU acceptance still outstanding. The accepted
near path and all range constants are unchanged. The discontinuity at `TREE_NEAR_M` exchanges
alpha-scissored coverage, foliage backlight, wind and branch geometry together. It is not
solely a silhouette-authoring defect. Mid and far also collapse all variants to variant zero:
the pine/spruce and birch/aspen mixtures survive only in the near band.

**Original instrument and limitations (16 px).** `godot/tests/vegetation_lod_measure.gd` exports actual indexed
vertices, normals, vertex colours, UVs and material bindings from the live catalogue, without
constructing a gameplay scene. `tools/vegetation_lod_measure.py` uses the existing tooling
dependency NumPy to rasterize those arrays on the CPU. It reads every authored DDS alpha mip,
uses repeating bilinear/trilinear filtering, a `0.4` scissor, back-face culling on solid
surfaces, two-sided cards, and a z buffer. Crown bounds are the projected bounds of green
foliage vertices before scissoring; wood inside that rectangle participates in the pixels.
Bounds do not shrink to whichever pixels survive the mask. Each level is independently
scaled to exactly **16 projected crown pixels high** in a 64-square frame. This samples an
800 m transition-sized crown, not close-up meshes at identical world distance. It is one
apparent-size sample, not a calibrated recreation of the reference screenshot's camera.
Four headings (`0/90/180/270` degrees), elevation `45` degrees and pixel offsets `(0,0)` and
`(.5,.5)` give 96 samples across the 12 near variants, and 8 for each runtime distant mesh.
Statistics average those sample fractions and covered-pixel RGB means with equal weight.

Lighting is an explicitly isolated model: linear vertex albedo, a fixed white key in
normalized direction `(0.3,0.5,-0.8)` with radiance `pi`, ambient irradiance `.2`, roughness
`1` Burley diffuse, and the shared palette-gated backlight with its sRGB uniform converted
to linear. The diffuse/backlight relation follows
[Godot's lighting implementation](https://github.com/godotengine/godot/blob/4.5/servers/rendering/renderer_rd/shaders/scene_forward_lights_inc.glsl).
There is no specular, sky, shadow, AO, fog, exposure, tone map or AA. Baseline wind is frozen
at time zero and instance origin zero. Controls make the near cards opaque, remove only
backlight, or advance the existing wind equation to two seconds. The CPU model mirrors
the current supported material equations; changes to those equations require updating the
instrument. It does **not** execute Godot shaders, prove their compilation/pass routing, or
measure GPU time. Godot `4.7.1.stable.official.a13da4feb` exposes only the dummy renderer
under `--headless` here; X11 and Wayland initialization both failed. The supplied aerial
reference was inspected, but no new in-engine capture was possible.

**Phase 1, before any runtime edits**, baseline `6ff3e638`:

| Species | Level | Coverage | Mean lit linear RGB | Indexed vertices | Triangles |
|---|---|---:|---|---:|---:|
| Conifer | Near | 14.1665% | .099427, .074731, .028881 | 2507–3485 | 895–1245 |
| Conifer | Mid | 66.3490% | .051704, .059977, .020682 | 290 | 102 |
| Conifer | Far | 56.3988% | .049625, .057565, .019850 | 77 | 28 |
| Broadleaf | Near | 15.2677% | .155700, .173385, .090827 | 2441–2741 | 865–965 |
| Broadleaf | Mid | 67.3260% | .114438, .136031, .041025 | 274 | 96 |
| Broadleaf | Far | 64.9725% | .110504, .131353, .039615 | 84 | 32 |

Coverage dominates this experiment. Making near cards opaque raises conifer coverage by
**41.1627 percentage points** and broadleaf by **50.3282 points**. Their opaque-card RGB
means become `(.048567,.059386,.019568)` and `(.117267,.144938,.037752)`: which surfaces
remain visible affects covered-pixel colour as well as occupied area. Removing backlight
changes near RGB by only `(-.000147,-.000537,-.000028)` and
`(-.000643,-.002152,-.000085)`, with no coverage change. The proposed second-place ranking
for backlight is therefore not supported under this light. At two seconds, mean coverage
changes by `+.04265 / -.04027` points, but `12.0429% / 10.7756%` of bbox pixels change
occupancy. Corresponding RGB means are `(.098285,.074493,.028641)` and
`(.158520,.176261,.093240)`. Mean absolute image RGB differences per bbox pixel, for
opaque cards / no backlight / two-second wind, are respectively
`.020577 / .000034 / .009556` (conifer) and `.063066 / .000146 / .019931` (broadleaf).

Geometry is not proved sub-pixel: at equal height the mean projected conifer width changes
from **14.9824 to 6.1648 to 6.0908 px**, and broadleaf from **12.5671 to 10.8315 to
13.6240 px**. The opaque-card control still differs from mid coverage by `11.0198 / 1.7302`
points. These are geometric diagnostics, not an additive causal decomposition: the change
in bounding box, normal distribution, variant mixture and internal occlusion interacts
with scissoring. Coverage, geometry and wind matter here; the exact ordering in a shadowed,
tonemapped gameplay frame remains unverified.

**Phase 2 (historical, `1bee14a7`).** Both distant levels used `vegetation_distant.gdshader`, one cached material
for both species. It keeps wood opaque, uses the existing backlight gate and scissoring,
and retains foliage fragments with a deterministic unsigned hash of quarter-metre
object-space cells. The shared retention `.22` comes from the phase-1 near/solid-mid
coverage ratios `.214 / .227`. No screen coordinates or time enter the cutout. This is a
spatial coverage mask, not a distance fade or a population decision. It costs `O(1)` integer
arithmetic per fragment, one interpolated local position, no textures, no CPU tick work and
no additional geometry. It can still alias at small sizes; it is not a filtered impostor.

| Species | Level | Coverage before → after | Lit RGB before → after |
|---|---|---|---|
| Conifer | Mid | 66.3490% → 18.6860% | (.051704,.059977,.020682) → (.054312,.062099,.021999) |
| Conifer | Far | 56.3988% → 13.3966% | (.049625,.057565,.019850) → (.050750,.059095,.020274) |
| Broadleaf | Mid | 67.3260% → 14.2128% | (.114438,.136031,.041025) → (.118060,.139771,.048084) |
| Broadleaf | Far | 64.9725% → 15.8465% | (.110504,.131353,.039615) → (.113840,.137092,.040634) |

The freshly rerun near rows match phase 1 exactly. All exported mesh arrays, including
near material bindings and the distant vertex colours, compare equal to baseline.

| Transition | Coverage delta before → after (percentage points) | Lit RGB delta before → after |
|---|---|---|
| Conifer near→mid | +52.1825 → +4.5195 | (-.047723,-.014754,-.008200) → (-.045115,-.012632,-.006883) |
| Conifer mid→far | -9.9501 → -5.2894 | (-.002079,-.002412,-.000832) → (-.003562,-.003004,-.001724) |
| Broadleaf near→mid | +52.0583 → -1.0548 | (-.041262,-.037355,-.049802) → (-.037640,-.033614,-.042743) |
| Broadleaf mid→far | -2.3535 → +1.6337 | (-.003935,-.004677,-.001411) → (-.004220,-.002679,-.007450) |

Absolute near→mid coverage error falls **91.3% / 98.0%**, and all near→mid RGB errors
shrink. Regressions: conifer's mid→far RGB step grows in every channel; broadleaf's grows
in red and blue, especially blue (`.001411` to `.007450`). Its coverage step reverses sign,
although its magnitude shrinks. Distant wind, pine geometry and variant collapse are not
fixed. The colour mismatch is not solved by matching geometric-area albedo or adding the
small backlight term. Do not mark `RENDER-02` complete or this result visually accepted.

**Primitive and draw budget.** All counts in the phase-1 table remain exact. A fixture
patch with 512 trees of each canopy species keeps **101376 mid triangles / 30720 far
triangles**, each with **two colour surface submissions**: deltas **0 / 0 triangles and
0 draws**. A populated near canopy still has 24 variant buckets × 2 surfaces = 48 canopy
submissions; its delta is also zero. Understory and shadow policy are untouched. These are
mesh/surface counts, not newly measured GPU pipeline counters. Keeping far saves 69.7%
of this patch's mid primitives. The measurements justify testing the material correction
before adding any intermediate geometry; they do not establish an optimal number of
levels or resolve the per-patch range problem. `RENDER-04` remains blocked on distance cost.

**Historical phase-2 verification and artifacts.** These runs preceded `1bee14a7`, over `6ff3e638`. Commands:

```sh
OPENBLAS_NUM_THREADS=1 python3 tools/vegetation_lod_measure.py /tmp/render02/phase1
OPENBLAS_NUM_THREADS=1 python3 tools/vegetation_lod_measure.py /tmp/render02/phase2
OPENBLAS_NUM_THREADS=1 python3 tools/vegetation_lod_measure.py /tmp/render02/phase2-final
python3 -m unittest discover -s tools -p test_vegetation_lod_measure.py
godot --headless --path godot --log-file /tmp/render02/final-tests/vegetation_appearance_test-engine.log --script res://tests/vegetation_appearance_test.gd
# Same direct invocation for vegetation_edit_test, vegetation_invalidation_test,
# and vegetation_land_cover_test. Inspect SCRIPT ERROR independently of exit/PASS.
```

Phase 1 ran before the material edit and phase 2 after it; each has 896 raw raster samples
(224 view/variant/level samples × 4 controls) in `measurements.json`, `summary.csv`, its
mesh export and source hashes. The later reporter also emits `transitions.csv`, reads the
distant coverage default from its shader, and hashes the atlas. It does not alter the
recorded raster equations or values. Both exports exit 0 with **0 SCRIPT ERROR**; the
initial standalone export also exits 0 with **0 SCRIPT ERROR**. The two analytic instrument
tests pass (run three times), checking full coverage, depth/alpha rejection and mip filtering.
The standalone/phase-1/phase-2 export catalogue times are `97.127 / 109.296 / 97.304 ms`;
these single setup samples are recorded separately from the matched benchmark below.
The final reporter run in `/tmp/render02/phase2-final` freshly exports the current tree,
exits 0 with **0 SCRIPT ERROR**, and reproduces all **896** phase-2 samples and summary
values exactly. Its catalogue setup is `106.546 ms`. It includes the transition CSV and
atlas hash. `/tmp/render02/final-source-sha256.txt` also identifies the exact changed
sources and deployed native library used for this handoff.

| Direct vegetation test | Baseline exit / SCRIPT ERROR | Final exit / SCRIPT ERROR | Final output |
|---|---|---|---|
| appearance | 0 / 0 | 0 / 0 | PASS vegetation appearance, shared material, LOD buckets, positions and empty density |
| edit | 0 / 0 | 0 / 0 | vegetation_edit_test: PASS |
| invalidation | 0 / 0 | 0 / 0 | No PASS line by design |
| land cover | 0 / 0 | 0 / 0 | PASS vegetation land cover publication, boundary edits and texture reuse |

The baseline correctness run's catalogue/upload-maximum times were `102.178 / 19.367 ms`
while the CPU raster was active; the final correctness run's were `106.175 / 19.675 ms`.
These are **not matched performance evidence**. Matched,
unprofiled runs use the existing appearance fixture (4096 placements per patch, four
warmup uploads, maximum of the following 24 uploads) sequentially after raster work ends.
The baseline scripts are exact `git show 6ff3e638:<path>` copies under
`/tmp/render02/benchmark/`, with only preload paths redirected to the copied baseline
species/renderer; the after runs use live scripts. Same official Godot executable and
deployed native library, default worker settings, no Rust rebuild, profiler or GPU renderer.

| Run order | Build | Catalogue ms | Upload maximum ms | Exit / SCRIPT ERROR |
|---:|---|---:|---:|---|
| 1 | Before | 97.469 | 18.231 | 0 / 0 |
| 2 | After | 100.310 | 28.055 | 0 / 0 |
| 3 | After | 112.892 | 19.010 | 0 / 0 |
| 4 | Before | 106.679 | 24.958 | 0 / 0 |
| 5 | After | 107.675 | 20.273 | 0 / 0 |
| 6 | Before | 97.390 | 19.401 | 0 / 0 |
| 7 | Before | 103.671 | 27.436 | 0 / 0 |
| 8 | After | 107.345 | 19.301 | 0 / 0 |

Median catalogue time regresses **100.570 → 107.510 ms (+6.940 ms, +6.9%)**; median upload
maximum is **22.1795 → 19.787 ms**. Wide ranges prevent claiming a reliable upload speedup.
The extra startup material remains a measured cost, not a performance acceptance. Full
logs/results are in `/tmp/render02/baseline-tests`, `final-tests` and `benchmark`; graphical
probe failures are `/tmp/render02-probe.log` and `/tmp/render02-x11.log` (0 SCRIPT ERROR,
but X11/Wayland initialization errors). Neither the unrelated failing full `run.sh --test`
chain nor Rust tests were run for this rendering-only change. GPU compilation, matched
unprofiled release frame times, other apparent sizes and an in-engine appearance check
remain required before accepting this candidate.

**Follow-up: filtered distant crowns and shadowed wood (`render02-tune`).**
The quarter-metre point mask is replaced by two deterministic object-space hashes at
adjacent power-of-two cell sizes. The maximum length of the local-position screen
derivatives selects the octave: `max(log2(footprint * 4), 0)`. Cells span approximately
half to two pixels. Their samples interpolate continuously, then use the CDF of a weighted
sum of uniform samples to avoid the retention dip of an uncorrected hash interpolation.
This is a procedural footprint filter, not a texture mip chain or temporal antialiasing.
The base crown retention was **.18** at this step, measured down from .22 because the
changed sampling increases the 16 px fill fraction. The recalibration below supersedes
that single figure with one retention per species. Between **4 and 8 metres per pixel**, a smoothstep
raises crown retention to one. This retention schedule never decreases with footprint;
finite pixel counts and sampled lighting can still fluctuate. Wood uses the same sample
with only **.04** retention and linear albedo **(.025,.030,.010)**, so it cannot remain as
an opaque bright stick. A fixed **1.08** foliage albedo multiplier was added at this step
to improve the RGB-vector match to near after removing bright wood. It is removed below,
because the near crown itself changed and the means now agree without it. Backlight still
uses the original palette gate, so recoloured wood does not acquire foliage backlight.

The fragment path remains **O(1)** with no textures, allocations, CPU simulation work or
new interpolants. It now evaluates two hashes, derivatives and octave/CDF arithmetic;
unchanged draw counts do **not** establish unchanged GPU cost. `tree_species.gd`, mesh
arrays, surface/material/instance counts and all ranges are unchanged. The complete live
before/after mesh exports compare equal. The shader token ban remains intact.

Fresh 16 px baseline (`1bee14a7`) → follow-up transitions, with linear covered-pixel RGB:

| Species / transition | Coverage step, pp before → after | RGB delta before → after |
|---|---:|---|
| Conifer 0->1 | +4.5195 → +3.6527 | (-0.045115,-0.012632,-0.006883) → (-0.030686,+0.005211,-0.001407) |
| Conifer 1->2 | -5.2894 → -3.1652 | (-0.003562,-0.003004,-0.001724) → (-0.009823,-0.011367,-0.003933) |
| Broadleaf 0->1 | -1.0548 → +1.7292 | (-0.037640,-0.033614,-0.042743) → (-0.032666,-0.025223,-0.046911) |
| Broadleaf 1->2 | +1.6337 → -1.0494 | (-0.004220,-0.002679,-0.007450) → (-0.009794,-0.011514,-0.003524) |

All four default-height coverage steps are inside ±5 pp; the fresh baseline conifer
mid→far value was actually **−5.2894 pp**, slightly outside that band. Near→mid RGB-vector
error (Euclidean norm) decreases for both species. This is not an all-channel improvement:
broadleaf near→mid blue regresses, conifer mid→far regresses in all channels, and broadleaf
mid→far red/green regress while blue improves. Forest appearance is not established by
these isolated means.

`--height` still defaults to 16 and retains `measurements.json`, `summary.csv` and
`transitions.csv`. The additional `--height-sweep` defaults to
`64,48,32,24,16,12,8,6,4,3,2`; values outside 2–64 are rejected before export (64 is the
frame height). `sweep.csv` averages the same headings, offsets, variants and four control
modes for each species/level/height. Pixel counts are sample means, coverage is the mean
of sample fractions, and RGB is the mean of covered-pixel means; an empty tiny mask has
zero coverage and RGB `(0,0,0)`. Up to four spawned standard-library workers measure
independent heights, preserving requested CSV order and reusing the single-height rows.
The work bound is O(number of heights × original raster cost), outside the game runtime.

The Python branch mirrors the shader's hash constants, octave/CDF equations, analytic
triangle derivatives, smoothstep and wood retention/albedo. A separate
scalar translation checked 49,152 hash/filter samples with **zero** discrepancy and
continuity around the .5/1/2/4/8 m octave boundaries. This validates the equations, not GPU
floating-point equivalence: the CPU raster uses NumPy doubles and analytic derivatives,
whereas GPU derivatives operate on fragment quads. The small new unit check compares
4 px and 64 px coverage over an isolated canopy plane's headings and pixel phases.

**The near crown was the defect all along: the foliage mask (`15fefe4a`).**
Every measurement above compares distant crowns against a near crown that was itself
wrong. At 16 px the near crown covered **.1417** of its own silhouette for a conifer and
**.1527** for a broadleaf. The `opaque_cards` control puts the same meshes at **.5533**
and **.6560**, so the geometry was never the shortfall: the baked alpha mask removed about
three quarters of it. The conifer's mean near colour came out at `(.0994,.0747,.0289)`,
redder than it was green, because most surviving pixels were bark rather than needles.

Three independent defects in `tools/bake_foliage_atlas.py` produced that:

- Each drawn cluster occupied only **.42 to .63** of its atlas cell, measured as the
  bounding box of alpha above the .4 threshold. A card quad maps the whole cell, so up to
  58% of every card was guaranteed empty. `fit_cell` now scales each cluster about its own
  centre until it fills the cell. Scaling about the centre, rather than recentring, keeps
  the cluster aligned with the branch tip its card hangs from.
- The clusters were sparse inside that extent, at **.29 to .55** covered. Counts and widths
  rise: 78 blades instead of 52 for the generic broadleaf, 7 twigs instead of 5 and 26
  needle pairs instead of 23 for the conifer sprays. Measured cell coverage rises from
  **.158-.231** to **.434-.502**.
- `correct()` chose the representable coverage closest to the target. Once a mip holds one
  texel per cell the closest choice to .44 is zero, so a card vanished outright. A 512 px
  atlas with 2x2 cells reaches that mip at roughly 800 m, the far edge of the near band. It
  now takes the smallest representable coverage that **reaches** the target, so an
  unresolvable cluster fills instead of disappearing. This is the same principle the
  distant hash uses above. On the larger mips the representable ratios are dense and the
  choice does not move; the corrected chain now reads .467, .467, .467, .467, .469, .477,
  .500, .562, 1.000 from level 0.

No geometry changed. Vertex and triangle counts, surfaces, instances, draw counts, LOD
ranges and the 512 px atlas size are all identical. More fragments survive the scissor.
E13 measured this scatter as vertex- and draw-bound rather than fill-bound, so this is the
cheap direction, but **the GPU cost is not measured**.

Near silhouette fill then required the distant retention to be recalibrated upward, and the
two species need different values: a solid spruce cone fills far more of its own bounding
box than a broadleaf dome fills its own, so one shared figure cannot match both. There are
now two materials sharing one shader, at **.50** for conifer and **.70** for broadleaf.
Placement already draws the species separately, so the second material adds no draw call.
The `1.08` foliage gain is removed with it.

Silhouette fill against apparent size, `baseline` mode, before and after:

| Species / level | 64 px | 48 px | 32 px | 24 px | 16 px | 12 px | 8 px | 6 px | 4 px |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Conifer near, before | .202 | | .171 | | .142 | | | | |
| Conifer near, after | .362 | .347 | .345 | .355 | .416 | .475 | .551 | .575 | .604 |
| Conifer mid, after | .355 | .364 | .366 | .401 | .424 | .421 | .442 | .469 | .552 |
| Conifer far, after | .336 | .340 | .350 | .357 | .376 | .363 | .246 | .383 | .677 |
| Broadleaf near, before | .334 | | .274 | | .153 | | | | |
| Broadleaf near, after | .473 | .474 | .479 | .485 | .500 | .507 | .540 | .610 | .785 |
| Broadleaf mid, after | .455 | .457 | .490 | .504 | .516 | .521 | .545 | .590 | .705 |
| Broadleaf far, after | .428 | .428 | .445 | .460 | .498 | .467 | .539 | .494 | .578 |

The before rows fall as a tree shrinks. That fall is the measured form of a forest that
gains weight with every step the camera takes toward it. The after rows never fall outside
sampling noise, which is the invariant this section now holds the catalogue to. The rise
below about 8 px is deliberate on both paths: a cluster whose cell no longer resolves
fills, and a real forest at that range is a solid dark mass. Values at and below 8 px are
heavily quantized, and the conifer far dip at 8 px is that quantization on a three-ring
lathe, not a retention change.

Default-height (16 px) transitions, against the `1bee14a7` measurement:

| Transition | Coverage step, pp at `1bee14a7` | pp now |
|---|---:|---:|
| Conifer near->mid | +4.5195 | +0.7574 |
| Conifer mid->far | -5.2894 | -4.7604 |
| Broadleaf near->mid | -1.0548 | +1.5754 |
| Broadleaf mid->far | +1.6337 | -1.8487 |

Every near->mid and mid->far RGB delta is now within `.0096` on each channel, against
`.0427` for the worst channel at `1bee14a7`. The conifer near mean turns from
`(.0994,.0747,.0289)` to `(.0535,.0614,.0207)`: green now exceeds red, so the near spruce
reads as needles rather than as bark.

One measurement bug was fixed alongside this. `raster()` framed the silhouette with a box
of exactly the projected size, which at the smallest sweep steps can contain no pixel
centre at all and left the coverage ratio with an empty denominator. That is why the first
`sweep.csv` was empty. The frame is now held at a minimum of one pixel either side of the
centre.

Still unverified: GPU shader compilation, frame cost and on-screen appearance. Patch
granularity, distant wind, and the collapse of every variant to zero at mid and far range
all remain open.

### Accepted canopy land cover — RENDER-05 (2026-09-12)

Before this change the ground mask had been completely removed: `stand_field()` only
selected candidate stands in Rust, and the terrain shader had no land-cover input. The
previous roadmap statement about a shipped water/slope-gated shader mirror was stale.
The following contract supersedes the historical ground-mask experiments below.

`get_vegetation_land_cover` publishes derived R8 bytes and world bounds for each terrain
patch. The world-aligned texel size is **8 m**, independent of saved `canopy_cell_m`.
A full 510 m patch carries 66–67 texels per axis including a one-texel filter border
(4356–4489 bytes). Neighboring patches publish identical samples where their borders overlap,
so the shader's single bilinear texture fetch is continuous across patch edges.

Generated stems pass the same `evaluate_cell` as the scatter; tombstones suppress them,
and authored canopy additions pass the same `placement_clear` on every rebuild. Building
site queries are prepared before the parallel walk. Authored bushes and rocks use the canopy
address grid but contribute no crown coverage. Each tree contributes a disc with a species
substrate radius of 3 m (conifer) or 3.5 m (broadleaf), multiplied by its accepted instance
scale. These footprints are independent of cosmetic mesh variants, LOD and wind. An 8×8
fixed sample grid per texel forms the crown-disc union at 1 m spacing, converted to an R8
area fraction. Atomic bitwise OR preserves overlap and produces identical bytes regardless
of Rayon scheduling; no candidate body allocates. The build allocates one temporary patch
bitmask buffer (~35 KB); conversion allocates the packed bridge output once. The final
~4 KB contiguous R8 packing pass is deliberately serial; indexed candidate work uses Rayon.

Cost is O((K + A) × indexed footprint-query cost + T): K candidate cells and A authored
additions in the patch plus crown/filter border, and T fixed-size output texels. A crown
touches at most nine texels with 64 samples each. The existing index preparation may pay
for a dirty building-site index rebuild once; no world population scan or new spatial index
is introduced. Coverage is built on residency/invalidation, never per tick or fragment.

Textures follow terrain's active/spare image and `ImageTexture` ownership, including resource
pool reuse and terrain stage/commit. Unchanged coverage is reused during unrelated terrain
mesh staging. Plant-only and neighboring surface changes refresh at most one resident
coverage patch per frame after terrain/network publication. Validity compares the **two
existing** revision streams in the 3×3 patch neighborhood, including nonresident neighbors;
there is no coverage generation, no new edit ownership, and no save-format change. Capturing
revisions and candidates under the same core lock prevents stamping old coverage as current.
Staged terrain rejects a coverage revision mismatch before publication.

The material blends needle litter/duff and moss hues, matching the incoming grass albedo's
luminance before blending. Coverage changes no lighting, shadow visibility, grass palette,
chroma gain or hillshade tint. Tree shadows remain responsible for canopy light occlusion.

Fresh headless validation uses the release GDExtension from `cargo build --release`, copied
to `godot/bin/libmetrum_rise.so`. Direct commands are
`godot --headless --path godot --log-file /tmp/render05/<test>-engine.log --script res://tests/<test>.gd`.
`vegetation_edit_test`, `vegetation_appearance_test`, `vegetation_invalidation_test`,
`vegetation_land_cover_test` and `network_tool_chunk_renderer_test` each exit **0**, with
**0 `SCRIPT ERROR` entries**. The new native test covers R8 publication, a boundary plant,
clear-cut/removal, both revision streams, stage isolation and `ImageTexture` reuse. The Rust
oracle compares every texel with accepted canopy products, allowing only the <0.1 mm
roundoff from reconstructing packed local f32 positions; overlapping border bytes must
match exactly. Logs are under `/tmp/render05/`.

Fresh Rust verification (`cargo ... --manifest-path rust/Cargo.toml`):
`fmt --check` exits 0; `test --release` exits 0 with
`test result: ok. 1684 passed; 0 failed; 11 ignored; 0 measured; 0 filtered out; finished in 74.99s`
(and an empty passing doc-test target). `doc --no-deps` exits 0 with **0** matches for
`warning: missing documentation`. `clippy --release` exits **101**, with 494 warnings and
the pre-existing `clippy::mut_from_ref` error at
`rust/src/simulation/economy/agents/tick/slices.rs:37` (`get_mut(&self) -> &mut T`). That
file is identical to parent `9ccc236b`; this task does not change its unsafe storage contract.
Full output is in `/tmp/render05/{fmt,rust-test,clippy,doc}.log`.

Fresh, unprofiled release timing on the Intel i5-3350P (four cores), rustc 1.93.1,
`RAYON_NUM_THREADS=4 cargo test --release vegetation_edit_benchmark -- --ignored --nocapture`
from `rust/`, exits 0. The existing Criterion fixture uses a 510 m patch at `(-255, -255)`,
default generator settings, 50 samples, 2 s warmup and 6 s measurement per arm. The painted
case is the existing 255 m brush-disc fixture; index preparation and background edit setup
are outside timing. These are matched controls in one run, after compilation and other tests
finished. Build identity: parent `9ccc236b` plus the RENDER-05 sources listed in
`/tmp/render05/source-sha256.txt` (coverage source SHA-256
`18141bafb5990b4876742793265756ba43701e99699334c895ee4446d7c0681d`).

| Background edits | Canopy scatter | Added coverage build | Painted scatter | Added painted coverage build |
|---:|---:|---:|---:|---:|
| 0 | 212.60 µs | 277.36 µs | 617.83 µs | 548.69 µs |
| 100,000 | 212.28 µs | 236.67 µs | 658.32 µs | 646.90 µs |

These coverage costs are **additional per patch**, for candidate acceptance and crown-disc
rasterization; they exclude R8 bridge conversion and Godot/GPU texture upload. The untouched
coverage confidence intervals are 256.55–296.58 µs and 234.06–239.88 µs; painted intervals are
544.38–553.55 µs and 621.78–680.91 µs. The painted background case is about **18% slower**,
so this run does not establish that background edits cost nothing. Inspection bounds the
candidate walk to the same local cells; authored edit lookups use the existing sparse store.
The brief's 242/632 µs scatter figures are historical, not current-build validation.
Raw output: `/tmp/render05/benchmark.log`; Criterion samples/reports:
`rust/target/criterion/VegetationEdits/`.
No game launch or GPU capture is part of this task; **on-screen appearance is unverified**.

### The generator became the saved population (2026-09-12)

The scatter's stand layout, density and arrangement were four constants in
`vegetation_api.rs`. They are now `VegetationConfig` in `rust/src/simulation/vegetation.rs`,
chosen per world and persisted in the save at format version 60. That is the whole vegetation
population: a Finnish stand density over the default world is around `10^10` stems, so storing
the trees is not an option, while storing the parameters that generate them costs sixteen bytes.
Player edits now ride on top as a sparse authored delta (`VEG-01`, save format 61).
`VegetationEdits` keys tombstones and authored additions by `(layer, cell_x, cell_z)`;
`evaluate_cell` is the single generated-candidate decision used by queries, removal and paint.
Queries retain O(K) bounded candidates, one expected O(1) edit lookup per cell, and O(A)
work for authored plants in those cells. Storage is O(edits), with no additional spatial index.
Parallel candidate evaluation allocates nothing per cell; actual edit commits own the sparse
store's persistent allocations. Point placement validates the footprint immediately; brush
stamps restore tombstones or fill rejected canopy candidates with the original jitter, yaw and
scale. The player addresses the canopy grid (including authored bush/rock species); the generated
understory remains a near-only layer. Authored placements remain visible when generation is disabled.

Vegetation patch revisions use the terrain renderer's layout but do not change terrain surface
revisions. Only patches containing changed plants advance. Save rows sort by layer, cell_z and
cell_x, preserving addition order within a cell; versions 58–60 load with no edits. Save/load
reproduces the packed placements. Existing save timestamps still vary with wall-clock save time.
A road, building site or terraform placed later clears an authored plant exactly as it clears a
generated candidate: `scatter_layer` re-runs `placement_clear` over the authored plants it is
about to return, as one parallel pass allocating once for the patch and skipped entirely for a
patch that holds none. The verdict is re-derived per fetch rather than stored, so removing the
surface restores the plant bit for bit, which is already how a generated candidate behaves. The
existing terrain payload revision drives the rebuild, because every road, site and terraform edit
funnels through `bump_terrain_payload_patch_generations`, which `_is_patch_stale` already reads.
Clearing a radius still finds and counts an authored plant hidden under a surface, which prunes
it from the store.

The player reaches all of this through `vegetation_tool.gd`, a Terrain-submenu tool with two
modes - plant one species, or clear plants - drawn as a ground ring at the cursor's surface hit.
The radius is the only size control, and it also selects the dispatch: at `MIN_RADIUS_M` a plant
lands exactly under the cursor through `add_vegetation_at`, and above it the disc fills the
4 m planting lattice plus generator candidates through `paint_vegetation`. That removes the point/brush mode and the
radius control from the panel, which leaves a preset choice and a clear. Ctrl and the mouse wheel
step the radius geometrically, because no fixed step serves both a one-tree cursor and a 1 km
clear-cut, and stepping down clamps on the minimum so the point dispatch stays reachable. The
camera must not zoom on that combination, so `_handle_zoom_wheel` refuses a ctrl-held wheel.
Shift and the wheel cycle the tool-owned ordered brush options (Conifer, Broadleaf, Bush, Rock),
wrapping in both directions and synchronizing the dropdown without firing its selection handler.
Each option maps a display label to an existing native species ordinal; adding an option requires
only extending that list. The popup forwards both wheel gestures; Ctrl takes priority when both
modifiers are held, and Shift also suppresses camera zoom. `E` toggles planting/erasing while the
vegetation tool is active, sharing mode state with the Remove toggle. The ring reports footprint
and mode by size and colour. On option or mode changes, an outlined billboard `Label3D` above its
centre shows the option name or `Remove`, using `UIStyle.TEXT_PRIMARY` or `TEXT_ALERT`. The label
holds for one second and fades over 0.25 seconds; projection keeps its size and offset constant
in pixels, including at the ring's last position during popup input grabs. Navigation and label
updates are O(1), independent of world population and brush radius; the label is allocated once.
A stroke continues on held-button motion whenever it is a brush or a clear, and restamps
after half a radius of travel, so it overlaps without issuing a native call per pixel.

Every one of those stamps is reversible. `add_vegetation_at`, `remove_vegetation_at` and
`paint_vegetation` each record a bounded inverse journal before they touch a cell: one entry per
changed cell holding that cell's delta as the stroke found it, plus the render-patch keys whose
revisions have to advance again on the way back. Storage is O(changed cells), and a cell the
generator still owned stores `None` and allocates nothing, which is the whole of a clear-cut over
unedited forest. A dragged stroke would otherwise fill the 30-entry history with fragments of one
gesture, so the tool mints a stroke id on the press that opens a drag, every stamp of that drag
carries it, and a stamp merges into the entry on top of the stack only when that entry carries the
same id, keeping each cell's earliest record. The id is the whole point and a "this is not the
first stamp" flag is not enough: a stamp that changes nothing journals nothing and pushes no
entry, so the first stamp of a clear that lands on bare ground leaves the previous action on top
of the stack, and the stamps after it then attached the clear to the planting underneath. One
`Ctrl+Z` reversed both. A point placement carries stroke zero, which matches no entry including
another zero, because one click is one action. Restoring a
journal is independent per cell and only has to change each patch revision, so the hashed
iteration order cannot alter the result. `Ctrl+Z` reaches this through the existing global
`undo_action`, which the vegetation tool does not intercept.

An authored plant can now name the modelled tree it is, not only its species. A species covers
several meshes that the renderer picks between from the placement's appearance seed - two of
every three conifer variants are pine and the rest spruce, two of every three broadleaf variants
birch and the rest aspen - so a conifer planted before this was a lottery the player could not
call. `AuthoredPlant` carries a `variant` pinned over that choice, biased by one so that zero
means the seed still decides, which is every generated plant and every plant authored before the
pin existed. It is persisted in save format 63; a save below that has no column and reads back
unpinned, which is the behaviour it was written under. A pin past the variants the renderer
models for that species is rejected at load rather than drawn as nothing.

Lane five of a packed placement carries both, the species ordinal in its low two bits and the
biased variant above them, because widening the stride from six floats would cost the whole
scatter buffer a seventh for a field that is zero on nearly every instance. An unpinned plant
therefore packs to the bare species ordinal exactly as it did before, so the generator's output
is unchanged bit for bit. The split reads out to `TREE_NEAR_M` and no further: past 800 m every
plant of a species draws variant zero, so a planted spruce stand reads as spruce up close and as
generic conifer in the distance.

The brush is now driven by named presets rather than by a bare species ordinal.
`vegetation_api/brush.rs` owns the table, because a preset decides what is planted and that is
a simulation decision, not a label: each entry carries the fraction of its lattice it keeps, the
instance scale band it plants in, and a weighted mix of species and the meshes each may use.
`paint_vegetation` and `add_vegetation_at` take a preset ordinal, and `BRUSH_OPTIONS` in
`vegetation_tool.gd` only names the presets and fixes the order the wheel cycles them in. The
first four ordinals are the species the brush offered before, unpinned and at full density, so
an unpinned preset produces the same plants it produced before the table existed.

The table holds eleven and the brush offers nine. Four are unpinned species, four are the named
trees (pine, spruce, birch, aspen), and three are mixes. `Mixed forest 1` keeps 0.85 of the
lattice at Finnish growing-stock weights - pine 50, spruce 30, birch 15, aspen 5 - which is
about 530 stems/ha against the real 400-700 of a managed stand. `Mixed forest 2` keeps 0.10,
about 62 stems/ha, and widens the scale band to `0.95-1.45` because a tree with room around it
is larger than one that grew up under a canopy. `Mixed forest 3` keeps 0.35, narrows the mix to
birch and pine, and lowers the band to `0.45-0.75`. The three are numbered rather than named
because only the first describes a real stand: the other two move the density dial and the size
dial without yet reading as any particular place.

The brush does not offer the unpinned conifer and broadleaf. Once pine and spruce are both
nameable, "conifer" is an unnamed two-to-one mix of them, inherited from how the renderer
numbers its variants rather than chosen, and a mix belongs in a mix preset where its ratio is
written down. They stay in the table because they are what the generator plants, and therefore
what a repaint of a cleared cell has to match to collapse back to no stored edit.

Density presets can only thin. The lattice stays at the 4 m `VEG-05` shipped, which is already
625 points/ha and above a real stand, so a preset subtracts from it rather than tightening it;
nothing here raises the instance count a stroke can reach. Thinning and the mix are pure
functions of the lattice cell and the stream's salt, so a stroke thins to the same plants
however its points are ordered and a repeat of it adds nothing rather than filling its own gaps.
The mix pick is a scan over a handful of weights and the variant pick is an index, so neither
allocates per lattice point. A stroke also clears nothing: painting a sparse mix over standing
forest adds scattered trees to it and does not thin the forest to match.

A preset costs the stroke a little. Matched fresh release runs on four workers, same machine
and same session, price a 64 m stroke of 862 plants at `738.74 us` without the preset table and
`793.08 us` with it, which is `+54 us` or `+7.4%`. The per-point work a preset adds is a
thinning compare, a weighted pick and a scale-band compare, all O(1) and none allocating; a
single-choice preset skips the weighted pick entirely, which is worth `56 us` of that on its
own. The path is per stroke and not per tick, and the figure sits against the `112.24 us` a
50-plant stroke cost before `VEG-05` made the brush dense. The `709.83 us` recorded under
`VEG-05` is an older build and is not comparable directly: the same commit measures `738.74 us`
here.

One consequence of the pin reaches back into the tombstone contract. A stroke that repaints a
cleared cell with the species the generator had chosen still collapses to no stored edit, but
only while it is unpinned. A player who named the tree gets that tree authored over the
tombstone instead, because the mesh the generator's own seed would regrow is not the one they
asked for.

Bulldoze can finally see a tree. `get_bulldoze_target_at` answered `building` and `road` only,
so the one class of object the vegetation brush had just made addressable was the one the
bulldoze cursor ignored, and clearing a single tree meant opening the Terrain submenu. It now
resolves a vegetation target after both of those and never before them, because a plant under a
building or a road is hidden rather than clickable. The lookup is `plant_at` in
`vegetation_api.rs` rather than in `editing.rs`, because the lattice arithmetic and
`evaluate_cell` belong to the module that owns them; it stays immutable, since bulldoze already
prepares the same building-site query index the brush does. It visits a fixed 4 m cursor disc
over both layers, which is at most four canopy cells and nine understory cells at the shipped
spacings, costs O(K + A) in those cells' candidates and additions rather than in world
population, and allocates nothing per candidate. Ties break by layer, then row-major cell, then
authored order, so two runs over the same state always pick the same plant.

It targets exactly what the renderer draws, which is why the clearance test is asymmetric: a
generated candidate was already cleared inside `evaluate_cell`, so only authored plants are put
through `placement_clear`, the same split `get_decorative_tree_patch` makes when it packs a
patch. The target id packs the plant's stored `f32` position bits, because the authored vector
is compacted on removal and an index into it does not survive an unrelated clear in the same
cell, while the position does; re-resolving at the target's own centre therefore finds that
plant at zero distance and reproduces its id, which is what `bulldoze_prepared_target_internal`
checks before it deletes anything.

Deletion adds no mutator. It calls the brush's own `remove_at` at radius zero and stroke zero,
so a bulldozed plant writes the same tombstone or the same authored removal the tool's clear
writes, reuses its inverse journal for one undo entry per click, and advances the same render
patch revision. Radius zero is an exact position match, which is also why a plant sharing an
exact position with another is refused rather than targeted: that mutator would take both, and
bulldoze is one target per click. The hover shape is a sixteen-point ring at the plant's crown
radius scaled by its instance size, so the existing polygon path draws it unchanged and
`bulldoze_tool.gd` gains only a `kind` branch; bushes and rocks have no entry in the canopy
crown table and fall back to a metre. The cursor ray still resolves against the ground, so the
player aims at the trunk rather than the crown and the 4 m pick radius is what makes that
forgiving.

`vegetation.gd` ORs the independent vegetation revision into `_is_patch_stale` and stamps
it alongside `surface_generation` at upload, reading both before the placement fetch so an edit
landing mid-fetch is not stamped as already rendered.

They are saved state and not a video setting. Once a tree can be planted and cleared it is
gameplay state, so two machines loading one save have to generate the same forest; a dense world
on a weak machine runs slower rather than thinning itself out. Saves written before version 60
load the shipped values, so an existing city keeps the forest it was built in.

**Coverage and density are separate axes with different costs.** Coverage is the fraction of the
world inside a stand. It only moves a threshold, so it is free: instance count follows the area
it selects. Density is canopy stems per hectare, and it is delivered by sizing the grid cell
rather than by changing the acceptance rate, so the accepted fraction of candidates stays put and
only the spacing moves. Halving the cell quadruples canopy instances and sixteens the understory,
which is why density is clamped at `121.875` stems/ha (an `8 m` cell, four times the shipped
grid). That ceiling is a renderer limit and not a generator one: a real Finnish stand is
`500-1500` stems/ha, which needs a `2.6-4.5 m` cell and `13-38x` the instances. `RENDER-04`
raises it. The default `30.46875` stems/ha is the shipped `16 m` grid at `0.78` acceptance kept
exact, so the default world generates the population every measurement in this file was taken
against. Cell size is quantised to the millimetre, because the spacing decides how many candidate
cells a patch covers and a float ulp either side of a round number could change that count at a
patch boundary.

**The two-sine field had to go, and not because of its shape.** `woodland()` was
`sin(x * 0.0031) * cos(z * 0.0043) + sin((x + z) * 0.008) * 0.3`, with wavelengths fixed in world
metres at `2027 m` and `785 m`. Across the `500 m` sandbox world in `user://worlds` the field
never completes a period: evaluating its corners puts the entire map inside one stand bar a
single corner sample that clips the threshold by `0.006`. A small map therefore had no stand
structure at all and a coverage dial would have done nothing on it. The replacement is three
octaves of smoothed value noise at `500 m`, `225 m` and `90 m`.

Coverage means an area fraction on any world, and it is measured rather than mapped. The field is
a sum of octaves with no closed-form distribution, so `resolve` sweeps it on a `192x192` grid over
the world, sorts, and takes the matching quantile as the threshold. That is `37k` samples and one
sort, once per world load, and never on a per-patch path. The default coverage of `0.565` is the
measured stand fraction of the two-sine field over the `18 km` world, so the default world keeps
the amount of forest it had while the stands themselves move.

**Global coverage being right says nothing about what one player sees, and the first attempt got
that wrong.** Scaling the stand feature size to an eighth of the world gave the `18 km` world
`2250 m` features, which is eight noise lattice cells per axis. Global coverage measured a correct
`0.565` while the `2 km` window around spawn measured `0.002`: the first E13 probe of that build
returned `43,162` resident trees against `80,815`, `2.26 M` primitives against `9.52 M`, because
the near patches had become open ground. Sampled across 225 spawn windows the local coverage ran
`0.000` to `1.000` at a standard deviation of `0.316`. The old two-sine field's own standard
deviation was `0.007`, which is the real reason it looked stable everywhere: it was not a stand
pattern at all but a regular `2 km` ripple, and every window of it held the same 56% forest.

The stand size is therefore a metric constant, `500 m`, and only a world too small to hold one
compresses it to an eighth of its shorter side. Across the same 225 windows that gives mean
`0.551`, standard deviation `0.123`, range `0.199` to `0.816` - landscape variation instead of a
coin flip - and the `2 km` window at spawn measures `0.566` against the old field's `0.564`,
which falls out of the scale rather than being fitted to it. A test pins the local range, because
nothing else would catch a retuned feature size putting the player back in a meadow.

One consequence is honest to record: peak cost now depends on where you stand. The densest `2 km`
window carries `0.816` coverage against `0.566` at the probe viewpoint, and that window is the
near understory ring, so the worst spot on the map should sit around `1.4x` the near geometry
measured below - roughly back at the `9.52 M` primitives and `12.26 ms` that the previous build
paid everywhere. That is an estimate scaled from the coverage ratio, not a measurement; the E13
pivot is fixed and does not visit that window.

The seed mixes into both layers' candidate salts as well as the stand field, so two worlds on the
same terrain differ in where the stands are and in how the plants sit inside them. It is cheap
because the scatter already hashed integer cell coordinates. Rocks do not scale with density:
they are geology rather than forest.

`scatter_layer` is now a free function rather than a method, because it never used `self` and a
test needs to drive one layer without an engine.

The parameters reach the simulation through `start_new_game`, which is a separate entry point
from `load_world_definition` rather than an extra argument on it. The two do different things:
`load_world_definition` opens the land with the shipped forest and is what the world editor,
the benchmarks and the GPU probes use, where the forest is not the subject and has to stay
fixed for results to stay comparable; `start_new_game` is the gameplay path that carries the
player's choice. Both resolve the config against the loaded extent inside
`reset_to_blank_world_runtime`, and both sanitise it there, so no menu can start a world the
generator cannot reproduce from its save.

The choosing happens in `new_game_dialog.gd`, which opens after a world is picked. Its defaults,
its density range and the grid spacing it reads back all come from `VegetationOptions`, a
`RefCounted` class that exists because the main menu holds no `SimulationNode` and therefore has
nothing to ask. The alternative was four literals in GDScript, which would have outlived the
renderer limit that sets the ceiling: when `RENDER-04` lifts it, the slider follows without a
second edit. Density is shown as stems per hectare next to the cell it produces, because that
cell is what the generator actually varies and it is quantised - a dialog computing it locally
would eventually disagree with the world it started.

### The woodland ground mask is gone (2026-09-12)

Three versions of a forest-floor ground mask shipped in one day and all three were wrong for
the same reason, which is not a colour reason. The shader decided where forest floor was drawn
by evaluating `woodland_field`, a hand-maintained GLSL copy of `woodland()` in
`rust/src/nodes/simulation_node/vegetation_api.rs`. Nothing linked the two at runtime, so the
ground was not being coloured by trees. It was being coloured by a sine wave that the scatter
happens to consult as well.

The consequence is easy to state and was the thing that finally settled it: disabling the
vegetation renderer leaves the mask completely intact. Ground reads as forest with no tree
standing on it, and no per-fragment mirror of a procedural field can ever do otherwise. The
earlier repairs - gating on watermap depth and slope, dropping the authored luminance from
`0.078` to `0.100` to `0.245`, capping the blend at `0.62` - each removed a symptom. The mask
still coloured ground the player never planted and could not clear.

Two things were conflated and both were derived from the same sine wave:

- **Land cover** - moss, needle litter, dwarf shrub. A real substrate difference that persists
  in full sun. This is map data and belongs in a terrain layer beside heightmap and watermap,
  sourced from the accepted population.
- **Canopy occlusion** - shade. This is lighting. It moves with the sun, and the vegetation
  renderer already casts it.

Painting albedo for the second is faking an occlusion term the renderer computes correctly.
The reason the floor does not go dark under our trees is density, not colour: canopy scatter is
about `30 trees/ha` against `500-1500/ha` in a real stand, so there are not enough crowns to
cast the shade. Both reference sources agree. Cities: Skylines 1 paints no forest floor at all
and its ground under a dense tree cluster measures `0.469` against `0.512` for its open ground,
a `92%` cast-shadow difference; its stands still read as stands. An autumn photograph from the
same viewpoint as the in-game captures shows park ground running uniform right up to and under
the trees, with the far hillside dark because it is a wall of crowns.

Removed: the uniforms `terrain_woodland_strength`, `terrain_woodland_floor`,
`terrain_woodland_floor_blend`, `terrain_woodland_litter_scale`,
`terrain_woodland_litter_strength`, `terrain_woodland_edge_break_scale` and
`terrain_woodland_edge_break_strength`; the `woodland_field` mirror; the mask's water and slope
gates; the `woodland_mask` parameter of `apply_natural_land_variation` and the `* 0.55` damping
of its three tints; and the `* 0.35` damping of the grass detail layers. Open-ground colour is
now what the whole land surface gets. `woodland()` stays in Rust, where it governs placement
and nothing else, and its Rust-side pin test went with the mirror it existed to protect.

This is not a deferral of the forest-floor idea. Land cover may come back, but only sourced
from the vegetation that is actually there, which cannot happen while vegetation is a pure
function of position with no identity, no persistence and no way for the player to add or
remove a single tree. That is `VEG-01`, and it is a must-decide before any upstream PR. Until
it lands, no ground shading is derived from `woodland()` in either language.

### Forest floor, understory density and where scatter cost actually lives (2026-09-12)

The ground under a forest used to keep the open-meadow colour, so a stand read as trees
standing on a lawn. The cause was that the terrain never knew where the forest was:
`terrain_surface_color` selected its biome from elevation, macro noise and ruggedness, and
its existing `conifer_dark` entry was reached through `ruggedness * 0.34`, so flat woodland
got none of it while a bare rocky slope got all of it. The scatter, meanwhile, decides where
trees go from `woodland()` in `vegetation_api.rs`, which shares nothing with that selection.

`woodland_field()` in `terrain.gdshader` now mirrors `woodland()`, and the ground blends to
`terrain_woodland_floor` across `smoothstep(-0.25, 0.30, ...)`, straddling the `> -0.1`
placement threshold so the substrate change reads as a boundary between stand and open
ground rather than a painted line. The meadow, scrub and dry-open tints are damped by the
same mask; all three brighten, and layering them at full strength over a forest floor undid
it. (This section originally described the tint as canopy shade and suppressed those three
outright. Both were wrong - see "The forest floor was shade baked into albedo" above.) Rock,
shore and cliff still win, so stone and
waterline are unaffected. The field covers about `57%` of the world.

Nothing in the build links the GLSL mirror to its Rust original, so
`woodland_field_matches_the_terrain_shader_mirror` pins four samples, two accepted and two
rejected, and a change to the formula now breaks that test rather than silently darkening
ground where no forest grows.

**The same density multiplier costs wildly different amounts on different layers, because
cost follows the area each layer draws across.** Density is not free - both experiments below
change density and both change cost. What the range term explains is why the *same* `4x`
costs `19.8 ms` on one layer and `1.89 ms` on the other. Candidate work is the sampled area
over the cell area; render work is the accepted population times mesh cost times pass count,
plus pixel overdraw. Matched `E13` sweeps on one release binary each, same world, unprofiled,
`yaw180` scatter isolated as `full - off`:

| Change | scatter | prims | note |
|---|---:|---:|---|
| baseline | `+5.59 ms` | `7.2 M` | `CANOPY_CELL_M 16`, `UNDERSTORY_CELL_M 8` |
| `4x` canopy density | `+25.4 ms` | `34.2 M` | `CANOPY_CELL_M 8`; unaffordable |
| `4x` understory density | `+8.73 ms` | `12.5 M` | `UNDERSTORY_CELL_M 4` |
| `4x` understory, no bush shadows | `+7.48 ms` | `9.7 M` | shipped |

Draw calls moved by six across the canopy experiment, so this is entirely primitive cost.
Canopy reaches `TREE_FAR_M = 4500 m` and understory only `BUSH_RANGE_M = 420 m`, a ratio of
`115x` in area, which is why quadrupling the understory costs a fifth of what quadrupling the
canopy costs and buys most of the same closed-forest read.

This also corrects an earlier expectation recorded in this file. Shortening `TREE_NEAR_M`
through finer LOD granularity would make the *near* band affordable, but the near band is not
what blocks canopy density: at `30 trees/ha` the band from `2000 m` to `4500 m` covers
`51 km2` against the near band's `2.0 km2`, so distant cones dominate the primitive count even
at `24` triangles each. Canopy density needs the far band to get cheaper — impostors, or a
distance-graded population — and finer near-band granularity is a separate, additional fix.

Understory now casts no shadow. The key light runs four PSSM cascades with blended splits, so
each bush was submitted up to five times, and bushes are the densest species; its own shadow
sits under a canopy shadow that already darkens the same ground. Measured at `1.26 ms` and
`2.8 M` primitives on `yaw180`.

Remaining, unfixed: open ground keeps a vivid saturated green that comes from the grass albedo
photo mixed at `terrain_grass_albedo_strength = 0.90`, not from the authored palette, and the
conifer patch-switch brightness step is not albedo. The area-weighted near-mesh mean derived
below moved the conifer distant crown only from `(0.049, 0.115, 0.039)` to
`(0.056, 0.122, 0.042)`, so the step is that the far cone is a solid volume where the near tree
is mostly holes. Albedo cannot express that; impostors can.

### The forest floor was shade baked into albedo, and it had to go (2026-09-12)

**Superseded:** the woodland ground mask this section tunes was removed the same day; see
"The woodland ground mask is gone" above. Kept for the measurements and for why the
intermediate fixes did not work.


Gating the woodland mask on water and slope stopped it appearing under lakes, but it did
not answer the prior question: what is a darkened ground *modelling*? The honest answer is
nothing. It was standing in for canopy occlusion that the scatter does not produce, and a
stand-in painted into the albedo cannot behave like shade - it stays dark in full sun, at
every hour of the day cycle, and through transparent water, which is exactly where it was
first noticed.

The reference photographs settle it. In `imgs/reference/topdown-1.png`, a closed boreal
canopy from directly overhead, **the ground is not visible anywhere**. Every dark value in
the frame is shadowed crown: the darkest decile measures luminance `0.060` at hue `163` and
saturation `0.88`, which is deep saturated green, not the desaturated near-black the floor
uniform held. The whole-frame medians across the three aerials are `0.261`, `0.296` and
`0.278`. Forest darkness is a high-frequency crown-to-crown structure. The mask was a
low-frequency field several hundred metres across.

Cities: Skylines 1 makes the same point from the other side (`imgs/games-external/cs-1.png`).
Its ground under a dense tree cluster measures luminance `0.469` against `0.512` for its open
ground - **92%**, a cast-shadow difference and nothing more. It paints no forest floor at all,
and its stands still read as stands, because the trees do that work.

So the floor is now a substrate change rather than a shade:

| | authored luminance | hue | vs open ground |
|---|---:|---:|---:|
| first version | `0.078` | `86` | `23%` |
| second version | `0.100` | `86` | `30%` |
| **now** | **`0.245`** | **`62`** | **`73%`** |
| open ground (`inland_base`) | `0.334` | `72` | - |

Moss, needle litter and dwarf shrub are genuinely browner and less saturated than meadow
grass, and genuinely close to it in value. That is all the ground can honestly say. Three
further changes stop the mask behaving like paint:

- `terrain_woodland_floor_blend = 0.62` caps the blend, so meadow always shows through and
  the macro variation and elevation ramp survive inside the stand. Full replacement is what
  made it one flat region with a hard edge. Effective woodland ground lands at `83%` of open
  ground.
- `apply_natural_land_variation` is damped by `woodland_mask * 0.55` instead of suppressed
  outright. A stand with no macro variation reads as a painted region.
- `apply_grass_detail` is damped by `woodland_mask * 0.35` instead of `0.90`. A forest floor
  has more small-scale fibre than a meadow, not less; removing it was the other half of why
  the mask read as paint.

Measured in the `E13` `yaw180` frame, foreground ground moved from luminance `0.134` at hue
`96` to `0.323` at hue `80`, which is inside the photographic band. Matched, one release
binary, unprofiled, scatter isolated as `full - off`: `+7.14 ms` and `9.52 M` primitives
against `+7.26 ms` and `9.52 M`. Identical primitive count - this is one `mix()` factor and
two attenuations on paths that already ran.

None of this closes the real gap, and it is not meant to. If ground is visible across a
stand at all then the canopy is not closed, and no ground colour substitutes for crowns that
are not there. That is `RENDER-04`.

### The land palette was 30 degrees off, and the woodland mask was ungated (2026-09-12)

**Superseded:** the woodland ground mask this section tunes was removed the same day; see
"The woodland ground mask is gone" above. Kept for the measurements and for why the
intermediate fixes did not work.


Reference photographs of the Kuopio region are in `imgs/reference`, with matched game
viewpoints in `imgs/reference/game`. `imgs/` is excluded through `.git/info/exclude`, so
nothing in it is tracked; `RENDER-03` still owns the question of where a durable set lives.
Crops were measured, not eyeballed.

**Hue, not value, made open ground look like a sports pitch.** Sunlit vegetation in the
photographs measures hue `59-79` degrees - yellow-green and olive - at luminance
`0.36-0.43`. The game's open ground measured luminance `0.434`, already inside that range,
but hue `109`. Every authored land green sat at hue `98-115`, which is blue-green, and at
that luminance blue-green reads as fluorescent. Two separate causes:

- `grass_material_layer` multiplied the grass photo's chroma by a fixed `3.00`. The photo
  is a muted yellow-green, mean RGB `(0.220, 0.294, 0.137)` at saturation `0.53`; the gain
  drove it to saturation `1.00` with the blue channel clamped to zero, three times over for
  the macro, mid and micro layers. It is now `terrain_grass_chroma_gain`, bound from
  `TERRAIN_GRASS_CHROMA_GAIN = 1.15`, so the photo contributes fibre and breakup, not hue.
- The authored entries in `terrain_surface_color` and `grass_meadow_variation` were rotated
  into the measured band, each keeping its original luminance and saturation. `dry_open`
  (hue `60`) and `dry_grass` (hue `71`) were already correct and are unchanged - the two
  colours that read as "dry" were the two that matched Finland.

Measured on matched `E13` `yaw000` crops of open foreground ground:

| Build | hue | sat | luminance |
|---|---:|---:|---:|
| shipped at `47ee891d` | `109` | `0.64` | `0.434` |
| chroma gain only | `95` | `0.55` | `0.415` |
| chroma gain and first rotation | `84` | `0.55` | `0.416` |
| shipped | `80` | `0.56` | `0.417` |
| reference photographs | `59-79` | `0.22-0.60` | `0.24-0.43` |

Luminance barely moves across the whole sequence, which is the point: the value was never
the error.

The foliage carried the same error and is corrected with it. `_leaf_color` and the
understory palette in `tree_species.gd` sat at hue `90-112` against `76-79` for sunlit
canopy in the photographs, so once the ground moved the trees read bluer than the ground
they stood on. Same treatment: rotate, keep luminance and saturation.

Forest floor under canopy now measures luminance `0.172` against `0.181` for shaded forest
mass in the Kuopio photograph, so the floor is not too dark despite looking it in a
screenshot. Its *hue* is still `105` against `73`, and that residue is not the palette:
`hillshade_shadow_tint` is `(0.82, 0.88, 0.90)`, a cool blue, so shaded green ground lands
blue-green. Changing it touches every shaded surface and the whole day cycle, so it is
left alone here.

**Contours were drawn in the plain world view.** `contour_minor_strength 0.14` and
`contour_major_strength 0.34` composited survey lines onto ordinary terrain colour at
`2.5 m` and `10 m` intervals. In an aerial view that is the strongest single cue that the
landscape is a topographic map, and no reference photograph has anything like it. They are
now gated on `overlay_mode > 0`, so the analysis overlays - which are read as a map - keep
them and the world view does not. Every interval and colour uniform is unchanged.

**The woodland mask darkened ground the scatter would never plant.** `woodland_field()` is a
pure function of world XZ, but `scatter_layer` also rejects a candidate for standing water,
for a road or building surface, and for relief across `clear_footprint`. The visible
consequence was lake beds: the terrain tint showed through the water, which clamps alpha at
`0.92` and samples scene colour for shallow refraction, so a darkened bed read as mud. The
mask is now multiplied down by watermap depth and by slope, both already available in the
fragment. Roads and buildings occlude the ground they own, so they need no gate here.

This is a narrow fix, not a complete one: it does not reproduce footprint clearance, density
or site exclusion, and a default or not-yet-resident water texture is not proof of dry
ground. The complete form is for Rust to publish a derived coverage tile per patch from the
accepted population, which is filed as `RENDER-05`. The Rust pin test now says explicitly
that only the *field* is mirrored and the gates must not be "restored" away.

Two further leaks the first version missed. `apply_natural_land_variation` was suppressed
under woodland, but `apply_grass_detail` runs after it and its `grass_detail_mask` excluded
only rock, cliff and shore - so `grass_material_layer` anchored the forest floor back toward
`grass_meadow_variation` and undid the darkening. It is now attenuated by the woodland mask
as well. And the floor colour itself sat at luminance `0.078`, dark enough to read as a
painted stain from the air rather than as shaded ground; it went to `0.100` with stronger
litter breakup here, which was still far too dark and is superseded by the section above.
The mask edge is broken up by noise at stand scale so a `2 km` sinusoid's boundary does not
read as an ellipse.

Scatter cost is unaffected: `yaw180` scatter isolated as `full - off` measured `+7.27 ms`
and `9.68 M` primitives against `+7.48 ms` and `9.7 M` for the shipped build, which is
inside run-to-run variation. All of the above is fragment arithmetic on paths that already
ran, and the contour gate removes work.

Still wrong, and now more visible because the ground moved: the foliage colours carry the
same hue error the ground did. `tree_species.gd` holds conifer needle at hue `112`,
broadleaf and bush core at `100`, birch leaf at `90`, against `76-79` for sunlit canopy in
the photographs. The trees now read bluer than the ground they stand on.

### The understory stopped being tents (2026-09-12)

`_bush()` built a plant `2.8-3.4 m` tall and `2.15-2.65 m` wide on a **five-segment
lathe**, so its silhouette was a pentagonal pyramid with facets big enough to read
individually - the "tent" - and it stood taller than a person while being called
undergrowth. Its core was `Color(0.072, 0.132, 0.042)`, luminance `0.11`, identical across
all six variants, with four foliage cards stuck around the outside of a solid cone.

The six variants are now a catalogue of boreal ground layer rather than six of one shrub:

| Variant | Form | Height | Triangles |
|---|---|---:|---:|
| 0, 1 | blueberry and lingonberry mats, cards only, no trunk or apex | `0.06-0.09 m` | `36`, `42` |
| 2 | grass tuft, eight bent tapered blades, two-sided | `0.34-0.65 m` | `48` |
| 3 | fern, three arching fronds with six leaflet pairs each | `0.45-0.60 m` | `48` |
| 4 | prostrate juniper, irregular outward sprays | `0.15-0.36 m` | `56` |
| 5 | spruce sapling, separated whorls on a `24 mm` stem | `1.6 m` | `64` |

Against a flat `64` triangles before, so the budget went down, not up, and the grass tuft
is the cheapest honest answer to there being no grass: a dedicated grass layer cannot use
this node layout at all, because visibility lives on one `MultiMeshInstance3D` per `510 m`
patch, so **any** range must exceed the `361 m` half-diagonal or plants vanish at the
camera's feet (`RENDER-02`).

Only the two woody forms carry both a stem surface and a card surface; a mat is cards alone
and a grass tuft is blades alone. Ground plants bend from their own base, so sway weight is
now proportional to height above ground rather than following a shared ring schedule.

`vegetation_appearance_test.gd` had encoded three assumptions this breaks, and each was
rewritten to state the new contract rather than relaxed: a bush had to have exactly two
surfaces, bush card sway had to span exactly `0.2-0.8`, and `max_height / min_height` had
to stay under `1.3` for every species. The last one is the interesting one - a `0.06 m`
blueberry mat and a `1.6 m` sapling are both ground layer, and forcing one height across
the variants is precisely what produced a field of identical `3 m` cones. Trees and rocks
still hold the proportion invariant, and every surface must still use one of the three
shared materials, because a new material is another draw call on the densest species in
the world.

Matched `E13`, one release binary, unprofiled, `yaw180` scatter isolated as `full - off`:
`+7.26 ms` and `9.52 M` primitives against `+7.48 ms` and `9.7 M` shipped. Slightly
cheaper, from the lower average triangle count.

### Birch, crown density and distant albedo (2026-09-12)

Broadleaf variants with `variant % 3 != 2` are birch: `0, 1, 3, 4, 6, 7, 9, 10`.
The other four retain generic broadleaf foliage. Species and variant counts remain
`4` and `[12, 12, 6, 6]`. Birch uses near-white bark `(0.78, 0.76, 0.70)`, a dark root
flare, and eight narrow horizontal lenticels, each interrupted deterministically around
the trunk. Nineteen near trunk rings replace five; per-band vertex colours supply the
markings in the existing opaque surface, with no bark texture or extra material.
The mid trunk retains four rings and represents a basal scar/band and pale shaft.

Birch has a narrower, upright crown, fine brown outer twigs that turn downward, and
yellow-green leaves `(0.165, 0.265, 0.065)`. Its atlas cell contains small toothed ovate
leaves on seven hanging twigs, with transparent cell borders. Generic broadleaf and spruce
retain their palettes. Spruce primary roots now begin at 12% of tree height. Primary
counts increase from 10 to 16 for spruce and 8 to 12 for broadleaf; each still has two
children. Children attach at seeded positions in the outer 60–89% of their parent, taper
to 24% of parent radius for birch and 32% otherwise, and carry their clusters at the tips.
This gives 32/24 clusters instead of 20/16 without adding recursion. Cores and tree cards
retain sway weight `1`; bush weights and all shader contracts are unchanged.

Exact indexed lod-0 counts, summed over both surfaces **per tree**, against `a5a81e44`:

| Species / variants | Vertices before → after (delta) | Triangles before → after (delta) |
|---|---:|---:|
| Conifer 0, 1, 3, 5, 6, 7, 9, 10, 11 | 1805 → 2813 (+1008) | 645 → 1005 (+360) |
| Conifer 2 | 1805 → 2811 (+1006) | 645 → 1005 (+360) |
| Conifer 4, 8 | 1803 → 2813 (+1010) | 645 → 1005 (+360) |
| Broadleaf birch 0, 1, 3, 4, 6, 7, 9, 10 | 1469 → 2561 (+1092) | 525 → 905 (+380) |
| Broadleaf generic 2 | 1469 → 2139 (+670) | 525 → 765 (+240) |
| Broadleaf generic 5, 8, 11 | 1469 → 2141 (+672) | 525 → 765 (+240) |
| Bush, all variants | 160 → 160 (0) | 64 → 64 (0) |
| Rock, all variants | 132 → 132 (0) | 48 → 48 (0) |

Every near variant remains below 2× its original vertices. Conifer mid vertices fall
296 → 290 and far vertices fall 81 → 77 (73 → 69 for variants 3, 5, 7); triangles remain
102/28. Broadleaf mid/far counts remain exactly 274 and 84/92 vertices, 96/32 triangles.
The far broadleaf profile retains its original geometry to preserve indexed vertex reuse;
the mid birch profile narrows. Mesh surface counts, materials and MultiMesh counts do not grow.

`build_meshes()` constructs all near variants first, then measures their actual stored
8-bit vertex RGB across both surfaces, including wood and foliage cores and cards.
For triangle `i`, `A_i = length((b-a) cross (c-a))/2` and
`C_i = (C_a + C_b + C_c)/3`; distant colour is `sum(A_i*C_i)/sum(A_i)` across all 12
variants of that species. Both distant levels use this mean without applying the old
additional face brightening. This represents the species mix because the distant scatter
uses variant zero for the entire population. Alpha remains sway weight and is excluded
from RGB arithmetic. The measured sums and means are:

| Species | Sum of area (m²) | Sum of area × RGB | Derived distant RGB |
|---|---:|---|---|
| Conifer | 7345.923828 | (408.690094, 895.270569, 308.559570) | (0.05563495, 0.12187311, 0.04200419) |
| Broadleaf mix | 11976.332031 | (1972.277100, 3151.504883, 933.372253) | (0.16468123, 0.26314440, 0.07793473) |

This is an emitted-area albedo match, not a prediction of screen luminance: it does not
model alpha-mask coverage, overlap, background, normals or backlighting. Actual patch-switch
brightness is unverified. The per-instance tint boundary is outside this change.
The new work is O(emitted triangles) once per catalogue, with bounded branch depth and
O(1) atlas selection per cluster; there is no new per-frame work.

Fresh CPU validation used Godot `4.7.1.stable.official.a13da4feb`, headless dummy rendering,
default worker settings and no affinity overrides. Baseline is `a5a81e44` with only the
same FIXTURE timing/count instrumentation added; after is this revision. The fixture times
`host.add_child(vegetation)`, whose `_ready()` builds the catalogue, excluding later checks
and patch uploads. Three separate process runs measured before **54.586, 56.501, 54.858 ms**
and after **90.944, 99.424, 94.523 ms**: medians **54.858 → 94.523 ms**, a **39.665 ms / 72.3%**
startup increase. These are unprofiled CPU measurements on the shared machine, not GPU
acceptance results. Additional geometry and the area integration increase startup cost.

Commands (full stdout/stderr under `/tmp/birch-validation/`):

```bash
godot --path godot --headless --log-file /tmp/birch-validation/after-engine-1.log --script res://tests/vegetation_appearance_test.gd > /tmp/birch-validation/after-appearance-1.log 2>&1
godot --path godot --headless --log-file /tmp/birch-validation/invalidation-engine.log --script res://tests/vegetation_invalidation_test.gd > /tmp/birch-validation/invalidation.log 2>&1
grep -c 'SCRIPT ERROR' /tmp/birch-validation/after-appearance-1.log /tmp/birch-validation/invalidation.log
XDG_CONFIG_HOME=/tmp/birch-validation/config godot --path godot --headless --log-file /tmp/birch-validation/import-engine.log --import > /tmp/birch-validation/import.log 2>&1
```

Appearance prints `PASS vegetation appearance, shared material, LOD buckets, positions and
empty density`, with **0 SCRIPT ERROR** lines on all three before and after runs.
Invalidation exits **0**, with **0 SCRIPT ERROR** and **0 ERROR** lines; that harness has
no PASS print, so no PASS line is claimed for it. New targeted checks cover every geometry
budget, birch split/bark/cell/palette, analytic unequal-area integration and all distant
crown colours within one 8-bit quantization step. Existing shader, sway, placement and
population assertions remain. Position and appearance digests are unchanged.

Both final atlas bakes used Blender `5.2.1 LTS`, Cycles CPU, one render thread, seed 137,
32 samples, with existing mip coverage correction. To avoid a sandbox PulseAudio shutdown
hang after files were written, the validation invocation exits after the bake script returns:

```bash
blender --background -noaudio --python tools/bake_foliage_atlas.py --python-expr 'import sys, os; sys.stdout.flush(); sys.stderr.flush(); os._exit(0)' -- /tmp/birch-validation/bake-1/foliage_atlas.png
# Repeat with bake-2, then compare:
sha256sum /tmp/birch-validation/bake-{1,2}/foliage_atlas.png
```

Both PNGs hash to `15e7fa5d263076c0258e8abbee37f0b0b3a1a40391f0092b27eca5905202d990`;
DDS and coverage JSON are also byte-identical across the two bakes. The headless import
completed and retained unchanged `.png.import`/`.dds.uid` companions. Its editor socket
cannot listen inside the sandbox; this did not prevent texture import. An explicit log
path was needed because Godot crashes when its default user log cannot be written here.
No game window, GPU benchmark or shader change was part of this validation.

Run `godot --path godot --script res://tests/local_vegetation_demo.gd` from the repository
root for an interactive Kuopio preview. F9 toggles trees. The GPU probe's `E07` experiment
compares densities at fixed views. `E08` compares horizon-facing lateral pans. `E09` measures
the shadow lever. `E10` measures the tree budget inside the laptop playable preset. `E12`
attributes the cost of an eye-level horizon view, and `E13` sweeps that view through four
compass yaws at two render scales. `godot --headless --script res://tests/vegetation_invalidation_test.gd` covers the
per-patch invalidation contract.
Local results and continuation notes belong in `benchmark-results/VEGETATION-DIARY.txt`.

## Purpose

This document owns world extent, terrain storage, water storage, and the deterministic
implementation path from the current chunk-aware runtime to a real large-world authoring pipeline.

It answers questions like:

- what `WorldConfig` means today
- what terrain and water state is authoritative
- when dense buffers are still allowed
- how terrain and water render/upload boundaries must stay local as world size or terrain density
  increases
- what is already implemented
- what the next deterministic implementation slices must do

It does not own zoning legality, building placement rules, or multi-tier inactive-region
simulation behavior. Those remain owned by their respective docs.

## Document Conventions

Interpretation rules:

- Sections under `Implemented Runtime Contract` describe the live code unless explicitly marked as a
  compatibility gap.
- Sections under `Implemented Editor / Rendering Slices` describe shipped editor, renderer, and
  authored-world behavior.
- Sections under `Remaining Planned Deterministic Implementation` are intended next steps, not
  shipped behavior.
- `must` means required for the owning contract.
- `should` means intended unless a better measured implementation replaces it.
- `may` means allowed but optional.

Terminology:

- `world extent`: the authored width and height of the map
- `runtime terrain cell`: one live terrain sample in the current in-memory terrain grid
- `authored terrain chunk`: the canonical chunk span described by `WorldConfig`
- `source terrain`: the authoritative player- or importer-authored terrain surface
- `visual terrain`: the derived terrain surface after engineered-ground earthworks
- `WorldDefinition`: the reusable authored-world asset for blank-world v1

## Implemented Runtime Contract

### 1. `WorldConfig` Is The Authoritative World Metadata

The live runtime now uses `WorldConfig` instead of the old `MapConfig`.

```rust
pub struct WorldConfig {
    pub width_m: f32,
    pub height_m: f32,
    pub terrain_cell_m: f32,
    pub terrain_chunk_m: f32,
    pub terrain_base_elevation_m: f32,
    pub env_cell_m: f32,
    pub zone_cell_m: f32,
}
```

Current defaults:

- fallback gameplay world: `20_000 m × 20_000 m`
- editor sandbox: `500 m × 500 m`
- default terrain sample cell: `10 m`
- canonical terrain chunk span: `512 m`
- default base terrain elevation: `0.0`
- default environmental cell: `40 m`
- default zoning cell: `10 m`

Current deterministic rules:

- `WorldConfig` is saved and loaded as part of every city save.
- City saves, world definitions and blank-world creation validate the same metadata before
  allocating world storage. Invalid spacing is rejected rather than clamped. Terrain spacing
  must meet the runtime minimum; terrain/environment/chunk dimensions must fit signed grid
  coordinates and dense-buffer allocation layouts. Environmental grids must contain at least
  one cell per axis. These structural checks do not impose a fixed world-size or RAM budget.
- old save migration is intentionally not required; version mismatch is a hard rejection.
- authored terrain chunk count is:
  - `terrain_chunk_columns = ceil(width_m / terrain_chunk_m)`
  - `terrain_chunk_rows = ceil(height_m / terrain_chunk_m)`

### 2. Terrain And Water Grid Sizing Is Now Explicit

Current deterministic rule:

- `terrain_cell_m` is an explicit `WorldConfig` field
- `terrain.width = terrain_grid_width()`
- `terrain.height = terrain_grid_height()`
- `water.width = terrain_grid_width()`
- `water.height = terrain_grid_height()`
- `terrain_grid_width = round(width_m / terrain_cell_m) + 1`
- `terrain_grid_height = round(height_m / terrain_cell_m) + 1`
- runtime sparse chunk span in cells comes from `WorldConfig.terrain_storage_chunk_cells()`:
  - `max(1, ceil(terrain_chunk_m / terrain_cell_m))` (shared by terrain, water and deposits)

Implication:

- terrain sample density is now configurable independently from zoning density
- runtime world-space XZ is now canonical metres
- terrain, zoning, and environment each use their own cell spacing explicitly

Environmental pollution and noise keep their existing `DataGrid<f32>` buffers. The daily order
is pollution, noise, then desirability. Their shared four-neighbor diffusion kernel preserves
left/right/up/down summation order and adds today's emissions before the existing 0–100 limit.
Pollution retains own/neighbor weights `0.60/0.40` and retention `0.995`; noise uses `0.50/0.50`
and `0.90`. The kernel expands separately for each field so Rayon receives constant coefficients.
Desirability reads the two resulting fields; it has no zoning dependency.

Environmental bilinear sampling clamps to the edge and interpolates along the remaining axis
for single-row/column grids; a single cell stays constant and an empty grid returns zero
(`AUDIT-01-G3`). Five CPU-0 matched release pairs (24 configured Rayon workers; sampling itself
is serial) measure 1,048,576 queries per sample, 11 samples per process:
1 × 1 costs 5.671 → 1.059 ms; 1 × 512 costs 5.672 → 2.798 ms; 512 × 1 costs
5.769 → 2.711 ms; 512 × 512 costs 6.548 → 6.165 ms. The ordinary 2D interpolation
path is unchanged. Command: the recorded release test binary with
`--exact simulation::grid::data_grid::tests::benchmark_bilinear_grid_sampling --ignored --nocapture`.
Identities and raw results are under `/tmp/metrum-full-audit/grid-sampling-*`. This isolates
sampling arithmetic and does not measure whole-overlay rasterization or GPU upload.

Environmental RGBA pixels sample their world-space texture centres through
`WorldConfig.world_to_env_grid`, so rounded source-grid dimensions cannot stretch cell spacing
(`AUDIT-01-G4`). Texture UVs cover the actual terrain-render extent, which can differ from the
authored world dimensions after terrain-sample rounding; environmental coordinates retain the
authored origin and spacing. The bridge uses the same terrain extent as the published shader
snapshot (`AUDIT-01-G4` follow-up). The bridge retains terrain-sized images, existing colors, the 0.01 transparent
threshold and the 0–200 alpha scale for 0–100 field values. All three getters share dimensions
and image conversion. Raster rows use Rayon; column coordinates are calculated once and reused.
Work is O(W×H), with one RGBA output and O(W) temporary coordinates, no per-pixel allocation or
persistent duplicate state. The small O(W) setup stays serial to avoid a second parallel dispatch.

Five matched unprofiled release process pairs on eight physical P cores
(`RAYON_NUM_THREADS=8`, affinity `0,2,4,6,8,10,12,14`) measure complete CPU raster generation,
including allocations and destruction:

| Image | Before (ms) | Corrected (ms) |
| --- | ---: | ---: |
| 129 × 129 | 0.113324 | 0.027582 |
| 1025 × 1025 | 7.056572 | 1.030466 |
| 2001 × 2001 | 26.893081 | 3.880396 |

Three CPU-0 single-worker pairs cost 0.113 → 0.125 ms, 7.065 → 7.336 ms, and
26.941 → 27.957 ms, respectively: about 4% additional large-image cost for correct world mapping.
The first corrected version recomputed column coordinates in every row and cost about 24% extra;
that version is retained only as a measured intermediate (`overlay-raster-matched-bench.json`).
Column reuse preserves its exact result bytes across 1/8/24 workers. Three mixed-core 24-worker
pairs also improve the larger workloads, but physical-core comparisons above are the primary evidence.

Command: the recorded release test binary with
`--exact nodes::sim::render::zoning::tests::benchmark_environmental_overlay_raster --ignored --nocapture --test-threads=1`.
The deterministic source fields include zero/negative, intermediate and saturated values;
three warmups precede 21 samples of four rasterizations each. No world-constructor work is timed.
Identities/source snapshots are `overlay-raster-{before,columns}-*`, and raw results are
`/tmp/metrum-full-audit/overlay-columns-matched-bench.json`. Corrected alignment intentionally
changes old pixels; checksums match within each build across all repetitions/worker counts.
This CPU test excludes the Godot array bridge and GPU texture upload.

The later explicit-terrain-extent correction retains comparable raster cost: five eight-core
pairs measure 0.0281 → 0.0278, 1.0288 → 1.0305 and 3.8871 → 3.8820 ms; three CPU-0
single-worker pairs remain comparable as well. Aligned-world bytes match exactly; the added
45 × 35 m authored / 50 × 40 m rendered fixture verifies the intentionally corrected case.
Build identities and raw rows are in `/tmp/metrum-full-audit/extraction-{before,final}-identity.json`
and `extraction-matched-bench.json`; the raster baseline is `overlay-raster-columns-tests`.

Deposit sampling, mine attachment and field/pit rendering share their exact f32 ray-crossing
predicate (`AUDIT-01-G6`). Separate boundary-inclusive and higher-precision road predicates retain
their own contracts. Five CPU-0 release pairs of the ignored
`nodes::sim::render::resources::tests::benchmark_work_area_polygon_queries` benchmark cost
1.400 → 1.417 / 3.970 → 4.038 / 14.416 → 15.358 ms for 262,144 queries against
4 / 16 / 64 vertices, respectively. This records a 1.2–6.5% measured cost for sharing, not a
speedup. Outputs match exactly; work remains O(vertices), allocation-free per query. Three warmups
precede 21 samples; polygon construction and weighted output checksums are outside timing.
The same extraction identity/result artifacts retain source, binary hashes, commands and rows.

Five further alternating headless Godot process pairs measure the actual three image getters,
including the core lock, raster allocation and Godot byte-array copy. They use the same eight
physical cores, blank worlds, three warmups and nine measured calls per getter; world creation,
payload validation and hashing are outside timing. Per-getter process medians:

| Image | Pollution before → after (ms) | Noise before → after (ms) | Desirability before → after (ms) |
| --- | ---: | ---: | ---: |
| 129 × 129 | 0.101 → 0.051 | 0.101 → 0.030 | 0.101 → 0.026 |
| 1025 × 1025 | 6.372 → 1.187 | 6.376 → 1.157 | 6.533 → 1.171 |
| 2001 × 2001 | 25.574 → 5.821 | 25.470 → 5.873 | 25.447 → 5.710 |

All blank RGBA bytes/checksums match. This is CPU bridge acceptance; GPU upload and full gameplay
frame cost remain outside the timing. Command/script and raw rows are preserved under
`/tmp/metrum-full-audit/overlay-raster-bridge-benchmark.gd` and
`overlay-bridge-matched-bench.json`; extension identities are in the corresponding
`overlay-raster-{before,columns}-identity.json` files. The initial harness used an unavailable
`PackedByteArray.hash()` member; it was corrected to Godot's `hash()` before any matched timing.

`AUDIT-01-G2` preserves O(C) diffusion work and existing row parallelism, with no per-cell
allocation or new grid state. Five matched unprofiled release pairs on eight physical P cores
(`RAYON_NUM_THREADS=8`, affinity `0,2,4,6,8,10,12,14`) measure combined pollution/noise passes:

| Grid per field | Before (ms) | After (ms) |
| --- | ---: | ---: |
| 32 × 32 | 0.014645 | 0.014453 |
| 256 × 256 | 0.111181 | 0.113025 |
| 512 × 512 | 0.303329 | 0.298051 |

Command: the recorded release test binary with
`--exact simulation::grid::tests::benchmark_environmental_diffusion --ignored --nocapture`.
Each sample resets deterministic source fields outside timing and measures eight paired ticks;
25 samples produce each process median. Empty building/road collections isolate diffusion,
excluding emission scaling and world construction. All output-bit checksums match across builds
and 1/8/24 workers. CPU-0 single-worker timings also remain comparable; 24-worker timings on
mixed core types are noisy. Exact identities and raw results are in
`/tmp/metrum-full-audit/environment-{before,zipped}-identity.json`,
`environment-zipped-matched-bench.json` and `environment-physical-matched-bench.json`.

The terrain shader blends environmental colors with their encoded alpha (`alpha × 0.6`) and
clamps overlay sampling at the world edge (`AUDIT-01-G5`). Previously every nonzero value used
the same blend, and the default sampler wrapped the opposite edge into the border. The
constant-zero baked-normal blend control/varying and constant grass-visibility aliases are
removed; height-derived normals and existing active material settings remain authoritative.

An actual OpenGL compatibility render regression (`terrain_overlay_shader_test.gd`) checks
increasing intensity in all three environmental modes and both texture edges. The old shader
fails six intensity and two edge assertions. The corrected shader passes; 44 reference images
covering normal, terrain-debug and grass-debug views of baked/unbaked sloped fixtures match
exactly after cleanup. This is rendered-pixel verification, rather than headless shader parsing.

Five matched unprofiled software-renderer process pairs use Godot 4.7.2, Mesa 26.2.2 llvmpipe,
`LP_NUM_THREADS=8`, and CPU affinity `0,2,4,6,8,10,12,14`. Three warmups precede 21 samples per
viewport/mode; timing includes frame synchronization, rendering and image readback:

| Viewport | Overlay | Before (ms) | After (ms) |
| --- | --- | ---: | ---: |
| 128 × 128 | Off | 0.891 | 0.887 |
| 128 × 128 | Pollution | 0.890 | 0.866 |
| 512 × 512 | Off | 4.661 | 4.756 |
| 512 × 512 | Pollution | 4.730 | 4.812 |

These timings are comparable, not a hardware-GPU or gameplay speedup claim. Work remains
O(pixels); alpha adds one scalar multiply and cleanup removes unused shader state. Output is
repeatable within each build, and inactive-overlay controls match across builds. Exact shader
identities, raw timings and image comparison results are under
`/tmp/metrum-full-audit/terrain-shader-{before,after}-identity.json`,
`terrain-shader-matched-bench.json`, and `terrain-shader-reference-comparison.json`.
`terrain-shader-benchmark.gd` and `match_terrain_shader.py` preserve the workload/commands.
The sandbox denied Xvfb's local listening socket; rendered checks ran with that restriction
lifted. No display/network service is needed by the simulation tests.

### 3. Terrain Uses Dual Sparse Buffers

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

### 5. Terrain Height Storage Is Scaled At Query / Render Boundaries

The live runtime stores raw terrain sample values and multiplies them by `HEIGHT_SCALE` when
converting to world-space `y`.

Current deterministic rule:

- `world_y = terrain_sample * HEIGHT_SCALE`
- `HEIGHT_SCALE = 20.0`

This means the live terrain buffer is not currently a direct world-space metre heightfield.

Important note:

- `terrain_base_elevation_m` is currently forwarded into the raw terrain sample storage before
  `HEIGHT_SCALE` is applied
- the `_m` suffix is therefore ahead of the current implementation and should not be treated as a
  proof that the live terrain buffer already stores fully world-space metres

### 6. Terrain Coordinates Are Centered At World Origin

The live terrain surface is centered around `(0, 0)` in world XZ space.

Current deterministic rules:

- terrain local grid coordinates span `0..width-1` and `0..height-1`
- world-space XZ is centered by:
  - `half_w = ((width - 1) * terrain_cell_m) * 0.5`
  - `half_h = ((height - 1) * terrain_cell_m) * 0.5`
- world-to-grid conversion for terrain queries uses that centered origin convention
- terrain samples sit on world-edge coordinates
- zoning and environmental cells remain centre-aligned metre cells

### 7. Sparse Chunk Storage Is Authoritative At Rest

Terrain and water now use sparse chunk-backed storage internally.

Current deterministic rules:

- a sparse chunk is allocated only when at least one cell in that chunk differs from the default
  value
- resetting a cell back to the default value may cause its chunk to be removed if the whole chunk
  becomes default again
- random cell `get` / `set` is expected-average `O(1)` against the chunk map
- dense materialization is `O(width × height)` and must remain a boundary-only operation

Allowed dense boundaries today:

- save/load
- renderer upload to Godot
- temporary compatibility scratch buffers inside water ticking
- undo snapshots

Dense restoration scans independent storage chunks with Rayon and only allocates payloads for
chunks containing a non-default cell (`AUDIT-01-G7`). It retains O(width × height) work and the
existing sparse map, with O(materialized chunks) temporary collection entries and no per-cell
allocation. Sixteen-chunk minimum work units avoid dispatching small grids. Default-only chunks
have no payload allocation; partial edge padding and copy-on-write snapshots retain their contracts.

Five alternating, unprofiled release process pairs on eight physical P cores
(`RAYON_NUM_THREADS=8`, affinity `0,2,4,6,8,10,12,14`) measure repeated dense replacement with
52-cell chunks, the default 512 m / 10 m storage layout:

| Grid | Blank before → after (ms) | Sparse before → after (ms) | Full before → after (ms) |
| --- | ---: | ---: | ---: |
| 129 × 129 | 0.00833 → 0.00713 | 0.00852 → 0.00746 | 0.00272 → 0.00286 |
| 1025 × 1025 | 0.50869 → 0.06358 | 0.52016 → 0.06455 | 0.20430 → 0.05371 |
| 2001 × 2001 | 1.93417 → 0.22558 | 1.97056 → 0.23241 | 0.97296 → 0.33166 |

Three CPU-0 single-worker pairs improve blank/sparse cases by roughly 10–16%; full cases cost
0.00279 → 0.00288, 0.20644 → 0.21799 and 0.95955 → 0.99002 ms. Those small full-grid
costs are retained explicitly. The first four-chunk batch had roughly 4–7 µs excess small-grid
overhead; it is a measured intermediate, not the accepted implementation.

Command: preserved release test binary with
`--exact simulation::core::sparse_chunk_grid::tests::benchmark_sparse_dense_replacement --ignored --nocapture --test-threads=1`.
Three warmups precede 21 samples of eight replacements, including old-chunk destruction and
allocation. Input setup, dense reconstruction and weighted-bit checksums are outside timing.
Sparse fixtures fill the final cell of every sixty-fourth chunk; all output checksums and chunk
counts match across builds and worker counts. Identities/source snapshots are
`/tmp/metrum-full-audit/sparse-load-{before,final}-*`; raw rows are `sparse-grain-matched-bench.json`.
This isolates sparse reconstruction, excluding SQLite decoding, water filling and renderer upload.

### 8. Engineered Ground Is A Chunk-Local Visual Derivation Step

Shared engineered-ground semantics now live in [`earthworks.md`](earthworks.md). This document owns
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
  tie-ins; future engineered-ground clients should extend [`earthworks.md`](earthworks.md) instead
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
  not a claim of watertightness for arbitrary inputs (see `ROAD-24` in [`roads.md`](roads.md))
- terrain density alone is no longer the target fix for road / terrain gaps
- the live road-touched seam path is the Spade CDT patch builder described in
  [`roads.md`](roads.md):
  road footprint loops become hard constraints, terrain faces inside those loops are omitted, and
  road seam constraint edges are preserved exactly
- current grounded-road terrain editing must keep placed `Standard` road geometry fixed and rebuild
  derived terrain outputs around the committed roadbed instead of silently resynchronizing the road
  to later source-terrain edits

Authoritative rule:

- engineered-ground earthworks change the visual terrain only
- source terrain remains the authored ground surface

### 9. Water Uses Sparse Baseline-Depth Storage

The live `WaterSystem` stores one sparse chunk-backed layer:

- `baseline depth`

Current deterministic rules:

- untouched water cells are implicitly dry
- `Lake Fill` and `Open Water` rebuild baseline depth from authored still-water records
- water save/load persists one dense row-major baseline-depth snapshot at the serialization boundary
- no source/sink, velocity, or flux state exists in the shipped runtime

Water diagnostics compare cached upload bytes with the current authored baseline (`AUDIT-01-W1`).
The retired `depth_data` array fallback and duplicate baseline/visible source-statistics fields are
removed. Cached min/max/count/sum decode the actual native f32 payload; they no longer report a
filled patch as all-zero. Decoding occurs only for explicitly requested diagnostics, with O(patch
samples) temporary data, and does not add work to regular texture uploads. The headless
`surface_patch_debug_test.gd` regression exercises an actual authored fill, asynchronous native
payload, live source statistics and formatted diagnostic output, alongside regular/refined terrain
payloads with negative heights.

Three CPU-0 headless helper runs compare the existing calculation on a prepared float array with
native-byte decoding plus that same calculation: 7 × 7 samples cost 0.00309 → 0.00341 ms,
55 × 55 cost 0.16156 → 0.16113 ms, and 515 × 515 cost 14.382 → 14.121 ms. Outputs match.
This isolates diagnostic decoding overhead; it does **not** compare with the old production
reader that incorrectly returned zero. Three warmups precede 21 samples, with 128 / 16 / 1
repetitions respectively; setup is excluded. Command/script, renderer identity and raw results
are under `/tmp/metrum-full-audit/water-stats-benchmark.gd`, `water-stats-final-identity.json`
and `water-stats-benchmark.json`. These timings do not cover full diagnostic formatting or frames.

Terrain, including refined and road-preview patches, exports a single native `height_bytes` buffer
(`AUDIT-01-G8`). The retired parallel `height_data` export and frontend fallback are removed;
failed refinement payloads still omit drawable heights intentionally. Terrain and water share
`render_debug.gd` for count/min/max/sum with the same 0.001 visibility threshold and summation
order. This is stateless, explicitly requested diagnostic work, not simulation state.

Three CPU-0 headless comparisons against the retained previous water reader give 7 × 7 / 55 × 55 /
515 × 515 diagnostic costs of 0.00337 → 0.00307 / 0.15981 → 0.14063 / 13.985 → 12.381 ms.
Both sides decode the same byte payload and produce equal statistics; eliminating the redundant
per-value conversion is included. Setup is excluded; three warmups precede 21 measured samples.
Scripts, before/after renderer identities and raw rows are `/tmp/metrum-full-audit/surface-stats-benchmark.gd`,
`surface-payload-{before,after}-identity.json` and `surface-stats-benchmark.json`.

Five alternating native bridge process pairs on eight physical P cores measure completed
asynchronous payload requests for blank nonzero terrain, with regular and refined paths:

| Authored patch span | Regular before → after (ms) | Refined before → after (ms) | Removed duplicate height bytes per refined payload |
| --- | ---: | ---: | ---: |
| 40 m | 0.07075 → 0.07050 | 0.07525 → 0.07488 | 676 |
| 512 m | 0.08263 → 0.08063 | 0.09963 → 0.09850 | 14,400 |
| 1024 m | 0.09975 → 0.10025 | 0.11225 → 0.12125 | 49,284 |

This records comparable default-patch latency and a 9 µs larger refined median for the largest
fixture; there is no general latency-speedup claim. Largest refined process ranges overlap
(0.0831–0.1238 ms before, 0.1125–0.1305 ms after). Measurements include request/poll scheduling,
with 10 µs sleeps between empty polls, native export and completed return; cold world creation and
first refinement are excluded. Only completed payloads count, and explicit generation retries are
honored. Three warmups precede 21 samples of eight requests. All canonical render dictionaries
have equal checksums after excluding the removed duplicate array; raw height bytes, metadata and
mesh products are unchanged. Work remains O(payload samples/mesh size), with one fewer height
buffer and bulk byte copies. The benchmark changes no road-planning algorithm.

Command: `taskset -c 0,2,4,6,8,10,12,14 godot --headless --path godot --script
/tmp/metrum-full-audit/surface-payload-benchmark.gd`, `RAYON_NUM_THREADS=8`.
Raw commands, rows and extension identities are `surface-payload-matched-bench.json` and
`surface-payload-{before,after}-identity.json`. This excludes texture upload and full gameplay frames.

Geometry diagnostics also share clip aggregation, signed polygon area, bounds formatting and mesh
labels (`AUDIT-01-G9`). The clip result is built once from the decoded groups; it no longer validates
the same packed loops twice or initializes a duplicate statistics dictionary. Mesh labels read
`ArrayMesh.surface_get_array_len()` instead of requesting all vertex attributes. This leaves
O(surface count) work for mesh labels and O(decoded clip points/groups) work for clip diagnostics.
No persistent cache or simulation state is added.

Three CPU-0 headless before/after helper runs preserve every result. Single-surface mesh labels
cost 0.00128 → 0.00093 ms at three vertices, 0.00300 → 0.00094 ms at 3,072 vertices and
0.07450 → 0.00100 ms at 98,304 vertices. Clip diagnostics with one hole per outer loop cost
0.01027 → 0.00873 / 0.21669 → 0.21094 / 1.748 → 1.708 ms at 1 / 32 / 256 groups.
Three warmups precede 21 samples; fixture/mesh construction is excluded and output checksums match.
The existing surface regression also verifies multiple surfaces, primitive/null meshes, hole area,
negative-coordinate bounds and actual native payload logs. Commands, source identities and raw rows
are `/tmp/metrum-full-audit/render-geometry-benchmark.gd`, `render-geometry-{before,after}-identity.json`
and `render-geometry-benchmark.json`. These are explicit diagnostic costs, not gameplay frame timings.

### 10. Live Water Runtime Is Baseline Still Water Only

The live repository intentionally keeps only deterministic authored still water.

Current repository state:

- `Lake Fill` and `Open Water` rebuild into a baseline-water layer
- baseline water stores flat still-water depth above terrain
- procedural waves are shader-only visual motion and do not imply runtime velocity
- future rivers or flowing water must be a new design, not a continuation of the removed dense
  source/sink solver

### 11. Save / Load Remains Dense At The Serialization Boundary

Sparse runtime storage does not change the save format boundary yet.

Current deterministic rules:

- terrain source data is serialized as one dense row-major `f32` blob
- water baseline depth is serialized as one dense row-major `f32` blob
- sparse chunk topology is not currently saved as chunk records
- loading reconstructs sparse storage from those dense blobs
- road loading compiles saved geometry first, locally finalizes only rejected grounded junctions,
  and rejects an unrenderable detached graph; it does not regrade all saved roads to source terrain

This is acceptable while city saves remain runtime snapshots rather than reusable authored world
assets.

### 12. Blank-World `WorldDefinition` Exists As A Separate Asset

The live runtime now has a separate authored-world asset path for blank worlds.

Current deterministic rules:

- `WorldDefinition` is stored as a single-file SQLite asset with its own schema version
- one `world_definition_meta` row stores:
  - world name
  - `WorldConfig` values needed to instantiate the world
- authored terrain is stored as zero or more `world_terrain_chunks` rows
- terrain chunk rows are keyed by zero-based `(chunk_x, chunk_z)` from the world minimum corner
- each chunk payload is one dense row-major `f32` source-terrain block
- only chunks containing at least one non-base terrain sample are persisted
- authored resource deposits are stored as zero or more `world_resource_deposit_chunks` rows
- resource deposit chunks are terrain-aligned `u16` richness grids keyed by `resource_id`,
  starting with `coal`
- loading a `WorldDefinition` resets runtime state to a fresh blank city on that world
- City load and blank-world reset share transient cleanup, including terrain brush state,
  render ownership, undo history, diagnostics and camera culling bounds. The final network-render
  invalidation advances the global terrain payload generation; an extra pre-publication bump
  is not needed. New Game retains the loaded asset registry by ownership transfer.
  Audit `AUDIT-01-AS4`: five alternating unprofiled release process pairs, pinned to CPU 0,
  `RAYON_NUM_THREADS=24`, three warmups then 11 samples of five resets. The fixture has no agents,
  buildings or roads; every measured replacement creates the same flat 256 m square world
  with 8 m samples and retains 128 / 4,096 registered assets. Registration and the initial test
  world are outside timing. Median reset time is `0.056421 / 1.804271 → 0.002045 / 0.002033 ms`.
  This isolates Rust world replacement and catalog retention, excluding file I/O and rendering.
  Command: `nodes::sim::core::tests::benchmark_blank_world_reset_with_assets --exact --ignored --nocapture`
  on matched release executables. Raw rows and source/executable identities are in
  `/tmp/metrum-full-audit/world-reset-matched-bench.json` and `world-reset-{before,after}-identity.json`.
- world replacement clears derived terrain ownership, caches and asynchronous render requests;
  its global terrain payload generation advances so old-world payloads and acknowledgements
  cannot match an unchanged patch key in the new world
- the current `WorldDefinition` format stores:
  - world metadata
  - terrain config
  - source terrain chunks
  - authored water records
  - authored coal deposit chunks
- the current `WorldDefinition` format does not store:
  - roads
  - zoning paint
  - water runtime state
  - agents
  - households
  - treasury history
  - derived visual terrain

Authoritative rule:

- city saves and `WorldDefinition` are separate persistence products with different ownership
- both writers share temporary-file publication: commit and close the replacement database
  before atomically renaming it over the destination; failed saves preserve the previous file

### 13. Godot Bridge Uses Patch Snapshots For Terrain / Water Rendering

The live Godot render bridge now consumes chunk-local terrain / water patch snapshots instead of
whole-map render buffers.

Current deterministic rules:

- `terrain.gd` consumes `get_terrain_patch_layout()`, generation-tagged dirty patch states,
  `request_terrain_patch_payloads()`, `poll_ready_terrain_patch_payloads()`,
  `acknowledge_terrain_patches()`, and `get_terrain_border_loop()`
- `water.gd` consumes generation-tagged dirty patch states, `request_water_patch_payloads()`,
  `poll_ready_water_patch_payloads()`, `acknowledge_water_patches()`, and
  `get_water_border_depths()`
- dirty acknowledgements clear only the exact uploaded patch/network revision; a mutation between
  upload and acknowledgement remains dirty, and live terrain brush steps advance touched patch
  revisions before their asynchronous payloads are requested
- world replacement explicitly rebuilds renderer residency. Dirty flags reflect actual patch
  ledgers: a new flat world with empty ledgers must not wait for nonexistent acknowledgements.
  Residency/payload queues still gate renderer readiness.
- renderer polling, ownership lookup, layout/border reads, and road-mesh retrieval consume immutable
  render snapshots or nonblocking job queues rather than waiting on the simulation mutex
- terrain material shoreline/depth sampling reuses the Water renderer's resident patch depth
  texture binding; it must not request a second terrain-aligned water snapshot or duplicate
  `ImageTexture` upload from GDScript
- terrain and water patch snapshots expose texture-ready `PackedByteArray` height/depth payloads
  so Godot image uploads do not convert `PackedFloat32Array` data on the render path
- the terrain shader keeps separate terrain-height and water-depth UV layouts because terrain and
  water patch textures may use different border widths
- terrain and water now keep patch identity stable while choosing a deterministic mesh-detail tier
  per resident patch from camera distance, so zoomed-out views do not pay near-field vertex
  density for every resident patch
- road-touched terrain patches switch from cached rectangular `PlaneMesh` topology to
  Rust-generated baked local `ArrayMesh` topology
- visible water patches always build depth-owned local `ArrayMesh` topology; dry cells emit no water
  mesh instead of relying on shader discard, and road-touched water patches suppress every water
  cell touched by the road footprint after a network edit instead of emitting partial transparent
  clip fragments
- water patch mesh topology is generated through async Rust/Rayon cache jobs by patch, LOD,
  road-clip signature, and depth signature; Godot submits requests in small time-capped batches,
  Rust owns the ready queue, and Godot polls completed mesh buffers without rebuilding pending-key
  request lists every frame
- the water mesh queue exports request/cache/job/ready/stale perf counters and compacts stale queued
  patch keys before submission when the queue grows beyond the deduped request set
- water mesh uploads apply under a measured per-frame time budget with pending-job backpressure,
  estimated payload-byte limits, and pending-job backpressure; Godot rejects stale ready meshes
  whose road/depth signatures no longer match the current resident patch before `ArrayMesh` upload
- fully wet unclipped water patches use indexed grid mesh buffers instead of expanded per-cell
  triangles so large still-water interiors upload less duplicate vertex data, and Godot reuses
  shared `ArrayMesh` resources for matching full-grid LOD/topology/size variants; regular
  full-grid variants are prewarmed during renderer load so the first matching lake patch can hit
  the cache instead of creating the mesh on the visible apply path
- regular rectangular terrain `PlaneMesh` variants are prewarmed from the active world layout so
  ordinary LOD changes assign cached meshes instead of constructing predictable resources mid-frame
- terrain and water patch `MeshInstance3D`, `ShaderMaterial`, `Image`, and `ImageTexture`
  resources are pooled and prewarmed before first visible residency activation, keeping cold
  resource construction out of the first streaming frames where possible
- perf summaries include viewport size, draw calls, rendered objects/primitives, video/texture/
  buffer memory, vsync mode, the `Engine.max_fps` cap, and terrain/water resource-pool counters
  before deeper renderer architecture work
- the old whole-map terrain / water Godot render APIs were removed from the steady terrain / water
  bridge
- dense terrain or water materialization may still exist at save/load, undo, or other explicit
  compatibility boundaries, but it is no longer the gameplay or WorldEditor terrain / water render
  path

This is a rendering boundary, not an excuse for simulation systems to depend on dense storage.

### 14. Terrain, Site Ground, Water, And Lighting Materials Are Runtime Presentation

Terrain, water, and building-site ground materials are Godot-side presentation contracts over
Rust-owned terrain, water, engineered-ground, and building state. They must not become gameplay
state or hidden repair paths for missing geometry.

Current deterministic rules:

- terrain grass uses world-space UVs and the Grass002 texture stack
- the runtime `grass002_2k_albedo.jpg` and `grass002_2k_height.jpg` imports generate complete
  mipmap chains (11 levels below the 2048x2048 base). Their shader samplers already request
  mipmapped anisotropic filtering; the normal imported-resource path must provide those levels,
  just as the source-image fallback does. Mipmaps are built at import time, with no new per-frame
  CPU work; full-chain texture storage grows by approximately one third
- terrain grass combines macro, mid, and micro layers through stochastic anti-tiling and screen
  footprint fade
- macro / mid / micro grass fading must preserve average luminance; the fade may reduce detail
  contrast but must not accidentally brighten distant terrain or darken close terrain
- any later atmospheric or horizon brightening must be an explicit render effect with its own
  parameters, not an emergent side effect of mip/detail fade
- building-site ground uses the same grass texture stack and world-space material semantics as
  terrain while remaining a separate flat support pad mesh
- authored building asphalt and concrete site surfaces remain separate materials from grass/site
  ground
- water rendering consumes the visible baseline water field and applies a dark Baltic-blue depth
  tint, reaching its deep-water palette by `3.5 m`; increased shallow opacity prevents submerged
  terrain from turning open water muddy, while shoreline foam, restrained Fresnel/sky response,
  and procedural wave normals remain presentation only; the wave field uses rotated aperiodic
  noise octaves with analytic gradients so high camera views do not expose periodic sine bands or
  an axis-aligned sampling grid
- water reflection remains dark in downward views but uses a restrained grazing-angle Fresnel
  response derived from the smooth base surface so procedural normal gradients cannot imprint
  their source cells into the reflected sky; the sun response uses a softened glitter shoulder
  around its bright core, while fine ripple detail remains deferred until a seamless mipmapped
  normal texture is available
- scene lighting is centralized through `scene_lighting.gd` so terrain, water, roads, yards,
  buildings, cars, and debug/editor helpers use one deterministic sun/sky/shadow policy
- the visible background uses one continuous procedural hemisphere gradient; its upper and lower
  halves meet at the same color without literal horizon geometry
- the day/night cycle is a pure function of the operational clock. `day_cycle.gd` maps the day
  fraction to a solar position and a palette; `scene_lighting.gd` applies it. The same clock
  reading always resolves to the same lighting, a paused simulation parks the sun, and no lighting
  state is stored anywhere. `METRUM_TIME_OF_DAY` pins the rendered hour for captures and probe
  trials without touching the simulation
- the sun follows a real solar-position solve for a fixed latitude and declination, so the arc,
  the day length and the length of golden hour follow from two constants instead of hand-placed
  keyframes. Palette stops are keyed to solar elevation, not to the clock, which makes sunrise and
  sunset symmetric and keeps the palette correct if the latitude is retuned
- the key light is one `DirectionalLight3D` that carries the sun by day and an antisolar full moon
  at night. It may only swap bodies inside the window where both contribute nothing, and it must
  never have energy above the horizon crossing, so no light ever shines up through the ground
- terrain grass and site grass suppress environment ambient through `IRRADIANCE`/`RADIANCE`
  overrides while retaining SSIL, and bake their authored ambient floor into `EMISSION`. The cycle therefore scales and desaturates that floor through shader globals;
  dimming alone leaves a saturated ground reading as daylight with the brightness turned down.
  Night bottoms out near a fifth of daylight because the city has no street lighting yet
- the low-sun key energy is deliberately not physical. With no auto-exposure an honestly
  attenuated sun makes golden hour dark instead of golden; the ambient drops at the same stops so
  the result reads as contrast rather than as a brightness change
- palette stop spacing is a continuity budget. The sun crosses the horizon at up to `0.107`
  degrees per authored minute, so a stop gap of `G` degrees carries at most about `0.19 * G` of
  colour change before the transition reads as a wipe. `godot/tests/day_cycle_test.gd` sweeps the
  whole day at one sample per authored minute and enforces this
- `scene_sun_direction`, `scene_sun_color`, `scene_sky_color`, `scene_ambient_strength`,
  `scene_ambient_light` and `scene_ambient_desaturation` are shader globals declared in
  `project.godot`, not per-material parameters. One write per frame relights every resident patch;
  a per-material write would be `O(patches)` per frame and could not follow a moving sun anyway
- the cartographic hillshade baked into terrain and site-ground albedo stays pinned to its authored
  azimuth and altitude. It is a readability device: swinging it with the cycle would double the
  directional light's own shading and would snap 180 degrees at the sun-to-moon swap
- the sky shader places its own sun and moon disks from explicit uniforms rather than from
  `LIGHT0`, so disk brightness and key-light energy can disagree. A sun on the horizon has to stay
  visible after it has stopped being a useful key light, and the moon is not a Godot light
- the cloud panorama is a daylight capture used only as a shape source. Its rotation is fixed and
  its colour comes from the cycle's shadow/light tints; rotating it with the sun would spin the
  whole cloud field around the sky once per day
- the sky uses `PROCESS_MODE_INCREMENTAL` because its palette now changes every frame
- static equirectangular cloud imagery is sampled only as a restrained half-resolution upper-sky
  cover; baked panorama sky color, lower hemisphere, and lighting do not replace the shared
  procedural gradient or directional-light contract
- one shared depth-fog pass fades all ordinary world geometry into the horizon sky color; terrain,
  water, roads, buildings, vehicles, and agents must not implement competing per-material horizon
  cutoffs. Aerial perspective and the far cull curtain are that one ramp, so its colour tracks the
  day cycle and its sun scatter rises at low sun; a uniform haze reads as a flat wall at the
  horizon rather than as light in the air
- shadow policy must be applied through the shared helper rather than per-renderer ad hoc flags
- terrain and site ground receive real shadows; final buildings and cars cast shadows; construction
  pads, debug overlays, and temporary authoring helpers should not cast shadows unless a specific
  debug mode asks for that

Grass mipmap verification (`TERRAIN-02`, 2026-09-10): both imported runtime grass textures
load as `CompressedTexture2D` with 11 mip levels below the base. The same 48-fixture paired
road workload passes with and without GPU profiling on the RX 7900 XTX / i9-12900K,
Godot 4.7.2 Forward+, 1920x1080, 60 FPS cap, V-Sync enabled and 24 Rayon workers. Against the
earlier same-session zero-mip baseline, median periodic GPU time falls `2.978 -> 0.638 ms`;
opaque-pass mean falls `2.789 -> 0.505 ms`. These are one-process diagnostic comparisons,
not uncapped FPS or individual-frame GPU-tail measurements. Matching unprofiled release runs
retain about `16.37 ms` mean observed frame intervals. The user later reported moving windows
between desktops during benchmarking, without identifying affected runs/time ranges. The frame
outliers do not establish a game-side issue, and the exact GPU reduction needs an uninterrupted
repeat with the window stationary. The verified mipmap chains are unaffected. Commands are
`./run.sh --benchmark-gameplay-roads --gpu-profile` and `./run.sh --benchmark-gameplay-roads`,
with the default paired matrix, one warm-up and five measured repetitions. Workload signatures
and library hashes match across states. Build identity, texture hashes, mip probes, commands and
captures are retained in `benchmark-results/grass-mipmaps-dfVqPd/`; the before captures live in
`benchmark-results/gpu-road-analysis-Jecrzy/`. Filtering/grass-scale visual retuning is separate.

The subsequent stationary-window repeat with mipmaps enabled reproduces the low GPU cost:
`0.630 ms` median periodic GPU time, with all 48 fixtures passing in both profiled and unprofiled
runs. Fresh source/import/texture hash checks match the previously verified mip chains. The
unprofiled run still records a `40.742 ms` maximum CPU callback interval, so desktop switching
does not explain all longer intervals. This repeat has no zero-mip baseline and does not settle
the exact speedup or the interval cause. See `roads.md` and
`benchmark-results/stationary-mipmaps-LZ6tB3/results.txt` for the full run conditions and results.

Rendering non-repair rule:

- shader masks, material order, transparency, lighting, water, terrain color, or debug overlays must
  not be used to hide missing road, terrain, water, building, or raised-step topology
- when a visual hole, dark chunk, or wrong overlap appears, the owning mesh/provenance/patch state
  must expose enough debug data to locate the source instead of adding a color workaround

## Implemented Compatibility Gaps We Should Not Extend

The following are live behaviors, but they are not the intended long-term ownership model.

### 1. Dense Scratch Buffers Still Exist In Hot Adjacent Systems

These are compatibility-only.

Current gap:

- terrain renderer upload still materializes one full dense visual-terrain buffer every refresh
- some terrain/water bridge boundaries still materialize dense local patch payloads before handing
  data to Godot

Required direction:

- these paths should later localize to chunk windows or active areas instead of whole-map buffers
- denser authored terrain must not be adopted by extending this whole-map upload path

### 2. Dense Water Snapshots Are Still Used At Save Boundaries

Current gap:

- city saves still persist dense baseline-depth snapshots instead of sparse chunk records

Required direction:

- this refactor may intentionally break existing saves and authored worlds
- no migration is required for the current dense water snapshot layout
- dense runtime water blobs should stay a serialization boundary detail, not the internal ownership
  model

### 3. Undo Is Not Yet Fully Authoritative For Terrain Ownership

Current repository state:

- road edits retain bounded local graph deltas; road undo restores the affected pre-edit compiled
  surface records, removes post-edit owners, and rebuilds old-plus-restored surface, earthwork, and
  query chunks without cold-compiling an unchanged junction; the immutable previous refined
  generation stays reusable, while attached zoning removal uses an index-stable local parcel
  journal rather than cloning the zoning system
- accepted road placement and failed adoption retain an exact touched-storage-chunk visual
  checkpoint, so undo/rollback restores prior visual overrides as well as graph and split references;
  the remaining dense authoring-undo limitation below is separate from `RoadEditPlan`
- road bulldoze and undo queue their graph/surface/terrain work on the simulation thread; the
  Godot input path never performs the road-surface, road-mesh, or refined-CDT rebuild synchronously
- building deletion retains an operation-local inverse journal for touched buildings, sites,
  agents, households, and freight records rather than cloning complete runtime systems
- terrain-authoring and water-authoring undo still capture dense visual terrain or baseline-depth
  snapshots for compatibility with the pre-sparse mutation path

Required direction:

- terrain undo must become authoritative over source terrain and any derived visual state it
  invalidates
- world-authoring workflows must not depend on visual-only terrain snapshots

### 5. Terrain Height Storage Is Still Scaled At Query / Render Boundaries

Current gap:

- terrain samples are still multiplied by `HEIGHT_SCALE` when converted to world-space `y`
- `terrain_base_elevation_m` still enters raw sample storage before that scale is applied

Required direction:

- terrain base elevation and sample values should eventually become direct world-space metres
- no future authored-world format should assume the current scaled-height compatibility contract

### 6. `WorldDefinition` V1 Keeps The First Authored-Water Slice

Current state:

- a dedicated `--world-editor` launch mode and `WorldEditor` scene now exist
- world-editor UI now calls:
  - `create_blank_world`
  - `save_world_definition`
  - `load_world_definition`
- `WorldDefinition` now stores authored baseline-water records and rebuilds deterministic still
  water from them
- `WorldDefinition` still has no richer hydrology ownership beyond the first authored-water slice,
  no preview image, and no richer metadata

Remaining direction:

- the current authored-water slice is the shipped water-authoring baseline and does not require a
  separate richer hydrology system to be considered valid
- richer metadata and optional later water-authoring extensions may still happen, but only if the
  current `Lake Fill` / `Open Water` workflow later proves insufficient

## Implemented Editor / Rendering Slices

### 1. Blank-World WorldEditor V1 Is Live

The first dedicated authored-world shell now exists around `WorldDefinition`.

Current deterministic rules:

- `--world-editor` routes to a dedicated `WorldEditor` scene
- world-editor UI is separate from gameplay save/load UI
- the world-editor top menu exposes:
  - `New World`
  - `Open World`
  - `Save`
  - `Save As`
  - `Quit`
- the world-editor bottom toolbar is the primary authoring surface
- a newly created blank world is dry and flat except for the authored base elevation

### 2. WorldEditor V1 Terrain And Water Authoring Is Live

The current world editor uses the shared `SimulationNode` runtime but does not expose gameplay
simulation controls or gameplay HUD surfaces.

Current deterministic rules:

- world editor starts with the simulation thread available
- world editor does not expose pause / speed controls or gameplay HUD widgets
- world editor terrain authoring is live through:
  - `Raise`
  - `Lower`
  - `Level`
  - `Smooth`
  - `Slope`
- world editor water authoring is live through:
  - `Lake Fill`
  - `Open Water`
- world editor resource authoring is live through:
  - `Coal`
  - `Erase`
- terrain brush picking uses `intersect_terrain()` and therefore targets authored source terrain,
  not the visible engineered surface
- `Raise`, `Lower`, `Level`, `Smooth`, and `Slope` write authoritative source terrain only
- completing a terrain brush stroke rebuilds touched engineered-ground clients and derived terrain
  outputs from the updated source terrain
- terrain brushes must not directly deform road top surfaces, placed flat pads, or future local
  earthwork meshes
- selecting `Raise`, `Lower`, `Level`, `Smooth`, or `Slope` opens a terrain brush submenu on the bottom toolbar
- that terrain brush submenu owns the shared editor `Diameter m` and `Strength` controls
- active terrain brushes show their footprint directly on the terrain so brush diameter is visible before and during sculpting
- `Level` captures the clicked source-terrain height at the start of the brush stroke and moves terrain toward that height while the stroke remains active
- `Smooth` moves terrain toward the local neighborhood average inside the brush footprint and is intended for relaxing jagged cuts, banks, and shorelines after carving
- `Slope` is a two-phase terrain brush:
  - first click captures the slope start anchor and its source-terrain height
  - second click captures the slope end anchor and its source-terrain height
  - after both anchors exist, brushing moves terrain toward the clamped linear grade between those two anchor heights
- `Slope` must not extrapolate beyond the two captured anchors; samples before the first anchor clamp to the start height and samples beyond the second anchor clamp to the end height
- `Lake Fill` and `Open Water` use a preview-first workflow:
  - first click seeds the preview
  - `Surface +m` adjusts the previewed surface
  - `OK` confirms
  - `Cancel` / `Esc` dismisses the preview without writing authored state
- `Coal` paints authored deposit richness into a sparse terrain-aligned resource layer
- the shared Deposits overlay currently renders richer coal deposits darker through terrain overlay mode `4`
- `Erase` clears authored coal cells without modifying terrain height, water, zoning, roads, or
  economy state
- coal deposit rendering must stay shader-overlay based; it must not introduce terrain-following
  mesh decals or CDT geometry just to show authored richness
- world editor save/load is `WorldDefinition` only, not city-save persistence

Current compatibility gap:

- this is not yet the final terrain / water-only runtime boundary
- the shared runtime bundle still contains gameplay systems; world editor simply keeps gameplay
  controls and HUD surfaces absent
- after a terrain stroke, the current runtime still allows placed `Standard` roads to resync to
  edited source terrain; the intended long-term contract is to keep placed roads and future
  foundations fixed and reform terrain / earthworks around them instead

### 3. Baseline Water Is The Required Stable Model

The dynamic source/sink solver has been removed. The shipped water contract is deterministic
authored still water only.

Authoritative rule:

- still water is represented by one baseline-depth layer derived from authored records and terrain

Deterministic baseline-water rules:

- baseline water is authored-world state, not emergent solver output
- `Lake Fill` and `Open Water` are the only shipped water-authoring tools
- each connected baseline water body owns one flat `surface_elevation_m`
- baseline water depth is always derived as:
  - `max(surface_elevation_m - terrain_world_y, 0.0)`
- baseline water does not own velocity or flux buffers
- baseline water is rebuilt from authored records and current terrain
- terrain edits that affect a baseline water body must recompute that baseline body immediately

Deterministic rendering rules:

- renderer consumes baseline depth directly
- visible still water must render as flat at the authored baseline surface elevation
- shader-side waves are cosmetic only and must not require runtime velocity or flux data

Deterministic authored-water rules:

- `Lake Fill` is an authored baseline-water record with:
  - one world-space seed position
  - one target water surface elevation
- `Open Water` is an authored baseline-water record with:
  - one world-space seed position
  - one target water surface elevation
- authored water records belong to `WorldDefinition`, not to renderer state or solver scratch
- `Lake Fill` and `Open Water` preview state remain transient editor runtime state and must never
  be serialized unless confirmed

Deterministic world-load rules:

- loading a `WorldDefinition` must rebuild baseline water from authored `Lake Fill` and
  `Open Water` records
- loading a world must not require running the dynamic shallow-water solver to reconstruct still
  lakes, coasts, or seas

Deterministic preview rules:

- `Lake Fill` preview must show a flat baseline surface preview, not a solver-generated depth field
- `Open Water` preview must show a flat edge-connected baseline surface preview, not a
  solver-generated depth field
- preview validity stays:
  - `Lake Fill` valid only if the filled region stays off the world edge
  - `Open Water` valid only if the filled region reaches the world edge
- confirming a valid preview writes authored baseline-water state only
- canceling the preview restores committed authored baseline water only

Deterministic persistence rules:

- this refactor may intentionally break existing city saves and existing `WorldDefinition` assets
- migration of the current dense water runtime blobs is not required
- the long-term authoritative format must not persist dense runtime water `depth`, `velocity`, and
  `flux` as the definition of still water

Legacy features removed:

- `Source` and `Sink` authoring tools
- dynamic depth, velocity, flux, source lists, and the low-rate runtime water tick
- rebuilding `Lake Fill` or `Open Water` through the old shared runtime solver field
- using the dynamic shallow-water solver to keep authored lakes or seas visually flat
- treating one shared runtime depth field as the source of truth for both authored still water and
  dynamic flowing water

Deterministic non-goals of this refactor:

- no arbitrary freehand water-depth paint brush
- no automatic river extraction from imported DEMs
- no automatic hydrology extraction from hydrography vectors or rasters
- no full river-channel authoring tool yet

### 4. Gameplay `New Game` Now Loads A Selected `WorldDefinition`

Gameplay now has a first-pass authored-world handoff.

Current deterministic rules:

- startup seeds missing bundled world entries from `res://bootstrap/worlds/` into `user://worlds/`
- gameplay `File -> New Game` opens a world picker rooted at `user://worlds/`
- selecting one `WorldDefinition` loads it into the live gameplay scene
- gameplay world load reuses the same scene refresh path as save-load:
  - terrain rebuild
  - water rebuild
  - network mesh refresh
  - building renderer refresh
  - zoning overlay refresh
  - agent renderer refresh
- gameplay pauses immediately after loading the selected world

Remaining direction:

- `New Game` still loads directly into gameplay rather than through a richer front-end menu flow
- city saves must remain runtime snapshots layered on top of that authored world baseline

### 5. Chunk-Local Terrain / Water Rendering Is Now Live

Now that `terrain_cell_m` exists, the live render boundary is chunk-local instead of whole-world.

Authoritative rule:

- whole-map dense mesh and texture refresh is no longer the steady terrain / water render path
- it must not be reintroduced to justify denser authored terrain, larger worlds, or local
  engineered-ground geometry

Deterministic terrain-render rules:

- visible terrain now renders as chunk-local terrain patches rather than one whole-world plane
- terrain render patches follow authored terrain chunk boundaries by default
- if a different terrain render patch span is used later, it must be:
  - derived from `terrain_chunk_m`
  - a fixed integer multiple of `terrain_chunk_m`
  - stable in code rather than camera-dependent
- each terrain render patch owns local GPU resources for that patch only
- rebuilding or uploading one patch may read only:
  - the local visual-terrain window for that patch
  - one fixed border sample ring if needed for interpolation, normals, or shading continuity
  - exact indexed contributor and grading-influence coverage required by local engineered-ground
    clients
- unchanged patches must keep their existing GPU resources
- camera motion alone must not rebuild or reupload already resident unchanged patch textures
- camera motion may change the mesh-detail tier of an already resident patch, but that change must
  reuse the resident patch snapshot and must not fall back to whole-map terrain or water uploads
- only dirty patches and newly required visible patches may rebuild or upload
- the visible terrain patch set must be derived from the camera or editor interest region plus one
  fixed patch of desired-set padding, two resident hysteresis patches, and one speculative prewarm
  patch; at normal height this keeps resident and prepared terrain behind the `8 km` cull boundary
- gameplay uses a `9 km` minimum camera far plane so the full normal-height terrain range is
  renderable; the shared non-volumetric depth fade begins at about `6.2 km` and reaches the sky
  color half a terrain patch before the `8 km` cull boundary
- the depth-fade range follows the terrain renderer's height-scaled cull distance in high-altitude
  WorldEditor views instead of turning into a fixed fog wall above the map
- road earthworks and future engineered-ground clients must continue to invalidate only touched
  terrain chunks; the renderer must reflect that locality instead of reintroducing full-world
  uploads
- asynchronous terrain payload workers may hold the authoritative simulation lock only while
  validating a request revision and copying bounded patch-local terrain/site inputs; road clipping,
  road/site grading, CDT input construction, triangulation, and Godot payload conversion must run
  after that lock is released
- Rayon terrain/water workers must never block or spin waiting for the simulation mutex: an agent
  tick can hold that mutex while waiting for Rayon. Snapshot acquisition uses `try_lock`; contention
  returns the unchanged request through the existing frame-driven retry protocol. Completed refined
  payloads remain queued until nonblocking Godot polling can insert their cache entries with current
  generation checks, before exposing them as ready. Publication is `O(B)` for the completed batch,
  with an `O(1)` busy path; no terrain geometry is rebuilt or discarded merely because the lock is busy
- terrain payload publication revisions are patch-local for local terrain, road, and building-site
  changes; global invalidation is reserved for world-wide source/layout changes; one physical build
  per patch/render-step may be in flight, stale results are rejected, and revision churn coalesces
  into at most one current follow-up build
- terrain/site height sampling and road-footprint collection must use the existing building and
  road-surface ownership indices; full-building or full-road scans are not allowed in patch jobs or
  repeated point queries
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
  CDT builder; contributor-free neighbors return to the regular filler, which consumes the exact
  adjacent window side manifest, and contributor coverage/guide generation share one adaptive
  grading-margin probe result
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
  Segment incidence tolerances are distances, never fractions of arbitrarily long segments.
  Noded-edge cleanup deduplicates canonical vertex IDs, not nearby segment parameters:
  even sub-millimetre intersections in distinct identity cells must remain on both incident
  boundaries. Otherwise a road/site corner can leave crossing constraints and suppress an
  entire render patch after loading. CDT contract revision `13` includes that noding fix,
  imported-bounds building support, symmetric shared-side sample welding and editor-consistent
  building-part yaw, invalidating earlier oversized/misrotated pads and one-sided apron products;
  Rust and Godot enforce the same revision. Invalid geometry still blocks publication, and both
  `terrain` and `road` debugging expose the renderer's rejection reason.
- successful tile output caches vertices, normals, UVs, offset-ready indices, pre-normalized local
  normal-sum magnitudes, and side-seam manifests; reuse therefore skips triangle conversion,
  window-side vertex scans, and triangle-index normal reconstruction for unchanged tiles
- canonical world-aligned boundary-lattice samples remain local to directly adjacent seam filler;
  only non-lattice geometry breakpoints propagate into patch-wide filler partitions, so adding
  fixed tiles cannot create a Cartesian filler-grid expansion
- atomic composition still copies the complete bounded render-patch mesh because Godot owns one
  surface per patch; concatenation, regular filler, duplicate-normal reconciliation, and upload
  remain `O(patch output vertices + indices)`, while contributor queries, CDT input assembly,
  triangulation, and tile conversion are proportional to changed plus deterministic seam tiles and
  never revisit every connected road in the patch
- refined patch publication is atomic for the exact requested generation; one failed tile, a failed
  road clip query, or missing road ownership on a road-owned patch suppresses the complete new
  payload, stale jobs cannot publish, and the immutable last accepted generation remains visible
  and available for reuse until a complete replacement is accepted
- Godot applies the same transaction boundary across every resident dirty patch in one visual
  batch: it validates exact patch/generation/render-step identity and payload structure, builds
  height textures plus terrain/retaining meshes into inactive resources, and swaps only after every
  dirty payload is stageable. Terrain-only changes and road-coordinated changes use this same path;
  no handled failure is acknowledgement-eligible. Standalone publication rechecks network dirtiness
  and generation after staging, residency additions wait while network publication is pending, and
  transient detached-resource build failures remain retryable in the same generation.
- road insertion also validates the production terrain payloads before accepting simulation state.
  The simulation thread borrows the staged world, rebuilds only affected patch/tile products, and
  stores successful buffers in the existing cache. A failure rolls back graph and split dependents
  before lane, entrance, parcel-maintenance, routing, or treasury changes. GPU staging remains a
  separate generation check; it is no longer the first place a terrain-CDT failure can reject a road.
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

Deterministic water-render rules:

- visible water no longer depends on one whole-world depth texture refresh for every change
- water rendering consumes chunk-local bounded window snapshots aligned to the same fixed terrain
  render patch grid
- water patches render from baseline depth only; procedural waves remain shader-side
- resident water patch depth textures are the shared renderer-owned depth source for terrain
  shoreline tint/debug sampling

Deterministic Godot-bridge rules:

- the primary terrain/water render path now uses chunk-local snapshot APIs
- whole-map Godot render APIs such as `get_heightmap_data()`, `get_water_data()`, and
  `get_water_velocity_data()` were removed from the steady-state terrain / water render bridge
- any future dense helper kept for compatibility, debug tooling, or offline export must stay
  outside the steady-state gameplay and WorldEditor render path
- `road` water diagnostics must report authored baseline depth and final visible depth separately
  so dark water-patch regressions identify the owning layer instead of only the final rendered sum
- when a road-touched water patch contains authored baseline water, `road` diagnostics
  must also list the committed `Lake Fill` / `Open Water` records or active preview that actually
  contributed non-zero samples inside that patch
- chunk-local snapshot APIs expose enough metadata for deterministic reconstruction of one
  patch window:
  - patch identity
  - local dimensions
  - world origin
  - `terrain_cell_m`
  - packed sample payloads for that window

Deterministic overlay-separation rule:

- zoning, parcel, or other editor overlays may keep their own representation if that remains cheap
- those overlays must not force terrain or water back onto one whole-world mesh or one whole-world
  texture upload path

Deterministic density gate:

- a default authored-terrain move from `10 m` to `5 m` or finer must not happen before this
  chunk-local terrain / water render split is live
- the density decision must now be re-measured on the split path using the same world-space test
  cases for:
  - system RAM
  - GPU VRAM
  - terrain upload cost
  - water upload cost
  - terrain brush cost
  - earthwork restamp cost
- the accepted road / terrain seam fix is not a density move; it is the Spade CDT terrain-patch
  hardcut in [`roads.md`](roads.md)

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

The chunk-local render path is now the live large-world terrain / water runtime boundary.

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
Logs: `/tmp/metrum-chunk-full-tests.log`, `/tmp/metrum-chunk-bridge.log`, and
`/tmp/metrum-chunk-renderer-tests.log`.

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
tile kernel; outer Rayon scheduling and whole-patch upload are outside this measurement.
The old output has one invalid and three missing constraints, so these are correction-cost
measurements, not an equivalent-output speedup or a city-scale FPS claim. Harnesses and logs:
`/tmp/metrum-cdt-{before,after}/harness.rs`, `/tmp/metrum-chunk-kernel-{before,after}-*.log`,
and `/tmp/metrum-chunk-kernel-production.log`. Matched binary SHA-256 prefixes:
`9c810e195d67e7d5` (before), `3952a9f86230267ee` (after).

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
Neither assets, the diagnostic save nor the checked-in terrain fixture were modified. Logs:
`/tmp/metrum-yard-seam-{red,cdt-tests,full-tests,scene,bridge,renderer}.log` and
`/tmp/metrum-yard-seam-{before,after}-audit.log`. Full-suite command from repository root:
`RAYON_NUM_THREADS=24 rust/target/release/deps/metrum_rise-9cc8d57998aa4451 --test-threads=8`.

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
variation. Commands for each retained release test binary use
`RAYON_NUM_THREADS=24 <binary> <benchmark_name> --ignored --nocapture --test-threads=1`.
Before binary: `/tmp/metrum-yard-seam-before-tests`, SHA-256
`e3b5bb7b17e17d09f987c6a299d20028f90ba9fa6726f6a5350e38926fe7a1a2`;
after binary: `rust/target/release/deps/metrum_rise-9cc8d57998aa4451`, SHA-256
`7767a8db42fa886644764739ad7833eced7afca243462e11678c23a6fe2f5e6f`.
Artifacts: `/tmp/metrum-yard-seam-kernel-{before,after}-{1,2,3}.log` and
`/tmp/metrum-yard-seam-locality-{before,after}.log`. Deployed library SHA-256:
`8e8daecd8676bfe2a6d914531fc59b0587be111a45429ed2fa47ff3431e4e832`.

### 6. Offline Heightmap / DEM Import Is Live

Authoritative rule:

- imported terrain writes authoritative source terrain only
- visual terrain is always derived from source terrain plus later structural road or water
  derivations; ordinary grounded road seams are generated as CDT patch meshes

Current implemented slice:

- real-map terrain import now exists as an offline editor-time tool:
  - `tools/import_dem_world_definition.py`
- the importer writes a normal `WorldDefinition` SQLite asset directly
- the first validated source case is the National Land Survey of Finland Kuopio `324 km²`
  `Korkeusmalli 2 m` tile batch under:
  - `maps/raw/Kuopio/324km2/`
- the default generated authored world is:
  - `maps/processed/Kuopio/kuopio_324km2_10m.sqlite`

Current source format rules:

- v1 import accepts raster DEM/DTM data in single-band `GeoTIFF`
- the first concrete target source class is tiled National Land Survey of Finland elevation data
  such as `Korkeusmalli 2 m`:
  - single-band `Float32`
  - projected horizontal CRS
  - explicit pixel size in metres
  - explicit `NoData` value
- v1 does not accept:
  - hillshade rasters
  - hydrography rasters
  - RGB orthoimagery
  - arbitrary grayscale PNG/JPEG images
  - mixed DEM/DSM source batches

Current ownership rules:

- DEM import is world-editor only
- the current importer creates a new `WorldDefinition`; it does not merge into an already edited
  world
- imported terrain becomes the new authoritative source terrain for that world extent
- runtime coordinates remain centred world-local metres after import; source georeferencing is not
  preserved as gameplay-space coordinates
- import provenance may be stored as non-authoritative metadata, but gameplay must not depend on it
- because the live runtime still multiplies terrain samples by `HEIGHT_SCALE` at render/query
  boundaries, the importer currently converts DEM elevation metres into that pre-scaled runtime
  sample space before writing source terrain

Deterministic validation rules:

- every selected source file must be readable and must expose georeferencing metadata
- all selected tiles in one import batch must share:
  - the same projected horizontal CRS
  - the same pixel size
  - the same sample type
  - the same north-up axis orientation
- v1 must reject source rasters with rotation / skew terms
- v1 must reject missing or malformed `NoData` metadata
- v1 must reject overlapping tiles that do not align exactly on pixel boundaries
- v1 must reject any selected crop extent that contains `NoData` inside the requested world area
- v1 must reject target `terrain_cell_m` values finer than the source raster pixel size; the
  importer must not invent terrain detail by upsampling to a finer authored resolution

Current deterministic import sequence:

1. select one or more `GeoTIFF` DEM tiles
2. validate source metadata and pixel-grid compatibility
3. mosaic the source tiles into one temporary import raster in source CRS
4. choose a rectangular import extent inside that mosaic
5. create a new `WorldConfig` for the imported world:
   - `width_m` and `height_m` come directly from the chosen import extent
   - `terrain_cell_m` is user-selected but must be `>=` source pixel size
   - `terrain_chunk_m` follows the normal authored-world chunk rules
   - `terrain_base_elevation_m` stays an authored-world default only; it must not be used to
     reinterpret imported heights
6. resample the import raster into the authored terrain grid in canonical world metres
7. write the resampled values into authoritative source terrain
8. reset visual terrain from source terrain
9. save the result as a normal `WorldDefinition`

Current deterministic resampling rules:

- resampling happens at terrain sample positions, not cell centres of a separate import-only grid
- v1 uses bilinear interpolation from source DEM values
- border-only nodata introduced by edge-aligned resampling is clamped from the nearest interior
  valid sample; any remaining nodata after that is a hard rejection
- source elevation values are numerically preserved apart from that resampling step; v1 does not
  apply erosion, exaggeration, or artistic normalization during import
- vertical datum conversion is out of scope for v1; import assumes the source height values are
  already the desired authored elevations

Allowed compatibility boundary:

- DEM import may materialize one dense temporary mosaic and one dense target raster because import
  is an offline editor operation, not a hot simulation path

Explicit non-goals of v1 DEM import:

- automatic extraction of rivers, lakes, or hydrology from the DEM
- automatic terrain texturing or biome painting
- importing hydrography vectors/raster as water gameplay state
- preserving external CRS coordinates as live gameplay-space coordinates
- patch-importing one DEM over part of an already edited authored world

Remaining direction:

- integrate DEM import into the WorldEditor UI instead of keeping it as an offline tool only
- remove the importer's current dependency on the pre-`HEIGHT_SCALE` runtime compatibility layer

## Remaining Planned Deterministic Implementation

The remaining slices below are still intended next steps.

### 7. Optional Future: Richer Water Authoring

The current authored-water model is sufficient as the shipped baseline:

- `Lake Fill`
- `Open Water`

We do not currently require a separate richer hydrology layer beyond that baseline.

If richer water authoring is ever added later, the rules should remain:

- any richer river/channel ownership stays separate from raw terrain elevation
- richer water authoring must not be stored as "just paint some water depth into the live runtime
  buffer"
- imported hydrography may be used as an editor reference, but not as implicit gameplay water state
- imported DEMs and imported hydrography remain separate inputs; one does not silently create the
  other
- the current `Lake Fill` / `Open Water` workflow remains valid even if no richer river-path
  tooling is ever added

### 8. Use A Hybrid Water Model For Very Large Worlds

The intended large-world water model is hybrid.

Deterministic rules:

- the current authored-water baseline defines still water bodies for the whole
  world
- local active areas may run dynamic water simulation later only if a new bounded flow design is
  added
- the engine must not require a full-world dense shallow-water solve for every map size

### 9. Keep The First Terrain / Water Realism Pass Render-Only

Terrain and water should look more natural, but the first realism pass must stay strictly on the
render side.

Deterministic rules:

- the first terrain/water realism pass must work in both gameplay and WorldEditor from the same
  live terrain and water buffers
- the first realism pass must not require external base textures
- the first realism pass must not add authored-material data to `WorldDefinition`
- the first realism pass must not add visual-material data to city saves
- realism tuning should live in renderer/shader parameters, not in simulation state
- terrain realism may derive from:
  - world elevation
  - slope / local normal
  - procedural hillshade
  - shoreline proximity
  - low-frequency procedural color breakup
- water realism may derive from:
  - water depth
  - shoreline proximity
  - view-angle / fresnel-like response
  - small procedural surface breakup
- coastline smoothing may be handled visually with soft shoreline masking, shoreline foam bands,
  or similar shader-side treatment
- on coarse authored grids such as `10 m`, shoreline improvement should prefer sub-texel render
  coverage / contour-style masking before increasing `terrain_cell_m` density
- these visual passes must never change:
  - source terrain
  - visual terrain ownership rules
  - authored water records
  - runtime baseline water depth state
  - save schema

Deterministic v1 realism scope:

- terrain should move away from one flat height-color ramp toward:
  - slope-aware rock versus soil/vegetation weighting
  - stronger relief readability from hillshade
  - shoreline color transitions
  - subtle macro variation so large areas are not one uniform tint
- water should move away from one flat translucent surface toward:
  - shallow-versus-deep color separation
  - stronger coast readability
  - view-angle-dependent reflectance / specular response
  - mild procedural breakup so calm water does not look perfectly flat
- the first realism pass must stay cheap enough to share the existing terrain/water renderer path
  between gameplay and WorldEditor

Deterministic terrain color ownership after the first realism pass:

- the long-term terrain color model must be surface-classification-first and absolute-height-second
- absolute world elevation may still influence the palette, but it must not be the primary terrain
  material classifier
- the primary terrain color inputs should be derived from the live visible terrain field:
  - slope / local normal
  - local relief / ruggedness
  - shoreline or visible-water proximity
  - narrow shore-transition cues derived from visible water, not only from `0 m`
  - macro variation / large-scale breakup
- absolute world elevation should be treated as a secondary modifier that nudges an already chosen
  surface class rather than choosing the surface class by itself
- default dry terrain must read as normal inland ground / forest floor, not as marsh, shoreline,
  or tidal flats just because it is low or near one of many lakes
- shoreline influence should remain a relatively narrow visual transition near visible water, not a
  dominant lowland terrain class
- imported DEM worlds and hand-authored blank worlds must share the same terrain-color ownership
  model; terrain color must not assume a specific real-world altitude band such as Kuopio's
  imported range
- the renderer may still use absolute elevation to bias:
  - colder / harsher upland tones
  - alpine / snow transition
  - gentle broad lowland hue shifts where appropriate
  but those biases must remain weaker than the primary surface-classification cues
- the terrain-color model must remain render-only and must not introduce authored material records,
  biome records, or save-schema changes
- the terrain-color model must remain cheap enough to share the same terrain renderer path between
  gameplay and WorldEditor

Deterministic shoreline contour rendering slice:

- on coarse authored grids such as `10 m`, the preferred next shoreline-quality step is contour
  extraction from the live visible water field, not additional blur radius and not a denser
  authored map by default
- shoreline contour extraction must remain render-only and must never become authored world state
- the extraction input must be the same composed visible water depth used by the water renderer:
  baseline water after terrain alignment
- shoreline extraction must use a waterline threshold at the visible shoreline boundary, not a
  post-hoc artistic painted mask
- the extraction algorithm may be marching squares or an equivalent contour/isoband method, but
  the output contract is:
  - smoother diagonal coastlines than raw cell edges
  - smoother narrow channel shorelines than raw cell edges
  - no visible whole-cell stair stepping as the primary shoreline shape in common camera views
  - shoreline position must stay tied to the live water field, not drift arbitrarily for style
- the renderer may realize that contour result as either:
  - a dedicated shoreline mesh
  - a higher-resolution shoreline mask or distance field
  - another equivalent render-only contour representation
- whichever render representation is chosen, it must update whenever terrain or visible water is
  refreshed in gameplay or WorldEditor
- the shoreline contour slice must not require any save-schema change, `WorldDefinition` schema
  change, or additional authored shoreline records
- sub-texel shoreline coverage smoothing remains an allowed fallback/interim treatment, but it is
  not the long-term primary shoreline-quality solution on coarse grids
- shoreline contour rendering may improve the visible water edge only; it does not create
  sub-cell terrain-bank geometry and must not pretend to solve blocky terrain cuts by itself

Deterministic cliff breakline / cliff band rendering slice:

- on coarse authored grids such as `10 m`, the preferred next cliff-quality step is render-only
  cliff extraction from the live terrain field, not manual authored cliff painting and not a
  denser authored map by default
- cliff rendering must remain render-only and must never become authored world state
- the extraction input must be the same live visible terrain field used by terrain rendering after
  terrain refresh, plus its derived slope / local-normal information
- cliff detection must classify steep terrain from the live terrain field, not from a post-hoc
  artist-painted mask
- the extraction algorithm may use slope thresholds, hysteresis, marching squares, contour
  extraction, or another equivalent method, but the output contract is:
  - one upper cliff breakline tied to the visible terrain top edge
  - one lower cliff breakline tied to the visible terrain toe / bottom edge
  - one rendered cliff-face band or equivalent representation between those two lines
  - smoother diagonal cliff edges than raw cell silhouettes in common camera views
  - visibly stronger cliff readability than the current raw terrain mesh alone
  - no arbitrary drift away from the live terrain field for style
- the renderer may realize that cliff result as either:
  - a dedicated cliff ribbon / band mesh
  - a higher-resolution cliff mask or distance field
  - another equivalent render-only breakline representation
- whichever render representation is chosen, it must update whenever visible terrain is refreshed
  in gameplay or WorldEditor
- the cliff rendering slice must not require any save-schema change, `WorldDefinition` schema
  change, or additional authored cliff records
- cliff rendering may darken or re-shade the cliff face, but that shading must remain derived from
  the live terrain field and must not introduce authored material ownership
- cliff rendering improves visible cliff readability only; it does not create sub-cell terrain
  geometry and must not pretend to solve coarse side silhouettes by itself

Deterministic terrain-border skirt rendering slice:

- the preferred map-edge treatment is a render-only terrain-border skirt derived from the live
  terrain edge, not authored border geometry and not simulation-owned world walls
- terrain-border skirt rendering must remain render-only and must never become authored world
  state
- the skirt input must be the same live visible terrain field used by terrain rendering after
  terrain refresh
- the skirt must be built from the outer terrain edge of the current world and extruded downward
  to one fixed render-only depth
- the terrain-border skirt output contract is:
  - the map edge reads as a visible cut through the terrain instead of a paper-thin plane
  - contour lines or equivalent elevation bands visibly continue down the side surface
  - the skirt stays aligned to the live terrain edge and updates whenever terrain visuals refresh
  - the top terrain surface remains the authoritative playable surface; the skirt is presentation
    only
- the renderer may realize that skirt result as either:
  - one dedicated side-wall mesh plus one bottom cap
  - one equivalent render-only border representation
- whichever render representation is chosen, it must update whenever visible terrain is refreshed
  in gameplay or WorldEditor
- the terrain-border skirt slice must not require any save-schema change, `WorldDefinition` schema
  change, or additional authored border records
- the skirt material may use contour lines, sediment-style depth banding, or other earth-layer
  cues, but those cues must remain render-only and derived from the live terrain field
- the terrain-border skirt may improve the perceived thickness and readability of the map edge
  only; it must not introduce playable vertical terrain, collision ownership, or simulation-owned
  edge walls

Deterministic asset policy:

- external terrain/water material textures are optional later enhancements, not a prerequisite for
  the first realism pass
- if material textures are added later, they remain visual assets only
- authored worlds and imported DEM worlds must remain valid and usable even when no external
  material textures are present

Explicit non-goals of the first realism pass:

- authored material painting
- biome simulation
- erosion or sediment simulation for visual purposes
- dependency on downloaded hillshade rasters
- dependency on scanned PBR material libraries before terrain/water can look acceptable
- increasing `terrain_cell_m` density solely to get a smoother visible shoreline before contour
  rendering has been attempted
- increasing `terrain_cell_m` density solely to get smoother visible cliff edges before
  breakline/band rendering has been attempted
- adding authored border-wall geometry or persistence merely to make the map edge look thicker
- using one absolute-height ramp as the long-term primary terrain material classifier

## Current Deterministic Non-Goals

The following are explicitly not implemented yet and should not be assumed by other systems:

- interactive WorldEditor DEM / GeoTIFF import UI
- optional richer river-path / channel authoring beyond the current water tool set
- chunk-streamed terrain renderer
- chunk-window water simulation
- explicit atmospheric / horizon brightening independent from terrain material detail fade
- authoritative terrain undo across source plus derived state
- direct-metre terrain height storage without `HEIGHT_SCALE`

## Implementation Guardrails

These rules must stay true as the terrain/world system grows:

- source terrain is authoritative; derived terrain is never the only source of truth
- sparse chunk-backed storage is the resting runtime representation
- dense buffers are boundary tools, not the runtime ownership model
- new terrain import or authoring paths must write authoritative source terrain
- old save migration is not required unless a future change explicitly decides otherwise
- large-world support must avoid whole-world dense simulation assumptions

## Short Version

What is implemented now:

- `WorldConfig` replaced the legacy map config
- `terrain_cell_m` exists and terrain/water sample density is independently configurable
- runtime world-space XZ is canonical metres again
- terrain and water are sparse chunk-backed at rest
- terrain keeps authoritative source plus derived visual buffers
- water keeps sparse baseline depth at rest
- blank-world `WorldDefinition` exists as a separate authored-world asset
- authored world load resets runtime state to a fresh blank city baseline
- offline DEM import can now generate a normal `WorldDefinition` from real GeoTIFF elevation tiles
- first authored-water tools are live in WorldEditor through `Lake Fill` and `Open Water`
- `WorldDefinition` now persists inland lake fills and edge-connected open-water fills
- live water now keeps authored baseline still water only; the dynamic flowing-water prototype was
  removed
- save/load and renderer boundaries still use dense materialization
- terrain shoreline/debug water sampling now binds the Water renderer's resident patch depth
  texture directly instead of materializing a second terrain-aligned water texture
- water mesh refresh now uses async Rust/Rayon preparation plus a Rust-owned ready queue,
  Godot-side ready polling, pending-job backpressure, stale road/depth signature rejection, and a
  measured time/byte apply budget, with indexed buffers for fully wet unclipped patches, shared
  Godot `ArrayMesh` reuse for matching full-grid variants, and Godot-side stale queue compaction
  guided by request/cache/job perf counters; ready polling is adaptive and fills a bounded
  camera-sorted apply queue, ready/apply drainage receives a conservative headroom boost only while
  backlog is high, and the mesh refresh scheduler polls ready work, applies ready uploads, then
  submits new work last so cache-hit request work does not compete with expensive `ArrayMesh`
  uploads in the same frame
- terrain/water non-mesh patch payloads for residency, speculative prewarm, and resident dirty
  uploads now prepare asynchronously in Rust and are polled by Godot before main-thread resource
  apply; patch texture uploads consume Rust-provided byte payloads, regular terrain mesh variants
  are prewarmed by active layout, terrain/water patch resources are pooled/prewarmed before first
  visible residency activation, water prewarms shared full-grid `ArrayMesh` variants, and perf
  summaries include render stats for viewport, draw calls, primitives, memory buckets, vsync,
  FPS cap, and resource-pool counters; refined terrain preparation snapshots only bounded local
  inputs under the simulation lock, performs road/site grading and CDT work off-lock, and uses
  patch-local revisions with one physical in-flight build per patch/render-step. Road-loop source
  attribution uses a sorted exact-edge index before its metric-overlap fallback, and canonical CDT
  skips source-vertex recovery only after proving every source endpoint is already represented
- gameplay world-load refresh leaves terrain/water/network revisions dirty until the renderers
  acknowledge the exact payloads and road mesh they actually uploaded; resident terrain batches
  are prevalidated and staged as detached/inactive resources before an atomic scene swap
- terrain and water patch residency plus speculative cache prewarm now use elapsed-time budgets
  with camera-prioritized patch order; water follows terrain's resident-set revision for the
  steady-state no-change path, and terrain/water mesh-LOD refreshes plus terrain-to-water texture
  sync drain through time-budgeted queues instead of sweeping every resident patch in one frame
- terrain/water activation removes out-of-window patches farthest-first, drains downstream texture /
  LOD / mesh queues closest-first, and exports residency add/remove/pending counters for streaming
  perf captures
- ready terrain and water residency can each activate at most `12` patches per frame and stop after
  `4 ms`; this is still `O(k)` with `k <= 12`, while cached/cheap rotation handoffs are no longer
  forced through the old two-patch-per-frame bottleneck
- ordinary terrain payload requests may enqueue `32` camera-prioritized patches per frame; refined
  road/site terrain retains its separate two-request cap so faster raw streaming does not flood the
  expensive CDT path
- ready water mesh uploads apply at most `4` patches normally or `6` only under the existing
  measured-headroom boost, with `2.5/3.5 ms` and `1.4/2.2 MB` time/byte limits respectively
- terrain/water LOD refreshes are movement-gated and cap checked/changed patches per frame so
  periodic LOD validation does not rescan or rebuild the whole resident set in one frame; new
  movement-triggered resident sweeps replace stale pending sweep entries and expose replaced-count
  counters so queue buildup is measurable, and resident sweeps only enqueue patches whose target
  LOD/subdivision differs from current state while reporting skipped-count counters; baked/CDT
  terrain patches skip no-op LOD mesh rebuilds, and speculative terrain/water prewarm now covers
  only a bounded halo around the resident/activation region
- terrain rendering now derives hillshade procedurally from the live heightmap in both gameplay
  and WorldEditor; it is not stored as separate world data
- terrain and water rendering now use a first render-only realism pass with slope-aware terrain
  shading, shoreline-aware terrain tinting, depth-aware water color, and shader-side shoreline /
  fresnel / procedural breakup treatment, including contour-style shoreline rendering on the
  existing `10 m` grid from the live visible water field plus render-only cliff breakline / cliff
  band treatment from the live terrain field
- terrain grass and building-site grass now use the same Grass002 material stack with world-space
  UVs, stochastic anti-tiling, and luminance-preserving detail fade so camera distance changes
  reduce detail contrast rather than changing base brightness
- centralized scene lighting now provides a deterministic procedural hemisphere sky, static
  texture-derived upper-sky cloud cover, a terrain-range-aware far-distance fade, and a shared
  directional sun / shadow baseline for terrain, site ground, buildings, cars, roads, water, and
  editor/debug helpers
- terrain coloring now uses surface classification first and absolute height second, so flat blank
  worlds and imported DEM worlds share the same inland-first palette model instead of depending
  mainly on one absolute-height ramp
- terrain-border skirt rendering now adds a render-only side wall plus bottom cap derived from the
  live terrain edge, with contour continuation, noise-warped earth strata, distance-faded surface
  relief, and a shallow irregular topsoil lip so maps read as a visible cut through terrain instead
  of a paper-thin top plane
- water rendering now also adds a render-only edge curtain where visible water reaches the map
  boundary, so outside-of-map views do not expose the submerged terrain plane through the
  transparent water surface

What is next:

1. interactive DEM / GeoTIFF import UI for real-map authored worlds
2. dedicated shoreline mesh or distance-field rendering if the current contour-style shoreline
   field still is not enough for close camera work
3. chunk-window runtime processing
4. later texture-assisted terrain/water materials if the shader-only realism pass is not enough

Optional later only:

- richer river-path / channel authoring if the current water-tool workflow later proves
  insufficient for map making
