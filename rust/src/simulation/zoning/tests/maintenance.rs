// SPDX-License-Identifier: GPL-2.0-only

//! Zoning maintenance behavior tests.

use super::helpers::{make_straight_road, make_zoning};
use crate::simulation::zoning::ZoneType;
use godot::prelude::Vector3;
use std::collections::HashSet;

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

#[test]
fn test_no_build_edge_cleanup_removes_attached_parcels() {
    let (graph, edge_idx) = make_straight_road();
    let mut z = make_zoning();
    let residential = z
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();

    z.place_or_rezone_default_parcel_at(0.0, -20.0, residential, &graph)
        .expect("parcel");

    assert_eq!(z.remove_parcels_attached_to_edge(edge_idx), 1);
    assert!(z.parcels().is_empty());
}

#[test]
fn road_corridor_overlap_query_finds_blocking_parcel() {
    let (graph, _) = make_straight_road();
    let mut z = make_zoning();
    let residential = z
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();

    let parcel_id = z
        .place_or_rezone_default_parcel_at(0.0, -20.0, residential, &graph)
        .expect("parcel");

    let crossing = [Vector3::new(0.0, 0.0, -80.0), Vector3::new(0.0, 0.0, 80.0)];
    let clear = [
        Vector3::new(40.0, 0.0, -80.0),
        Vector3::new(40.0, 0.0, 80.0),
    ];

    assert_eq!(
        z.parcel_ids_overlapping_road_corridor(&crossing, 5.0),
        vec![parcel_id.raw()]
    );
    assert!(
        z.parcel_ids_overlapping_road_corridor(&clear, 5.0)
            .is_empty()
    );
}

#[test]
fn remove_parcels_by_raw_ids_removes_only_requested_parcels() {
    let (graph, _) = make_straight_road();
    let mut z = make_zoning();
    let residential = z
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();

    let left = z
        .place_or_rezone_default_parcel_at(-30.0, -20.0, residential, &graph)
        .expect("left parcel");
    let right = z
        .place_or_rezone_default_parcel_at(30.0, -20.0, residential, &graph)
        .expect("right parcel");

    assert_eq!(z.remove_parcels_by_raw_ids(&HashSet::from([left.raw()])), 1);
    assert_eq!(z.parcels().len(), 1);
    assert_eq!(z.parcels()[0].id(), right);
}

#[test]
fn zoning_overlay_revision_tracks_geometry_changes_not_occupancy() {
    let (graph, edge_idx) = make_straight_road();
    let mut z = make_zoning();
    let residential = z
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();
    let commercial = z
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Commercial)
        .unwrap();

    let initial_revision = z.overlay_revision();
    let parcel_id = z
        .place_or_rezone_default_parcel_at(0.0, -20.0, residential, &graph)
        .expect("parcel");
    let placed_revision = z.overlay_revision();
    assert_ne!(placed_revision, initial_revision);

    z.place_or_rezone_default_parcel_at(0.0, -20.0, residential, &graph)
        .expect("same profile");
    assert_eq!(z.overlay_revision(), placed_revision);

    z.place_or_rezone_default_parcel_at(0.0, -20.0, commercial, &graph)
        .expect("rezone");
    let rezoned_revision = z.overlay_revision();
    assert_ne!(rezoned_revision, placed_revision);
    let occupancy_revision = z.overlay_occupancy_revision();

    assert!(z.occupy_parcel(parcel_id.raw(), 11));
    assert_eq!(z.overlay_revision(), rezoned_revision);
    let occupied_revision = z.overlay_occupancy_revision();
    assert_ne!(occupied_revision, occupancy_revision);
    assert!(!z.occupy_parcel(parcel_id.raw(), 12));
    assert_eq!(z.overlay_revision(), rezoned_revision);
    assert_eq!(z.overlay_occupancy_revision(), occupied_revision);

    assert!(z.clear_parcel_occupancy(parcel_id.raw()));
    assert_eq!(z.overlay_revision(), rezoned_revision);
    let cleared_revision = z.overlay_occupancy_revision();
    assert_ne!(cleared_revision, occupied_revision);
    assert!(!z.clear_parcel_occupancy(parcel_id.raw()));
    assert_eq!(z.overlay_revision(), rezoned_revision);
    assert_eq!(z.overlay_occupancy_revision(), cleared_revision);

    assert_eq!(z.remove_parcels_attached_to_edge(edge_idx), 1);
    assert_ne!(z.overlay_revision(), rezoned_revision);
}
