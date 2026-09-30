// SPDX-License-Identifier: GPL-2.0-only

//! Matched JunctionN compile timings in a dedicated pool, with a cross-process product digest.

use super::*;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::hint::black_box;
use std::time::Instant;

const SAMPLES: usize = 21;

struct CompileBenchCase {
    name: &'static str,
    angles_degrees: &'static [f32],
    widths_m: &'static [f32],
    sloped: bool,
}

const CASES: [CompileBenchCase; 6] = [
    CompileBenchCase {
        name: "flat_t",
        angles_degrees: &[0.0, 90.0, 180.0],
        widths_m: &[7.0, 7.0, 7.0],
        sloped: false,
    },
    CompileBenchCase {
        name: "sloped_t",
        angles_degrees: &[0.0, 90.0, 180.0],
        widths_m: &[7.0, 7.0, 7.0],
        sloped: true,
    },
    CompileBenchCase {
        name: "flat_cross",
        angles_degrees: &[0.0, 90.0, 180.0, 270.0],
        widths_m: &[7.0, 7.0, 7.0, 7.0],
        sloped: false,
    },
    CompileBenchCase {
        name: "sloped_cross",
        angles_degrees: &[0.0, 90.0, 180.0, 270.0],
        widths_m: &[7.0, 7.0, 7.0, 7.0],
        sloped: true,
    },
    CompileBenchCase {
        name: "sloped_oblique",
        angles_degrees: &[0.0, 73.0, 180.0, 244.0],
        widths_m: &[7.0, 10.5, 5.5, 8.75],
        sloped: true,
    },
    CompileBenchCase {
        name: "sloped_five",
        angles_degrees: &[0.0, 65.0, 145.0, 230.0, 310.0],
        widths_m: &[7.0, 7.0, 7.0, 7.0, 7.0],
        sloped: true,
    },
];

#[test]
#[ignore = "matched release JunctionN compile timing; setup and digests are not timed"]
fn junction_compile_constant_factors() {
    let flat = planar_world_terrain(1601, 1601, 1.0, 150.0, 0.0, 0.0);
    let sloped = planar_world_terrain(1601, 1601, 1.0, 150.0, 0.035, -0.014);
    let mut total_ms = 0.0;
    // Compile inside a dedicated pool, like the preview worker, so parallel stages start on a pool
    // thread. RAYON_NUM_THREADS sizes it; 1 gives the single-thread timing. The test-only
    // full-scan check would dominate timings, so every pool thread turns it off.
    let pool = rayon::ThreadPoolBuilder::new()
        .start_handler(|_| {
            crate::simulation::network::surface::node::set_explicit_step_reference_check(false)
        })
        .build()
        .expect("benchmark thread pool must start");
    for case in &CASES {
        let terrain = if case.sloped { &sloped } else { &flat };
        let (graph, center) = generated_multiway_junction_graph_with_edge_widths(
            96.0,
            case.angles_degrees,
            case.widths_m,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
            GeneratedEndpointProfileMode::SolveJunctionEndpointProfiles,
            |xz| {
                Vector3::new(
                    xz.x,
                    terrain.sample_height_world(xz.x, xz.y) * crate::config::HEIGHT_SCALE,
                    xz.y,
                )
            },
            |start_xz, end_xz| grounded_polyline_points_from_terrain(terrain, start_xz, end_xz, 24),
        );
        let mut surface = RoadSurfaceSystem::new(16.0);
        for edge_idx in 0..graph.edge_count() {
            surface.compiled_sections.insert(
                edge_idx,
                std::sync::Arc::new(surface.compile_edge_sections(&graph, edge_idx)),
            );
        }
        for edge_idx in 0..graph.edge_count() {
            let span_piece = surface.compile_visual_span_piece(&graph, terrain, edge_idx);
            surface.apply_span_compile_result(edge_idx, span_piece);
        }
        let input = surface
            .visual_node_compile_input(&graph, center)
            .expect("benchmark fixture must produce a JunctionN input");

        // Debug formatting covers every product field, so the digest changes if any value or
        // order does. DefaultHasher::new() uses fixed keys and is stable across processes.
        let digest = |piece: &RoadSurfaceVisualNodePiece| {
            let mut hash = DefaultHasher::new();
            format!("{piece:?}").hash(&mut hash);
            hash.finish()
        };
        let reference = pool
            .install(|| {
                surface.compile_visual_node_piece_from_input(&graph, terrain, center, &input)
            })
            .unwrap_or_else(|| panic!("{} must compile", case.name));
        let products = digest(&reference);
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let start = Instant::now();
            let piece = pool.install(|| {
                surface.compile_visual_node_piece_from_input(
                    black_box(&graph),
                    black_box(terrain),
                    center,
                    black_box(&input),
                )
            });
            samples.push(start.elapsed().as_secs_f64() * 1e3);
            let piece = piece.expect("benchmark fixture must keep compiling");
            assert_eq!(
                digest(&piece),
                products,
                "{} products differ between compiles",
                case.name
            );
        }
        samples.sort_by(f64::total_cmp);
        total_ms += samples[SAMPLES / 2];
        eprintln!(
            "JUNCTION_COMPILE_BENCH {}",
            serde_json::json!({
                "case": case.name, "samples": SAMPLES, "min_ms": samples[0],
                "median_ms": samples[SAMPLES / 2], "p90_ms": samples[SAMPLES * 9 / 10],
                "products": format!("{products:016x}"),
            })
        );
    }
    eprintln!(
        "JUNCTION_COMPILE_BENCH {}",
        serde_json::json!({ "case": "total", "median_ms": total_ms })
    );
}
