// SPDX-License-Identifier: GPL-2.0-only

//! Zone, vacancy, and incremental-index consistency tests.

use super::support::*;
use super::*;

#[test]
fn explicit_cell_site_summary_tracks_append_rebuild_and_clear() {
    let mut allocator = BuildingAllocator::new();
    allocator.buildings.push(indexed_test_building(
        String::new(),
        ZoneType::Residential,
        0,
    ));
    allocator.rebuild_building_site_clients(10.0);
    allocator.rebuild_zone_index();
    assert!(!allocator.has_explicit_cell_sites());
    let mut explicit = indexed_test_building(String::new(), ZoneType::None, 0);
    explicit.set_construction_hours_left(8);
    allocator.buildings.push(explicit);
    allocator.rebuild_building_site_clients(10.0);
    assert!(allocator.index_appended_building(1));
    assert_eq!(allocator.explicit_cell_site_count, 1);
    assert!(allocator.has_explicit_cell_sites());
    allocator.buildings[1].parcel_id = 7;
    allocator.dirty_index = true;
    assert!(
        allocator.has_explicit_cell_sites(),
        "stale metadata must not exclude a candidate"
    );
    allocator.rebuild_zone_index();
    assert!(!allocator.has_explicit_cell_sites());
    allocator.buildings[1].parcel_id = 0;
    allocator.dirty_index = true;
    allocator.rebuild_zone_index();
    assert!(allocator.has_explicit_cell_sites());
    allocator.clear();
    assert_eq!(allocator.explicit_cell_site_count, 0);
    assert!(!allocator.has_explicit_cell_sites());
}

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
        allocator.buildings[0].set_construction_hours_left(1);
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
    building.set_construction_hours_left(2);
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

    assert_eq!(allocator.buildings[0].construction_hours_left(0), 2);
    assert_eq!(
        allocator.buildings[0].construction_hours_left(SLOTS_PER_HOUR + 1),
        1
    );
    assert_eq!(allocator.household_capacity(0), 0);

    allocator.complete_construction(0);
    assert!(
        !allocator.dirty_index,
        "completion updates a clean index in place"
    );
    let mut expected = allocator.clone();
    expected.rebuild_zone_index();
    assert_eq!(allocator.zone_index, expected.zone_index);
    assert_eq!(allocator.vacancy_index, expected.vacancy_index);
    assert_eq!(allocator.vacancy_pos, expected.vacancy_pos);
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

#[test]
fn test_admission_order_matches_vacancy_scan() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "base",
        "b.res.admission",
        ZoneClass::Residential,
    );
    for i in 0..40 {
        let mut building =
            indexed_test_building(residential_asset.clone(), ZoneType::Residential, i);
        building.parcel_id = 0;
        building.level = 1 + (i % 3) as u8;
        allocator.buildings.push(building);
    }
    allocator.rebuild_zone_index();

    let residential = zone_bucket(ZoneType::Residential);
    let mut seed = 0x9E37_79B9_u64;
    for step in 0..2_000usize {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let roll = (seed >> 33) as usize;
        let building_idx = roll / 8 % allocator.buildings.len();
        match roll % 8 {
            0..=3 => allocator.claim_vacancy(building_idx),
            4 | 5 => allocator.release_vacancy(building_idx),
            6 if step % 50 == 0 => allocator.mark_building_deserted(building_idx),
            6 => {
                let idx = allocator.buildings.len();
                let mut building = indexed_test_building(
                    residential_asset.clone(),
                    ZoneType::Residential,
                    idx as i32,
                );
                building.parcel_id = 0;
                building.level = 1 + (idx % 3) as u8;
                allocator.buildings.push(building);
                assert!(allocator.index_appended_building(idx));
            }
            _ if step % 200 == 0 => allocator.rebuild_zone_index(),
            _ => {
                allocator.buildings[building_idx].pending_redevelopment ^= true;
                allocator.admission_occupancy_changed(building_idx);
            }
        }
        assert!(!allocator.dirty_index);
        for max_level in 1..=3 {
            let scanned = allocator.vacancy_index[residential]
                .iter()
                .copied()
                .filter(|&idx| {
                    let home = &allocator.buildings[idx];
                    home.level <= max_level
                        && !home.pending_redevelopment
                        && allocator.household_capacity(idx) > home.occupancy
                })
                .min_by_key(|&idx| (std::cmp::Reverse(allocator.buildings[idx].level), idx));
            assert_eq!(
                allocator.first_rehousing_vacancy(|level| level <= max_level),
                scanned,
                "rehousing after mutation {step}"
            );
        }
        assert!(
            allocator
                .admission_order
                .covers(allocator.vacancy_index[residential].len())
        );
        for household_id in [step, step * 7 + 3] {
            for prefer_worker_capable in [false, true] {
                assert_eq!(
                    allocator.next_household_admission_candidate_for_household(
                        household_id,
                        prefer_worker_capable,
                    ),
                    allocator.scan_household_admission_candidate(
                        &allocator.vacancy_index[residential],
                        household_id,
                        prefer_worker_capable,
                    ),
                    "step {step}, household {household_id}, worker {prefer_worker_capable}"
                );
            }
        }
    }
}

