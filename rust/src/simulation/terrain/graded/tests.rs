// SPDX-License-Identifier: GPL-2.0-only

//! `TERRAIN-07-A` acceptance: exact pruning, batch, order and worker independence, shared
//! border and level samples, coverage against the roadbed's own query, an open chain's
//! locality, the straight-edge seam height, dirty tile selection, and the benchmarks.

use super::tile::{Evaluation, bake_tiles_with, lattice};
use super::*;
use crate::config::HEIGHT_SCALE;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::fixtures::{
    SLOPE, bend, connect, point, road_edge, t_junction,
};
use crate::simulation::network::roadbed::{PieceId, RoadbedSystem};
use crate::simulation::network::types::NodeType;
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::Vector3;
use std::time::Instant;

const LEVELS: [Level; 4] = [Level::Gl0, Level::Gl1, Level::Gl2, Level::Gl3 { step: 2 }];

fn compiled(graph: &RegionGraph) -> RoadbedSystem {
    let mut roadbed = RoadbedSystem::new(64.0);
    roadbed.compile_dirty(graph);
    assert!(roadbed.failed_nodes().is_empty(), "fixture nodes compile");
    roadbed
}

// A junction hub with arms at `arms` degrees, 60 m long.
fn hub(arms: &[f32], sloped: bool) -> RegionGraph {
    let mut graph = RegionGraph::new();
    let centre = graph.add_node(point(0.0, 0.0, sloped), NodeType::Junction);
    for (index, angle) in arms.iter().enumerate() {
        let (s, c) = angle.to_radians().sin_cos();
        let end = graph.add_node(point(60.0 * c, 60.0 * s, sloped), NodeType::Junction);
        if index == 0 {
            connect(&mut graph, end, centre, 7.0, sloped);
        } else {
            connect(&mut graph, centre, end, 7.0, sloped);
        }
    }
    graph.rebuild_intersection_clips();
    graph
}

// Two parallel straight roads 30 m apart and 12 m apart in height: their footprints are closer
// than 1:2 allows, so the cones conflict between them.
fn conflict() -> RegionGraph {
    let mut graph = RegionGraph::new();
    for (z, y) in [(0.0, 0.0), (30.0, 12.0)] {
        let a = graph.add_node(Vector3::new(-90.0, y, z), NodeType::Junction);
        let b = graph.add_node(Vector3::new(90.0, y, z), NodeType::Junction);
        let points = (0..=9)
            .map(|i| Vector3::new(-90.0 + 20.0 * i as f32, y, z))
            .collect();
        graph.add_edge(road_edge(a, b, points, 7.0));
    }
    graph.rebuild_intersection_clips();
    graph
}

// A 53 x 53 heightmap of 10 m cells centred on the origin, heights from `height(x, z)`.
fn terrain(height: impl Fn(f32, f32) -> f32) -> TerrainSystem {
    let mut terrain = TerrainSystem::with_chunking(53, 53, 10.0, 53, 0.0);
    for z in 0..53 {
        for x in 0..53 {
            let (wx, wz) = (x as f32 * 10.0 - 260.0, z as f32 * 10.0 - 260.0);
            terrain.set_height(x, z, height(wx, wz) / HEIGHT_SCALE);
        }
    }
    terrain
}

// The reference fixtures with their terrain: on the road plane, the conflict pair on an 8%
// slope across it, and a mound under a T.
fn fixtures() -> Vec<(&'static str, RegionGraph, TerrainSystem)> {
    let plane = |sloped: bool| terrain(move |x, _| if sloped { SLOPE * x } else { 0.0 });
    vec![
        ("t_slope", t_junction(true), plane(true)),
        ("t_flat", t_junction(false), plane(false)),
        (
            "four_way_slope",
            hub(&[180.0, 0.0, 90.0, 270.0], true),
            plane(true),
        ),
        ("y_25_slope", hub(&[180.0, 12.5, -12.5], true), plane(true)),
        ("bend_slope", bend(60.0, 60.0, true), plane(true)),
        ("conflict_8", conflict(), terrain(|_, z| 0.08 * z)),
        (
            "t_mound",
            t_junction(false),
            terrain(|x, z| 3.0 * (1.0 - ((x * x + (z - 30.0).powi(2)) / 900.0)).max(0.0)),
        ),
    ]
}

// The 16 tiles of the 256 m window around the origin.
fn window_tiles() -> Vec<TileKey> {
    let mut tiles = Vec::new();
    for x in -2..2 {
        for z in -2..2 {
            tiles.push(TileKey { x, z });
        }
    }
    tiles
}

fn bake(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    tiles: &[TileKey],
    level: Level,
    evaluation: Evaluation,
) -> Vec<TilePayload> {
    bake_with(terrain, roadbed, &NoSites, tiles, level, evaluation)
}

pub(super) fn bake_with(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    sites: &impl SiteSource,
    tiles: &[TileKey],
    level: Level,
    evaluation: Evaluation,
) -> Vec<TilePayload> {
    let mut out = Vec::new();
    bake_tiles_with(terrain, roadbed, sites, tiles, level, evaluation, &mut out)
        .expect("tiles bake");
    out
}

fn texel_bits(payload: &TilePayload) -> Vec<[u32; 4]> {
    payload
        .texels
        .iter()
        .map(|texel| texel.map(f32::to_bits))
        .collect()
}

fn assert_same(a: &TilePayload, b: &TilePayload, what: &str) {
    assert_eq!(a.tile, b.tile, "{what}");
    assert!(
        texel_bits(a) == texel_bits(b),
        "{what}: texels of {:?}",
        a.tile
    );
    let rows = |p: &TilePayload| -> Vec<[u32; 4]> {
        p.seams.iter().map(|r| r.map(f32::to_bits)).collect()
    };
    assert!(rows(a) == rows(b), "{what}: seam rows of {:?}", a.tile);
    let edges = |p: &TilePayload| -> Vec<[u32; 4]> {
        p.edges.iter().map(|r| r.map(f32::to_bits)).collect()
    };
    assert!(edges(a) == edges(b), "{what}: edge rows of {:?}", a.tile);
    let walls = |p: &TilePayload| -> Vec<[u32; 8]> {
        p.walls
            .iter()
            .map(|w| {
                [w.a, w.b, w.bottom, w.top]
                    .concat()
                    .try_into()
                    .map(|w: [f32; 8]| w.map(f32::to_bits))
                    .expect("eight words")
            })
            .collect()
    };
    assert!(walls(a) == walls(b), "{what}: walls of {:?}", a.tile);
    assert_eq!(a.inputs, b.inputs, "{what}: inputs of {:?}", a.tile);
}

fn on_workers<R: Send>(workers: usize, op: impl FnOnce() -> R + Send) -> R {
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .expect("test pool")
        .install(op)
}

// The fixtures whose cones conflict or whose terrain is not the road plane.
fn conflict_fixtures() -> Vec<(&'static str, RegionGraph, TerrainSystem)> {
    fixtures()
        .into_iter()
        .filter(|(name, ..)| ["t_slope", "conflict_8", "t_mound"].contains(name))
        .collect()
}

