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

fn add_vertical_road_at_x(graph: &mut RegionGraph, x: f32) -> usize {
    let start = graph.add_node(Vector3::new(x, 0.0, -80.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(x, 0.0, 80.0), NodeType::Junction);
    graph.add_edge(Edge {
        start_node: start,
        end_node: end,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 160.0,
        physical_length: 160.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(x, 0.0, -80.0), Vector3::new(x, 0.0, 80.0)],
        physical_geometry: vec![Vector3::new(x, 0.0, -80.0), Vector3::new(x, 0.0, 80.0)],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    })
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
fn test_parcel_overlap_with_nearby_road_is_rejected() {
    let (mut graph, _) = make_straight_road();
    add_vertical_road_at_x(&mut graph, 14.0);
    let z = make_zoning();

    let result = z.preview_parcel_at(0.0, -7.0, 20.0, 30.0, &graph);

    assert!(matches!(result, Err(ParcelPlacementError::OverlapsRoad)));
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
fn test_parcel_drag_run_rejects_nearby_road_overlap() {
    let (mut graph, _) = make_straight_road();
    add_vertical_road_at_x(&mut graph, 14.0);
    let z = make_zoning();

    let result = z.preview_parcel_run_at(-20.0, -7.0, 20.0, -7.0, 20.0, 30.0, 0.0, &graph);

    assert!(matches!(result, Err(ParcelPlacementError::OverlapsRoad)));
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
fn test_parcel_drag_run_extends_from_existing_start_parcel() {
    let (graph, _) = make_straight_road();
    let mut z = make_zoning();
    let residential = z
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();

    z.place_or_rezone_parcel_at(0.0, -20.0, residential, 20.0, 30.0, &graph)
        .expect("existing parcel");

    let preview = z
        .preview_parcel_run_at(0.0, -20.0, 50.0, -20.0, 20.0, 30.0, 0.0, &graph)
        .expect("extension preview");

    assert_eq!(preview.len(), 2);
    assert!((preview[0].front_center.x - 20.0).abs() < 1e-4);
    assert!((preview[1].front_center.x - 40.0).abs() < 1e-4);

    let ids = z
        .place_parcel_run_at(
            0.0,
            -20.0,
            50.0,
            -20.0,
            residential,
            20.0,
            30.0,
            0.0,
            &graph,
        )
        .expect("extension commit");

    assert_eq!(ids.len(), 2);
    assert_eq!(z.parcels().len(), 3);
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
    assert_eq!(
        z.parcel_profile_runtime_id_at(0.0, -20.0),
        Some(residential)
    );
    assert_eq!(z.parcel_profile_runtime_id_at(1000.0, 1000.0), None);
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
