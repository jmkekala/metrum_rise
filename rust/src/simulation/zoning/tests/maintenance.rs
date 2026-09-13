// SPDX-License-Identifier: GPL-2.0-only

//! Zoning maintenance behavior tests.

use super::helpers::{make_straight_road, make_zoning};
use crate::simulation::zoning::parcels::{ParcelGeometry, geometry_from_attachment};
use crate::simulation::zoning::{ParcelId, ZoneType};
use godot::prelude::{Vector2, Vector3};
use std::collections::HashSet;

fn translated_geometry(mut geometry: ParcelGeometry, offset: Vector2) -> ParcelGeometry {
    geometry.front_center += offset;
    geometry.center += offset;
    geometry.aabb_min += offset;
    geometry.aabb_max += offset;
    for corner in &mut geometry.corners {
        *corner += offset;
    }
    geometry
}

#[test]
fn parcel_geometry_replacement_preserves_storage_order_and_occupancy() {
    let (graph, edge) = make_straight_road();
    let base = geometry_from_attachment(&graph, edge, 1, 0.5, 5.0, 5.0);
    let originals =
        [-20.0, 520.0, 1_040.0].map(|x| translated_geometry(base, Vector2::new(x, 0.0)));
    let ids = [90, 3, 70].map(ParcelId::from_raw);
    let mut zoning = make_zoning();
    for (idx, (&id, &geometry)) in ids.iter().zip(&originals).enumerate() {
        // Loaded IDs need not follow storage order. Overlapping store fixtures expose pick ties.
        zoning.parcels.insert_loaded(id, geometry, 0);
        assert!(zoning.occupy_parcel(id.raw(), idx + 7));
    }
    assert!(!zoning.parcels.replace_geometry(ParcelId::NONE, base));
    assert!(zoning.parcels.replace_geometry(ids[0], originals[1]));
    assert!(zoning.parcel_at(originals[0].center).is_none());
    assert_eq!(zoning.parcel_at(originals[1].center).unwrap().id(), ids[0]);
    assert_eq!(
        zoning.parcels.find_touching_segment(
            originals[1].center - Vector2::new(1.0, 0.0),
            originals[1].center + Vector2::new(1.0, 0.0)
        ),
        vec![ids[1], ids[0]],
    );
    assert!(zoning.parcels.replace_geometry(ids[1], originals[2]));
    assert_eq!(zoning.parcel_at(originals[2].center).unwrap().id(), ids[1]);
    assert_eq!(zoning.parcel_at(originals[1].center).unwrap().id(), ids[0]);
    assert!(zoning.parcels.replace_geometry(ids[0], originals[0]));
    assert!(zoning.parcel_at(originals[1].center).is_none());
    assert!(zoning.parcels.replace_geometry(ids[1], originals[1]));
    for (idx, parcel) in zoning.parcels().iter().enumerate() {
        assert_eq!(parcel.id(), ids[idx]);
        assert_eq!(parcel.center(), originals[idx].center);
        assert_eq!(parcel.occupied_building(), Some(idx + 7));
        assert_eq!(
            zoning.parcel_at(originals[idx].center).unwrap().id(),
            ids[idx]
        );
    }
}

