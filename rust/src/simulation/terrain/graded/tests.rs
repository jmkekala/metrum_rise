// SPDX-License-Identifier: GPL-2.0-only

//! `TERRAIN-07-A` acceptance: exact pruning, batch, order and worker independence, shared
//! border and level samples, coverage against the roadbed's own query, an open chain's
//! locality, the straight-edge seam height, dirty tile selection, and the benchmarks.

use super::tile::{bake_tiles_with, lattice};
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
    prune: bool,
) -> Vec<TilePayload> {
    let mut out = Vec::new();
    bake_tiles_with(terrain, roadbed, tiles, level, prune, &mut out).expect("tiles bake");
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

#[test]
fn pruned_bake_equals_every_seam_in_any_batch_order_and_worker_count() {
    let tiles = window_tiles();
    let mut conflicts = 0;
    for (name, graph, terrain) in conflict_fixtures() {
        let roadbed = compiled(&graph);
        for level in [Level::Gl0, Level::Gl2] {
            let reference = bake(&terrain, &roadbed, &tiles, level, true);
            conflicts += reference.iter().map(|p| p.stats.conflicts).sum::<u32>();
            // Every seam within the cap at every sample: all 16 tiles at `GL2`, the two tiles
            // at the junction and between the conflicting roads at `GL0`.
            let full_tiles: &[TileKey] = match level {
                Level::Gl2 => &tiles,
                _ => &[TileKey { x: -1, z: -1 }, TileKey { x: 0, z: 0 }],
            };
            let full = bake(&terrain, &roadbed, full_tiles, level, false);
            for unpruned in &full {
                let pruned = reference
                    .iter()
                    .find(|p| p.tile == unpruned.tile)
                    .expect("baked");
                assert_same(pruned, unpruned, &format!("{name} {level:?} unpruned"));
            }
            for workers in [1, 4, 8] {
                let batch = on_workers(workers, || bake(&terrain, &roadbed, &tiles, level, true));
                for (pruned, reference) in batch.iter().zip(&reference) {
                    assert_same(
                        pruned,
                        reference,
                        &format!("{name} {level:?} {workers} workers"),
                    );
                }
            }
            let mut reversed = tiles.clone();
            reversed.reverse();
            let backwards = bake(&terrain, &roadbed, &reversed, level, true);
            for (pruned, reference) in backwards.iter().rev().zip(&reference) {
                assert_same(pruned, reference, &format!("{name} {level:?} reversed"));
            }
            for (index, &tile) in tiles.iter().enumerate() {
                let alone = bake(&terrain, &roadbed, &[tile], level, true);
                assert_same(
                    &alone[0],
                    &reference[index],
                    &format!("{name} {level:?} alone"),
                );
            }
        }
    }
    assert!(conflicts > 0, "the fixtures hold conflicting cones");
}

#[test]
fn borders_and_levels_share_their_samples_bitwise() {
    for (name, graph, terrain) in fixtures() {
        let roadbed = compiled(&graph);
        let tiles = window_tiles();
        let baked: Vec<Vec<TilePayload>> = LEVELS
            .iter()
            .map(|&level| bake(&terrain, &roadbed, &tiles, level, true))
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
            for payload in bake(&terrain, &roadbed, &window_tiles(), level, true) {
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

#[test]
fn a_missing_node_piece_uncovers_its_gap_and_flips_no_other_region() {
    let graph = t_junction(true);
    let terrain = terrain(|x, _| SLOPE * x);
    let mut roadbed = compiled(&graph);
    let tiles = window_tiles();
    let before = bake(&terrain, &roadbed, &tiles, Level::Gl0, true);
    let node = roadbed
        .piece_mesh(PieceId::Node(0))
        .expect("the T's node piece")
        .bounds_xz();
    roadbed.remove_piece_for_test(PieceId::Node(0));
    let after = bake(&terrain, &roadbed, &tiles, Level::Gl0, true);
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

#[test]
fn queries_answer_from_the_baked_lattice() {
    for (name, graph, terrain) in fixtures() {
        let roadbed = compiled(&graph);
        let surface = GradedSurface {
            terrain: &terrain,
            roadbed: &roadbed,
        };
        let mut scratch = QueryScratch::default();
        let tile = TileKey { x: -1, z: 0 };
        let payload = &bake(&terrain, &roadbed, &[tile], Level::Gl0, true)[0];
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
            let before = bake(&terrain, &edited, &tiles, level, true);
            edited.remove_piece_for_test(PieceId::Node(0));
            let changes = edited.take_terrain_changes();
            let mut dirty = Vec::new();
            dirty_tiles_for_road_changes(
                &terrain,
                &edited,
                &changes,
                level,
                &mut SelectScratch::default(),
                &mut dirty,
            );
            let after = bake(&terrain, &edited, &tiles, level, true);
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
                bake_tiles(&terrain, &roadbed, &tiles, level, &mut out).expect("tiles bake");
                if run >= 5 {
                    totals.push(start.elapsed().as_secs_f64() * 1e3);
                }
            }
            let mut per_tile = Vec::new();
            let mut one = Vec::new();
            for &tile in &tiles {
                let start = Instant::now();
                bake_tiles_with(&terrain, &roadbed, &[tile], level, true, &mut one).expect("bakes");
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
            f64::from(surface.mask_at(x, z, &mut scratch) as u8)
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
                &edited,
                &changes,
                Level::Gl0,
                &mut scratch,
                &mut dirty,
            );
            bake_tiles(&terrain, &edited, &dirty, Level::Gl0, &mut out).expect("bakes");
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

/// Whole-world graded tile census for `T2` and `T7` (diagnostics, never timed as a stage):
/// every tile within the cap of a grounded road seam baked at `level`, in batches of 64, as a
/// JSON record of the tiles, graded tiles, conflict samples, conflict samples within one
/// lattice spacing of a road seam, and the largest seam row and window.
pub(crate) fn graded_census(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    level: Level,
) -> serde_json::Value {
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
    let mut out = Vec::new();
    let (mut graded, mut conflicts, mut seam_conflicts) = (0usize, 0u64, 0u64);
    let (mut row_max, mut window_max, mut reach_max) = (0usize, 0usize, 0.0f64);
    let (mut candidates, mut samples) = (0u64, 0u64);
    let mut overflow = 0usize;
    let start = Instant::now();
    for batch in tiles.chunks(64) {
        if let Err(error) = bake_tiles(terrain, roadbed, batch, level, &mut out) {
            overflow += 1;
            eprintln!("graded census: {error:?}");
        }
        for payload in &out {
            graded += usize::from(payload.graded);
            conflicts += u64::from(payload.stats.conflicts);
            seam_conflicts += u64::from(payload.stats.seam_conflicts);
            row_max = row_max.max(payload.seams.len());
            window_max = window_max.max(payload.stats.window_seams);
            reach_max = reach_max.max(payload.stats.reach_max_m);
            candidates += payload.stats.candidates;
            samples += payload.texels.len() as u64;
        }
    }
    serde_json::json!({
        "level": format!("{level:?}"), "seams": segments.len(), "tiles": tiles.len(),
        "graded": graded, "conflicts": conflicts, "seam_conflicts": seam_conflicts,
        "row_seams_max": row_max, "window_seams_max": window_max, "reach_max_m": reach_max,
        "seams_per_sample": candidates as f64 / samples.max(1) as f64,
        "capacity_errors": overflow, "bake_s": start.elapsed().as_secs_f64(),
    })
}
