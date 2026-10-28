// SPDX-License-Identifier: GPL-2.0-only

//! `TERRAIN-07-D` acceptance on the Rust side: the graded patch tile lists against the bake, the
//! slot packing, and a port of the Godot cell shader's arithmetic
//! (`godot/scripts/shaders/terrain_cells.gdshaderinc`) checked against the bake: the cell
//! lattices equal the bake's, stitched borders share their vertices and texel values bitwise,
//! and the exact seam mask equals coverage. The port mirrors the shader line by line (integer
//! half-metres, `f32` for the mask); change both together.

use super::coverage::covered_height_at;
use super::tests::{LEVELS, compiled, fixtures, window_tiles};
use super::tile::{Evaluation, bake_tiles_with, lattice};
use super::*;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::{PieceId, RoadbedSystem};
use crate::simulation::terrain::TerrainSystem;

const SIDE_HM: i32 = 128;

// --- The shader's lattice arithmetic, ported. ---

fn floor_div(a: i32, b: i32) -> i32 {
    a.div_euclid(b)
}

fn imod(a: i32, b: i32) -> i32 {
    a - b * floor_div(a, b)
}

fn code_spacing_hm(code: i32, cell_hm: i32) -> i32 {
    if code < 3 {
        1 << code
    } else {
        cell_hm << (code - 3)
    }
}

fn code_phase_hm(code: i32, spacing: i32, anchor_hm: i32, origin_hm: i32) -> i32 {
    if code < 3 {
        0
    } else {
        imod(anchor_hm - origin_hm, spacing)
    }
}

fn vertex_hm(code: i32, k: i32, spacing: i32, phase: i32) -> i32 {
    if code < 3 {
        k * spacing
    } else {
        (phase + spacing * (k - 1)).clamp(0, SIDE_HM)
    }
}

fn snap_hm(p: i32, spacing: i32, phase: i32) -> i32 {
    if p >= SIDE_HM {
        return SIDE_HM;
    }
    (phase + spacing * floor_div(p - phase, spacing)).max(0)
}

fn first_line_hm(spacing: i32, phase: i32) -> i32 {
    if phase == 0 { spacing } else { phase }
}

fn inner_lines(spacing: i32, phase: i32) -> i32 {
    let first = first_line_hm(spacing, phase);
    if first < SIDE_HM {
        (SIDE_HM - 1 - first) / spacing + 1
    } else {
        0
    }
}

fn index_at(code: i32, p: i32, spacing: i32, phase: i32) -> i32 {
    if code < 3 {
        return 1 + p / spacing;
    }
    let first = first_line_hm(spacing, phase);
    if p >= SIDE_HM {
        2 + inner_lines(spacing, phase)
    } else if p < first {
        1
    } else {
        2 + (p - first) / spacing
    }
}

fn index_hm(code: i32, idx: i32, spacing: i32, phase: i32) -> i32 {
    if code < 3 {
        return (idx - 1) * spacing;
    }
    let first = first_line_hm(spacing, phase);
    let end_index = 2 + inner_lines(spacing, phase);
    if idx <= 0 {
        if phase == 0 {
            -spacing
        } else {
            phase - spacing
        }
    } else if idx == 1 {
        0
    } else if idx < end_index {
        first + (idx - 2) * spacing
    } else if idx == end_index {
        SIDE_HM
    } else if imod(SIDE_HM - phase, spacing) == 0 {
        SIDE_HM + spacing
    } else {
        first + inner_lines(spacing, phase) * spacing
    }
}

// One cell's lattice frame, as the shader's `CellFrame`.
#[derive(Clone, Copy, Debug)]
struct Frame {
    code: i32,
    origin_hm: [i32; 2],
    spacing: i32,
    phase: [i32; 2],
}

fn frame(code: i32, tile: TileKey, anchor_hm: [i32; 2], cell_hm: i32) -> Frame {
    let origin_hm = [tile.x * SIDE_HM, tile.z * SIDE_HM];
    let spacing = code_spacing_hm(code, cell_hm);
    Frame {
        code,
        origin_hm,
        spacing,
        phase: [0, 1].map(|axis| code_phase_hm(code, spacing, anchor_hm[axis], origin_hm[axis])),
    }
}

