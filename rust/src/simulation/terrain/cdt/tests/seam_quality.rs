// SPDX-License-Identifier: GPL-2.0-only

//! Seam simplification and source-provenance quality tests.

use super::*;

#[test]
fn cdt_merges_subbudget_same_authority_seam_fragments_before_triangulation() {
    let source_a = test_span_boundary_source_range(
        78,
        TerrainCdtRoadBandKind::Sidewalk,
        5,
        15,
        16,
        10.0,
        10.004,
    );
    let source_b = test_span_boundary_source_range(
        78,
        TerrainCdtRoadBandKind::Sidewalk,
        5,
        16,
        17,
        10.004,
        12.0,
    );
    let source_c = test_span_boundary_source_range(
        78,
        TerrainCdtRoadBandKind::Sidewalk,
        5,
        17,
        18,
        12.0,
        14.0,
    );
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
    match sample.source {
        TerrainCdtRoadBoundarySource::SpanSupportBoundary {
            edge_idx,
            source_band_index,
            start_section_index,
            end_section_index,
            start_s_m,
            end_s_m,
            ..
        } => {
            assert_eq!(edge_idx, 78);
            assert_eq!(source_band_index, 5);
            assert_eq!(start_section_index, 15);
            assert_eq!(end_section_index, 17);
            assert_eq!(start_s_m, 10.0);
            assert_eq!(end_s_m, 12.0);
        }
        other => panic!("merged seam must preserve span authority, got {other:?}"),
    }
}
