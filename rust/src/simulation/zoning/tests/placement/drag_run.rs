//! Parcel drag-run placement behavior tests.

use super::super::helpers::{
    add_vertical_road_at_x, inward_arc_point, make_quarter_arc_road, make_straight_road,
    make_zoning,
};
use crate::simulation::zoning::parcels;
use crate::simulation::zoning::{ParcelPlacementError, ZoneType};

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