// The chunk lists hold each chunk's buildings in building order, whether a rebuild fills them
// over the dense grid of the chunk bounds or, for chunks spread far apart, the map directly
// (`SIM-02-K`).
#[test]
fn rebuilt_chunk_lists_hold_each_chunks_buildings_in_building_order() {
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "base",
        "b.res.chunk_lists",
        ZoneClass::Residential,
    );
    let centers = [
        (3000.0, 0.0),
        (10.0, -900.0),
        (3000.0, 0.0),
        (-700.0, 40.0),
        (10.0, 5.0),
    ];
    for (idx, (x, z)) in centers.into_iter().enumerate() {
        let mut building = indexed_test_building(asset.clone(), ZoneType::Residential, idx as i32);
        building.center_x = x;
        building.center_y = z;
        allocator.buildings.push(building);
    }
    let direct = |allocator: &BuildingAllocator| {
        let mut chunks = std::collections::HashMap::<(i32, i32), Vec<usize>>::new();
        for (idx, b) in allocator.buildings.iter().enumerate() {
            let chunk = RegionGraph::get_chunk_coords(Vector3::new(b.center_x, 0.0, b.center_y));
            chunks.entry(chunk).or_default().push(idx);
        }
        chunks
    };
    allocator.rebuild_zone_index();
    assert_eq!(allocator.building_chunks, direct(&allocator));
    assert_eq!(allocator.building_chunks.len(), 4);
    // Far apart, the grid would hold more than four cells a building; then back again.
    allocator.buildings[4].center_x = 4.0e6;
    allocator.rebuild_zone_index();
    assert_eq!(allocator.building_chunks, direct(&allocator));
    allocator.buildings[4].center_x = 3000.0;
    allocator.rebuild_zone_index();
    assert_eq!(allocator.building_chunks, direct(&allocator));
    assert_eq!(allocator.building_chunks.len(), 3);
}

// `SIM-02-K`: a zone index kept across the swap-removes of building removals equals a rebuild:
// the zone, chunk and vacancy lists, the building order a rebuild gives the vacancy lists
// (`ALLOC-03`) after claims took homes out of it, the support maximum and chunk bounds a removed
// building held, and the admission order.
#[test]
fn a_zone_index_kept_across_removals_equals_a_rebuild() {
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::economy::agents::AgentSystem;
    use crate::simulation::economy::households::HouseholdSystem;
    use crate::simulation::economy::logistics::ShipmentSystem;
    use crate::simulation::zoning::ZoningSystem;

    let mut allocator = BuildingAllocator::new();
    let residential = register_test_asset(
        &mut allocator,
        "base",
        "b.res.kept_index",
        ZoneClass::Residential,
    );
    let commercial = register_test_asset(
        &mut allocator,
        "base",
        "b.com.kept_index",
        ZoneClass::Commercial,
    );
    for i in 0..24 {
        let (asset, zone) = if i % 3 == 0 {
            (&commercial, ZoneType::Commercial)
        } else {
            (&residential, ZoneType::Residential)
        };
        let mut building = indexed_test_building(asset.clone(), zone, i);
        // Six chunks along x; the last building alone in a far chunk, with the highest support.
        building.center_x = (i % 6) as f32 * 600.0 + 10.0;
        building.center_y = if i == 23 { 3_000.0 } else { 10.0 };
        building.support_height_m = if i == 23 { 9.0 } else { (i % 4) as f32 };
        building.parcel_id = 0;
        allocator.buildings.push(building);
    }
    allocator.rebuild_building_site_clients(10.0);
    allocator.rebuild_zone_index();
    // Filling homes 4 and 1 and freeing 1 again takes them out of building order.
    for building_idx in [4, 4, 4, 4, 4, 4, 1, 1, 1, 1, 1, 1, 7] {
        allocator.claim_vacancy(building_idx);
    }
    allocator.release_vacancy(1);
    let slot = zone_bucket(ZoneType::Residential);
    assert!(!allocator.vacancy_index[slot].is_sorted());

    let mut zoning = ZoningSystem::new(&WorldConfig::default());
    let mut agents = AgentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut logistics = ShipmentSystem::new();
    // The last building, then buildings the last one replaces: a home, a store, a full home.
    for removed in [23, 5, 0, 4] {
        assert!(allocator.remove_building_for_bulldoze(
            removed,
            &mut zoning,
            &mut agents,
            &mut households,
            &mut logistics,
            &mut 0.0
        ));
        assert!(allocator.zone_index_kept_for_test());
    }
    let mut rebuilt = allocator.clone();
    rebuilt.rebuild_zone_index();
    allocator.rebuild_zone_index_after_removals();

    assert!(!allocator.dirty_index);
    assert_eq!(allocator.zone_index, rebuilt.zone_index);
    assert_eq!(allocator.vacancy_index, rebuilt.vacancy_index);
    assert_eq!(allocator.vacancy_pos, rebuilt.vacancy_pos);
    assert_eq!(allocator.building_chunks, rebuilt.building_chunks);
    assert_eq!(
        allocator.building_chunk_bounds,
        rebuilt.building_chunk_bounds
    );
    assert_eq!(
        allocator.max_building_support_m.to_bits(),
        rebuilt.max_building_support_m.to_bits()
    );
    assert_eq!(allocator.max_building_support_m, 3.0);
    assert!(allocator.admission_order.same_as(&rebuilt.admission_order));
}
