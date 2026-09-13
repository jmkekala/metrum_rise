// SPDX-License-Identifier: GPL-2.0-only

//! Zone, vacancy, and incremental-index consistency tests.

use super::support::*;
use super::*;

#[test]
fn nearest_building_pick_preserves_radius_ties_and_index_refresh() {
    for boundary in [-512.0, 0.0, 512.0] {
        let mut allocator = BuildingAllocator::new();
        for x in [boundary + 1.0, boundary - 1.0] {
            let mut building = indexed_test_building(String::new(), ZoneType::Residential, 0);
            building.center_x = x;
            allocator.buildings.push(building);
        }
        // The lower ID lies in the later-visited chunk; broken/unfinished buildings are pickable.
        allocator.buildings[0].broken = true;
        allocator.buildings[0].construction_remaining_hours = 1;
        assert_eq!(allocator.nearest_building_idx_at(boundary, 0.0), Some(0));
        assert_eq!(
            allocator.nearest_building_idx_at(boundary + 31.0, 0.0),
            None
        );
        assert_eq!(
            allocator.nearest_building_idx_at(boundary + 30.5, 0.0),
            Some(0)
        );

        allocator.buildings.swap_remove(0);
        allocator.dirty_index = true;
        assert_eq!(allocator.nearest_building_idx_at(boundary, 0.0), Some(0));
        allocator.buildings[0].center_x = boundary + 512.0;
        allocator.dirty_index = true;
        assert_eq!(allocator.nearest_building_idx_at(boundary, 0.0), None);
        assert_eq!(
            allocator.nearest_building_idx_at(boundary + 512.0, 0.0),
            Some(0)
        );

        allocator.dirty_index = true;
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(allocator.nearest_building_idx_at(invalid, 0.0), None);
            assert_eq!(allocator.nearest_building_idx_at(0.0, invalid), None);
        }
        assert!(
            allocator.dirty_index,
            "invalid input must not rebuild indices"
        );
    }
}

#[test]
#[ignore = "release building-pick locality benchmark"]
fn benchmark_building_pick_locality() {
    use std::hint::black_box;
    use std::time::Instant;

    for background in [0, 1_024, 65_536] {
        let mut allocator = BuildingAllocator::new();
        for idx in 0..32 + background {
            let mut building = indexed_test_building(String::new(), ZoneType::Residential, idx);
            if idx < 32 {
                building.center_x = 508.0 + (idx % 8) as f32;
                building.center_y = (idx / 8) as f32 * 8.0;
            } else {
                building.center_x = 20_000.0 + (idx % 256) as f32 * 16.0;
                building.center_y = 20_000.0 + (idx / 256) as f32 * 16.0;
            }
            allocator.buildings.push(building);
        }
        // Measure clean local queries separately from cold index construction.
        allocator.rebuild_zone_index();
        assert_eq!(allocator.nearest_building_idx_at(512.0, 0.0), Some(4));
        let mut samples = Vec::new();
        for sample in 0..14 {
            let start = Instant::now();
            for _ in 0..2_000 {
                black_box(allocator.nearest_building_idx_at(black_box(512.0), black_box(0.0)));
            }
            if sample >= 3 {
                samples.push(start.elapsed().as_secs_f64() * 1e9 / 2_000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        assert_eq!(allocator.nearest_building_idx_at(512.0, 0.0), Some(4));
        eprintln!(
            "building_pick: local=32 background={background} median_ns={:.3}",
            samples[samples.len() / 2]
        );
    }
}

#[test]
fn building_site_raycast_prepares_and_uses_the_chunk_index() {
    let mut allocator = BuildingAllocator::new();
    let mut near = indexed_test_building(String::new(), ZoneType::Residential, 0);
    near.support_height_m = 2.0;
    let mut far = indexed_test_building(String::new(), ZoneType::Residential, 1);
    far.center_x = RegionGraph::CHUNK_SIZE * 4.0;
    far.support_height_m = 8.0;
    allocator.buildings = vec![near, far];

    allocator.prepare_building_site_query_index(10.0);

    assert_eq!(allocator.building_sites.len(), 2);
    assert_eq!(
        allocator.site_candidate_indices_for_bounds(-1.0, -1.0, 1.0, 1.0),
        vec![0]
    );
    let hit = allocator
        .raycast_building_site_surface(
            Vector3::new(0.0, 20.0, 0.0),
            Vector3::DOWN,
            (-4096.0, -4096.0, 4096.0, 4096.0),
        )
        .expect("indexed near site should be raycastable");
    assert!((hit.y - 2.0).abs() <= f32::EPSILON);
}

#[test]
fn test_zone_index_consistency() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "zone_index_res",
        ZoneClass::Residential,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "zone_index_com",
        ZoneClass::Commercial,
    );

    for i in 0..10 {
        let (asset, zone) = if i % 2 == 0 {
            (&residential_asset, ZoneType::Residential)
        } else {
            (&commercial_asset, ZoneType::Commercial)
        };
        let mut building = indexed_test_building(asset.clone(), zone, i);
        building.center_x = i as f32;
        building.parcel_id = 0;
        allocator.buildings.push(building);
    }
    allocator.dirty_index = true;
    allocator.rebuild_zone_index();

    assert_eq!(
        allocator.zone_index[zone_bucket(ZoneType::Residential)].len(),
        5
    );
    assert_eq!(
        allocator.zone_index[zone_bucket(ZoneType::Commercial)].len(),
        5
    );

    allocator.buildings.swap_remove(0);
    allocator.dirty_index = true;
    allocator.rebuild_zone_index();

    assert_eq!(allocator.buildings.len(), 9);
    assert_eq!(
        allocator.zone_index[zone_bucket(ZoneType::Residential)].len(),
        4
    );
    assert_eq!(
        allocator.zone_index[zone_bucket(ZoneType::Commercial)].len(),
        5
    );
}

#[test]
fn test_vacancy_index_consistency() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "base",
        "b.res.vacancy",
        ZoneClass::Residential,
    );

    for i in 0..5 {
        let mut building =
            indexed_test_building(residential_asset.clone(), ZoneType::Residential, i);
        building.center_x = i as f32;
        building.parcel_id = 0;
        allocator.buildings.push(building);
    }
    allocator.rebuild_zone_index();

    assert_eq!(
        allocator.vacancy_index[zone_bucket(ZoneType::Residential)].len(),
        5
    );

    for _ in 0..5 {
        allocator.claim_vacancy(0);
    }
    assert_eq!(
        allocator.vacancy_index[zone_bucket(ZoneType::Residential)].len(),
        5
    );
    allocator.claim_vacancy(0);

    assert_eq!(
        allocator.vacancy_index[zone_bucket(ZoneType::Residential)].len(),
        4
    );
    assert!(!allocator.vacancy_index[zone_bucket(ZoneType::Residential)].contains(&0));

    allocator.release_vacancy(0);
    assert_eq!(
        allocator.vacancy_index[zone_bucket(ZoneType::Residential)].len(),
        5
    );
    assert!(allocator.vacancy_index[zone_bucket(ZoneType::Residential)].contains(&0));

    allocator.buildings.swap_remove(1);
    allocator.rebuild_zone_index();

    assert_eq!(allocator.buildings.len(), 4);
    assert_eq!(
        allocator.zone_index[zone_bucket(ZoneType::Residential)].len(),
        4
    );
    assert_eq!(
        allocator.vacancy_index[zone_bucket(ZoneType::Residential)].len(),
        4
    );
}

