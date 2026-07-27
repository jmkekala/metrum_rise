//! Constraint noding and road-owned face rejection tests.

use super::*;

#[test]
fn crossing_road_constraints_are_noded_before_triangulation() {
    let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]);
    let road_a = road_loop_from_centerline(
        TerrainCdtVertex::new(4.0, 0.0, 20.0),
        TerrainCdtVertex::new(36.0, 0.0, 20.0),
        5.0,
    );
    let road_b = road_loop_from_centerline(
        TerrainCdtVertex::new(20.0, 0.0, 4.0),
        TerrainCdtVertex::new(20.0, 0.0, 36.0),
        5.0,
    );

    let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
        patch,
        vec![
            TerrainCdtRoadLoop::new(21, 0, road_a),
            TerrainCdtRoadLoop::new(22, 0, road_b),
        ],
        piece_source_samples(),
    ))
    .expect("crossing road loops must not panic the terrain bridge");

    assert_eq!(
        mesh.stats.invalid_constraint_edges, 0,
        "road constraints must be split at deterministic intersections before Spade sees them"
    );
    assert!(
        mesh.stats.road_constraint_edges > 8,
        "crossing road loops should gain noded roadbed constraints"
    );
    for vertex in &mesh.vertices {
        assert!(patch_contains(*vertex, patch));
    }
}

#[test]
fn unpreserved_constraint_filter_catches_centroid_outside_overlap() {
    let road_loop = canonical_square_road_loop(4.0, 6.0);
    let points = [
        TerrainCdtVertex::new(0.0, 0.0, 4.5),
        TerrainCdtVertex::new(10.0, 0.0, 4.5),
        TerrainCdtVertex::new(0.0, 0.0, 0.0),
    ];

    assert!(!point_inside_any_road_footprint(
        centroid(points),
        std::slice::from_ref(&road_loop)
    ));
    assert!(terrain_triangle_is_road_owned(
        [0, 1, 2],
        points,
        &BTreeMap::new(),
        std::slice::from_ref(&road_loop),
    ));
    assert!(triangle_crosses_road_constraint(
        points,
        road_loop.vertices[0],
        road_loop.vertices[3],
    ));
}

#[test]
fn road_owned_triangle_rejection_keeps_exterior_seam_without_source_edge() {
    let road_loop = canonical_square_road_loop(4.0, 6.0);
    let points = [
        TerrainCdtVertex::new(4.0, 0.0, 4.0),
        TerrainCdtVertex::new(6.0, 0.0, 4.0),
        TerrainCdtVertex::new(4.0, 0.0, 0.0),
    ];

    assert!(!terrain_triangle_is_road_owned(
        [0, 1, 2],
        points,
        &BTreeMap::new(),
        &[road_loop],
    ));
}

#[test]
fn road_owned_triangle_rejection_catches_boundary_chord_with_centroid_outside() {
    let road_loop = canonical_square_road_loop(4.0, 6.0);
    let points = [
        TerrainCdtVertex::new(4.0, 0.0, 4.0),
        TerrainCdtVertex::new(6.0, 0.0, 6.0),
        TerrainCdtVertex::new(2.0, 0.0, 6.0),
    ];
    let road_loops = [road_loop.clone()];

    assert!(!point_strictly_inside_any_road_footprint(
        centroid(points),
        &road_loops,
    ));
    assert!(terrain_triangle_is_road_owned(
        [0, 1, 2],
        points,
        &BTreeMap::new(),
        &road_loops,
    ));
}

