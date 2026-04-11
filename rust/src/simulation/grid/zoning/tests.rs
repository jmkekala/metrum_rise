use super::*;
use crate::simulation::core::config::MapConfig;

fn make_zoning() -> ZoningSystem {
    ZoningSystem::new(&MapConfig::default())
}

fn paint_zone_rect(zoning: &mut ZoningSystem, zone: ZoneType, x0: f32, z0: f32, x1: f32, z1: f32) {
    let runtime_id = zoning
        .profiles
        .default_runtime_id_for_zone_type(zone)
        .unwrap_or(0);
    zoning.set_zone_profile_rect(x0, z0, x1, z1, runtime_id);
}

fn zone_at_world(zoning: &ZoningSystem, x: f32, z: f32) -> ZoneType {
    zoning
        .profiles
        .zone_type_for_runtime_id(zoning.get_zone_profile_runtime_id_world(x, z))
}

#[test]
fn test_set_zone_rect_fills_cells() {
    let mut z = make_zoning();
    // Paint a 20×20 m rectangle centred at world origin.
    paint_zone_rect(&mut z, ZoneType::Residential, -10.0, -10.0, 10.0, 10.0);

    // Origin cell must be Residential.
    assert_eq!(zone_at_world(&z, 0.0, 0.0), ZoneType::Residential);
    // Cells well outside the rect must be None.
    assert_eq!(zone_at_world(&z, 500.0, 500.0), ZoneType::None);
}

#[test]
fn test_set_zone_rect_clear() {
    let mut z = make_zoning();
    paint_zone_rect(&mut z, ZoneType::Commercial, -100.0, -100.0, 100.0, 100.0);
    assert_eq!(zone_at_world(&z, 0.0, 0.0), ZoneType::Commercial);

    paint_zone_rect(&mut z, ZoneType::None, -100.0, -100.0, 100.0, 100.0);
    assert_eq!(zone_at_world(&z, 0.0, 0.0), ZoneType::None);
}

#[test]
fn test_clear_resets_everything() {
    let mut z = make_zoning();
    paint_zone_rect(&mut z, ZoneType::Industrial, -500.0, -500.0, 500.0, 500.0);
    z.clear();
    assert_eq!(zone_at_world(&z, 0.0, 0.0), ZoneType::None);
}

#[test]
fn test_zone_subrect_roundtrip() {
    let mut z = make_zoning();
    let runtime_id = z
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Commercial)
        .unwrap();
    z.set_zone_profile_rect(-50.0, -50.0, 50.0, 50.0, runtime_id);

    // Capture, then clear, then restore.
    let saved = z.capture_patch(950, 950, 101, 101);
    paint_zone_rect(&mut z, ZoneType::None, -50.0, -50.0, 50.0, 50.0);
    assert_eq!(zone_at_world(&z, 0.0, 0.0), ZoneType::None);

    z.restore_patch(950, 950, 101, 101, &saved);
    assert_eq!(zone_at_world(&z, 0.0, 0.0), ZoneType::Commercial);
}

#[test]
fn test_occupied_rect_mark_and_check() {
    let mut z = make_zoning();
    let tangent = godot::prelude::Vector2::new(1.0, 0.0);

    // Mark a 20×10 m rect occupied at origin.
    z.mark_occupied_rect(0.0, 0.0, tangent, 20.0, 10.0, true);
    assert!(z.is_rect_occupied(0.0, 0.0, tangent, 20.0, 10.0));

    // A non-overlapping rect should not be occupied.
    assert!(!z.is_rect_occupied(200.0, 200.0, tangent, 10.0, 10.0));

    // Clear and verify.
    z.mark_occupied_rect(0.0, 0.0, tangent, 20.0, 10.0, false);
    assert!(!z.is_rect_occupied(0.0, 0.0, tangent, 20.0, 10.0));
}

#[test]
fn test_texture_data_length() {
    let z = make_zoning();
    let w = MapConfig::default().zone_grid_width();
    let h = MapConfig::default().zone_grid_height();
    assert_eq!(z.get_zone_profile_texture_data_rg8().len(), w * h * 2);
    assert_eq!(z.get_occupied_texture_data().len(), w * h);
    assert_eq!(z.get_distance_texture_data().len(), w * h);
}

#[test]
fn test_update_edge_indices_noop() {
    let mut z = make_zoning();
    paint_zone_rect(&mut z, ZoneType::Industrial, -10.0, -10.0, 10.0, 10.0);
    let map = std::collections::HashMap::new();
    z.update_edge_indices(&map); // must not panic or clear data
    assert_eq!(zone_at_world(&z, 0.0, 0.0), ZoneType::Industrial);
}
