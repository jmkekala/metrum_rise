// SPDX-License-Identifier: GPL-2.0-only

//! Seam simplification and source-provenance quality tests.

use super::*;
use std::collections::BTreeSet;

#[test]
fn cdt_stops_after_reaching_a_near_degenerate_three_vertex_seam_loop() {
    let source = test_span_boundary_source(77, 4);
    let points = vec![
        TerrainCdtVertex::new(1.0, 0.12, 1.0),
        TerrainCdtVertex::new(1.004, 0.12, 1.0),
        TerrainCdtVertex::new(1.008, 0.12, 1.0),
        TerrainCdtVertex::new(5.0, 0.12, 1.000_001),
    ];
    let source_edges = points
        .iter()
        .copied()
        .enumerate()
        .map(|(index, start)| TerrainCdtRoadLoopSourceEdge {
            start,
            end: points[(index + 1) % points.len()],
            source,
        })
        .collect::<Vec<_>>();

    let quality = harden_terrain_cdt_road_loop_seams(
        points.clone(),
        &source_edges,
        TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
    );

    assert_eq!(quality.points, vec![points[0], points[2], points[3]]);
    assert_eq!(quality.edge_sources.len(), 3);
    assert_eq!(quality.merged_subbudget_seam_edges, 1);
}

#[test]
fn cdt_merges_subbudget_same_authority_seam_fragments_before_triangulation() {
    let source_a = test_span_boundary_source_range(78, 5, 15, 16, 10.0, 10.004);
    let source_b = test_span_boundary_source_range(78, 5, 16, 17, 10.004, 12.0);
    let source_c = test_span_boundary_source_range(78, 5, 17, 18, 12.0, 14.0);
    let road = vec![
        TerrainCdtVertex::new(3.0, 0.12, 3.0),
        TerrainCdtVertex::new(5.0, 0.12, 3.0),
        TerrainCdtVertex::new(5.004, 0.12, 3.0),
        TerrainCdtVertex::new(7.0, 0.12, 3.0),
        TerrainCdtVertex::new(7.0, 0.12, 7.0),
        TerrainCdtVertex::new(3.0, 0.12, 7.0),
    ];
    let source_edges = vec![
        TerrainCdtRoadLoopSourceEdge {
            start: road[0],
            end: road[1],
            source: source_a,
        },
        TerrainCdtRoadLoopSourceEdge {
            start: road[1],
            end: road[2],
            source: source_b,
        },
        TerrainCdtRoadLoopSourceEdge {
            start: road[2],
            end: road[3],
            source: source_c,
        },
        TerrainCdtRoadLoopSourceEdge {
            start: road[3],
            end: road[4],
            source: source_c,
        },
        TerrainCdtRoadLoopSourceEdge {
            start: road[4],
            end: road[5],
            source: source_c,
        },
        TerrainCdtRoadLoopSourceEdge {
            start: road[5],
            end: road[0],
            source: source_c,
        },
    ];
    let input = TerrainCdtInput::new(
        TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
        vec![TerrainCdtRoadLoop::new_with_source_edges(
            78,
            0,
            road,
            source_edges,
        )],
        Vec::new(),
    );

    let mesh = build_road_touched_terrain_patch(input)
        .expect("source-compatible seam fragments should merge before Spade input");

    assert_eq!(mesh.stats.invalid_constraint_edges, 0);
    assert_eq!(mesh.stats.merged_subbudget_seam_edges, 1);
    assert_eq!(mesh.stats.blocking_degenerate_seam_edges, 0);
    assert_eq!(mesh.seam_quality_samples.len(), 1);
    let sample = mesh.seam_quality_samples[0];
    assert_eq!(
        sample.kind,
        TerrainCdtSeamQualityKind::MergedSubbudgetSeamEdge
    );
    assert!(sample.length_m > MIN_SOURCE_OWNED_SEAM_EDGE_LENGTH_M as f32);
    // Adjacent segments of one chain merge into the lower segment.
    assert_eq!(sample.source, source_a);
}

