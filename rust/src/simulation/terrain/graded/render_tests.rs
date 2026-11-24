// SPDX-License-Identifier: GPL-3.0-or-later

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

fn code_spacing_hm(code: i32, cell_hm: i32) -> i32 {
    if code < 3 {
        1 << code
    } else {
        cell_hm << (code - 3)
    }
}

// The world rules put every code's lattice in phase with the tile (a line at every multiple of
// its spacing from the tile's origin), so one index rule serves every code (grid alignment A3).
fn vertex_hm(code: i32, k: i32, spacing: i32) -> i32 {
    if code < 3 {
        k * spacing
    } else {
        (spacing * (k - 1)).clamp(0, SIDE_HM)
    }
}

fn snap_hm(p: i32, spacing: i32) -> i32 {
    p / spacing * spacing
}

fn index_at(p: i32, spacing: i32) -> i32 {
    1 + p / spacing
}

fn index_hm(idx: i32, spacing: i32) -> i32 {
    (idx - 1) * spacing
}

// One cell's lattice frame, as the shader's `CellFrame`.
#[derive(Clone, Copy, Debug)]
struct Frame {
    code: i32,
    spacing: i32,
}

fn frame(code: i32, cell_hm: i32) -> Frame {
    Frame {
        code,
        spacing: code_spacing_hm(code, cell_hm),
    }
}

