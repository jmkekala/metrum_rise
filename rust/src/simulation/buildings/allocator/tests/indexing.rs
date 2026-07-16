//! Zone, vacancy, and incremental-index consistency tests.

use super::support::*;
use super::*;

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
    let mut rng = rand::rngs::StdRng::seed_from_u64(42);
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
        allocator.buildings.push(Building {
            center_x: i as f32,
            center_y: 0.0,
            support_height_m: 0.0,
            width_cells: 3,
            depth_cells: 3,
            zone_profile_runtime_id: 0,
            parcel_id: 0,
            zone_type: if i % 2 == 0 {
                ZoneType::Residential
            } else {
                ZoneType::Commercial
            },
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.5,
            side_offset: 0.0,
            is_deserted: false,
            budget_distress: false,
            edge_idx: 0,
            side: 1,
            cell_x: i,
            cell_y: 0,
            occupancy: 0,
            worker_count: 0,
            service_funding_override: -1.0,
            asset_id: if i % 2 == 0 {
                residential_asset.clone()
            } else {
                commercial_asset.clone()
            },
            level: 1,
            construction_total_hours: 0,
            construction_remaining_hours: 0,
            broken: false,
            economy_profile_runtime_id: 0,
            economy_broken: false,
            resource_inventory: Vec::new(),
            revenue: 0.0,
            operating_budget: 500.0,
            profit_tax_budget_baseline: 500.0,
            last_day_profit: 0.0,
            shipment_cooldown_hours: 0,
            daily_owa_input_value: 0.0,
            daily_local_input_value: 0.0,
            daily_city_funded_input_cost: 0.0,
            daily_household_sales_value: 0.0,
            daily_power_service_units: 0.0,
            daily_power_served_units: 0.0,
            recent_power_service_units: 0.0,
            recent_power_served_units: 0.0,
            recent_household_sales_value: 0.0,
            commercial_activity_floor_scale: 0.0,
            pending_redevelopment: false,
            rezone_grace_days_remaining: 0,
        });
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

    let pick = allocator.get_random_building_by_zone(ZoneType::Commercial, &mut rng);
    assert!(pick.is_some());
    assert_eq!(
        allocator.buildings[pick.unwrap()].zone_type,
        ZoneType::Commercial
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
    let _rng = rand::rngs::StdRng::seed_from_u64(42);

    for i in 0..5 {
        allocator.buildings.push(Building {
            center_x: i as f32,
            center_y: 0.0,
            support_height_m: 0.0,
            width_cells: 3,
            depth_cells: 3,
            zone_profile_runtime_id: 0,
            parcel_id: 0,
            zone_type: ZoneType::Residential,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.5,
            side_offset: 0.0,
            is_deserted: false,
            budget_distress: false,
            edge_idx: 0,
            side: 1,
            cell_x: i,
            cell_y: 0,
            occupancy: 0,
            worker_count: 0,
            service_funding_override: -1.0,
            asset_id: residential_asset.clone(),
            level: 1,
            construction_total_hours: 0,
            construction_remaining_hours: 0,
            broken: false,
            economy_profile_runtime_id: 0,
            economy_broken: false,
            resource_inventory: Vec::new(),
            revenue: 0.0,
            operating_budget: 500.0,
            profit_tax_budget_baseline: 500.0,
            last_day_profit: 0.0,
            shipment_cooldown_hours: 0,
            daily_owa_input_value: 0.0,
            daily_local_input_value: 0.0,
            daily_city_funded_input_cost: 0.0,
            daily_household_sales_value: 0.0,
            daily_power_service_units: 0.0,
            daily_power_served_units: 0.0,
            recent_power_service_units: 0.0,
            recent_power_served_units: 0.0,
            recent_household_sales_value: 0.0,
            commercial_activity_floor_scale: 0.0,
            pending_redevelopment: false,
            rezone_grace_days_remaining: 0,
        });
    }
    allocator.rebuild_zone_index();

    assert_eq!(
        allocator.vacancy_index[zone_bucket(ZoneType::Residential)].len(),
        5
    );

    allocator.claim_vacancy(0);
    allocator.claim_vacancy(0);
    allocator.claim_vacancy(0);
    allocator.claim_vacancy(0);
    allocator.claim_vacancy(0);
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

    let mut agents = AgentSystem::new();
    for _ in 0..5 {
        agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
    }

    let last_idx = allocator.buildings.len() - 1;
    let i = 1;
    let mut mapping = std::collections::HashMap::new();
    mapping.insert(last_idx, i);
    agents.remap_building_indices(&mapping);

    allocator.buildings.swap_remove(i);
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

    allocator.buildings.push(Building {
        center_x: 0.0,
        center_y: 0.0,
        support_height_m: 0.0,
        width_cells: 3,
        depth_cells: 3,
        zone_profile_runtime_id: 0,
        parcel_id: 0,
        zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(0.0, 1.0),
        frontage_t: 0.5,
        side_offset: 0.0,
        is_deserted: false,
        budget_distress: false,
        edge_idx: 0,
        side: 1,
        cell_x: 0,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        service_funding_override: -1.0,
        asset_id: residential_asset,
        level: 1,
        construction_total_hours: 2,
        construction_remaining_hours: 2,
        broken: false,
        economy_profile_runtime_id: 0,
        economy_broken: false,
        resource_inventory: Vec::new(),
        revenue: 0.0,
        operating_budget: 500.0,
        profit_tax_budget_baseline: 500.0,
        last_day_profit: 0.0,
        shipment_cooldown_hours: 0,
        daily_owa_input_value: 0.0,
        daily_local_input_value: 0.0,
        daily_city_funded_input_cost: 0.0,
        daily_household_sales_value: 0.0,
        daily_power_service_units: 0.0,
        daily_power_served_units: 0.0,
        recent_power_service_units: 0.0,
        recent_power_served_units: 0.0,
        recent_household_sales_value: 0.0,
        commercial_activity_floor_scale: 0.0,
        pending_redevelopment: false,
        rezone_grace_days_remaining: 0,
    });
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