/// The exact pruning (`TERRAIN-07-A`, `T18` (a)) on `tiles` at `GL0`, `GL2` and `GL3` (step 2,
/// bins at its spacing): texels, rows and inputs of the pruned bake equal the evaluation over
/// every seam within the cap bitwise (every tile at `GL2` and `GL3`, `gl0_reference` at `GL0`),
/// and do not depend on the batch (all tiles,
/// or one at a time), the tile order or the worker count (1, 4 and 8). Returns the pruned bakes'
/// conflict samples and second-pass samples.
pub(crate) fn assert_exact_pruning(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    sites: &impl SiteSource,
    tiles: &[TileKey],
    gl0_reference: &[TileKey],
    name: &str,
) -> (u32, u32) {
    let (mut conflicts, mut second_pass) = (0, 0);
    for level in [Level::Gl0, Level::Gl2, Level::Gl3 { step: 2 }] {
        let reference = bake_with(terrain, roadbed, sites, tiles, level, Evaluation::Lanes);
        conflicts += reference.iter().map(|p| p.stats.conflicts).sum::<u32>();
        second_pass += reference
            .iter()
            .map(|p| p.stats.second_pass_samples)
            .sum::<u32>();
        let full_tiles = match level {
            Level::Gl0 => gl0_reference,
            _ => tiles,
        };
        assert_lanes_equal_scalar(terrain, roadbed, sites, tiles, level, &reference, name);
        let full = bake_with(terrain, roadbed, sites, full_tiles, level, Evaluation::Unpruned);
        for unpruned in &full {
            let pruned = reference
                .iter()
                .find(|p| p.tile == unpruned.tile)
                .expect("baked");
            assert_same(pruned, unpruned, &format!("{name} {level:?} unpruned"));
        }
        for workers in [1, 4, 8] {
            let batch = on_workers(workers, || {
                bake_with(terrain, roadbed, sites, tiles, level, Evaluation::Lanes)
            });
            for (pruned, reference) in batch.iter().zip(&reference) {
                assert_same(
                    pruned,
                    reference,
                    &format!("{name} {level:?} {workers} workers"),
                );
            }
        }
        let mut reversed = tiles.to_vec();
        reversed.reverse();
        let backwards = bake_with(terrain, roadbed, sites, &reversed, level, Evaluation::Lanes);
        for (pruned, reference) in backwards.iter().rev().zip(&reference) {
            assert_same(pruned, reference, &format!("{name} {level:?} reversed"));
        }
        for (index, &tile) in tiles.iter().enumerate() {
            let alone = bake_with(terrain, roadbed, sites, &[tile], level, Evaluation::Lanes);
            assert_same(
                &alone[0],
                &reference[index],
                &format!("{name} {level:?} alone"),
            );
        }
    }
    (conflicts, second_pass)
}

/// The lanes (`TERRAIN-07-A` round 3) against one point at a time: `lanes`, the bake of
/// `tiles` at `level`, equals the scalar pruned bake bitwise, its measurements included (each
/// lane stops where the scalar evaluation stops).
pub(crate) fn assert_lanes_equal_scalar(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    sites: &impl SiteSource,
    tiles: &[TileKey],
    level: Level,
    lanes: &[TilePayload],
    name: &str,
) {
    let scalar = bake_with(terrain, roadbed, sites, tiles, level, Evaluation::Scalar);
    for (lanes, scalar) in lanes.iter().zip(&scalar) {
        assert_same(lanes, scalar, &format!("{name} {level:?} lanes"));
        assert_eq!(lanes.stats, scalar.stats, "{name} {level:?} lanes stats");
    }
}

/// [`assert_lanes_equal_scalar`] on every tile within the cap of a grounded road seam at
/// `level`; returns how many.
pub(crate) fn assert_lanes_equal_scalar_on_road_tiles(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    level: Level,
    name: &str,
) -> usize {
    let (_, tiles) = road_tiles(roadbed);
    let lanes = bake(terrain, roadbed, &tiles, level, Evaluation::Lanes);
    assert_lanes_equal_scalar(terrain, roadbed, &NoSites, &tiles, level, &lanes, name);
    tiles.len()
}

#[test]
fn lanes_equal_one_point_at_a_time_on_every_fixture_and_level() {
    let tiles = window_tiles();
    for (name, graph, terrain) in fixtures() {
        let roadbed = compiled(&graph);
        for level in LEVELS {
            let lanes = bake(&terrain, &roadbed, &tiles, level, Evaluation::Lanes);
            assert_lanes_equal_scalar(&terrain, &roadbed, &NoSites, &tiles, level, &lanes, name);
        }
    }
}

#[test]
fn a_seam_in_lanes_equals_the_seam_at_each_point() {
    // Interior projections, both clamps, a point on the seam and a zero-length seam.
    let seams = [
        super::seams::Seam {
            a: [-3.25, 1.5, 2.0],
            b: [7.75, -0.375, -4.5],
            kind: SeamKind::Road,
        },
        super::seams::Seam {
            a: [0.1, 2.0, 0.3],
            b: [0.1, 2.0, 0.3],
            kind: SeamKind::Road,
        },
    ];
    for seam in &seams {
        for row in -40..40 {
            let z = f64::from(row) * 0.37 - 0.01;
            for start in -40..40 {
                let xs: [f64; 4] =
                    std::array::from_fn(|lane| f64::from(start * 4 + lane as i32) * 0.29);
                let (distance, height) = seam.at_lanes(&xs, z);
                for lane in 0..4 {
                    let at = seam.at(xs[lane], z);
                    assert_eq!(distance[lane].to_bits(), at.distance.to_bits());
                    assert_eq!(height[lane].to_bits(), at.height.to_bits());
                }
            }
        }
    }
}

#[test]
fn pruned_bake_equals_every_seam_in_any_batch_order_and_worker_count() {
    let tiles = window_tiles();
    let (mut conflicts, mut second_pass) = (0, 0);
    for (name, graph, terrain) in conflict_fixtures() {
        let roadbed = compiled(&graph);
        // Every seam within the cap at every sample: all 16 tiles at `GL2`, the two tiles at
        // the junction and between the conflicting roads at `GL0`.
        let gl0 = [TileKey { x: -1, z: -1 }, TileKey { x: 0, z: 0 }];
        let (c, s) = assert_exact_pruning(&terrain, &roadbed, &NoSites, &tiles, &gl0, name);
        conflicts += c;
        second_pass += s;
    }
    assert!(conflicts > 0, "the fixtures hold conflicting cones");
    assert!(
        second_pass > 0,
        "the fixtures hold cuts the second pass finishes"
    );
}