#[test]
#[ignore = "manual matched release locality timing of parcel geometry replacement"]
fn benchmark_parcel_geometry_replacement() {
    use std::hash::{DefaultHasher, Hash, Hasher};
    use std::hint::black_box;
    use std::time::Instant;

    let (graph, edge) = make_straight_road();
    let base = geometry_from_attachment(&graph, edge, 1, 0.5, 5.0, 5.0);
    let original = translated_geometry(base, Vector2::new(-20.0, 0.0));
    let moved = translated_geometry(base, Vector2::new(520.0, 0.0));
    for count in [1, 1_024, 65_536, 262_144] {
        let mut zoning = make_zoning();
        let id = zoning.parcels.insert_new(original, 0);
        assert!(zoning.occupy_parcel(id.raw(), 7));
        for idx in 1..count {
            let offset = Vector2::new(
                2_048.0 + (idx % 512) as f32 * 16.0,
                2_048.0 + (idx / 512) as f32 * 16.0,
            );
            zoning
                .parcels
                .insert_new(translated_geometry(base, offset), 0);
        }
        let checksum = |zoning: &crate::simulation::zoning::ZoningSystem| {
            let mut hash = DefaultHasher::new();
            for parcel in zoning.parcels() {
                (parcel.id().raw(), parcel.occupied_building()).hash(&mut hash);
                for point in parcel.corners() {
                    (point.x.to_bits(), point.y.to_bits()).hash(&mut hash);
                }
            }
            hash.finish()
        };
        let expected = checksum(&zoning);
        let occupancy_revision = zoning.overlay_occupancy_revision();
        let mut round_trip = || {
            assert!(
                zoning
                    .parcels
                    .replace_geometry(black_box(id), black_box(moved))
            );
            assert_eq!(zoning.parcel_at(black_box(moved.center)).unwrap().id(), id);
            assert!(
                zoning
                    .parcels
                    .replace_geometry(black_box(id), black_box(original))
            );
            assert_eq!(
                zoning.parcel_at(black_box(original.center)).unwrap().id(),
                id
            );
        };
        for _ in 0..3 {
            round_trip();
        }
        let mut samples = [0.0; 21];
        for sample in &mut samples {
            let start = Instant::now();
            for _ in 0..2 {
                round_trip();
            }
            *sample = start.elapsed().as_secs_f64() * 1_000.0 / 4.0;
        }
        samples.sort_by(f64::total_cmp);
        assert_eq!(checksum(&zoning), expected);
        assert_eq!(zoning.overlay_occupancy_revision(), occupancy_revision);
        eprintln!(
            "parcel_geometry_replacement parcels={count} median_ms={:.9} checksum={expected}",
            samples[10]
        );
    }
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
    let opposite_side = [Vector3::ZERO, Vector3::new(0.0, 0.0, 80.0)];
    assert!(
        z.parcel_ids_overlapping_road_corridor(&opposite_side, 5.0)
            .is_empty(),
        "a branch leaving on the opposite side must not hit the parcel"
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

    z.remap_parcel_occupancy(parcel_id.raw(), 11, 12);
    assert_eq!(
        z.parcel_by_raw_id(parcel_id.raw())
            .unwrap()
            .occupied_building(),
        Some(12)
    );
    assert_eq!(z.overlay_revision(), rezoned_revision);
    let remapped_revision = z.overlay_occupancy_revision();
    assert_ne!(remapped_revision, occupied_revision);
    z.remap_parcel_occupancy(parcel_id.raw(), 11, 13);
    assert_eq!(
        z.parcel_by_raw_id(parcel_id.raw())
            .unwrap()
            .occupied_building(),
        Some(12)
    );
    assert_eq!(z.overlay_occupancy_revision(), remapped_revision);
    z.remap_parcel_occupancy(parcel_id.raw(), 12, 11);
    assert_eq!(
        z.parcel_by_raw_id(parcel_id.raw())
            .unwrap()
            .occupied_building(),
        Some(11)
    );

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

#[test]
#[ignore = "manual matched release locality timing of parcel occupancy remapping"]
fn benchmark_parcel_occupancy_remap() {
    use std::hash::{DefaultHasher, Hash, Hasher};
    use std::hint::black_box;
    use std::time::Instant;

    let (graph, edge) = make_straight_road();
    let local_geometry = geometry_from_attachment(&graph, edge, 1, 0.5, 5.0, 5.0);
    for count in [1, 1_024, 65_536, 262_144] {
        let mut zoning = make_zoning();
        let parcel = zoning.parcels.insert_new(local_geometry, 0);
        assert!(zoning.occupy_parcel(parcel.raw(), 7));
        for idx in 1..count {
            // Replicate isolated parcel-store records in distant chunks; no gameplay placement,
            // agent creation or road construction is part of this occupancy-index measurement.
            let offset = Vector2::new(
                1_024.0 + (idx % 512) as f32 * 16.0,
                1_024.0 + (idx / 512) as f32 * 16.0,
            );
            let id = zoning
                .parcels
                .insert_new(translated_geometry(local_geometry, offset), 0);
            assert!(zoning.occupy_parcel(id.raw(), idx + 100));
        }
        let checksum = |zoning: &crate::simulation::zoning::ZoningSystem| {
            let mut hash = DefaultHasher::new();
            for parcel in zoning.parcels() {
                (parcel.id().raw(), parcel.occupied_building()).hash(&mut hash);
            }
            hash.finish()
        };
        let expected = checksum(&zoning);
        let geometry_revision = zoning.overlay_revision();
        let mut round_trip = || {
            zoning.remap_parcel_occupancy(black_box(parcel.raw()), black_box(7), black_box(13));
            zoning.remap_parcel_occupancy(black_box(parcel.raw()), black_box(13), black_box(7));
        };
        for _ in 0..3 {
            round_trip();
        }
        let mut samples = [0.0; 21];
        for sample in &mut samples {
            let start = Instant::now();
            for _ in 0..16 {
                round_trip();
            }
            *sample = start.elapsed().as_secs_f64() * 1_000.0 / 32.0;
        }
        samples.sort_by(f64::total_cmp);
        assert_eq!(checksum(&zoning), expected);
        assert_eq!(zoning.overlay_revision(), geometry_revision);
        eprintln!(
            "parcel_occupancy_remap parcels={count} median_ms={:.9} checksum={expected}",
            samples[10]
        );
    }
}
