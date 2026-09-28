// SPDX-License-Identifier: GPL-2.0-only

//! Exact road-removal inverses and relationship-index locality without gameplay setup.

use super::*;

fn geometry(edge_idx: usize, origin: Vector2) -> ParcelGeometry {
    ParcelGeometry {
        edge_idx,
        side: 1,
        frontage_center_t: 0.5,
        frontage_m: 10.0,
        depth_m: 10.0,
        front_center: origin + Vector2::new(5.0, 0.0),
        center: origin + Vector2::new(5.0, 5.0),
        tangent: Vector2::new(1.0, 0.0),
        normal: Vector2::new(0.0, 1.0),
        corners: [
            origin,
            origin + Vector2::new(10.0, 0.0),
            origin + Vector2::new(10.0, 10.0),
            origin + Vector2::new(0.0, 10.0),
        ],
        aabb_min: origin,
        aabb_max: origin + Vector2::new(10.0, 10.0),
    }
}

fn assert_indices(store: &ParcelStore) {
    let mut rebuilt = ParcelStore::default();
    for parcel in store.parcels() {
        assert!(rebuilt.restore_local(parcel.clone()));
    }
    assert_eq!(store.id_to_index, rebuilt.id_to_index);
    assert_eq!(store.chunk_index, rebuilt.chunk_index);
    for edge in store
        .parcels()
        .iter()
        .map(ZoningParcel::edge_idx)
        .collect::<HashSet<_>>()
    {
        let expected: Vec<_> = store
            .parcels()
            .iter()
            .enumerate()
            .filter(|(_, p)| removed_with_road(p, edge))
            .map(|(i, p)| (i, p.id()))
            .collect();
        let actual: Vec<_> = store
            .capture_attached_to_edge(edge)
            .iter()
            .map(|(i, p)| (*i, p.id()))
            .collect();
        assert_eq!(actual, expected);
    }
    for parcel in store.parcels() {
        assert_eq!(
            store.find_at_point(parcel.center()),
            rebuilt.find_at_point(parcel.center())
        );
    }
}

#[test]
fn every_road_removal_subset_restores_dense_order_and_local_indices() {
    for mask in 0_u16..256 {
        let mut store = ParcelStore::default();
        let ids: Vec<_> = (0..8).map(|i| ParcelId::from_raw(100 - i * 7)).collect();
        for (i, &id) in ids.iter().enumerate() {
            let edge = if mask & (1 << i) != 0 { 7 } else { 11 };
            // Equal rectangles expose storage-order picking independently of id order.
            store.insert_loaded(id, geometry(edge, Vector2::ZERO), 1);
            if i % 2 == 0 {
                assert!(store.set_occupied_building(id, i));
            }
        }
        let captured = store.capture_attached_to_edge(7);
        assert_eq!(captured.len(), mask.count_ones() as usize);
        assert_eq!(store.remove_attached_to_edge(7, |_| {}), captured.len());
        assert_indices(&store);
        assert!(store.capture_attached_to_edge(7).is_empty());
        assert!(store.can_restore_removed(ids.len(), &captured));
        store.restore_removed(ids.len(), captured);
        assert_eq!(
            store
                .parcels()
                .iter()
                .map(ZoningParcel::id)
                .collect::<Vec<_>>(),
            ids
        );
        for (i, parcel) in store.parcels().iter().enumerate() {
            assert_eq!(parcel.occupied_building(), (i % 2 == 0).then_some(i));
        }
        assert_indices(&store);
    }
}

#[test]
fn attachment_replacement_local_removal_and_bulk_cleanup_keep_query_views() {
    let mut store = ParcelStore::default();
    let ids: Vec<_> = (0..5)
        .map(|_| store.insert_new(geometry(7, Vector2::ZERO), 1))
        .collect();
    let mut changed = geometry(19, Vector2::new(12000.0, -8000.0));
    assert!(store.replace_geometry(ids[1], changed));
    assert_eq!(store.capture_attached_to_edge(19)[0].1.id(), ids[1]);
    assert_indices(&store);
    // A relationship change in the same spatial chunks must update its index too.
    changed.edge_idx = 23;
    assert!(store.replace_geometry(ids[1], changed));
    assert!(store.capture_attached_to_edge(19).is_empty());
    assert_indices(&store);
    let removed = store.remove_local(ids[0]).unwrap();
    assert_indices(&store);
    assert!(store.restore_local(removed));
    assert_indices(&store);
    assert_eq!(
        store.remove_ids(&HashSet::from([ids[2], ids[3]]), |_| {}),
        2
    );
    assert_indices(&store);
    store.clear();
    assert!(store.capture_attached_to_edge(7).is_empty());
    assert!(store.capture_attached_to_edge(23).is_empty());
    store.insert_new(geometry(19, Vector2::ZERO), 1);
    assert_indices(&store);
}

#[test]
#[ignore = "matched release road-parcel capture/removal/undo; background setup excluded"]
fn benchmark_road_parcel_removal_locality() {
    use std::hint::black_box;
    use std::time::Instant;

    for count in [0, 1_024, 10_000, 100_000] {
        let mut store = ParcelStore::default();
        let local: Vec<_> = (0..6)
            .map(|i| store.insert_new(geometry(7, Vector2::new(i as f32 * 20.0, 0.0)), 1))
            .collect();
        for i in 0..count {
            let point = Vector2::new(
                2000.0 + (i % 128) as f32 * 20.0,
                2000.0 + (i / 128) as f32 * 20.0,
            );
            store.insert_new(geometry(100 + i / 32, point), 1);
        }
        let run = |store: &mut ParcelStore| {
            let before = store.parcels().len();
            let undo = store.capture_attached_to_edge(7);
            assert_eq!(undo.len(), 6);
            assert_eq!(
                store.remove_attached_to_edge(7, |parcel| {
                    black_box(parcel.corners());
                }),
                6
            );
            assert!(store.can_restore_removed(before, &undo));
            store.restore_removed(before, undo);
        };
        for _ in 0..4 {
            run(&mut store);
        }
        let mut samples = [0.0_f64; 15];
        for sample in &mut samples {
            let start = Instant::now();
            for _ in 0..8 {
                run(black_box(&mut store));
            }
            *sample = start.elapsed().as_secs_f64() * 1e6 / 8.0;
        }
        samples.sort_unstable_by(f64::total_cmp);
        assert_eq!(store.parcels().len(), count + 6);
        assert_eq!(
            store.parcels()[..6]
                .iter()
                .map(ZoningParcel::id)
                .collect::<Vec<_>>(),
            local
        );
        assert_indices(&store);
        println!(
            "background_parcels={count} local_parcels=6 removal_undo_us={:.3}",
            samples[7]
        );
    }
}