// Side codes west, east, south, north; the shader's `cell_vertex_position_hm`.
fn vertex_position_hm(frame: Frame, k: [i32; 2], sides: [i32; 4], cell_hm: i32) -> [i32; 2] {
    let mut p = [0, 1].map(|axis| vertex_hm(frame.code, k[axis], frame.spacing));
    if p[0] == 0 || p[0] == SIDE_HM {
        let side = if p[0] == 0 { sides[0] } else { sides[1] };
        p[1] = snap_hm(p[1], code_spacing_hm(side, cell_hm));
    }
    if p[1] == 0 || p[1] == SIDE_HM {
        let side = if p[1] == 0 { sides[2] } else { sides[3] };
        p[0] = snap_hm(p[0], code_spacing_hm(side, cell_hm));
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
fn axis_stop(frame: Frame, x: f32) -> (i32, f32) {
    let p = ((x * 2.0).floor() as i32).clamp(0, SIDE_HM - 1);
    let index = index_at(p, frame.spacing);
    let p0 = index_hm(index, frame.spacing) as f32 * 0.5;
    let p1 = index_hm(index + 1, frame.spacing) as f32 * 0.5;
    (index, ((x - p0) / (p1 - p0).max(1e-6)).clamp(0.0, 1.0))
}

// The first atlas layer of a tested tile's row run, as Godot writes it into the header: not 0,
// so the port exercises the run's offset.
const ATLAS_FIRST: usize = 3;

fn decode(bytes: &[u8]) -> Vec<[f32; 4]> {
    bytes
        .as_chunks::<16>()
        .0
        .iter()
        .map(|texel| {
            std::array::from_fn(|c| f32::from_le_bytes(texel[c * 4..c * 4 + 4].try_into().unwrap()))
        })
        .collect()
}

// A baked tile as a page holds it (docs/terrain.md "Pages and the row atlas"), read back from the
// bytes `pack_slot` packs: its slot layer, the header's first atlas layer written as Godot writes
// it, and its row run placed at that layer of an atlas.
struct Slot {
    side: usize,
    texels: Vec<[f32; 4]>,
    atlas: Vec<[f32; 4]>,
}

impl Slot {
    fn packed(payload: &TilePayload, cell_m: f64) -> Self {
        let side = slot_side(payload.level, cell_m);
        let (mut texels, mut rows) = (Vec::new(), Vec::new());
        pack_slot(payload, side, &mut texels, &mut rows);
        let mut texels = decode(&texels);
        texels[side * side][0] = ATLAS_FIRST as f32;
        let mut atlas = vec![[0.0; 4]; ATLAS_FIRST * ATLAS_WIDTH];
        atlas.extend(decode(&rows));
        Self {
            side,
            texels,
            atlas,
        }
    }

    // The shader's `cell_texel`.
    fn texel(&self, i: i32, j: i32) -> [f32; 4] {
        self.texels[j as usize * self.side + i as usize]
    }

    // The header texel: (first atlas layer, edges, heights, lists).
    fn header(&self) -> [f32; 4] {
        self.texels[self.side * self.side]
    }

    // The shader's `cell_row`: row `i` of the tile's run, from the header's first layer.
    fn row(&self, i: i32) -> [f32; 4] {
        let layer = self.header()[0] as i32 + (i >> 8);
        self.atlas[layer as usize * ATLAS_WIDTH + (i & 255) as usize]
    }
}

// The shader's `cell_seam_list` and `cell_seam_row`.
fn seam_list(channel: f32) -> i32 {
    (channel as i32 + 1) >> 11
}

fn seam_row(channel: f32) -> i32 {
    channel as i32 - (seam_list(channel) << 11)
}

// The signed distance the shader's `cell_mask` tests against zero, from the packed slot.
fn mask_sdf(frame: Frame, slot: &Slot, p: [f32; 2]) -> f32 {
    let (ix, fx) = axis_stop(frame, p[0]);
    let (iz, fz) = axis_stop(frame, p[1]);
    let t: [[f32; 4]; 4] =
        std::array::from_fn(|c| slot.texel(ix + (c & 1) as i32, iz + (c >> 1) as i32));
    let mut sdf = mix(mix(t[0][1], t[1][1], fx), mix(t[2][1], t[3][1], fx), fz);
    let spacing_m = frame.spacing as f32 * 0.5;
    if sdf.abs() < 2.0 * spacing_m {
        let mut best = NO_HIT;
        for texel in &t {
            let index = seam_row(texel[2]);
            if index < 0 {
                continue;
            }
            let hit = segment_at(slot.row(index), p, index);
            if nearer(&hit, &best) {
                best = hit;
            }
        }
        // The square's extra candidates (T48 (a)), by the same order: (count, row, row, row),
        // then four rows a texel, from the header's lists offset.
        let list = seam_list(t[0][2]);
        if list > 0 {
            let at = slot.header()[3] as i32 + list - 1;
            let count = slot.row(at)[0] as i32;
            for k in 1..=count {
                let index = slot.row(at + k / 4)[(k % 4) as usize] as i32;
                let hit = segment_at(slot.row(index), p, index);
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
        cases += assert_cell_lattices(name, &terrain, &window_tiles());
    }
    assert!(cases > 0);
}

// Every level and far step: the bake's lattice of each tile is the shader's (lattice point `idx`
// lies at the port's `index_hm(idx)` and a point in the tile maps back to its index), it fits the
// slot, and every mesh vertex lies on a lattice point. Returns the cases checked.
fn assert_cell_lattices(name: &str, terrain: &TerrainSystem, tiles: &[TileKey]) -> usize {
    let mut cases = 0;
    let cell = cell_hm(terrain);
    let (mut xs, mut zs) = (Vec::new(), Vec::new());
    for level in LEVELS
        .into_iter()
        .chain([2, 4, 8].map(|step| Level::Gl3 { step }))
    {
        let code = level_code(level);
        for &tile in tiles {
            lattice(terrain, tile, level, &mut xs, &mut zs);
            let frame = frame(code, cell);
            let [ox, oz] = tile.origin();
            assert!(
                xs.len() <= slot_side(level, f64::from(cell) * 0.5),
                "{name}: slot side"
            );
            for (axis, coords, origin) in [(0, &xs, ox), (1, &zs, oz)] {
                for (idx, &world) in coords.iter().enumerate() {
                    let hm = (world - origin) * 2.0;
                    assert_eq!(
                        f64::from(index_hm(idx as i32, frame.spacing)),
                        hm,
                        "{name} {level:?} {tile:?} axis {axis} index {idx}"
                    );
                    if (0.0..=128.0).contains(&hm) {
                        assert_eq!(
                            index_at(hm as i32, frame.spacing),
                            idx as i32,
                            "{name} {level:?} {tile:?} axis {axis} at {hm}"
                        );
                    }
                }
                // Every mesh vertex lies on a lattice point.
                for k in 0..mesh_vertices(code, cell) {
                    let p = vertex_hm(code, k, frame.spacing);
                    let idx = index_at(p, frame.spacing);
                    assert_eq!(index_hm(idx, frame.spacing), p);
                }
            }
            cases += 1;
        }
    }
    cases
}

// The legal 2 m and 4 m cells (every other fixture is 8 m), on a 640 m world: a side that is a
// multiple of 128 m but not of 1,024 m, so the patches from the west edge (-320 m) meet at
// x = 192 m and the last one is 128 m wide. A level road along z = 0 from x = 100 m to 300 m
// over ground rising 8% in z crosses that patch border into the partial last patch. Tiles on
// the west edge, on both sides of the border and in the last patch, at every level and far step
// (2, 4 and 8 cells): the port's index arithmetic holds on the bake's lattice, each bake samples
// that lattice, and the two tiles at the border share their border samples bitwise.
#[test]
fn two_and_four_metre_cells_index_and_bake_their_lattices() {
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::network::roadbed::fixtures::{connect, point};
    use crate::simulation::network::types::NodeType;
    let mut graph = RegionGraph::new();
    let a = graph.add_node(point(100.0, 0.0, false), NodeType::Junction);
    let b = graph.add_node(point(300.0, 0.0, false), NodeType::Junction);
    connect(&mut graph, a, b, 7.0, false);
    graph.rebuild_intersection_clips();
    let roadbed = compiled(&graph);
    let tiles: Vec<TileKey> = [-5, 2, 3, 4]
        .into_iter()
        .flat_map(|x| [-5, -1, 0, 4].map(|z| TileKey { x, z }))
        .collect();
    for cell_m in [2.0_f32, 4.0] {
        let name = format!("{cell_m} m cell");
        let config = WorldConfig::new(640.0, 640.0, 40.0, 10.0).with_terrain_resolution(cell_m);
        let mut terrain = TerrainSystem::from_world_config(&config);
        let samples = (640.0 / cell_m) as usize + 1;
        for z in 0..samples {
            for x in 0..samples {
                terrain.set_height(x, z, 0.08 * (z as f32 * cell_m - 320.0));
            }
        }
        let cell = cell_hm(&terrain);
        assert_eq!(cell, (cell_m * 2.0) as i32);
        assert!(assert_cell_lattices(&name, &terrain, &tiles) > 0);
        let (mut xs, mut zs) = (Vec::new(), Vec::new());
        let mut graded = 0;
        for level in LEVELS
            .into_iter()
            .chain([4, 8].map(|step| Level::Gl3 { step }))
        {
            let payloads = bake_level(&terrain, &roadbed, &tiles, level);
            let baked = |tile: TileKey| {
                payloads
                    .iter()
                    .find(|payload| payload.tile == tile)
                    .unwrap_or_else(|| panic!("{name} {level:?}: {tile:?} baked"))
            };
            for &tile in &tiles {
                lattice(&terrain, tile, level, &mut xs, &mut zs);
                assert_eq!(baked(tile).samples, [xs.len(), zs.len()], "{name} {level:?} {tile:?}");
            }
            let frame = frame(level_code(level), cell);
            for z in [-1, 0] {
                let (west, east) = (baked(TileKey { x: 2, z }), baked(TileKey { x: 3, z }));
                for p in (0..=SIDE_HM).step_by(frame.spacing as usize) {
                    let texel = |payload: &TilePayload, x: i32| {
                        let ix = index_at(x, frame.spacing) as usize;
                        let iz = index_at(p, frame.spacing) as usize;
                        payload.texels[iz * payload.samples[0] + ix][0]
                    };
                    let shared = texel(west, SIDE_HM);
                    assert_eq!(
                        shared.to_bits(),
                        texel(east, 0).to_bits(),
                        "{name} {level:?} z {z} at {p} hm: the border's delta"
                    );
                    graded += usize::from(shared != 0.0);
                }
            }
        }
        assert!(graded > 0, "{name}: the road grades the patch border");
    }
}

#[test]
fn stitched_borders_share_vertices_and_texels_bitwise() {
    // `t_slope`: graded on both sides of the shared side x = 0 between tiles (-1, z) and (0, z).
    let (_, graph, terrain) = fixtures().swap_remove(0);
    let roadbed = compiled(&graph);
    let cell = cell_hm(&terrain);
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
                let fine_frame = frame(fine_code, cell);
                let coarse_frame = frame(coarse_code, cell);
                // The finer west cell snaps its east side to the coarser east cell's lattice.
                let fine_sides = [fine_code, coarse_code, fine_code, fine_code];
                let coarse_sides = [coarse_code; 4];
                let border = |frame: Frame, sides: [i32; 4], x_side: i32| {
                    let mut points = Vec::new();
                    for kz in 0..mesh_vertices(frame.code, cell) {
                        for kx in 0..mesh_vertices(frame.code, cell) {
                            let p = vertex_position_hm(frame, [kx, kz], sides, cell);
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
                        let ix = index_at(x, frame.spacing) as usize;
                        let iz = index_at(p, frame.spacing) as usize;
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
        let cell = cell_hm(&terrain);
        let cell_m = f64::from(terrain.cell_size_m());
        for level in [Level::Gl0, Level::Gl1] {
            let payloads = bake_level(&terrain, &roadbed, &window_tiles(), level);
            let (mut tested, mut wrong) = (0, 0);
            let mut first = String::new();
            for payload in &payloads {
                let slot = Slot::packed(payload, cell_m);
                let frame = frame(level_code(level), cell);
                let [ox, oz] = payload.tile.origin();
                // A 0.37 m grid: off the lattice, through every kind of cell.
                for j in 0..173 {
                    for i in 0..173 {
                        let p = [i as f32 * 0.37 + 0.011, j as f32 * 0.37 + 0.017];
                        let (x, z) = (ox + f64::from(p[0]), oz + f64::from(p[1]));
                        let covered = covered_height_at(&roadbed, x, z, &mut pieces).is_some();
                        let sdf = mask_sdf(frame, &slot, p);
                        let discarded = sdf < 0.0;
                        tested += 1;
                        if covered != discarded && first.is_empty() {
                            let nearest = payload
                                .seams
                                .iter()
                                .enumerate()
                                .map(|(i, &s)| (segment_at(s, p, i as i32).signed_distance, i))
                                .min_by(|a, b| a.0.abs().total_cmp(&b.0.abs()));
                            let (ix, _) = axis_stop(frame, p[0]);
                            let (iz, _) = axis_stop(frame, p[1]);
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
    let cell = cell_hm(&terrain);
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
    let cell_m = f64::from(terrain.cell_size_m());
    let slots: Vec<Slot> = payloads.iter().map(|payload| Slot::packed(payload, cell_m)).collect();
    let mut pieces = Vec::new();
    let (mut wrong, mut tested, mut first) = (0, 0, None);
    for iz in 0..=nz {
        for ix in 0..=nx {
            let [x, z] = point(ix, iz);
            let tile = TileKey::at(x, z);
            let slot = &slots[tiles.binary_search(&tile).expect("baked tile")];
            let [ox, oz] = tile.origin();
            let local = [(x - ox) as f32, (z - oz) as f32];
            let frame = frame(level_code(level), cell);
            let covered = covered_height_at(&roadbed, x, z, &mut pieces).is_some();
            let sdf = mask_sdf(frame, slot, local);
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

// T49 (c): the conflict pair's tiles hold terrain above the road top along a seam (its conflict
// walls), so some are flagged for the behind-the-road variant; the flat T's terrain meets its
// seams at the road top, and none is. (On a slope a cut's lattice interpolation rises above the
// road top at convex vertices, so the sloped T's tiles there are flagged too.) `GL1`, the plan
// view's level.
#[test]
fn the_behind_flag_marks_the_tiles_whose_terrain_rises_above_a_seam() {
    for (name, graph, terrain) in fixtures() {
        if !["conflict_8", "t_flat"].contains(&name) {
            continue;
        }
        let roadbed = compiled(&graph);
        let payloads = bake_level(&terrain, &roadbed, &window_tiles(), Level::Gl1);
        let flagged = payloads.iter().filter(|payload| payload.behind).count();
        assert_eq!(
            flagged > 0,
            name == "conflict_8",
            "{name}: {flagged} of {} tiles flagged",
            payloads.len()
        );
    }
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
    let cell = cell_hm(&terrain);
    let tiles = [-1, 0].map(|x| [-1, 0].map(|z| TileKey { x, z })).concat();
    for level in [Level::Gl0, Level::Gl1] {
        let payloads = bake_level(&terrain, &roadbed, &tiles, level);
        let (mut tested, mut wrong) = (0, 0);
        let mut first = String::new();
        for payload in &payloads {
            let slot = Slot::packed(payload, f64::from(terrain.cell_size_m()));
            let frame = frame(level_code(level), cell);
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
                    let discarded = mask_sdf(frame, &slot, p) < 0.0;
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
        // A patch-like rectangle on tile lines, as every patch side is on a legal world.
        let rect = [-128.0, -128.0, 128.0, 128.0];
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
    let bits = |texel: [f32; 4]| texel.map(f32::to_bits);
    for level in LEVELS {
        let side = slot_side(level, cell_m);
        for payload in bake_level(&terrain, &roadbed, &window_tiles(), level) {
            let (mut texels, mut rows) = (Vec::new(), Vec::new());
            pack_slot(&payload, side, &mut texels, &mut rows);
            assert_eq!(texels.len(), side * (side + 1) * 16);
            let rows_needed = 2 * payload.seams.len() + payload.edges.len() + payload.extras.len();
            assert_eq!(rows.len(), rows_needed.div_ceil(ATLAS_WIDTH) * ATLAS_WIDTH * 16);
            let (texels, rows) = (decode(&texels), decode(&rows));
            let [width, depth] = payload.samples;
            for j in 0..depth {
                for i in 0..width {
                    let point = (j * width + i) as u32;
                    let mut texel = payload.texels[point as usize];
                    let list = payload.square_extras.iter().find(|&&(at, _)| at == point);
                    if let Some(&(_, first)) = list {
                        texel[2] = seam_channel(texel[2], first);
                    }
                    let packed = texels[j * side + i];
                    assert_eq!(bits(packed), bits(texel));
                    // The shader's decoding gives the row and the list back.
                    assert_eq!(seam_row(packed[2]), payload.texels[point as usize][2] as i32);
                    assert_eq!(
                        seam_list(packed[2]),
                        list.map_or(0, |&(_, first)| first as i32 + 1)
                    );
                }
            }
            // The header: no run layer yet (Godot writes it), then the offsets of the edges,
            // the heights and the lists in the run.
            let header = texels[side * side];
            let (seams, edges) = (payload.seams.len(), payload.edges.len());
            let at = [seams, seams + edges, 2 * seams + edges];
            assert_eq!(bits(header), bits([0.0, at[0] as f32, at[1] as f32, at[2] as f32]));
            for (index, &seam) in payload.seams.iter().enumerate() {
                assert_eq!(bits(rows[index]), bits(seam));
            }
            for (index, &edge) in payload.edges.iter().enumerate() {
                assert_eq!(bits(rows[at[0] + index]), bits(edge));
            }
            assert_eq!(payload.seam_heights.len(), seams);
            for (index, &[a, b]) in payload.seam_heights.iter().enumerate() {
                assert_eq!(bits(rows[at[1] + index]), bits([a, b, 0.0, 0.0]));
            }
            for (index, &texel) in payload.extras.iter().enumerate() {
                assert_eq!(bits(rows[at[2] + index]), bits(texel));
            }
            assert!(rows[at[2] + payload.extras.len()..].iter().all(|&t| t == [0.0; 4]));
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