// A street on a tile line (`ROAD-68-D`): the road's west seam is the side `x = -5` shared by a
// tile holding the road and one beside it. Both tiles put exactly the seam's vertices on that
// side at the seam's heights, drop the side's terrain samples on the seam, and emit no sliver.
#[test]
fn a_seam_along_a_tile_side_is_that_side_in_both_tiles() {
    let seam_height = 0.12;
    let west = [250.0, 230.0, 210.0, 190.0, 170.0, 150.0];
    let mut points = vec![
        TerrainCdtVertex::new(-5.0, seam_height, 150.0),
        TerrainCdtVertex::new(5.0, seam_height, 150.0),
        TerrainCdtVertex::new(5.0, seam_height, 250.0),
    ];
    points.extend(
        west[..west.len() - 1]
            .iter()
            .map(|&z| TerrainCdtVertex::new(-5.0, seam_height, z)),
    );
    let source_edges = (0..points.len())
        .map(|index| TerrainCdtRoadLoopSourceEdge {
            start: points[index],
            end: points[(index + 1) % points.len()],
            source: test_span_boundary_source_range(3, 0, index as u32, 0, 0.0, 0.0),
        })
        .collect();
    let road = TerrainCdtRoadLoop::new_with_source_edges(3, 0, points, source_edges);
    let tile_input = |min_x: f64, max_x: f64| {
        let (min_z, max_z) = (192.0, 256.0);
        let mut samples = Vec::new();
        for step in 0..=32 {
            let z = min_z + f64::from(step) * 2.0;
            samples.push(TerrainCdtVertex::new(min_x, 0.0, z));
            samples.push(TerrainCdtVertex::new(max_x, 0.0, z));
        }
        for step in 0..=5 {
            let x = min_x + f64::from(step);
            samples.push(TerrainCdtVertex::new(x, 0.0, min_z));
            samples.push(TerrainCdtVertex::new(x, 0.0, max_z));
            for row in 1..16 {
                samples.push(TerrainCdtVertex::new(x, 0.0, min_z + f64::from(row) * 4.0));
            }
        }
        TerrainCdtInput::new(
            TerrainCdtPatch::new(min_x, min_z, max_x, max_z, [0.0; 4]),
            vec![road.clone()],
            samples,
        )
    };
    let mut sides = Vec::new();
    for (min_x, max_x) in [(-5.0, 0.0), (-10.0, -5.0)] {
        let mesh = build_road_touched_terrain_patch(tile_input(min_x, max_x))
            .unwrap_or_else(|error| panic!("tile x {min_x}..{max_x}: {error:?}"));
        assert!(
            !mesh.triangles.is_empty(),
            "tile x {min_x}..{max_x} has terrain"
        );
        let mut side = BTreeSet::new();
        for triangle in &mesh.triangles {
            let [a, b, c] = triangle.map(|index| mesh.vertices[index]);
            let twice_area = ((b.x - a.x) * (c.z - a.z) - (c.x - a.x) * (b.z - a.z)).abs();
            assert!(
                twice_area > 1.0e-6,
                "tile x {min_x}..{max_x}: sliver {a:?} {b:?} {c:?}"
            );
            let center = centroid([a, b, c]);
            assert!(
                !(center.x > -5.0 && center.x < 5.0 && center.z > 150.0 && center.z < 250.0),
                "tile x {min_x}..{max_x}: terrain face at {center:?} on the road"
            );
            for vertex in [a, b, c] {
                if vertex.x == -5.0 {
                    if vertex.z <= 250.0 {
                        assert_eq!(vertex.height_m, seam_height, "seam height at {vertex:?}");
                    }
                    side.insert((vertex.z.to_bits(), vertex.height_m.to_bits()));
                }
            }
        }
        sides.push(side);
    }
    // The road tile has no terrain on its seam side below the seam's end; the other tile's side
    // vertices there are exactly the seam's.
    let seam_side = |side: &BTreeSet<(u64, u32)>| {
        side.iter()
            .filter(|&&(z, _)| f64::from_bits(z) <= 250.0)
            .map(|&(z, _)| f64::from_bits(z))
            .collect::<Vec<_>>()
    };
    assert_eq!(seam_side(&sides[1]), vec![192.0, 210.0, 230.0, 250.0]);
    assert!(seam_side(&sides[0]).iter().all(|&z| z == 250.0));
}
