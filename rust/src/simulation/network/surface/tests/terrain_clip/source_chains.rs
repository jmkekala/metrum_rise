//! Terrain clip source-chain provenance tests.

use super::*;

#[test]
fn terrain_clip_union_splits_union_segment_through_source_owned_boundary_chain() {
    let p0 = Vector3::new(0.0, 10.0, 0.0);
    let p1 = Vector3::new(0.45, 10.8, 0.18);
    let p2 = Vector3::new(1.0, 11.4, 0.0);
    let p3 = Vector3::new(1.0, 11.4, 0.5);
    let p4 = Vector3::new(0.0, 10.0, 0.5);
    let source_loop = RoadSurfaceTerrainClipLoop {
        source_edges: vec![
            terrain_clip_source_edge_for_test(p0, p1),
            terrain_clip_source_edge_for_test(p1, p2),
            terrain_clip_source_edge_for_test(p2, p3),
            terrain_clip_source_edge_for_test(p3, p4),
            terrain_clip_source_edge_for_test(p4, p0),
        ],
        points_world: vec![p0, p2, p3, p4],
    };
    let unioned = RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&[source_loop])
        .expect("source-owned split segment should survive terrain clip union");

    assert_eq!(
        unioned.len(),
        1,
        "unioned terrain clip contour must not drop a source-owned segment that spans adjacent boundary edges"
    );
    let points = &unioned[0].points_world;
    assert!(
        points
            .iter()
            .any(|point| (point.x - p1.x).abs() <= SAMPLE_EPSILON_M
                && (point.z - p1.z).abs() <= SAMPLE_EPSILON_M),
        "source-owned split vertex must be stitched back into the unioned cutter"
    );
    assert_eq!(
        unioned[0].source_edges.len(),
        points.len(),
        "every emitted clip segment must keep an owner-backed source edge"
    );
    assert!(
        unioned[0]
            .source_edges
            .iter()
            .any(|edge| (edge.start.x - p0.x).abs() <= SAMPLE_EPSILON_M
                && (edge.end.x - p1.x).abs() <= SAMPLE_EPSILON_M),
        "first split subsegment must preserve its original source edge"
    );
    assert!(
        unioned[0]
            .source_edges
            .iter()
            .any(|edge| (edge.start.x - p1.x).abs() <= SAMPLE_EPSILON_M
                && (edge.end.x - p2.x).abs() <= SAMPLE_EPSILON_M),
        "second split subsegment must preserve its original source edge"
    );
}

#[test]
fn terrain_clip_union_stitches_source_chain_across_same_xz_height_step() {
    let p0 = Vector3::new(0.0, 10.0, 0.0);
    let p1_low = Vector3::new(0.45, 10.8, 0.18);
    let p1_high = Vector3::new(0.45, 11.0, 0.18);
    let p2 = Vector3::new(1.0, 11.4, 0.0);
    let p3 = Vector3::new(1.0, 11.4, 0.5);
    let p4 = Vector3::new(0.0, 10.0, 0.5);
    let source_loop = RoadSurfaceTerrainClipLoop {
        source_edges: vec![
            terrain_clip_source_edge_for_test(p0, p1_low),
            terrain_clip_source_edge_for_test(p1_high, p2),
            terrain_clip_source_edge_for_test(p2, p3),
            terrain_clip_source_edge_for_test(p3, p4),
            terrain_clip_source_edge_for_test(p4, p0),
        ],
        points_world: vec![p0, p2, p3, p4],
    };
    let unioned = RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&[source_loop])
        .expect("source-owned height step should survive terrain clip union");

    assert_eq!(
        unioned.len(),
        1,
        "same-XZ source chain height steps must not drop the road-owned terrain clip loop"
    );
    let stitched = unioned[0]
        .points_world
        .iter()
        .find(|point| {
            (point.x - p1_low.x).abs() <= SAMPLE_EPSILON_M
                && (point.z - p1_low.z).abs() <= SAMPLE_EPSILON_M
        })
        .copied()
        .expect("source chain split vertex should survive terrain clip union");
    assert!(
        stitched.y >= p1_high.y - SAMPLE_EPSILON_M,
        "shared XZ source chain vertex should use the highest visible road height"
    );
    assert_eq!(
        unioned[0].source_edges.len(),
        unioned[0].points_world.len(),
        "every stitched segment must keep owner-backed source provenance"
    );
}

#[test]
fn terrain_clip_union_blocks_partial_export_when_shape_has_no_source_owner() {
    let y = 4.0;
    let valid = [
        Vector3::new(0.0, y, 0.0),
        Vector3::new(1.0, y, 0.0),
        Vector3::new(1.0, y, 1.0),
        Vector3::new(0.0, y, 1.0),
    ];
    let unowned = [
        Vector3::new(3.0, y, 0.0),
        Vector3::new(4.0, y, 0.0),
        Vector3::new(4.0, y, 1.0),
        Vector3::new(3.0, y, 1.0),
    ];
    let raw_clip_sources = vec![
        RoadSurfaceTerrainClipLoop {
            source_edges: valid
                .iter()
                .zip(valid.iter().cycle().skip(1))
                .take(valid.len())
                .map(|(&start, &end)| terrain_clip_source_edge_for_test(start, end))
                .collect(),
            points_world: valid.to_vec(),
        },
        RoadSurfaceTerrainClipLoop {
            source_edges: Vec::new(),
            points_world: unowned.to_vec(),
        },
    ];

    let unioned =
        RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&raw_clip_sources);

    assert!(
        matches!(
            unioned,
            Err(RoadSurfaceTerrainClipExportError::MissingOuterBoundaryOwner { .. })
                | Err(RoadSurfaceTerrainClipExportError::MissingOutputBoundaryOwner { .. })
        ),
        "terrain clip union must return a blocking source-ownership error instead of silently dropping an unowned cutter shape"
    );
}