#[test]
fn borders_and_levels_share_their_samples_bitwise() {
    for (name, graph, terrain) in fixtures() {
        let roadbed = compiled(&graph);
        let tiles = window_tiles();
        let baked: Vec<Vec<TilePayload>> = LEVELS
            .iter()
            .map(|&level| bake(&terrain, &roadbed, &tiles, level, Evaluation::Lanes))
            .collect();
        // Every lattice point of every level and tile, by world position: equal delta, and
        // equal footprint distance once clamped to the narrowest band.
        let mut seen = std::collections::HashMap::<(u64, u64), [u32; 2]>::new();
        let mut shared = 0;
        for payloads in &baked {
            for payload in payloads {
                let level = payload.level;
                let (mut xs, mut zs) = (Vec::new(), Vec::new());
                lattice(&terrain, payload.tile, level, &mut xs, &mut zs);
                for (j, &z) in zs.iter().enumerate() {
                    for (i, &x) in xs.iter().enumerate() {
                        let texel = payload.texels[j * xs.len() + i];
                        let sdf = (texel[1] as f64).clamp(-2.0, 2.0) as f32;
                        let sdf = if texel[1].is_sign_negative() {
                            -sdf.abs()
                        } else {
                            sdf.abs()
                        };
                        let value = [texel[0].to_bits(), sdf.to_bits()];
                        let key = (x.to_bits(), z.to_bits());
                        if let Some(previous) = seen.insert(key, value) {
                            shared += 1;
                            assert_eq!(
                                previous, value,
                                "{name}: {level:?} at ({x}, {z}) differs from another tile or level"
                            );
                        }
                    }
                }
            }
        }
        assert!(
            shared > 10_000,
            "{name}: borders, aprons and levels overlap ({shared})"
        );
    }
}

#[test]
fn coverage_equals_the_roadbed_height_query_at_every_lattice_point() {
    for (name, graph, terrain) in fixtures() {
        let roadbed = compiled(&graph);
        for level in LEVELS {
            for payload in bake(
                &terrain,
                &roadbed,
                &window_tiles(),
                level,
                Evaluation::Lanes,
            ) {
                let (mut xs, mut zs) = (Vec::new(), Vec::new());
                lattice(&terrain, payload.tile, level, &mut xs, &mut zs);
                for (j, &z) in zs.iter().enumerate() {
                    for (i, &x) in xs.iter().enumerate() {
                        let covered = payload.texels[j * xs.len() + i][1].is_sign_negative();
                        assert_eq!(
                            covered,
                            roadbed.height_at(x as f32, z as f32).is_some(),
                            "{name} {level:?} ({x}, {z})"
                        );
                    }
                }
            }
        }
    }
}