#[test]
fn source_sample_on_road_seam_splits_the_road_constraint() {
    let patch = TerrainCdtPatch::new(0.0, 0.0, 20.0, 20.0, [0.0; 4]);
    let road = vec![
        TerrainCdtVertex::new(4.0, 1.0, 4.0),
        TerrainCdtVertex::new(16.0, 1.0, 4.0),
        TerrainCdtVertex::new(16.0, 1.0, 10.0),
        TerrainCdtVertex::new(4.0, 1.0, 10.0),
    ];

    let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
        patch,
        vec![TerrainCdtRoadLoop::new(41, 0, road)],
        vec![TerrainCdtVertex::new(16.0, 1.0, 7.0)],
    ))
    .expect("terrain source samples on a road seam must not invalidate the CDT");

    assert_eq!(mesh.stats.invalid_constraint_edges, 0);
    assert!(
        mesh.stats.road_constraint_edges > 4,
        "the road seam constraint must be split at the existing source sample vertex"
    );
    assert_eq!(
        mesh.stats.preserved_road_constraint_edges,
        mesh.stats.road_constraint_edges
    );
    assert!(mesh.vertices.iter().any(|vertex| {
        same_coord(vertex.x, 16.0) && same_coord(vertex.z, 7.0) && same_height(vertex.height_m, 1.0)
    }));
}

#[test]
fn conflicting_height_road_constraints_are_not_welded_by_height_max() {
    let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]);
    let road_a = road_loop_from_centerline(
        TerrainCdtVertex::new(4.0, 0.0, 20.0),
        TerrainCdtVertex::new(36.0, 0.0, 20.0),
        8.0,
    );
    let mut road_b = road_loop_from_centerline(
        TerrainCdtVertex::new(20.0, 0.0, 4.0),
        TerrainCdtVertex::new(20.0, 0.0, 36.0),
        8.0,
    );
    for vertex in &mut road_b {
        vertex.height_m = 1.0;
    }

    let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
        patch,
        vec![
            TerrainCdtRoadLoop::new(31, 0, road_a),
            TerrainCdtRoadLoop::new(32, 0, road_b),
        ],
        piece_source_samples(),
    ))
    .expect("conflicting road constraints should report invalid constraints without panicking");

    assert!(
        mesh.stats.invalid_constraint_edges > 0,
        "conflicting road seam heights must stay visible as CDT diagnostics instead of being welded by max-height"
    );
    assert!(
        !mesh.vertices.iter().any(|vertex| {
            vertex.height_m > 0.9
                && vertex.x > 15.0
                && vertex.x < 25.0
                && vertex.z > 15.0
                && vertex.z < 25.0
        }),
        "conflicting road constraints must not create synthesized max-height intersection vertices"
    );
}

#[test]
fn building_site_boundary_crossing_road_uses_road_height_without_cdt_conflict() {
    let patch = TerrainCdtPatch::new(0.0, 0.0, 32.0, 32.0, [0.0; 4]);
    let road = vec![
        TerrainCdtVertex::new(4.0, 0.0, 8.0),
        TerrainCdtVertex::new(20.0, 0.0, 8.0),
        TerrainCdtVertex::new(20.0, 0.0, 12.0),
        TerrainCdtVertex::new(4.0, 0.0, 12.0),
    ];
    let site = vec![
        TerrainCdtVertex::new(8.0, 2.0, 10.0),
        TerrainCdtVertex::new(16.0, 2.0, 10.0),
        TerrainCdtVertex::new(16.0, 2.0, 22.0),
        TerrainCdtVertex::new(8.0, 2.0, 22.0),
    ];
    let site_source = TerrainCdtRoadBoundarySource::BuildingSiteBoundary {
        building_idx: 7,
        local_loop_index: 0,
        local_edge_index: 0,
    };

    let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
        patch,
        vec![
            TerrainCdtRoadLoop::new(31, 0, road),
            sourced_road_loop(88, 0, site, site_source),
        ],
        piece_source_samples(),
    ))
    .expect("a yard seam crossing road-owned terrain must not reject the whole terrain patch");

    assert_eq!(mesh.stats.invalid_constraint_edges, 0);
    assert_eq!(mesh.stats.spade_missing_road_constraint_edges, 0);
    assert!(mesh.vertices.iter().any(|vertex| {
        same_coord(vertex.x, 8.0) && same_coord(vertex.z, 12.0) && same_height(vertex.height_m, 0.0)
    }));
    assert!(mesh.vertices.iter().any(|vertex| {
        same_coord(vertex.x, 16.0)
            && same_coord(vertex.z, 12.0)
            && same_height(vertex.height_m, 0.0)
    }));
}

