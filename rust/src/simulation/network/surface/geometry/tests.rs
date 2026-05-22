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