// The raster tests only the ends of each row's held run (`ROAD-68-K23`), and still answers as
// the predicate at each point: every lattice point's road height, bit for bit, is the highest
// the per-point coverage query finds there, on every fixture and level.
#[test]
fn the_coverage_raster_equals_the_predicate_at_every_lattice_point() {
    let (mut xs, mut zs, mut pieces, mut road) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (name, graph, terrain) in fixtures() {
        let roadbed = compiled(&graph);
        for level in LEVELS {
            for tile in window_tiles() {
                lattice(&terrain, tile, level, &mut xs, &mut zs);
                coverage::rasterize(&roadbed, &xs, &zs, &mut pieces, &mut road, None);
                for (j, &z) in zs.iter().enumerate() {
                    for (i, &x) in xs.iter().enumerate() {
                        let point = coverage::covered_height_at(&roadbed, x, z, &mut pieces);
                        assert_eq!(
                            road[j * xs.len() + i].to_bits(),
                            point.unwrap_or(f32::NEG_INFINITY).to_bits(),
                            "{name} {level:?} ({x}, {z})"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn a_missing_node_piece_uncovers_its_gap_and_flips_no_other_region() {
    let graph = t_junction(true);
    let terrain = terrain(|x, _| SLOPE * x);
    let mut roadbed = compiled(&graph);
    let tiles = window_tiles();
    let before = bake(&terrain, &roadbed, &tiles, Level::Gl0, Evaluation::Lanes);
    let node = roadbed
        .piece_mesh(PieceId::Node(0))
        .expect("the T's node piece")
        .bounds_xz();
    roadbed.remove_piece_for_test(PieceId::Node(0));
    let after = bake(&terrain, &roadbed, &tiles, Level::Gl0, Evaluation::Lanes);
    // Two texels around the node piece: where its open chain ends lie.
    let near = [node[0] - 1.0, node[1] - 1.0, node[2] + 1.0, node[3] + 1.0];
    let mut uncovered = 0;
    for (old, new) in before.iter().zip(&after) {
        let origin = old.tile.origin();
        for (k, (a, b)) in old.texels.iter().zip(&new.texels).enumerate() {
            let (i, j) = (k % old.samples[0], k / old.samples[0]);
            let x = origin[0] + (i as f64 - 1.0) * 0.5;
            let z = origin[1] + (j as f64 - 1.0) * 0.5;
            let inside = x >= near[0] && x <= near[2] && z >= near[1] && z <= near[3];
            let (was, is) = (a[1].is_sign_negative(), b[1].is_sign_negative());
            if was != is {
                assert!(inside && was, "({x}, {z}) changed side outside the gap");
                uncovered += 1;
            }
        }
    }
    assert!(
        uncovered > 100,
        "the gap is uncovered ({uncovered} samples)"
    );
}

#[test]
fn gl0_triangles_pass_through_the_seam_height_across_straight_edges() {
    // A level road on an 8% cross slope: a cut on one side, a fill on the other.
    let mut graph = RegionGraph::new();
    let a = graph.add_node(Vector3::new(-90.0, 0.0, 0.3), NodeType::Junction);
    let b = graph.add_node(Vector3::new(90.0, 0.0, 0.3), NodeType::Junction);
    let points = (0..=9)
        .map(|i| Vector3::new(-90.0 + 20.0 * i as f32, 0.0, 0.3))
        .collect();
    graph.add_edge(road_edge(a, b, points, 7.0));
    graph.rebuild_intersection_clips();
    let roadbed = compiled(&graph);
    let terrain = terrain(|_, z| 0.08 * z);
    let surface = GradedSurface {
        terrain: &terrain,
        roadbed: &roadbed,
        sites: &NoSites,
    };
    let mut scratch = QueryScratch::default();
    let (mut pieces, mut segments) = (Vec::new(), Vec::new());
    roadbed.grounded_seams_in_bounds([-40.0, -20.0, 40.0, 20.0], &mut pieces, &mut segments);
    assert!(segments.len() >= 4);
    let mut worst: f64 = 0.0;
    for [start, end] in &segments {
        if start.x.min(end.x) < -40.0 || start.x.max(end.x) > 40.0 {
            continue;
        }
        for step in 1..8 {
            let t = step as f64 / 8.0;
            let p = *start + (*end - *start) * t;
            worst = worst.max((surface.ground_height_at(p.x, p.z, &mut scratch) - p.y).abs());
        }
    }
    assert!(worst < 1.0e-4, "graded ground misses the seam by {worst} m");
}

// TD7 (`T2` (c)): at every seam vertex the graded ground is at least the seam height, exactly
// the seam height outside a conflict, and above it (a wall) only in one.
#[test]
fn the_graded_ground_never_lies_below_a_seam() {
    use super::sample::evaluate;
    use super::seams::{SeamScratch, gather_road_seams};
    let mut walls = 0;
    for (name, graph, terrain) in fixtures() {
        let roadbed = compiled(&graph);
        let (mut scratch, mut seams) = (SeamScratch::default(), Vec::new());
        gather_road_seams(
            &roadbed,
            [-200.0, -200.0, 200.0, 200.0],
            &mut scratch,
            &mut seams,
        );
        let mut base = super::sample::BaseWindow::default();
        base.fill(&terrain, [-200.0, -200.0, 200.0, 200.0]);
        for seam in seams
            .iter()
            .filter(|s| s.a[0].abs() < 100.0 && s.a[2].abs() < 100.0)
        {
            let [x, h, z] = seam.a;
            let all = (0..seams.len() as u32).map(super::sample::Listed::unbounded);
            let value = evaluate(x, z, base.at(x, z), None, None, &seams, all, 2.0);
            assert!(
                value.height >= h,
                "{name}: ground {} below seam {h}",
                value.height
            );
            if value.conflict {
                walls += usize::from(value.height > h);
            } else {
                assert_eq!(value.height.to_bits(), h.to_bits(), "{name}: ({x}, {z})");
            }
        }
    }
    assert!(walls > 0, "the conflict fixture walls its lower road");
}

#[test]
fn queries_answer_from_the_baked_lattice() {
    for (name, graph, terrain) in fixtures() {
        let roadbed = compiled(&graph);
        let surface = GradedSurface {
            terrain: &terrain,
            roadbed: &roadbed,
            sites: &NoSites,
        };
        let mut scratch = QueryScratch::default();
        let tile = TileKey { x: -1, z: 0 };
        let payload = &bake(&terrain, &roadbed, &[tile], Level::Gl0, Evaluation::Lanes)[0];
        let origin = tile.origin();
        for j in (1..130).step_by(7) {
            for i in (1..130).step_by(5) {
                let (x, z) = (
                    origin[0] + (i - 1) as f64 * 0.5,
                    origin[1] + (j - 1) as f64 * 0.5,
                );
                let texel = payload.texels[j * payload.samples[0] + i];
                let base = surface.base_height_at(x, z, &mut scratch);
                let ground = surface.ground_height_at(x, z, &mut scratch);
                assert_eq!(
                    ground.to_bits(),
                    (base + f64::from(texel[0])).to_bits(),
                    "{name} ({x}, {z})"
                );
                let mask = surface.mask_at(x, z, &mut scratch);
                assert_eq!(
                    mask == GroundClass::Road,
                    texel[1].is_sign_negative(),
                    "{name} ({x}, {z})"
                );
                if mask != GroundClass::Road {
                    assert_eq!(
                        mask == GroundClass::Graded,
                        texel[0] != 0.0,
                        "{name} ({x}, {z})"
                    );
                }
                let normal = surface.normal_at(x, z, &mut scratch);
                assert!(
                    (normal[0].powi(2) + normal[1].powi(2) + normal[2].powi(2) - 1.0).abs() < 1e-9
                );
            }
        }
        // A ray straight down onto bare ground hits the base, onto a road the road.
        let far = surface.raycast_surface([-100.0, 50.0, -100.0], [0.0, -1.0, 0.0], &mut scratch);
        let base = surface.base_height_at(-100.0, -100.0, &mut scratch);
        assert!(
            (far.expect("hits the ground")[1] - base).abs() < 1e-6,
            "{name}"
        );
        if let Some(height) = roadbed.height_at(-30.0, 0.0) {
            let road = surface.raycast_surface([-30.0, 50.0, 0.0], [0.0, -1.0, 0.0], &mut scratch);
            let hit = road.expect("hits the road")[1];
            assert!((hit - f64::from(height)).abs() < 1e-3, "{name}");
        }
        let oblique = surface.raycast_base([-100.0, 40.0, -100.0], [1.0, -1.0, 0.5], &mut scratch);
        let [x, y, z] = oblique.expect("hits the base");
        assert!(
            (y - surface.base_height_at(x, z, &mut scratch)).abs() < 1e-6,
            "{name}"
        );
    }
}

#[test]
fn road_changes_select_every_tile_they_alter() {
    for (name, graph, terrain) in conflict_fixtures() {
        let roadbed = compiled(&graph);
        let tiles: Vec<TileKey> = (-3..3)
            .flat_map(|x| (-3..3).map(move |z| TileKey { x, z }))
            .collect();
        for level in [Level::Gl0, Level::Gl2] {
            let mut edited = roadbed.clone();
            drop(edited.take_terrain_changes());
            let before = bake(&terrain, &edited, &tiles, level, Evaluation::Lanes);
            edited.remove_piece_for_test(PieceId::Node(0));
            let changes = edited.take_terrain_changes();
            let mut dirty = Vec::new();
            dirty_tiles_for_road_changes(
                &terrain,
                &roadbed,
                &edited,
                &NoSites,
                &changes,
                level,
                &mut SelectScratch::default(),
                &mut dirty,
            );
            let after = bake(&terrain, &edited, &tiles, level, Evaluation::Lanes);
            let mut changed = 0;
            for (old, new) in before.iter().zip(&after) {
                if texel_bits(old) != texel_bits(new) {
                    changed += 1;
                    assert!(
                        dirty.contains(&old.tile),
                        "{name} {level:?}: {:?} changed",
                        old.tile
                    );
                }
            }
            assert!(changed > 0 && dirty.len() < tiles.len(), "{name} {level:?}");
        }
    }
}

// The reference fixtures' 16 tiles around the junction at every level on the shared pool,
// p50 and p95 of 50 warm runs, with the reach, candidates and row maxima (`T1`, `T7`), and one
// tile at a time on the calling thread. Set the workers with RAYON_NUM_THREADS.
#[test]
#[ignore = "controlled release timing: cargo test --release --lib benchmark_graded_tiles -- --ignored --nocapture"]
fn benchmark_graded_tiles() {
    let tiles = window_tiles();
    let workers = crate::thread_pools::install(rayon::current_num_threads);
    for (name, graph, terrain) in fixtures() {
        let roadbed = compiled(&graph);
        for level in LEVELS {
            let mut out = Vec::new();
            let mut totals = Vec::new();
            for run in 0..55 {
                let start = Instant::now();
                bake_tiles(&terrain, &roadbed, &NoSites, &tiles, level, &mut out)
                    .expect("tiles bake");
                if run >= 5 {
                    totals.push(start.elapsed().as_secs_f64() * 1e3);
                }
            }
            let mut per_tile = Vec::new();
            let mut one = Vec::new();
            for &tile in &tiles {
                let start = Instant::now();
                bake_tiles_with(
                    &terrain,
                    &roadbed,
                    &NoSites,
                    &[tile],
                    level,
                    Evaluation::Lanes,
                    &mut one,
                )
                .expect("bakes");
                if one[0].graded {
                    per_tile.push(start.elapsed().as_secs_f64() * 1e3);
                }
            }
            totals.sort_by(f64::total_cmp);
            per_tile.sort_by(f64::total_cmp);
            let samples: u64 = out.iter().map(|p| p.texels.len() as u64).sum();
            let candidates: u64 = out.iter().map(|p| p.stats.candidates).sum();
            let reach_seams: usize = out.iter().map(|p| p.stats.window_seams).sum();
            let reach_sum: f64 = out.iter().map(|p| p.stats.reach_sum_m).sum();
            eprintln!(
                "GRADED_TILES {}",
                serde_json::json!({
                    "fixture": name, "level": format!("{level:?}"), "workers": workers,
                    "tiles": tiles.len(), "graded": out.iter().filter(|p| p.graded).count(),
                    "total_ms_p50": totals[totals.len() / 2],
                    "total_ms_p95": totals[totals.len() * 95 / 100],
                    "tile_ms_p50": per_tile.get(per_tile.len() / 2).copied().unwrap_or(0.0),
                    "tile_ms_max": per_tile.last().copied().unwrap_or(0.0),
                    "window_seams_max": out.iter().map(|p| p.stats.window_seams).max(),
                    "reach_mean_m": reach_sum / reach_seams.max(1) as f64,
                    "reach_max_m": out.iter().map(|p| p.stats.reach_max_m).fold(0.0, f64::max),
                    "seams_per_sample": candidates as f64 / samples.max(1) as f64,
                    "second_pass_samples": out.iter().map(|p| p.stats.second_pass_samples).sum::<u32>(),
                    "row_seams_max": out.iter().map(|p| p.seams.len()).max(),
                    "conflicts": out.iter().map(|p| p.stats.conflicts).sum::<u32>(),
                    "seam_conflicts": out.iter().map(|p| p.stats.seam_conflicts).sum::<u32>(),
                })
            );
        }
    }
}

// Point queries near the T junction: ground, surface, mask and normal, per query on the
// calling thread, median of 5 runs over 2,000 points.
#[test]
#[ignore = "controlled release timing: cargo test --release --lib benchmark_graded_point_queries -- --ignored --nocapture"]
fn benchmark_graded_point_queries() {
    for (name, graph, terrain) in fixtures() {
        let roadbed = compiled(&graph);
        let surface = GradedSurface {
            terrain: &terrain,
            roadbed: &roadbed,
            sites: &NoSites,
        };
        let mut scratch = QueryScratch::default();
        let points: Vec<(f64, f64)> = (0..2_000)
            .map(|i| {
                let t = i as f64 * 0.618_033_988_75;
                (
                    -60.0 + (t.fract() * 120.0),
                    -20.0 + ((t * 7.3).fract() * 70.0),
                )
            })
            .collect();
        let mut record = serde_json::json!({ "fixture": name });
        let mut time = |label: &str, run: &mut dyn FnMut(f64, f64) -> f64| {
            let mut runs = Vec::new();
            let mut sum = 0.0;
            for _ in 0..5 {
                let start = Instant::now();
                for &(x, z) in &points {
                    sum += run(x, z);
                }
                runs.push(start.elapsed().as_secs_f64() * 1e6 / points.len() as f64);
            }
            runs.sort_by(f64::total_cmp);
            record[format!("{label}_us")] = runs[2].into();
            std::hint::black_box(sum);
        };
        time("ground", &mut |x, z| {
            surface.ground_height_at(x, z, &mut scratch)
        });
        time("surface", &mut |x, z| {
            surface.surface_height_at(x, z, &mut scratch)
        });
        time("mask", &mut |x, z| {
            f64::from(u8::from(surface.mask_at(x, z, &mut scratch) == GroundClass::Graded))
        });
        time("normal", &mut |x, z| {
            surface.normal_at(x, z, &mut scratch)[1]
        });
        time("base", &mut |x, z| {
            surface.base_height_at(x, z, &mut scratch)
        });
        eprintln!("GRADED_QUERIES {record}");
    }
}

// Locality: a fixed T junction at the origin with a growing grid of background streets from
// 400 m out. The edit (the T's node piece dropped and rebuilt) selects and bakes the same
// tiles to the same bits at every size, at a flat cost. Background setup is not timed.
#[test]
#[ignore = "controlled release timing: cargo test --release --lib benchmark_graded_tiles_locality -- --ignored --nocapture"]
fn benchmark_graded_tiles_locality() {
    let terrain = TerrainSystem::with_chunking(2_001, 2_001, 10.0, 64, 0.0);
    // The first size's dirty tiles and their texel bits.
    type Local = (Vec<TileKey>, Vec<Vec<[u32; 4]>>);
    let mut reference: Option<Local> = None;
    for blocks in [0usize, 8, 24, 48] {
        let mut graph = t_junction(false);
        // Background: a square street grid of `blocks` x `blocks` 96 m blocks, south-east of the
        // junction from 400 m.
        let mut ids = Vec::new();
        for j in 0..=blocks {
            for i in 0..=blocks {
                let (x, z) = (400.0 + 96.0 * i as f32, 400.0 + 96.0 * j as f32);
                ids.push(graph.add_node(point(x, z, false), NodeType::Junction));
            }
        }
        let side = blocks + 1;
        for j in 0..=blocks {
            for i in 0..=blocks {
                if i < blocks {
                    connect(
                        &mut graph,
                        ids[j * side + i],
                        ids[j * side + i + 1],
                        7.0,
                        false,
                    );
                }
                if j < blocks {
                    connect(
                        &mut graph,
                        ids[j * side + i],
                        ids[(j + 1) * side + i],
                        7.0,
                        false,
                    );
                }
            }
        }
        graph.rebuild_intersection_clips();
        let roadbed = compiled(&graph);
        let edges = graph.edge_count();
        let mut times = Vec::new();
        let (mut dirty, mut out) = (Vec::new(), Vec::new());
        let mut scratch = SelectScratch::default();
        for run in 0..12 {
            let mut edited = roadbed.clone();
            drop(edited.take_terrain_changes());
            edited.remove_piece_for_test(PieceId::Node(0));
            let changes = edited.take_terrain_changes();
            let start = Instant::now();
            dirty_tiles_for_road_changes(
                &terrain,
                &roadbed,
                &edited,
                &NoSites,
                &changes,
                Level::Gl0,
                &mut scratch,
                &mut dirty,
            );
            bake_tiles(&terrain, &edited, &NoSites, &dirty, Level::Gl0, &mut out).expect("bakes");
            if run >= 2 {
                times.push(start.elapsed().as_secs_f64() * 1e3);
            }
        }
        times.sort_by(f64::total_cmp);
        let bits: Vec<Vec<[u32; 4]>> = out.iter().map(texel_bits).collect();
        match &reference {
            None => reference = Some((dirty.clone(), bits)),
            Some((tiles, texels)) => {
                assert_eq!(&dirty, tiles, "the same tiles at {edges} edges");
                assert!(texels == &bits, "the same bits at {edges} edges");
            }
        }
        eprintln!(
            "GRADED_LOCALITY {}",
            serde_json::json!({
                "edges": edges, "pieces": roadbed.pieces_in_bounds([-1.0e7, -1.0e7, 1.0e7, 1.0e7]).count(), "tiles": dirty.len(),
                "workers": crate::thread_pools::install(rayon::current_num_threads),
                "stage_ms_p50": times[times.len() / 2], "stage_ms_max": times[times.len() - 1],
            })
        );
    }
}

// The `GL0` slot's material edge capacity proposed in terrain.md "Data" (`TERRAIN-07-C` builds
// the edges and confirms it).
const GL0_EDGE_CAPACITY: usize = 256;

/// The graded terrain stage of one road edit (`TERRAIN-07-A`; no commit runs it yet): dirty
/// tile selection from the edit's terrain changes between `before` and `after`, then the `GL0`
/// bake of those tiles on the shared pool. Median of 7 warm runs, as JSON with the tiles, their
/// measurements, the undo record's slots under T20 (a) and a digest of the texel bits (to
/// compare builds and backgrounds).
pub(crate) fn graded_stage(
    terrain: &TerrainSystem,
    before: &RoadbedSystem,
    after: &RoadbedSystem,
    sites: &impl SiteSource,
    changes: &crate::simulation::network::roadbed::TerrainChanges,
) -> serde_json::Value {
    let (mut tiles, mut payloads, mut scratch) = (Vec::new(), Vec::new(), SelectScratch::default());
    let mut runs = Vec::new();
    for _ in 0..7 {
        let begin = Instant::now();
        dirty_tiles_for_road_changes(
            terrain,
            before,
            after,
            sites,
            changes,
            Level::Gl0,
            &mut scratch,
            &mut tiles,
        );
        let selected = begin.elapsed().as_secs_f64() * 1e3;
        bake_tiles(terrain, after, sites, &tiles, Level::Gl0, &mut payloads)
            .expect("the edit's tiles bake");
        runs.push((begin.elapsed().as_secs_f64() * 1e3, selected));
    }
    runs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (stage_ms, select_ms) = runs[3];
    let changed_tiles = assert_selection_covers(terrain, before, after, sites, changes, &tiles);
    // T20 (a): an undo record keeps the tiles the edit replaces, one `GL0` slot (the texels with
    // apron and the rows at capacity, terrain.md "Data") per selected tile graded before it.
    let replaced = bake_with(terrain, before, sites, &tiles, Level::Gl0, Evaluation::Lanes);
    let replaced = replaced.iter().filter(|payload| payload.graded);
    let undo_bytes: usize = replaced
        .clone()
        .map(|payload| 16 * (payload.texels.len() + Level::Gl0.seam_capacity() + GL0_EDGE_CAPACITY))
        .sum();
    let bits: Vec<Vec<[u32; 4]>> = payloads.iter().map(texel_bits).collect();
    let stats = payloads.iter().map(|p| p.stats);
    let samples = payloads
        .iter()
        .map(|p| p.texels.len())
        .sum::<usize>()
        .max(1) as f64;
    serde_json::json!({
        "workers": crate::thread_pools::install(rayon::current_num_threads),
        "stage_ms": stage_ms, "select_ms": select_ms, "bake_ms": stage_ms - select_ms,
        "changed_segments": changes.segments.len(), "changed_pieces": changes.pieces.len(),
        "tiles": tiles.len(), "tile_keys": tiles.iter().map(|t| (t.x, t.z)).collect::<Vec<_>>(),
        "changed_tiles": changed_tiles,
        "graded": payloads.iter().filter(|p| p.graded).count(),
        "undo_tiles": replaced.count(), "undo_mb": undo_bytes as f64 / 1e6,
        "window_seams_max": stats.clone().map(|s| s.window_seams).max(),
        "reach_max_m": stats.clone().map(|s| s.reach_max_m).fold(0.0, f64::max),
        "seams_per_sample": stats.clone().map(|s| s.candidates).sum::<u64>() as f64 / samples,
        "listed_per_sample": stats.clone().map(|s| s.listed).sum::<u64>() as f64 / samples,
        "second_pass_share": stats.clone().map(|s| u64::from(s.second_pass_samples)).sum::<u64>() as f64 / samples,
        "row_seams_max": payloads.iter().map(|p| p.seams.len()).max(),
        "row_edges_max": payloads.iter().map(|p| p.edges.len()).max(),
        "wall_faces": payloads.iter().map(|p| p.walls.len()).sum::<usize>(),
        "conflicts": stats.clone().map(|s| s.conflicts).sum::<u32>(),
        "seam_conflicts": stats.map(|s| s.seam_conflicts).sum::<u32>(),
        "digest": format!("{:016x}", crate::debug::work::debug_hash(&bits)),
    })
}

/// Asserts that dirty tile selection covers an edit: every `GL0` tile within the cap and band of
/// the edit's changed segments and pieces whose texel bits differ between `before` and `after`
/// is in `selected`. Returns how many changed. Diagnostics, never timed.
fn assert_selection_covers(
    terrain: &TerrainSystem,
    before: &RoadbedSystem,
    after: &RoadbedSystem,
    sites: &impl SiteSource,
    changes: &crate::simulation::network::roadbed::TerrainChanges,
    selected: &[TileKey],
) -> usize {
    let mut area = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let boxes = changes.pieces.iter().copied().chain(
        changes
            .segments
            .iter()
            .map(|[a, b]| super::seams::road_seam(*a, *b).box_xz()),
    );
    for b in boxes {
        area = [
            area[0].min(b[0]),
            area[1].min(b[1]),
            area[2].max(b[2]),
            area[3].max(b[3]),
        ];
    }
    if area[0] > area[2] {
        return 0;
    }
    let grow = CAP_M + Level::Gl0.band_m(f64::from(terrain.cell_size_m())) + 1.0;
    let (first, last) = (
        TileKey::at(area[0] - grow, area[1] - grow),
        TileKey::at(area[2] + grow, area[3] + grow),
    );
    let tiles: Vec<TileKey> = (first.x..=last.x)
        .flat_map(|x| (first.z..=last.z).map(move |z| TileKey { x, z }))
        .collect();
    let old = bake_with(terrain, before, sites, &tiles, Level::Gl0, Evaluation::Lanes);
    let new = bake_with(terrain, after, sites, &tiles, Level::Gl0, Evaluation::Lanes);
    let mut changed = 0;
    for (old, new) in old.iter().zip(&new) {
        if texel_bits(old) != texel_bits(new) {
            changed += 1;
            assert!(
                selected.contains(&old.tile),
                "{:?} changed unselected",
                old.tile
            );
        }
    }
    changed
}

/// Whole-world graded tile census for `T2` and `T7` (diagnostics, never timed as a stage):
/// every tile within the cap of a grounded road seam baked at `level`, in batches of 64, as a
/// JSON record of the tiles, graded tiles, conflict samples, conflict samples within one
/// lattice spacing of a road seam, and the largest seam row and window.
pub(crate) fn graded_census(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    sites: &impl SiteSource,
    level: Level,
) -> serde_json::Value {
    let (seams, tiles) = road_tiles(roadbed);
    let mut out = Vec::new();
    let (mut graded, mut conflicts, mut seam_conflicts) = (0usize, 0u64, 0u64);
    let (mut row_max, mut window_max, mut reach_max) = (0usize, 0usize, 0.0f64);
    let (mut edge_max, mut wall_faces) = (0usize, 0usize);
    let (mut candidates, mut samples, mut reach_sum, mut window_sum) = (0u64, 0u64, 0.0, 0usize);
    let (mut second_samples, mut second_candidates, mut listed) = (0u64, 0u64, 0u64);
    // The tiles with the most second-pass samples, for the exactness test's Kuopio block.
    let mut busiest: Vec<(u32, TileKey)> = Vec::new();
    let mut overflow = 0usize;
    let start = Instant::now();
    for batch in tiles.chunks(64) {
        if let Err(error) = bake_tiles(terrain, roadbed, sites, batch, level, &mut out) {
            overflow += 1;
            eprintln!("graded census: {error:?}");
        }
        for payload in &out {
            let stats = &payload.stats;
            graded += usize::from(payload.graded);
            conflicts += u64::from(stats.conflicts);
            seam_conflicts += u64::from(stats.seam_conflicts);
            row_max = row_max.max(payload.seams.len());
            edge_max = edge_max.max(payload.edges.len());
            wall_faces += payload.walls.len();
            window_max = window_max.max(stats.window_seams);
            reach_max = reach_max.max(stats.reach_max_m);
            reach_sum += stats.reach_sum_m;
            window_sum += stats.window_seams;
            candidates += stats.candidates;
            listed += stats.listed;
            samples += payload.texels.len() as u64;
            second_samples += u64::from(stats.second_pass_samples);
            second_candidates += stats.second_pass_candidates;
            busiest.push((stats.second_pass_samples, payload.tile));
        }
    }
    let bake_s = start.elapsed().as_secs_f64();
    busiest.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    serde_json::json!({
        "level": format!("{level:?}"), "seams": seams, "tiles": tiles.len(),
        "graded": graded, "conflicts": conflicts, "seam_conflicts": seam_conflicts,
        "row_seams_max": row_max, "row_edges_max": edge_max, "wall_faces": wall_faces,
        "window_seams_max": window_max, "reach_max_m": reach_max,
        "reach_mean_m": reach_sum / window_sum.max(1) as f64,
        "seams_per_sample": candidates as f64 / samples.max(1) as f64,
        "listed_per_sample": listed as f64 / samples.max(1) as f64,
        "second_pass_samples": second_samples,
        "second_pass_share": second_samples as f64 / samples.max(1) as f64,
        "second_pass_seams_per_sample": second_candidates as f64 / second_samples.max(1) as f64,
        "busiest_tiles": busiest.iter().take(3).map(|(n, t)| (t.x, t.z, *n)).collect::<Vec<_>>(),
        "workers": crate::thread_pools::install(rayon::current_num_threads),
        "capacity_errors": overflow, "bake_s": bake_s,
        "ms_per_tile_per_worker": bake_s * 1e3 * crate::thread_pools::install(rayon::current_num_threads) as f64 / tiles.len().max(1) as f64,
    })
}

// The tiles within the cap of a grounded road seam, sorted.
fn road_tiles(roadbed: &RoadbedSystem) -> (usize, Vec<TileKey>) {
    let (mut pieces, mut segments) = (Vec::new(), Vec::new());
    roadbed.grounded_seams_in_bounds([-1.0e7, -1.0e7, 1.0e7, 1.0e7], &mut pieces, &mut segments);
    let mut tiles = Vec::new();
    for [a, b] in &segments {
        let grow = CAP_M + 1.0;
        let first = TileKey::at(a.x.min(b.x) - grow, a.z.min(b.z) - grow);
        let last = TileKey::at(a.x.max(b.x) + grow, a.z.max(b.z) + grow);
        for x in first.x..=last.x {
            for z in first.z..=last.z {
                tiles.push(TileKey { x, z });
            }
        }
    }
    tiles.sort_unstable();
    tiles.dedup();
    (segments.len(), tiles)
}

// One seam sample where the graded ground stands above the seam: where, the wall's height and
// length, and the seam's piece and the piece of the seam that sets `L` there.
struct WallSample {
    x: f64,
    z: f64,
    height: f64,
    length: f64,
    near: PieceId,
    other: PieceId,
    height_difference: f64,
    distance: f64,
}

// The wall samples of one tile: every road seam sampled every 0.5 m (each point counted in the
// tile holding it), with `L` over every seam within the cap there.
fn tile_walls(terrain: &TerrainSystem, roadbed: &RoadbedSystem, tile: TileKey) -> Vec<WallSample> {
    use super::seams::{SeamScratch, gather_road_seams};
    let band = Level::Gl0.band_m(f64::from(terrain.cell_size_m()));
    let origin = tile.origin();
    let rect = [origin[0], origin[1], origin[0] + TILE_M, origin[1] + TILE_M];
    let window = super::tile::window(rect, band);
    let (mut scratch, mut seams, mut ids) = (SeamScratch::default(), Vec::new(), Vec::new());
    gather_road_seams(roadbed, window, &mut scratch, &mut seams);
    let mut owner = std::collections::BTreeMap::new();
    roadbed.grounded_pieces_in_bounds(window, &mut ids);
    for &id in &ids {
        let mesh = roadbed.piece_mesh(id).expect("published");
        for chain in mesh.outer_chains() {
            for pair in chain.vertices().windows(2) {
                let [a, b] = [pair[0], pair[1]].map(|v| mesh.positions()[v as usize]);
                owner.insert([a.x, a.y, a.z, b.x, b.y, b.z].map(f64::to_bits), id);
            }
        }
    }
    let piece = |index: usize| owner[&seams[index].key()];
    let mut walls = Vec::new();
    for (index, seam) in seams.iter().enumerate() {
        let length = (seam.b[0] - seam.a[0]).hypot(seam.b[2] - seam.a[2]);
        let parts = (length / 0.5).ceil().max(1.0) as usize;
        for part in 0..parts {
            let t = (part as f64 + 0.5) / parts as f64;
            let (x, z) = (
                seam.a[0] + (seam.b[0] - seam.a[0]) * t,
                seam.a[2] + (seam.b[2] - seam.a[2]) * t,
            );
            if TileKey::at(x, z) != tile {
                continue;
            }
            let height = seam.at(x, z).height;
            let (mut lower, mut setter) = (f64::NEG_INFINITY, index);
            for (other, candidate) in seams.iter().enumerate() {
                let at = candidate.at(x, z);
                let bound = at.height - TIE_IN_SLOPE * at.distance;
                if at.distance <= CAP_M && bound > lower {
                    (lower, setter) = (bound, other);
                }
            }
            if lower > height {
                let at = seams[setter].at(x, z);
                walls.push(WallSample {
                    x,
                    z,
                    height: lower - height,
                    length: length / parts as f64,
                    near: piece(index),
                    other: piece(setter),
                    height_difference: at.height - height,
                    distance: at.distance,
                });
            }
        }
    }
    walls
}

/// Conflict walls (`T2` (c), the user's decision): the graded ground stands above a seam, by
/// `g - h = L - h`, wherever another seam's lower bound passes over it. As JSON: the seam
/// samples (every 0.5 m along every road seam) with a wall, the total wall length, a histogram
/// and the largest wall height, and the `sites` worst sites by wall height, grouped by the two
/// pieces (the walled seam's and the one setting `L`; a span is its edge): the worst sample's
/// world x/z, the seams' height difference and the distance between them there, the largest wall
/// at each piece's edge and the number of walled samples. Diagnostics, never timed.
pub(crate) fn wall_sites(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    sites: usize,
) -> serde_json::Value {
    use rayon::prelude::*;
    use std::collections::BTreeMap;
    let (seams, tiles) = road_tiles(roadbed);
    let walls: Vec<WallSample> = crate::thread_pools::install(|| {
        tiles
            .par_iter()
            .map(|&tile| tile_walls(terrain, roadbed, tile))
            .collect::<Vec<_>>()
    })
    .into_iter()
    .flatten()
    .collect();
    let bins = [0.01, 0.1, 0.5, 1.0, 2.0, 5.0, f64::INFINITY];
    let mut histogram = [0usize; 7];
    for wall in &walls {
        histogram[bins.iter().position(|&top| wall.height <= top).unwrap()] += 1;
    }
    // Per pair of pieces: walled samples, the worst sample, the largest wall at each piece.
    type Site = (usize, usize, BTreeMap<PieceId, f64>);
    let mut grouped: BTreeMap<(PieceId, PieceId), Site> = BTreeMap::new();
    for (index, wall) in walls.iter().enumerate() {
        let pair = (wall.near.min(wall.other), wall.near.max(wall.other));
        let site = grouped.entry(pair).or_insert((0, index, BTreeMap::new()));
        site.0 += 1;
        if wall.height > walls[site.1].height {
            site.1 = index;
        }
        let edge = site.2.entry(wall.near).or_insert(0.0);
        *edge = edge.max(wall.height);
    }
    let mut ranked: Vec<_> = grouped.into_iter().collect();
    ranked.sort_by(|a, b| walls[b.1.1].height.total_cmp(&walls[a.1.1].height));
    let rows: Vec<_> = ranked
        .iter()
        .take(sites)
        .map(|((a, b), (samples, worst, edges))| {
            let worst = &walls[*worst];
            serde_json::json!({
                "x": worst.x, "z": worst.z, "pieces": [format!("{a:?}"), format!("{b:?}")],
                "wall_m": worst.height, "walled_piece": format!("{:?}", worst.near),
                "height_difference_m": worst.height_difference, "distance_m": worst.distance,
                "edge_walls_m": edges
                    .iter()
                    .map(|(piece, height)| (format!("{piece:?}"), *height))
                    .collect::<BTreeMap<_, _>>(),
                "walled_samples": samples,
            })
        })
        .collect();
    serde_json::json!({
        "seams": seams, "tiles": tiles.len(), "walled_samples": walls.len(),
        "wall_length_m": walls.iter().map(|w| w.length).sum::<f64>(),
        "wall_max_m": walls.iter().map(|w| w.height).fold(0.0, f64::max),
        "histogram_upto_m": bins.iter().map(|b| if b.is_finite() { b.to_string() } else { "more".into() }).zip(histogram).collect::<Vec<_>>(),
        "sites": rows,
    })
}

/// Writes one wall site's graded `GL0` samples for renders as JSON to `path`: every lattice
/// point of the 64 m square centred on `(x, z)` with its base, delta and footprint distance, the
/// road seams meeting the square and the grounded top triangles meeting it.
pub(crate) fn dump_site(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    x: f64,
    z: f64,
    path: &std::path::Path,
) {
    let square = [x - 32.0, z - 32.0, x + 32.0, z + 32.0];
    let (first, last) = (
        TileKey::at(square[0], square[1]),
        TileKey::at(square[2], square[3]),
    );
    let tiles: Vec<TileKey> = (first.x..=last.x)
        .flat_map(|tx| (first.z..=last.z).map(move |tz| TileKey { x: tx, z: tz }))
        .collect();
    let mut out = Vec::new();
    bake_tiles(terrain, roadbed, &NoSites, &tiles, Level::Gl0, &mut out).expect("tiles bake");
    let surface = GradedSurface {
        terrain,
        roadbed,
        sites: &NoSites,
    };
    let mut scratch = QueryScratch::default();
    let mut samples = Vec::new();
    let (mut xs, mut zs) = (Vec::new(), Vec::new());
    for payload in &out {
        lattice(terrain, payload.tile, Level::Gl0, &mut xs, &mut zs);
        let origin = payload.tile.origin();
        for (j, &pz) in zs.iter().enumerate() {
            for (i, &px) in xs.iter().enumerate() {
                // Each point once: the tile's own lattice, not its apron or far sides.
                let own = px >= origin[0]
                    && px < origin[0] + TILE_M
                    && pz >= origin[1]
                    && pz < origin[1] + TILE_M;
                if own && px >= square[0] && px <= square[2] && pz >= square[1] && pz <= square[3] {
                    let texel = payload.texels[j * xs.len() + i];
                    samples.push([
                        px,
                        pz,
                        surface.base_height_at(px, pz, &mut scratch),
                        f64::from(texel[0]),
                        f64::from(texel[1]),
                    ]);
                }
            }
        }
    }
    let (mut pieces, mut segments) = (Vec::new(), Vec::new());
    roadbed.grounded_seams_in_bounds(square, &mut pieces, &mut segments);
    let seams: Vec<[f64; 6]> = segments
        .iter()
        .map(|[a, b]| [a.x, a.y, a.z, b.x, b.y, b.z])
        .collect();
    let mut triangles = Vec::new();
    roadbed.for_each_grounded_top_triangle_in_bounds(square, &mut pieces, |t| {
        triangles.push(t.map(|p| [p.x, p.y, p.z]));
    });
    triangles.sort_by(|a, b| {
        a.map(|p| p.map(f64::to_bits))
            .cmp(&b.map(|p| p.map(f64::to_bits)))
    });
    triangles.dedup();
    let json = serde_json::json!({
        "centre": [x, z], "spacing_m": 0.5,
        "samples_x_z_base_delta_sdf": samples, "seams_a_b": seams, "top_triangles": triangles,
    });
    std::fs::write(path, json.to_string()).expect("site dump written");
}
