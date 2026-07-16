//! Zoning regression tests for the simulation-node bridge.

use super::*;

#[test]
fn zoning_parcel_surface_corners_use_visible_world_surface_height() {
    let raw_height = 3.25;
    let core = test_core_with_flat_terrain(raw_height);
    let geometry = crate::simulation::zoning::ParcelGeometry {
        edge_idx: 0,
        side: 1,
        frontage_center_t: 0.5,
        frontage_m: 20.0,
        depth_m: 30.0,
        front_center: Vector2::ZERO,
        center: Vector2::ZERO,
        tangent: Vector2::new(1.0, 0.0),
        normal: Vector2::new(0.0, -1.0),
        corners: [
            Vector2::new(-5.0, -5.0),
            Vector2::new(5.0, -5.0),
            Vector2::new(5.0, 5.0),
            Vector2::new(-5.0, 5.0),
        ],
        aabb_min: Vector2::new(-5.0, -5.0),
        aabb_max: Vector2::new(5.0, 5.0),
    };

    let corners = zoning_parcel_surface_corners(&core, &geometry);
    let expected_y = raw_height * config::HEIGHT_SCALE;

    assert!(corners.iter().all(|corner| {
        (corner.y - expected_y).abs() <= 1e-4 && corner.x.abs() == 5.0 && corner.z.abs() == 5.0
    }));
}

#[test]
fn zoning_parcel_cell_dimensions_use_world_zone_cell_size() {
    let mut config = WorldConfig::default();
    config.zone_cell_m = 10.0;

    assert_eq!(
        zoning_parcel_cell_dimensions(&config, 2, 3),
        Some((20.0, 30.0))
    );
    assert_eq!(zoning_parcel_cell_dimensions(&config, 0, 3), None);
}
