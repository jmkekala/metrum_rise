//! Single-parcel placement behavior tests.

use super::super::helpers::{add_vertical_road_at_x, make_straight_road, make_zoning};
use crate::simulation::zoning::{
    DEFAULT_PARCEL_DEPTH_M, DEFAULT_PARCEL_FRONTAGE_M, ParcelPlacementError, ZoneType,
};

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
