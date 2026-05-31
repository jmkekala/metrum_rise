use super::*;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{
    EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
};
use godot::prelude::{Vector2, Vector3};

fn make_straight_road() -> (RegionGraph, usize) {
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(-60.0, 0.0, 0.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(60.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(Edge {
        start_node: start,
        end_node: end,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 120.0,
        physical_length: 120.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    });
    (graph, edge_idx)
}

fn make_quarter_arc_road(radius_m: f32) -> (RegionGraph, usize) {
    let mut graph = RegionGraph::new();
    let mut points = Vec::new();
    for step in 0..=12 {
        let t = step as f32 / 12.0;
        let angle = -std::f32::consts::FRAC_PI_2 + t * std::f32::consts::FRAC_PI_2;
        points.push(Vector3::new(
            radius_m * angle.cos(),
            0.0,
            radius_m * angle.sin(),
        ));
    }
    let length = points
        .windows(2)
        .map(|window| window[0].distance_to(window[1]))
        .sum();
    let start = graph.add_node(points[0], NodeType::Junction);
    let end = graph.add_node(*points.last().unwrap(), NodeType::Junction);
    let edge_idx = graph.add_edge(Edge {
        start_node: start,
        end_node: end,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: length,
        physical_length: length,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: points.clone(),
        physical_geometry: points,
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    });
    (graph, edge_idx)
}

fn inward_arc_point(radius_m: f32, angle: f32, offset_m: f32) -> Vector2 {
    let road = Vector2::new(radius_m * angle.cos(), radius_m * angle.sin());
    let inward = -road.normalized();
    road + inward * offset_m
}

fn make_zoning() -> ZoningSystem {
    ZoningSystem::new(&WorldConfig::default())
}

fn world_to_zone_cell(config: &WorldConfig, x: f32, z: f32) -> (i32, i32) {
    let cx = ((x + config.width_m * 0.5) / config.zone_cell_m - 0.5).round() as i32;
    let cy = ((z + config.height_m * 0.5) / config.zone_cell_m - 0.5).round() as i32;
    (cx, cy)
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
    let config = WorldConfig::default();
    let runtime_id = z
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Commercial)
        .unwrap();
    z.set_zone_profile_rect(-50.0, -50.0, 50.0, 50.0, runtime_id);

    let (grid_x, grid_y) = world_to_zone_cell(&config, -50.0, -50.0);
    let (grid_x_max, grid_y_max) = world_to_zone_cell(&config, 50.0, 50.0);
    let width_cells = (grid_x_max - grid_x + 1) as usize;
    let height_cells = (grid_y_max - grid_y + 1) as usize;

    // Capture, then clear, then restore.
    let saved = z.capture_patch(grid_x, grid_y, width_cells, height_cells);
    paint_zone_rect(&mut z, ZoneType::None, -50.0, -50.0, 50.0, 50.0);
    assert_eq!(zone_at_world(&z, 0.0, 0.0), ZoneType::None);

    z.restore_patch(grid_x, grid_y, width_cells, height_cells, &saved);
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
    let w = WorldConfig::default().zone_grid_width();
    let h = WorldConfig::default().zone_grid_height();
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

#[test]
fn test_default_parcel_creation_on_both_road_sides() {
    let (graph, edge_idx) = make_straight_road();
    let mut z = make_zoning();
    let residential = z
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();

    let south = z
        .place_or_rezone_default_parcel_at(0.0, -20.0, residential, &graph)
        .expect("south parcel");
    let north = z
        .place_or_rezone_default_parcel_at(0.0, 20.0, residential, &graph)
        .expect("north parcel");

    assert_ne!(south.raw(), north.raw());
    assert_eq!(z.parcels().len(), 2);
    assert!(z.parcels().iter().all(|parcel| {
        parcel.edge_idx() == edge_idx
            && (parcel.frontage_m() - DEFAULT_PARCEL_FRONTAGE_M).abs() < 1e-4
            && (parcel.depth_m() - DEFAULT_PARCEL_DEPTH_M).abs() < 1e-4
    }));

    let mut sides: Vec<i8> = z.parcels().iter().map(|parcel| parcel.side()).collect();
    sides.sort_unstable();
    assert_eq!(sides, vec![-1, 1]);
}

#[test]
fn test_custom_parcel_dimensions_are_owned_by_rust() {
    let (graph, edge_idx) = make_straight_road();
    let mut z = make_zoning();
    let residential = z
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();

    z.place_or_rezone_parcel_at(0.0, -25.0, residential, 35.0, 45.0, &graph)
        .expect("custom parcel");

    let parcel = &z.parcels()[0];
    assert_eq!(parcel.edge_idx(), edge_idx);
    assert!((parcel.frontage_m() - 35.0).abs() < 1e-4);
    assert!((parcel.depth_m() - 45.0).abs() < 1e-4);
}

#[test]
fn test_invalid_custom_parcel_dimensions_are_rejected() {
    let (graph, _) = make_straight_road();
    let mut z = make_zoning();
    let residential = z
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();

    let result = z.place_or_rezone_parcel_at(0.0, -20.0, residential, 2.0, 30.0, &graph);
    assert!(matches!(
        result,
        Err(ParcelPlacementError::InvalidDimensions)
    ));
}

#[test]
fn test_parcel_drag_run_uses_same_edge_side_and_gap() {
    let (graph, edge_idx) = make_straight_road();
    let mut z = make_zoning();
    let residential = z
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();

    let ids = z
        .place_parcel_run_at(
            -30.0,
            -20.0,
            30.0,
            -20.0,
            residential,
            20.0,
            30.0,
            10.0,
            &graph,
        )
        .expect("parcel run");

    assert_eq!(ids.len(), 3);
    let centers: Vec<f32> = z
        .parcels()
        .iter()
        .map(|parcel| parcel.front_center().x)
        .collect();
    assert_eq!(z.parcels().len(), 3);
    assert!(
        z.parcels()
            .iter()
            .all(|parcel| parcel.edge_idx() == edge_idx && parcel.side() == 1)
    );
    assert!((centers[0] + 30.0).abs() < 1e-4);
    assert!(centers[1].abs() < 1e-4);
    assert!((centers[2] - 30.0).abs() < 1e-4);
}

#[test]
fn test_parcel_drag_run_is_direction_independent() {
    let (graph, _) = make_straight_road();
    let z = make_zoning();

    let forward = z
        .preview_parcel_run_at(-30.0, -20.0, 30.0, -20.0, 20.0, 30.0, 0.0, &graph)
        .expect("forward run");
    let backward = z
        .preview_parcel_run_at(30.0, -20.0, -30.0, -20.0, 20.0, 30.0, 0.0, &graph)
        .expect("backward run");

    let forward_centers: Vec<f32> = forward
        .iter()
        .map(|geometry| geometry.front_center.x)
        .collect();
    let backward_centers: Vec<f32> = backward
        .iter()
        .map(|geometry| geometry.front_center.x)
        .collect();
    assert_eq!(forward_centers, backward_centers);
}

#[test]
fn test_parcel_drag_run_inner_curve_widens_spacing_instead_of_rejecting() {
    let radius_m = 70.0;
    let (graph, _) = make_quarter_arc_road(radius_m);
    let z = make_zoning();
    let start = inward_arc_point(radius_m, -1.35, 20.0);
    let end = inward_arc_point(radius_m, -0.20, 20.0);

    let run = z
        .preview_parcel_run_at(start.x, start.y, end.x, end.y, 20.0, 30.0, 0.0, &graph)
        .expect("inner curve should place the non-overlapping parcels that fit");

    assert!(run.len() >= 2);
    assert!(run.iter().all(|geometry| geometry.side == -1));
    assert!(!parcels::geometries_have_overlap(&run));
    assert!(
        run.windows(2).any(|window| {
            let gap = (window[1].frontage_center_t - window[0].frontage_center_t)
                * graph.edge(window[0].edge_idx).physical_length
                - 20.0;
            gap > 0.5
        }),
        "inner curve should need wider than exact zero-gap spacing"
    );
}

#[test]
fn test_parcel_drag_run_rejects_existing_overlap() {
    let (graph, _) = make_straight_road();
    let mut z = make_zoning();
    let residential = z
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();

    z.place_or_rezone_parcel_at(0.0, -20.0, residential, 20.0, 30.0, &graph)
        .expect("existing parcel");

    let result = z.preview_parcel_run_at(-20.0, -20.0, 20.0, -20.0, 20.0, 30.0, 0.0, &graph);
    assert!(matches!(
        result,
        Err(ParcelPlacementError::OverlapsExistingParcel)
    ));
}

#[test]
fn test_parcel_drag_run_detects_internal_overlap() {
    let (graph, _) = make_straight_road();
    let z = make_zoning();
    let geometry = z
        .preview_parcel_at(0.0, -20.0, 20.0, 30.0, &graph)
        .expect("preview parcel");

    assert!(parcels::geometries_have_overlap(&[geometry, geometry]));
}

#[test]
fn test_parcel_drag_run_rejects_invalid_gap() {
    let (graph, _) = make_straight_road();
    let z = make_zoning();

    let result = z.preview_parcel_run_at(-20.0, -20.0, 20.0, -20.0, 20.0, 30.0, 25.0, &graph);
    assert!(matches!(result, Err(ParcelPlacementError::InvalidGap)));
}

#[test]
fn test_parcel_rezone_stroke_updates_touched_parcels() {
    let (graph, _) = make_straight_road();
    let mut z = make_zoning();
    let residential = z
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();
    let commercial = z
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Commercial)
        .unwrap();

    z.place_parcel_run_at(
        -30.0,
        -20.0,
        30.0,
        -20.0,
        residential,
        20.0,
        30.0,
        10.0,
        &graph,
    )
    .expect("parcel run");

    let preview = z.preview_rezone_stroke(-35.0, -20.0, 35.0, -20.0);
    assert_eq!(preview.len(), 3);

    let changed = z
        .rezone_stroke(-35.0, -20.0, 35.0, -20.0, commercial)
        .expect("rezone stroke");
    assert_eq!(changed.len(), 3);
    assert!(
        z.parcels()
            .iter()
            .all(|parcel| parcel.zone_profile_runtime_id() == commercial)
    );
}

#[test]
fn test_existing_parcel_geometry_preview_is_available_at_point() {
    let (graph, _) = make_straight_road();
    let mut z = make_zoning();
    let residential = z
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();

    z.place_or_rezone_default_parcel_at(0.0, -20.0, residential, &graph)
        .expect("parcel");

    assert!(z.has_parcel_at(0.0, -20.0));
    let geometry = z
        .parcel_geometry_at(0.0, -20.0)
        .expect("existing parcel geometry");
    assert!(geometry.center.y < 0.0);
}

#[test]
fn test_default_parcel_overlap_is_rejected() {
    let (graph, _) = make_straight_road();
    let mut z = make_zoning();
    let residential = z
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();

    z.place_or_rezone_default_parcel_at(0.0, -20.0, residential, &graph)
        .expect("initial parcel");

    let result = z.place_or_rezone_default_parcel_at(15.0, -20.0, residential, &graph);
    assert!(matches!(
        result,
        Err(ParcelPlacementError::OverlapsExistingParcel)
    ));
}

#[test]
fn test_parcel_edge_compaction_remaps_and_drops_missing_edges() {
    let (graph, edge_idx) = make_straight_road();
    let mut z = make_zoning();
    let residential = z
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();

    z.place_or_rezone_default_parcel_at(0.0, -20.0, residential, &graph)
        .expect("parcel");

    let mut map = std::collections::HashMap::new();
    map.insert(edge_idx, 7);
    z.update_edge_indices(&map);
    assert_eq!(z.parcels()[0].edge_idx(), 7);

    z.update_edge_indices(&std::collections::HashMap::new());
    assert!(z.parcels().is_empty());
}