#[test]
fn terrain_clip_union_blocks_partially_covered_source_segment() {
    let y = 5.0;
    let p0 = Vector3::new(0.0, y, 0.0);
    let p1 = Vector3::new(1.0, y, 0.0);
    let p2 = Vector3::new(1.0, y, 1.0);
    let p3 = Vector3::new(0.0, y, 1.0);
    let mid = Vector3::new(0.5, y, 0.0);
    let raw_clip_sources = vec![RoadSurfaceTerrainClipLoop {
        source_edges: vec![
            terrain_clip_source_edge_for_test(p0, mid),
            terrain_clip_source_edge_for_test(p1, p2),
            terrain_clip_source_edge_for_test(p2, p3),
            terrain_clip_source_edge_for_test(p3, p0),
        ],
        points_world: vec![p0, p1, p2, p3],
    }];

    let unioned =
        RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&raw_clip_sources);

    let Err(RoadSurfaceTerrainClipExportError::MissingOuterBoundaryOwner { context, .. }) = unioned
    else {
        panic!(
            "partially covered terrain clip segment must be a blocking source-coverage error, got {unioned:?}"
        );
    };
    assert!(
        context.contains("partial_coverage"),
        "partial source coverage should be diagnosed distinctly from ordinary missing ownership: {context}"
    );
}

#[test]
fn terrain_clip_source_endpoint_groups_use_key_derived_coordinates() {
    let p0_loop = Vector3::new(0.0, 10.0004, 0.0);
    let p0_source = Vector3::new(0.0000002, 10.00049, 0.0000002);
    let p1 = Vector3::new(1.0, 10.0, 0.0);
    let p2 = Vector3::new(1.0, 10.0, 1.0);
    let p3 = Vector3::new(0.0, 10.0, 1.0);
    let raw_clip_sources = vec![RoadSurfaceTerrainClipLoop {
        source_edges: vec![
            terrain_clip_source_edge_for_test(p0_source, p1),
            terrain_clip_source_edge_for_test(p1, p2),
            terrain_clip_source_edge_for_test(p2, p3),
            terrain_clip_source_edge_for_test(p3, p0_loop),
        ],
        points_world: vec![p0_loop, p1, p2, p3],
    }];

    let unioned =
        RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&raw_clip_sources)
            .expect("same key endpoint group should canonicalize through keys, not raw majority");
    let origin = unioned[0]
        .source_edges
        .iter()
        .flat_map(|edge| [edge.start, edge.end])
        .find(|point| {
            SurfaceXzKey::from_godot_world_xz(*point) == SurfaceXzKey::from_raw_keys(0, 0)
        })
        .expect("origin endpoint should remain present after terrain clip union");

    assert_eq!(
        SurfaceHeightMmKey::from_m_f32(origin.y),
        SurfaceHeightMmKey::from_m_f32(10.0),
        "same-key source endpoints must be emitted at the canonical height key, not at a majority raw Vector3"
    );
    assert!(
        origin.x.abs() <= SAMPLE_EPSILON_M && origin.z.abs() <= SAMPLE_EPSILON_M,
        "same-key source endpoints must be emitted at the canonical XZ key"
    );
}

#[test]
fn terrain_clip_union_preserves_endpoint_owned_numeric_connector() {
    let y = 12.0;
    let points = vec![
        Vector3::new(0.0, y, 0.0),
        Vector3::new(0.5, y, 0.0),
        Vector3::new(0.501, y, 0.0001),
        Vector3::new(1.0, y, 0.0),
        Vector3::new(1.0, y, 0.1),
        Vector3::new(0.0, y, 0.1),
    ];
    let raw_clip_sources = vec![RoadSurfaceTerrainClipLoop {
        source_edges: vec![
            terrain_clip_source_edge_for_test(points[0], points[1]),
            terrain_clip_source_edge_for_test(points[2], points[3]),
            terrain_clip_source_edge_for_test(points[3], points[4]),
            terrain_clip_source_edge_for_test(points[4], points[5]),
            terrain_clip_source_edge_for_test(points[5], points[0]),
        ],
        points_world: points,
    }];

    let clip_export = RoadSurfaceSystem::union_terrain_clip_boundary_export(&raw_clip_sources)
        .expect("endpoint-owned connector should preserve terrain clip export");

    assert_eq!(
        clip_export.loops.len(),
        1,
        "unioned terrain clip contour must keep source-owned endpoint continuity instead of dropping the road cutter"
    );
    assert!(
        clip_export.loops[0]
            .points_world
            .iter()
            .all(|point| (point.y - y).abs() <= SAMPLE_EPSILON_M),
        "accepted connector must reuse canonical source endpoint heights"
    );
}