// Side codes west, east, south, north; the shader's `cell_vertex_position_hm` (unclamped: the
// fixtures' tiles lie inside one patch).
fn vertex_position_hm(
    frame: Frame,
    k: [i32; 2],
    sides: [i32; 4],
    anchor_hm: [i32; 2],
    cell_hm: i32,
) -> [i32; 2] {
    let mut p = [0, 1].map(|axis| vertex_hm(frame.code, k[axis], frame.spacing, frame.phase[axis]));
    if p[0] == 0 || p[0] == SIDE_HM {
        let side = if p[0] == 0 { sides[0] } else { sides[1] };
        let s = code_spacing_hm(side, cell_hm);
        p[1] = snap_hm(
            p[1],
            s,
            code_phase_hm(side, s, anchor_hm[1], frame.origin_hm[1]),
        );
    }
    if p[1] == 0 || p[1] == SIDE_HM {
        let side = if p[1] == 0 { sides[2] } else { sides[3] };
        let s = code_spacing_hm(side, cell_hm);
        p[0] = snap_hm(
            p[0],
            s,
            code_phase_hm(side, s, anchor_hm[0], frame.origin_hm[0]),
        );
    }
    p
}

// Mesh vertices per axis for a code, as terrain_graded_tiles.gd builds the meshes.
fn mesh_vertices(code: i32, cell_hm: i32) -> i32 {
    let spacing = code_spacing_hm(code, cell_hm);
    if code < 3 {
        SIDE_HM / spacing + 1
    } else {
        (SIDE_HM + spacing - 1) / spacing + 2
    }
}

fn level_code(level: Level) -> i32 {
    match level {
        Level::Gl0 => 0,
        Level::Gl1 => 1,
        Level::Gl2 => 2,
        Level::Gl3 { step } => 3 + i32::from(step).trailing_zeros() as i32,
    }
}

fn anchor_hm(terrain: &TerrainSystem) -> [i32; 2] {
    let cell = f64::from(terrain.cell_size_m());
    [terrain.width, terrain.height].map(|n| (-(n as f64 - 1.0) * cell) as i32)
}

fn cell_hm(terrain: &TerrainSystem) -> i32 {
    (f64::from(terrain.cell_size_m()) * 2.0).round() as i32
}

// --- The shader's exact seam mask, ported (`f32`, tile-frame metres). ---

#[derive(Clone, Copy, Debug)]
struct Hit {
    distance: f32,
    line_distance: f32,
    signed_distance: f32,
    index: i32,
}

const NO_HIT: Hit = Hit {
    distance: 1e30,
    line_distance: 0.0,
    signed_distance: 1e30,
    index: 1 << 30,
};