#[test]
fn building_site_boundary_abutting_road_corner_uses_road_height_without_input_error() {
    let patch = TerrainCdtPatch::new(0.0, 0.0, 32.0, 32.0, [0.0; 4]);
    let road = vec![
        TerrainCdtVertex::new(4.0, 0.0, 8.0),
        TerrainCdtVertex::new(20.0, 0.0, 8.0),
        TerrainCdtVertex::new(20.0, 0.0, 12.0),
        TerrainCdtVertex::new(4.0, 0.0, 12.0),
    ];
    let site = vec![
        TerrainCdtVertex::new(4.0, 2.0, 12.0),
        TerrainCdtVertex::new(16.0, 2.0, 12.0),
        TerrainCdtVertex::new(16.0, 2.0, 22.0),
        TerrainCdtVertex::new(4.0, 2.0, 22.0),
    ];
    let site_source = TerrainCdtRoadBoundarySource::BuildingSiteBoundary {
        building_idx: 8,
        local_loop_index: 0,
        local_edge_index: 0,
    };

    let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
        patch,
        vec![
            TerrainCdtRoadLoop::new(31, 0, road),
            sourced_road_loop(88, 0, site, site_source),
        ],
        piece_source_samples(),
    ))
    .expect("a yard seam abutting a road edge must not fail on a shared X/Z boundary vertex");

    assert_eq!(mesh.stats.invalid_constraint_edges, 0);
    assert_eq!(mesh.stats.spade_missing_road_constraint_edges, 0);
    assert!(mesh.vertices.iter().any(|vertex| {
        same_coord(vertex.x, 4.0) && same_coord(vertex.z, 12.0) && same_height(vertex.height_m, 0.0)
    }));
    assert!(mesh.vertices.iter().any(|vertex| {
        same_coord(vertex.x, 16.0)
            && same_coord(vertex.z, 12.0)
            && same_height(vertex.height_m, 0.0)
    }));
}

#[test]
fn road_loop_endpoint_on_another_loop_edge_splits_the_roadbed_constraint() {
    let patch = TerrainCdtPatch::new(-96.0, -32.0, 64.0, 64.0, [0.0; 4]);
    let horizontal = vec![
        TerrainCdtVertex::new(-83.390, 0.12, -18.916),
        TerrainCdtVertex::new(49.610, 0.12, -18.916),
        TerrainCdtVertex::new(49.610, 0.12, -8.916),
        TerrainCdtVertex::new(-83.390, 0.12, -8.916),
    ];
    let incoming = vec![
        TerrainCdtVertex::new(-16.818, 0.12, -8.916),
        TerrainCdtVertex::new(-9.747, 0.12, -1.845),
        TerrainCdtVertex::new(-16.818, 0.12, 5.226),
        TerrainCdtVertex::new(-23.889, 0.12, -1.845),
    ];

    let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
        patch,
        vec![
            TerrainCdtRoadLoop::new(0, 0, horizontal),
            TerrainCdtRoadLoop::new(1, 0, incoming),
        ],
        Vec::new(),
    ))
    .expect("T-touching terrain roadbed constraints must be accepted");

    assert_eq!(mesh.stats.invalid_constraint_edges, 0);
    assert!(
        mesh.stats.road_constraint_edges > 8,
        "the horizontal roadbed edge must be split at the incoming mouth vertex"
    );
    assert_eq!(
        mesh.stats.preserved_road_constraint_edges,
        mesh.stats.road_constraint_edges
    );
    assert!(mesh.vertices.iter().any(|vertex| {
        same_coord(vertex.x, -16.818)
            && same_coord(vertex.z, -8.916)
            && (vertex.height_m - 0.12).abs() <= 0.0001
    }));
}
