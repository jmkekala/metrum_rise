// SPDX-License-Identifier: GPL-2.0-only

//! Parcel rezone stroke and point-query tests.

use super::helpers::{make_straight_road, make_zoning};
use crate::simulation::zoning::ZoneType;

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