#[test]
fn test_index_appended_building_matches_full_rebuild() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "base",
        "b.res.incremental_index",
        ZoneClass::Residential,
    );

    allocator.buildings.push(indexed_test_building(
        residential_asset.clone(),
        ZoneType::Residential,
        0,
    ));
    allocator.rebuild_zone_index();

    let mut expected = allocator.clone();
    allocator.buildings.push(indexed_test_building(
        residential_asset.clone(),
        ZoneType::Residential,
        1,
    ));
    expected.buildings.push(indexed_test_building(
        residential_asset,
        ZoneType::Residential,
        1,
    ));

    assert!(allocator.index_appended_building(1));
    expected.rebuild_zone_index();

    assert!(!allocator.dirty_index);
    assert_eq!(allocator.zone_index, expected.zone_index);
    assert_eq!(allocator.vacancy_index, expected.vacancy_index);
    assert_eq!(allocator.vacancy_pos, expected.vacancy_pos);
    assert_eq!(allocator.building_chunks, expected.building_chunks);
    assert_eq!(
        allocator.max_lot_radius_cells,
        expected.max_lot_radius_cells
    );
}

#[test]
fn test_construction_completion_enables_capacity_and_vacancy_indexing() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "base",
        "b.res.construction",
        ZoneClass::Residential,
    );

    let mut building = indexed_test_building(residential_asset, ZoneType::Residential, 0);
    building.construction_total_hours = 2;
    building.construction_remaining_hours = 2;
    allocator.buildings.push(building);
    allocator.rebuild_zone_index();

    assert_eq!(allocator.household_capacity(0), 0);
    assert_eq!(
        allocator.zone_index[zone_bucket(ZoneType::Residential)].len(),
        0
    );
    assert_eq!(
        allocator.vacancy_index[zone_bucket(ZoneType::Residential)].len(),
        0
    );

    allocator.advance_construction_hour();
    assert_eq!(allocator.buildings[0].construction_remaining_hours, 1);
    assert_eq!(allocator.household_capacity(0), 0);

    allocator.advance_construction_hour();
    assert!(!allocator.buildings[0].is_under_construction());
    assert_eq!(allocator.buildings[0].construction_total_hours, 0);
    assert_eq!(allocator.household_capacity(0), 6);
    assert_eq!(
        allocator.zone_index[zone_bucket(ZoneType::Residential)].len(),
        1
    );
    assert_eq!(
        allocator.vacancy_index[zone_bucket(ZoneType::Residential)].len(),
        1
    );
}
