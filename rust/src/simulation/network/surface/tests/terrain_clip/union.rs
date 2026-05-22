//! Terrain clip union and source-chain tests.

use super::*;

#[test]
fn terrain_clip_union_exports_hole_contours_for_ring_footprint() {
    let y = 2.0;
    let rectangles = [
        vec![
            Vector3::new(0.0, y, 0.0),
            Vector3::new(4.0, y, 0.0),
            Vector3::new(4.0, y, 1.0),
            Vector3::new(0.0, y, 1.0),
        ],
        vec![
            Vector3::new(0.0, y, 3.0),
            Vector3::new(4.0, y, 3.0),
            Vector3::new(4.0, y, 4.0),
            Vector3::new(0.0, y, 4.0),
        ],
        vec![
            Vector3::new(0.0, y, 1.0),
            Vector3::new(1.0, y, 1.0),
            Vector3::new(1.0, y, 3.0),
            Vector3::new(0.0, y, 3.0),
        ],
        vec![
            Vector3::new(3.0, y, 1.0),
            Vector3::new(4.0, y, 1.0),
            Vector3::new(4.0, y, 3.0),
            Vector3::new(3.0, y, 3.0),
        ],
    ];
    let raw_clip_sources = rectangles
        .iter()
        .enumerate()
        .map(|(index, points)| terrain_clip_loop_for_node_test(points, index as u32))
        .collect::<Vec<_>>();

    let export = RoadSurfaceSystem::union_terrain_clip_boundary_export(&raw_clip_sources)
        .expect("ring road footprint should export both outer and hole contours");

    assert_eq!(
        export.loops.len(),
        2,
        "unioned ring footprint must preserve its inner terrain-island contour"
    );
    assert_eq!(
        export
            .loop_topologies
            .iter()
            .filter(|topology| topology.role == RoadSurfaceTerrainClipContourRole::Outer)
            .count(),
        1
    );
    assert_eq!(
        export
            .loop_topologies
            .iter()
            .filter(|topology| topology.role == RoadSurfaceTerrainClipContourRole::Hole)
            .count(),
        1
    );
    let hole_loop = export
        .loops
        .iter()
        .zip(export.loop_topologies.iter())
        .find(|(_, topology)| topology.role == RoadSurfaceTerrainClipContourRole::Hole)
        .map(|(boundary_loop, _)| boundary_loop)
        .expect("hole topology should identify the inner contour");
    assert!(
        hole_loop.points_world.iter().all(|point| {
            point.x >= 1.0 - SAMPLE_EPSILON_M
                && point.x <= 3.0 + SAMPLE_EPSILON_M
                && point.z >= 1.0 - SAMPLE_EPSILON_M
                && point.z <= 3.0 + SAMPLE_EPSILON_M
        }),
        "hole contour must stay on the inner terrain island boundary: {hole_loop:?}"
    );
}

#[test]
fn terrain_clip_union_skips_same_key_dust_only_when_degenerate() {
    let y = 6.0;
    let p0 = Vector3::new(0.0, y, 0.0);
    let p0_same_key = Vector3::new(0.0000002, y + 0.25, 0.0000002);
    let p1 = Vector3::new(1.0, y, 0.0);
    let p2 = Vector3::new(1.0, y, 1.0);
    let p3 = Vector3::new(0.0, y, 1.0);
    let raw_clip_sources = vec![RoadSurfaceTerrainClipLoop {
        source_edges: vec![
            terrain_clip_source_edge_for_test(p0, p0_same_key),
            terrain_clip_source_edge_for_test(p0_same_key, p1),
            terrain_clip_source_edge_for_test(p1, p2),
            terrain_clip_source_edge_for_test(p2, p3),
            terrain_clip_source_edge_for_test(p3, p0),
        ],
        points_world: vec![p0, p0_same_key, p1, p2, p3],
    }];

    let unioned =
        RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&raw_clip_sources)
            .expect(
                "same-key degenerate dust should be skipped without losing the sourced clip loop",
            );

    assert_eq!(unioned.len(), 1);
    assert!(
        unioned[0]
            .source_edges
            .iter()
            .all(|edge| SurfaceXzKey::from_godot_world_xz(edge.start)
                != SurfaceXzKey::from_godot_world_xz(edge.end)),
        "same-key dust may be skipped only as a degenerate segment, never emitted as a sourced edge"
    );
}

#[test]
fn terrain_clip_union_preserves_boundary_only_connector_by_interpolation() {
    let raw_boundary_y = -99.0;
    let p0 = Vector3::new(0.0, 10.0, 0.0);
    let p1 = Vector3::new(0.5, 10.5, 0.0);
    let d0 = Vector3::new(0.50002, raw_boundary_y, 0.00008);
    let d1 = Vector3::new(0.49998, raw_boundary_y, 0.00016);
    let d2 = Vector3::new(0.50001, raw_boundary_y, 0.00024);
    let p2 = Vector3::new(0.5, 10.7, 0.00032);
    let p3 = Vector3::new(1.0, 11.0, 0.0);
    let p4 = Vector3::new(1.0, 11.0, 0.1);
    let p5 = Vector3::new(0.0, 10.0, 0.1);
    let raw_clip_sources = vec![RoadSurfaceTerrainClipLoop {
        source_edges: vec![
            terrain_clip_source_edge_for_test(p0, p1),
            terrain_clip_source_edge_for_test(p2, p3),
            terrain_clip_source_edge_for_test(p3, p4),
            terrain_clip_source_edge_for_test(p4, p5),
            terrain_clip_source_edge_for_test(p5, p0),
        ],
        points_world: vec![
            Vector3::new(p0.x, raw_boundary_y, p0.z),
            Vector3::new(p1.x, raw_boundary_y, p1.z),
            d0,
            d1,
            d2,
            Vector3::new(p2.x, raw_boundary_y, p2.z),
            Vector3::new(p3.x, raw_boundary_y, p3.z),
            Vector3::new(p4.x, raw_boundary_y, p4.z),
            Vector3::new(p5.x, raw_boundary_y, p5.z),
        ],
    }];

    let clip_export = RoadSurfaceSystem::union_terrain_clip_boundary_export(&raw_clip_sources)
        .expect("boundary-only connector should preserve terrain clip export");

    assert_eq!(
        clip_export.loops.len(),
        1,
        "unioned terrain clip cutter must survive a sub-budget boundary-only connector"
    );
    assert!(
        RoadSurfaceSystem::polygon_has_area_xz(&clip_export.loops[0].points_world),
        "preserved terrain clip cutter must remain a valid road footprint polygon"
    );
    assert!(
        clip_export.loops[0]
            .points_world
            .iter()
            .all(|point| (point.y - raw_boundary_y).abs() > SAMPLE_EPSILON_M),
        "boundary-only connector heights must come from solved source contour interpolation"
    );
    assert!(
        clip_export.loops[0]
            .points_world
            .iter()
            .any(|point| point.y > p1.y && point.y < p2.y),
        "sub-budget connector must carry interpolated seam heights between adjacent solved footprint vertices"
    );
}
