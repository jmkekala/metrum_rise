//! Earthwork geometry helper tests.

use super::*;

#[test]
fn earthwork_vertex_outward_rejects_degenerate_spur() {
    let points = vec![
        Vector3::new(-1.0, 0.0, 0.0),
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(-1.0, 0.0, 0.0),
    ];

    assert!(RoadSurfaceSystem::closed_loop_vertex_outward_xz(&points, 1).is_none());
}

#[test]
fn earthwork_edge_outward_accepts_short_nonzero_edges() {
    let outward =
        RoadSurfaceSystem::edge_outward_normal_xz(Vector2::new(SAMPLE_EPSILON_M * 10.0, 0.0), true);

    assert_eq!(outward, Some(Vector2::new(0.0, -1.0)));
}