fn segment_at(seg: [f32; 4], p: [f32; 2], index: i32) -> Hit {
    let a = [seg[0], seg[1]];
    let ab = [seg[2] - seg[0], seg[3] - seg[1]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1];
    let mut q = a;
    let mut cross = 0.0;
    if len2 > 0.0 {
        let t = ((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2;
        cross = ab[0] * (p[1] - a[1]) - ab[1] * (p[0] - a[0]);
        q = if t <= 0.0 {
            a
        } else if t >= 1.0 {
            [seg[2], seg[3]]
        } else {
            [a[0] + ab[0] * t, a[1] + ab[1] * t]
        };
    }
    let (ex, ez) = (p[0] - q[0], p[1] - q[1]);
    let d = (ex * ex + ez * ez).sqrt();
    let line = if len2 > 0.0 {
        cross.abs() / len2.sqrt()
    } else {
        0.0
    };
    Hit {
        distance: d,
        line_distance: line,
        signed_distance: if cross > 0.0 { -d } else { d },
        index,
    }
}

fn nearer(a: &Hit, b: &Hit) -> bool {
    if a.distance != b.distance {
        return a.distance < b.distance;
    }
    if a.line_distance != b.line_distance {
        return a.line_distance > b.line_distance;
    }
    a.index < b.index
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

// The fragment's axis stop: the index at or before `x` and the fraction to the next sample.
fn axis_stop(frame: Frame, x: f32, axis: usize) -> (i32, f32) {
    let p = ((x * 2.0).floor() as i32).clamp(0, SIDE_HM - 1);
    let index = index_at(frame.code, p, frame.spacing, frame.phase[axis]);
    let p0 = index_hm(frame.code, index, frame.spacing, frame.phase[axis]) as f32 * 0.5;
    let p1 = index_hm(frame.code, index + 1, frame.spacing, frame.phase[axis]) as f32 * 0.5;
    (index, ((x - p0) / (p1 - p0).max(1e-6)).clamp(0.0, 1.0))
}

// The signed distance the shader's `cell_mask` tests against zero.
fn mask_sdf(frame: Frame, payload: &TilePayload, p: [f32; 2]) -> f32 {
    let (ix, fx) = axis_stop(frame, p[0], 0);
    let (iz, fz) = axis_stop(frame, p[1], 1);
    let width = payload.samples[0];
    let t: [[f32; 4]; 4] = std::array::from_fn(|c| {
        let (i, j) = (ix as usize + (c & 1), iz as usize + (c >> 1));
        payload.texels[j * width + i]
    });
    let mut sdf = mix(mix(t[0][1], t[1][1], fx), mix(t[2][1], t[3][1], fx), fz);
    let spacing_m = frame.spacing as f32 * 0.5;
    if sdf.abs() < 2.0 * spacing_m {
        let mut best = NO_HIT;
        for texel in &t {
            let index = texel[2] as i32;
            if index < 0 {
                continue;
            }
            let hit = segment_at(payload.seams[index as usize], p, index);
            if nearer(&hit, &best) {
                best = hit;
            }
        }
        // The square's extra candidates (T48 (a)), by the same order.
        let point = (iz as usize * width + ix as usize) as u32;
        if let Ok(k) = payload.square_extras.binary_search_by_key(&point, |&(at, _)| at) {
            let first = payload.square_extras[k].1 as usize;
            let count = payload.extras[first][0] as usize;
            for k in 1..=count {
                let index = payload.extras[first + k / 4][k % 4] as i32;
                let hit = segment_at(payload.seams[index as usize], p, index);
                if nearer(&hit, &best) {
                    best = hit;
                }
            }
        }
        if best.index < NO_HIT.index {
            sdf = best.signed_distance;
        }
    }
    sdf
}

fn bake_level(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    tiles: &[TileKey],
    level: Level,
) -> Vec<TilePayload> {
    let mut out = Vec::new();
    bake_tiles_with(
        terrain,
        roadbed,
        &NoSites,
        tiles,
        level,
        Evaluation::Lanes,
        &mut out,
    )
    .expect("tiles bake");
    out
}

// Whether a texel renders differently from the base: graded, covered or painted.
fn texel_renders(texel: [f32; 4]) -> bool {
    texel[0] != 0.0 || texel[1] < 0.0 || (texel[3] as i32) & 15 != 0
}

#[test]
fn cell_lattices_are_the_bake_lattices() {
    let mut cases = 0;
    for (name, _, terrain) in fixtures() {
        let (anchor, cell) = (anchor_hm(&terrain), cell_hm(&terrain));
        let (mut xs, mut zs) = (Vec::new(), Vec::new());
        for level in LEVELS
            .into_iter()
            .chain([2, 4, 8].map(|step| Level::Gl3 { step }))
        {
            let code = level_code(level);
            for tile in window_tiles() {
                lattice(&terrain, tile, level, &mut xs, &mut zs);
                let frame = frame(code, tile, anchor, cell);
                let [ox, oz] = tile.origin();
                assert!(
                    xs.len() <= slot_side(level, f64::from(cell) * 0.5),
                    "{name}: slot side"
                );
                for (axis, coords, origin) in [(0, &xs, ox), (1, &zs, oz)] {
                    for (idx, &world) in coords.iter().enumerate() {
                        let hm = (world - origin) * 2.0;
                        assert_eq!(
                            f64::from(index_hm(code, idx as i32, frame.spacing, frame.phase[axis])),
                            hm,
                            "{name} {level:?} {tile:?} axis {axis} index {idx}"
                        );
                        if (0.0..=128.0).contains(&hm) {
                            assert_eq!(
                                index_at(code, hm as i32, frame.spacing, frame.phase[axis]),
                                idx as i32,
                                "{name} {level:?} {tile:?} axis {axis} at {hm}"
                            );
                        }
                    }
                    // Every mesh vertex lies on a lattice point.
                    for k in 0..mesh_vertices(code, cell) {
                        let p = vertex_hm(code, k, frame.spacing, frame.phase[axis]);
                        let idx = index_at(code, p, frame.spacing, frame.phase[axis]);
                        assert_eq!(index_hm(code, idx, frame.spacing, frame.phase[axis]), p);
                    }
                }
                cases += 1;
            }
        }
    }
    assert!(cases > 0);
}

#[test]
fn stitched_borders_share_vertices_and_texels_bitwise() {
    // `t_slope`: graded on both sides of the shared side x = 0 between tiles (-1, z) and (0, z).
    let (_, graph, terrain) = fixtures().swap_remove(0);
    let roadbed = compiled(&graph);
    let (anchor, cell) = (anchor_hm(&terrain), cell_hm(&terrain));
    let levels = [Level::Gl0, Level::Gl1, Level::Gl2, Level::Gl3 { step: 2 }];
    let mut shared = 0;
    for z in -2..2 {
        let (west, east) = (TileKey { x: -1, z }, TileKey { x: 0, z });
        for &fine in &levels {
            for &coarse in &levels {
                let (fine_code, coarse_code) = (level_code(fine), level_code(coarse));
                if fine_code > coarse_code {
                    continue;
                }
                let fine_payload = &bake_level(&terrain, &roadbed, &[west], fine)[0];
                let coarse_payload = &bake_level(&terrain, &roadbed, &[east], coarse)[0];
                let fine_frame = frame(fine_code, west, anchor, cell);
                let coarse_frame = frame(coarse_code, east, anchor, cell);
                // The finer west cell snaps its east side to the coarser east cell's lattice.
                let fine_sides = [fine_code, coarse_code, fine_code, fine_code];
                let coarse_sides = [coarse_code; 4];
                let border = |frame: Frame, sides: [i32; 4], x_side: i32| {
                    let mut points = Vec::new();
                    for kz in 0..mesh_vertices(frame.code, cell) {
                        for kx in 0..mesh_vertices(frame.code, cell) {
                            let p = vertex_position_hm(frame, [kx, kz], sides, anchor, cell);
                            if p[0] == x_side {
                                points.push(p[1]);
                            }
                        }
                    }
                    points.sort_unstable();
                    points.dedup();
                    points
                };
                let fine_border = border(fine_frame, fine_sides, SIDE_HM);
                let coarse_border = border(coarse_frame, coarse_sides, 0);
                assert_eq!(fine_border, coarse_border, "z {z}: {fine:?} | {coarse:?}");
                for &p in &fine_border {
                    let texel = |frame: Frame, payload: &TilePayload, x: i32| {
                        let ix = index_at(frame.code, x, frame.spacing, frame.phase[0]) as usize;
                        let iz = index_at(frame.code, p, frame.spacing, frame.phase[1]) as usize;
                        payload.texels[iz * payload.samples[0] + ix][0].to_bits()
                    };
                    assert_eq!(
                        texel(fine_frame, fine_payload, SIDE_HM),
                        texel(coarse_frame, coarse_payload, 0),
                        "z {z}: {fine:?} | {coarse:?} at {p} hm: delta bits"
                    );
                    shared += 1;
                }
            }
        }
    }
    assert!(shared > 0);
}

#[test]
fn the_exact_mask_equals_coverage() {
    let mut pieces: Vec<PieceId> = Vec::new();
    for (name, graph, terrain) in fixtures() {
        let roadbed = compiled(&graph);
        let (anchor, cell) = (anchor_hm(&terrain), cell_hm(&terrain));
        for level in [Level::Gl0, Level::Gl1] {
            let payloads = bake_level(&terrain, &roadbed, &window_tiles(), level);
            let (mut tested, mut wrong) = (0, 0);
            let mut first = String::new();
            for payload in &payloads {
                let frame = frame(level_code(level), payload.tile, anchor, cell);
                let [ox, oz] = payload.tile.origin();
                // A 0.37 m grid: off the lattice, through every kind of cell.
                for j in 0..173 {
                    for i in 0..173 {
                        let p = [i as f32 * 0.37 + 0.011, j as f32 * 0.37 + 0.017];
                        let (x, z) = (ox + f64::from(p[0]), oz + f64::from(p[1]));
                        let covered = covered_height_at(&roadbed, x, z, &mut pieces).is_some();
                        let sdf = mask_sdf(frame, payload, p);
                        let discarded = sdf < 0.0;
                        tested += 1;
                        if covered != discarded && first.is_empty() {
                            let nearest = payload
                                .seams
                                .iter()
                                .enumerate()
                                .map(|(i, &s)| (segment_at(s, p, i as i32).signed_distance, i))
                                .min_by(|a, b| a.0.abs().total_cmp(&b.0.abs()));
                            let (ix, _) = axis_stop(frame, p[0], 0);
                            let (iz, _) = axis_stop(frame, p[1], 1);
                            let named: Vec<f32> = (0..4)
                                .map(|c| {
                                    let (i, j) = (ix as usize + (c & 1), iz as usize + (c >> 1));
                                    payload.texels[j * payload.samples[0] + i][2]
                                })
                                .collect();
                            first = format!(
                                "tile {:?} p {p:?} covered {covered} sdf {sdf} nearest row seam {nearest:?} named {named:?}",
                                payload.tile
                            );
                        }
                        wrong += usize::from(covered != discarded);
                    }
                }
            }
            assert!(tested > 0);
            assert_eq!(
                wrong, 0,
                "{name} {level:?}: {wrong} of {tested} points; first: {first}"
            );
        }
    }
}

// The exact mask against the roadbed's closed coverage predicate on a dense `step` grid over
// the world rectangle `area` (`[x0, z0, x1, z1]`), at `level`, flat ground: every point's tile
// baked once. A grid through a straight edge's line samples its boundary, where the closed
// coverage holds and the mask's `sdf` is 0 (kept, drawn under the road), so callers offset it.
fn dense_mask_equals_coverage(graph: &RegionGraph, level: Level, area: [f64; 4], step: f64) {
    let terrain = super::tests::terrain(|_, _| 0.0);
    let roadbed = compiled(graph);
    let (anchor, cell) = (anchor_hm(&terrain), cell_hm(&terrain));
    let mut tiles: Vec<TileKey> = Vec::new();
    let [nx, nz] = [0, 1].map(|axis| ((area[axis + 2] - area[axis]) / step).round() as usize);
    let point = |ix: usize, iz: usize| [area[0] + ix as f64 * step, area[1] + iz as f64 * step];
    for iz in 0..=nz {
        for ix in 0..=nx {
            let [x, z] = point(ix, iz);
            tiles.push(TileKey::at(x, z));
        }
    }
    tiles.sort();
    tiles.dedup();
    let payloads = bake_level(&terrain, &roadbed, &tiles, level);
    let mut pieces = Vec::new();
    let (mut wrong, mut tested, mut first) = (0, 0, None);
    for iz in 0..=nz {
        for ix in 0..=nx {
            let [x, z] = point(ix, iz);
            let tile = TileKey::at(x, z);
            let payload = &payloads[tiles.binary_search(&tile).expect("baked tile")];
            let [ox, oz] = tile.origin();
            let local = [(x - ox) as f32, (z - oz) as f32];
            let frame = frame(level_code(level), tile, anchor, cell);
            let covered = covered_height_at(&roadbed, x, z, &mut pieces).is_some();
            let sdf = mask_sdf(frame, payload, local);
            tested += 1;
            if (sdf < 0.0) != covered {
                wrong += 1;
                first.get_or_insert(([x, z], sdf));
            }
        }
    }
    assert!(tested > 0);
    assert_eq!(
        wrong, 0,
        "{level:?}: {wrong} of {tested} points differ; first {first:?}"
    );
}

// The reference bend's outer tangent (`road_junction_reference_test.gd`'s `bend_45`): the first
// chord of the outer arc is 0.49 m long. Checked densely (5 mm) around the five plan pixels of
// `TERRAIN-07-G` (world (0.54-0.84, -4.97)).
fn bend_outer_tangent_mask_equals_coverage(level: Level) {
    let graph = super::tests::hub(&[180.0, 45.0], false);
    dense_mask_equals_coverage(&graph, level, [0.3, -5.3, 1.1, -4.7], 0.005);
}

// At `GL0` (0.5 m) the lattice square's corners at x = 0.5 m lie in the short chord's Voronoi
// region, so a corner names it.
#[test]
fn bend_outer_tangent_mask_equals_coverage_at_gl0() {
    bend_outer_tangent_mask_equals_coverage(Level::Gl0);
}

// At `GL1` (1 m, the plan view's level) no corner of the square names the short chord: without
// the square's extra candidates the mask took the nearer of its neighbours' clamped ends, whose
// half-planes put ground just outside the road inside it (`TERRAIN-07-G` round 3's GPU
// read-back, `docs/terrain.md` T48); the square lists the chord (T48 (a)).
#[test]
fn bend_outer_tangent_mask_equals_coverage_at_gl1() {
    bend_outer_tangent_mask_equals_coverage(Level::Gl1);
}

// `terminal`'s end cap (`road_junction_reference_test.gd`, 25 plan pixels at `GL1` per slope in
// `TERRAIN-07-G` round 3, the bend's signature, T48): the cap's arcs are chords shorter than
// `GL1`'s and `GL2`'s spacing. Every point of a 1 cm grid over the cap and its band.
fn terminal_cap_mask_equals_coverage(level: Level) {
    let graph = super::tests::hub(&[180.0], false);
    dense_mask_equals_coverage(&graph, level, [-2.0013, -9.0017, 9.0, 9.0], 0.01);
}

#[test]
fn terminal_cap_mask_equals_coverage_at_gl1() {
    terminal_cap_mask_equals_coverage(Level::Gl1);
}

#[test]
fn terminal_cap_mask_equals_coverage_at_gl2() {
    terminal_cap_mask_equals_coverage(Level::Gl2);
}

// The exact mask equals coverage across a bridge's mouth (`TERRAIN-07-D` round 9, the `bridge_t`
// `under_bridge` dashed line): on a 2 cm grid over the 4 m either side of the mouth row (along
// `z` at `x = 0`), `GL0` and `GL1`. Before the rows were named, 9,938 of 640,000 `GL0` points
// beyond the row were discarded, up to 0.98 m from it. (A tunnel's mouth is the portal
// opening's: the ground over its deck is cut past the row by design.)
#[test]
fn the_exact_mask_equals_coverage_across_a_bridge_mouth() {
    use crate::simulation::network::types::EdgeClass;
    let mut pieces: Vec<PieceId> = Vec::new();
    let (graph, terrain) = super::site_tests::structure_mouth(EdgeClass::Bridge, -0.5);
    let roadbed = compiled(&graph);
    let (anchor, cell) = (anchor_hm(&terrain), cell_hm(&terrain));
    let tiles = [-1, 0].map(|x| [-1, 0].map(|z| TileKey { x, z })).concat();
    for level in [Level::Gl0, Level::Gl1] {
        let payloads = bake_level(&terrain, &roadbed, &tiles, level);
        let (mut tested, mut wrong) = (0, 0);
        let mut first = String::new();
        for payload in &payloads {
            let frame = frame(level_code(level), payload.tile, anchor, cell);
            let [ox, oz] = payload.tile.origin();
            for j in 0..1600 {
                for i in 0..400 {
                    let (x, z) = (
                        -4.0 + i as f64 * 0.02 + 0.003,
                        -16.0 + j as f64 * 0.02 + 0.0013,
                    );
                    let p = [(x - ox) as f32, (z - oz) as f32];
                    if !(0.0..64.0).contains(&p[0]) || !(0.0..64.0).contains(&p[1]) {
                        continue;
                    }
                    let covered = covered_height_at(&roadbed, x, z, &mut pieces).is_some();
                    let discarded = mask_sdf(frame, payload, p) < 0.0;
                    tested += 1;
                    if covered != discarded && first.is_empty() {
                        first = format!("({x:.3}, {z:.3}) covered {covered}");
                    }
                    wrong += usize::from(covered != discarded);
                }
            }
        }
        assert!(tested > 600_000, "{level:?}: {tested} points");
        assert_eq!(
            wrong, 0,
            "{level:?}: {wrong} of {tested} points; first {first}"
        );
    }
}

#[test]
fn patch_tile_lists_hold_every_tile_that_renders_graded() {
    let mut scratch = PatchTilesScratch::default();
    let mut listed = Vec::new();
    for (name, graph, terrain) in fixtures() {
        let roadbed = compiled(&graph);
        // Two patch-like rectangles, one on tile borders and one off them.
        for rect in [[-128.0, -128.0, 128.0, 128.0], [-110.0, -97.0, 93.0, 121.0]] {
            graded_tiles_in_rect(
                &terrain,
                &roadbed,
                &NoSites,
                rect,
                &mut scratch,
                &mut listed,
            )
            .expect("list");
            assert!(listed.is_sorted(), "{name}: sorted");
            for level in LEVELS {
                let payloads = bake_level(&terrain, &roadbed, &window_tiles(), level);
                for payload in &payloads {
                    let (mut xs, mut zs) = (Vec::new(), Vec::new());
                    lattice(&terrain, payload.tile, level, &mut xs, &mut zs);
                    let renders = zs.iter().enumerate().any(|(j, &z)| {
                        xs.iter().enumerate().any(|(i, &x)| {
                            x >= rect[0].max(payload.tile.origin()[0])
                                && x <= rect[2].min(payload.tile.origin()[0] + TILE_M)
                                && z >= rect[1].max(payload.tile.origin()[1])
                                && z <= rect[3].min(payload.tile.origin()[1] + TILE_M)
                                && texel_renders(payload.texels[j * xs.len() + i])
                        })
                    });
                    if renders {
                        assert!(
                            listed.contains(&payload.tile),
                            "{name} {level:?}: {:?} renders graded but is not listed",
                            payload.tile
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn site_tiles_are_listed() {
    let sites = super::site_tests::fixture_sites();
    let (_, graph, terrain) = fixtures().swap_remove(1);
    let roadbed = compiled(&graph);
    let mut scratch = PatchTilesScratch::default();
    let mut listed = Vec::new();
    let rect = [-128.0, -128.0, 128.0, 128.0];
    graded_tiles_in_rect(&terrain, &roadbed, &sites, rect, &mut scratch, &mut listed)
        .expect("list");
    let mut payloads = Vec::new();
    bake_tiles_with(
        &terrain,
        &roadbed,
        &sites,
        &window_tiles(),
        Level::Gl0,
        Evaluation::Lanes,
        &mut payloads,
    )
    .expect("bake");
    for payload in &payloads {
        if payload.texels.iter().any(|&texel| texel_renders(texel)) {
            assert!(
                listed.contains(&payload.tile),
                "{:?} not listed",
                payload.tile
            );
        }
    }
}

#[test]
fn packed_slots_hold_the_payload_bitwise() {
    let (_, graph, terrain) = fixtures().swap_remove(0);
    let roadbed = compiled(&graph);
    let cell_m = f64::from(terrain.cell_size_m());
    for level in LEVELS {
        let side = slot_side(level, cell_m);
        for payload in bake_level(&terrain, &roadbed, &window_tiles(), level) {
            let (mut texels, mut rows) = (Vec::new(), Vec::new());
            pack_slot(&payload, side, &mut texels, &mut rows);
            assert_eq!(texels.len(), side * side * 16);
            assert_eq!(rows.len(), slot_row_texels(level) * 16);
            let read = |bytes: &[u8], texel: usize| -> [u32; 4] {
                std::array::from_fn(|c| {
                    let at = texel * 16 + c * 4;
                    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
                })
            };
            let [width, depth] = payload.samples;
            for j in 0..depth {
                for i in 0..width {
                    let point = (j * width + i) as u32;
                    let mut texel = payload.texels[point as usize];
                    let list = payload.square_extras.iter().find(|&&(at, _)| at == point);
                    if let Some(&(_, first)) = list {
                        texel[2] = seam_channel(texel[2], first);
                    }
                    let packed = read(&texels, j * side + i);
                    assert_eq!(packed, texel.map(f32::to_bits));
                    // The shader's decoding gives the row and the list back.
                    let channel = f32::from_bits(packed[2]) as i32;
                    let decoded = (channel + 1) >> 11;
                    assert_eq!(channel - (decoded << 11), payload.texels[point as usize][2] as i32);
                    assert_eq!(decoded, list.map_or(0, |&(_, first)| first as i32 + 1));
                }
            }
            for (index, texel) in payload.extras.iter().enumerate() {
                assert_eq!(
                    read(&rows, slot_extras_at(level) + index),
                    texel.map(f32::to_bits)
                );
            }
            for (index, seam) in payload.seams.iter().enumerate() {
                assert_eq!(read(&rows, index), seam.map(f32::to_bits));
            }
            for (index, edge) in payload.edges.iter().enumerate() {
                assert_eq!(
                    read(&rows, level.seam_capacity() + index),
                    edge.map(f32::to_bits)
                );
            }
            assert_eq!(payload.seam_heights.len(), payload.seams.len());
            for (index, &[a, b]) in payload.seam_heights.iter().enumerate() {
                assert_eq!(
                    read(&rows, level.seam_capacity() + level.edge_capacity() + index),
                    [a, b, 0.0, 0.0].map(f32::to_bits)
                );
            }
        }
    }
}

// The wall instances' bytes (`TERRAIN-07-D` round 4): every face's eight values in order,
// bitwise, on every fixture and level; the conflict fixture has faces to check.
#[test]
fn packed_walls_hold_the_faces_bitwise() {
    let mut faces = 0;
    for (name, graph, terrain) in fixtures() {
        let roadbed = compiled(&graph);
        for level in LEVELS {
            for payload in bake_level(&terrain, &roadbed, &window_tiles(), level) {
                let mut bytes = Vec::new();
                pack_walls(&payload, &mut bytes);
                assert_eq!(bytes.len(), payload.walls.len() * WALL_FACE_BYTES, "{name}");
                let packed = bytes.as_chunks::<WALL_FACE_BYTES>().0;
                for (face, packed) in payload.walls.iter().zip(packed) {
                    let read: Vec<u32> = packed
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|value| u32::from_le_bytes(*value))
                        .collect();
                    let want = [
                        face.a[0], face.a[1], face.b[0], face.b[1], face.bottom[0],
                        face.bottom[1], face.top[0], face.top[1],
                    ]
                    .map(f32::to_bits);
                    assert_eq!(read, want, "{name} {level:?} {:?}", payload.tile);
                }
                faces += payload.walls.len();
            }
        }
    }
    assert!(faces > 0, "some fixture walls its lower road");
}
