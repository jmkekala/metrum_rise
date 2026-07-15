//! Low-level geometry regression tests.

use super::*;

#[test]
fn road_triangle_double_area_uses_xz_plane() {
    let area = RoadSurfaceSystem::road_triangle_double_area_xz_m2([
        RoadVec3::new(0.0, 8.0, 0.0),
        RoadVec3::new(2.0, -4.0, 0.0),
        RoadVec3::new(0.0, 2.0, 3.0),
    ]);

    assert_eq!(area, 6.0);
}

#[test]
fn triangle_has_area_xz_rejects_area_positive_needle_triangle() {
    let triangle = [
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(3.687, 0.0, 0.0),
        RoadVec3::new(0.0, 0.0, 0.000002826),
    ];

    assert!(
        RoadSurfaceSystem::road_triangle_double_area_xz_m2(triangle)
            > f64::from(NODE_OVERLAY_MIN_AREA_M2)
    );
    assert!(!RoadSurfaceSystem::triangle_has_area_xz(triangle));
}

#[test]
fn triangle_has_area_xz_accepts_stable_triangle() {
    assert!(RoadSurfaceSystem::triangle_has_area_xz([
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(2.0, 0.0, 0.0),
        RoadVec3::new(0.0, 0.0, 2.0),
    ]));
}
