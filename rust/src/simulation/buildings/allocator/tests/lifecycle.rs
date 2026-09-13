// SPDX-License-Identifier: GPL-2.0-only

//! Removal, immigration, and hourly admission lifecycle tests.

use super::support::*;
use super::*;

#[test]
#[ignore = "manual matched release timing of daily building compatibility maintenance"]
fn benchmark_rezone_maintenance() {
    use crate::simulation::network::graph::Edge;
    use crate::simulation::network::types::NodeType;
    use crate::simulation::zoning::parcels::geometry_from_attachment;
    use std::hash::{DefaultHasher, Hash, Hasher};
    use std::hint::black_box;
    use std::time::Instant;

    for count in [1, 1_024, 65_536, 262_144] {
        let mut allocator = BuildingAllocator::new();
        let asset = register_test_asset(&mut allocator, "test", "house", ZoneClass::Residential);
        let mut zoning = ZoningSystem::new(&WorldConfig::default());
        let mut agents = AgentSystem::new();
        let mut households = HouseholdSystem::new();
        let mut logistics = ShipmentSystem::new();
        let mut treasury = 0.0;
        let network = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let length = count as f32 * 20.0;
        let points = vec![Vector3::ZERO, Vector3::new(length, 0.0, 0.0)];
        let start = graph.add_node(points[0], NodeType::Junction);
        let end = graph.add_node(points[1], NodeType::Junction);
        let edge = graph.add_edge(Edge {
            start_node: start,
            end_node: end,
            width: 7.0,
            physical_length: length,
            geometry: points.clone(),
            physical_geometry: points,
            ..Edge::default()
        });
        for idx in 0..count {
            let pending = idx % 2 == 0;
            let zone = if pending {
                ZoneType::Commercial
            } else {
                ZoneType::Residential
            };
            let profile = zoning
                .profiles
                .default_runtime_id_for_zone_type(zone)
                .unwrap();
            let geometry = geometry_from_attachment(
                &graph,
                edge,
                1,
                (idx as f32 + 0.5) * 20.0 / length,
                20.0,
                20.0,
            );
            let mut building =
                indexed_test_building(asset.clone(), ZoneType::Residential, idx as i32);
            building.center_x = geometry.center.x;
            building.center_y = geometry.center.y;
            building.width_cells = 1;
            building.depth_cells = 1;
            building.parcel_id = zoning.parcels.insert_new(geometry, profile).raw();
            building.pending_redevelopment = pending;
            // Keep the countdown alive for all 87 measured/warmup updates; setup and expiry
            // side effects are outside this compatibility/countdown benchmark's scope.
            building.rezone_grace_days_remaining = if pending { u8::MAX } else { 0 };
            assert!(zoning.occupy_parcel(building.parcel_id, idx));
            allocator.buildings.push(building);
        }
        let revision = allocator.building_ref_revision();
        let occupancy_revision = zoning.overlay_occupancy_revision();
        let mut execute = || {
            allocator.cleanup_stale_buildings(
                1,
                black_box(&mut zoning),
                &mut agents,
                &mut households,
                &mut logistics,
                &mut treasury,
                &graph,
                &network.lane_system,
            )
        };
        for _ in 0..3 {
            execute();
        }
        let mut samples = [0.0; 21];
        for sample in &mut samples {
            let start = Instant::now();
            for _ in 0..4 {
                execute();
            }
            *sample = start.elapsed().as_secs_f64() * 1_000.0 / 4.0;
        }
        samples.sort_by(f64::total_cmp);
        assert_eq!(allocator.buildings.len(), count);
        assert_eq!(allocator.building_ref_revision(), revision);
        assert_eq!(zoning.overlay_occupancy_revision(), occupancy_revision);
        let mut hash = DefaultHasher::new();
        for (idx, building) in allocator.buildings.iter().enumerate() {
            assert_eq!(building.pending_redevelopment, idx % 2 == 0);
            assert_eq!(
                building.rezone_grace_days_remaining,
                if idx % 2 == 0 { 168 } else { 0 }
            );
            assert_eq!(
                zoning
                    .parcel_by_raw_id(building.parcel_id)
                    .unwrap()
                    .occupied_building(),
                Some(idx)
            );
            (
                building.parcel_id,
                building.rezone_grace_days_remaining,
                building.center_x.to_bits(),
                building.center_y.to_bits(),
            )
                .hash(&mut hash);
        }
        eprintln!(
            "rezone_maintenance buildings={count} median_ms={:.9} checksum={}",
            samples[10],
            hash.finish()
        );
    }
}

#[test]
fn explicit_transform_rebuild_uses_saved_frontage_on_graded_bend() {
    use crate::simulation::network::graph::Edge;
    use crate::simulation::network::types::NodeType;

    let config = WorldConfig::new(1000.0, 1000.0, 10.0, 10.0);
    let zoning = ZoningSystem::new(&config);
    let mut graph = RegionGraph::new();
    let points = vec![
        Vector3::ZERO,
        Vector3::new(200.0, 10.0, 0.0),
        Vector3::new(200.0, 20.0, 200.0),
    ];
    let start = graph.add_node(points[0], NodeType::Junction);
    let end = graph.add_node(points[2], NodeType::Junction);
    let edge_idx = graph.add_edge(Edge {
        start_node: start,
        end_node: end,
        width: 7.0,
        physical_length: graph.calculate_length(&points),
        geometry: points.clone(),
        physical_geometry: points,
        ..Edge::default()
    });
    let mut allocator = BuildingAllocator::new();
    // Equal-length graded segments put t=0.25/0.75 halfway along each arm.
    let cases = [
        (0.25, 1, Vector2::new(100.0, -20.0), Vector2::new(0.0, 1.0)),
        (0.25, -1, Vector2::new(100.0, 20.0), Vector2::new(0.0, -1.0)),
        (0.75, 1, Vector2::new(220.0, 100.0), Vector2::new(-1.0, 0.0)),
        (0.75, -1, Vector2::new(180.0, 100.0), Vector2::new(1.0, 0.0)),
    ];
    for (frontage_t, side, _, _) in cases {
        let mut building =
            indexed_test_building("test:explicit".to_owned(), ZoneType::Industrial, 0);
        building.edge_idx = edge_idx;
        building.frontage_t = frontage_t;
        building.side = side;
        building.support_height_m = 17.25;
        allocator.buildings.push(building);
    }
    allocator
        .recompute_derived_transforms(&graph, &zoning)
        .unwrap();
    for (idx, (_, _, center, facing)) in cases.into_iter().enumerate() {
        let building = &allocator.buildings[idx];
        assert!(Vector2::new(building.center_x, building.center_y).distance_to(center) < 0.0001);
        assert_eq!(building.facing_dir, facing);
        assert_eq!(building.side_offset, 5.0);
        assert_eq!(building.support_height_m, 17.25);
        assert_eq!(allocator.building_sites[idx].support_height_m, 17.25);
    }
}

fn rezoning_fixture() -> (BuildingAllocator, ZoningSystem, RegionGraph) {
    use crate::simulation::network::graph::Edge;
    use crate::simulation::network::types::NodeType;

    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(&mut allocator, "test", "house", ZoneClass::Residential);
    let mut zoning = ZoningSystem::new(&WorldConfig::default());
    let mut graph = RegionGraph::new();
    let points = vec![Vector3::ZERO, Vector3::new(100.0, 0.0, 0.0)];
    let start = graph.add_node(points[0], NodeType::Junction);
    let end = graph.add_node(points[1], NodeType::Junction);
    let edge = graph.add_edge(Edge {
        start_node: start,
        end_node: end,
        width: 7.0,
        physical_length: 100.0,
        geometry: points.clone(),
        physical_geometry: points,
        ..Edge::default()
    });
    let profile = zoning
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();
    for (idx, x) in [25.0, 75.0].into_iter().enumerate() {
        let id = zoning
            .place_or_rezone_default_parcel_at(x, -20.0, profile, &graph)
            .unwrap();
        let parcel = zoning.parcel_by_raw_id(id.raw()).unwrap();
        let center = parcel.front_center() + parcel.normal() * (zoning.config.zone_cell_m * 0.5);
        let mut building = indexed_test_building(asset.clone(), ZoneType::Residential, 0);
        building.center_x = center.x;
        building.center_y = center.y;
        building.width_cells = 1;
        building.depth_cells = 1;
        building.zone_profile_runtime_id = profile;
        building.parcel_id = id.raw();
        building.edge_idx = edge;
        building.facing_dir = -parcel.normal();
        building.frontage_t = parcel.frontage_center_t();
        building.side_offset = graph.edge(edge).width * 0.5 + crate::config::SIDEWALK_WIDTH;
        building.side = parcel.side();
        allocator.buildings.push(building);
        assert!(zoning.occupy_parcel(id.raw(), idx));
    }
    (allocator, zoning, graph)
}

#[test]
fn test_building_removal_clears_zoning_occupancy() {
    let (mut allocator, mut zoning, mut graph) = rezoning_fixture();
    let mut agents = AgentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut network = TransitNetwork::new();
    let mut logistics = ShipmentSystem::new();
    let parcel_id = allocator.buildings[0].parcel_id;
    let surviving_parcel_id = allocator.buildings[1].parcel_id;
    let center = zoning.parcel_by_raw_id(parcel_id).unwrap().center();
    let commercial_profile = zoning
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Commercial)
        .unwrap();
    assert_eq!(
        zoning
            .place_or_rezone_default_parcel_at(center.x, center.y, commercial_profile, &graph)
            .unwrap()
            .raw(),
        parcel_id
    );

    // Initial detection starts three full days; only the following daily calls consume them.
    for remaining in [3, 2, 1, 0] {
        allocator.maintain(
            1,
            &mut zoning,
            &mut agents,
            &mut households,
            &mut logistics,
            &mut 0.0,
            &mut network,
            &mut graph,
        );
        if remaining > 0 {
            assert_eq!(allocator.buildings.len(), 2);
            assert!(allocator.buildings[0].pending_redevelopment);
            assert_eq!(
                allocator.buildings[0].rezone_grace_days_remaining,
                remaining
            );
        }
    }

    assert_eq!(
        allocator.buildings.len(),
        1,
        "Only the rezoned building should be removed"
    );
    assert_eq!(
        zoning
            .parcel_by_raw_id(parcel_id)
            .unwrap()
            .occupied_building(),
        None
    );
    assert_eq!(allocator.buildings[0].parcel_id, surviving_parcel_id);
    assert_eq!(
        zoning
            .parcel_by_raw_id(surviving_parcel_id)
            .unwrap()
            .occupied_building(),
        Some(0),
        "the surviving parcel must follow the allocator swap"
    );
}

#[test]
fn test_rezoning_recovery_clears_pending_redevelopment() {
    let (mut allocator, mut zoning, mut graph) = rezoning_fixture();
    let mut agents = AgentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut network = TransitNetwork::new();
    let mut logistics = ShipmentSystem::new();
    let parcel_id = allocator.buildings[0].parcel_id;
    let center = zoning.parcel_by_raw_id(parcel_id).unwrap().center();
    for zone in [ZoneType::Commercial, ZoneType::Residential] {
        let profile = zoning
            .profiles
            .default_runtime_id_for_zone_type(zone)
            .unwrap();
        assert_eq!(
            zoning
                .place_or_rezone_default_parcel_at(center.x, center.y, profile, &graph)
                .unwrap()
                .raw(),
            parcel_id
        );
        allocator.maintain(
            0,
            &mut zoning,
            &mut agents,
            &mut households,
            &mut logistics,
            &mut 0.0,
            &mut network,
            &mut graph,
        );
        assert_eq!(allocator.buildings.len(), 2);
        let pending = zone == ZoneType::Commercial;
        assert_eq!(allocator.buildings[0].pending_redevelopment, pending);
        assert_eq!(
            allocator.buildings[0].rezone_grace_days_remaining,
            if pending { 3 } else { 0 }
        );
        assert_eq!(
            zoning
                .parcel_by_raw_id(parcel_id)
                .unwrap()
                .occupied_building(),
            Some(0)
        );
    }
}

#[test]
fn test_immigration_claims_vacant_home() {
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::economy::agents::AgentSystem;
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::zoning::{ZoneType, ZoningSystem};
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    let residential_asset_id = register_test_asset(
        &mut allocator,
        "base",
        "b.res.house",
        ZoneClass::Residential,
    );
    let map_cfg = WorldConfig::default();
    let mut zoning = ZoningSystem::new(&map_cfg);
    let mut agents = AgentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut network = TransitNetwork::new();
    let mut logistics = ShipmentSystem::new();
    let mut graph = RegionGraph::new();

    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    let edge_id = graph.edge_count() - 1;

    graph.set_node_type(0, crate::simulation::network::types::NodeType::Border);
    paint_zone_rect(
        &mut zoning,
        &graph,
        -50.0,
        -50.0,
        150.0,
        50.0,
        ZoneType::Residential,
    );
    let parcel = zoning
        .parcels()
        .iter()
        .find(|parcel| {
            zoning
                .profiles
                .zone_type_for_runtime_id(parcel.zone_profile_runtime_id())
                == ZoneType::Residential
                && parcel.is_available()
        })
        .expect("residential test parcel")
        .clone();
    let center = parcel.front_center() + parcel.normal() * (map_cfg.zone_cell_m * 0.5);
    let mut building = indexed_test_building(residential_asset_id, ZoneType::Residential, 0);
    building.center_x = center.x;
    building.center_y = center.y;
    building.width_cells = 1;
    building.depth_cells = 1;
    building.zone_profile_runtime_id = parcel.zone_profile_runtime_id();
    building.parcel_id = parcel.id().raw();
    building.facing_dir = parcel.normal();
    building.frontage_t = parcel.frontage_center_t();
    building.side_offset = 1.0;
    building.edge_idx = edge_id;
    building.side = parcel.side();
    allocator.buildings.push(building);
    zoning.occupy_parcel(parcel.id().raw(), 0);
    allocator.rebuild_zone_index();

    allocator.maintain(
        1,
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut 0.0,
        &mut network,
        &mut graph,
    );
    let expected_household_size = allocator
        .next_household_admission_candidate()
        .expect("vacant residential admission candidate")
        .1;
    allocator.execute_demand_household_admission(1, &mut agents, &network, &graph);

    assert_eq!(
        agents.len(),
        1,
        "One household should launch one arrival carrier from the demand-owned output"
    );
    assert_eq!(
        agents.home_building[0], 0,
        "Arrival carrier should reserve home index 0"
    );
    assert_eq!(
        agents.target_building[0], 0,
        "Arrival carrier should target its reserved home"
    );
    assert_eq!(
        agents.transit[0],
        crate::simulation::economy::agents::TRANSIT_IMMIGRATING,
        "Arrival carrier should start on the border-origin immigration path"
    );
    assert_eq!(
        agents.transit_mode[0],
        crate::simulation::economy::agents::MODE_CAR
    );
    assert_eq!(agents.pending_household_size[0], expected_household_size);
    assert_eq!(agents.current_building[0], usize::MAX);
    assert_eq!(agents.current_node[0], 0);
    assert_eq!(agents.current_lane_id[0], usize::MAX);
    assert_eq!(agents.access_flags[0], 0);
    let expected_door = allocator.entrances[0].door_pos;
    agents.transit[0] = crate::simulation::economy::agents::TRANSIT_IN_BUILDING;
    agents.current_building[0] = 0;
    agents.target_building[0] = usize::MAX;
    agents.pos_x[0] = expected_door.x;
    agents.pos_y[0] = expected_door.y;
    let mut treasury_balance = 0.0;
    households.operational_hour_tick(
        &mut agents,
        &mut allocator,
        &mut logistics,
        &network,
        &graph,
        0,
        0,
        &mut treasury_balance,
        &[],
        &crate::simulation::economy::fiscal::CityFiscalPolicy::default(),
    );
    assert_eq!(agents.len(), expected_household_size as usize);
    assert_eq!(households.households.len(), 1);
    assert_eq!(
        households.households[0].member_count,
        expected_household_size
    );
    for agent_idx in 0..expected_household_size as usize {
        assert_eq!(agents.household_id[agent_idx], agents.household_id[0]);
    }
    assert_eq!(agents.pending_household_size[0], 0);
    assert!((agents.pos_x[0] - expected_door.x).abs() < 1e-4);
    assert!((agents.pos_y[0] - expected_door.y).abs() < 1e-4);
    assert_eq!(
        allocator.buildings[0].occupancy, 1,
        "Building occupancy should match the admitted household count (1)"
    );
}

#[test]
fn test_hourly_startup_admission_avoids_zero_rounding() {
    use crate::simulation::economy::demand::DemandSystem;
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "base",
        "b.res.house",
        ZoneClass::Residential,
    );
    let commercial_asset =
        register_test_asset(&mut allocator, "base", "b.com.shop", ZoneClass::Commercial);
    let mut agents = AgentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut graph = RegionGraph::new();

    let mut zoning = crate::simulation::zoning::ZoningSystem::new(&WorldConfig::default());
    let mut network = crate::simulation::network::TransitNetwork::new();
    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    let edge_id = graph.edge_count() - 1;
    graph.set_node_type(0, crate::simulation::network::types::NodeType::Border);
    let catalog = load_runtime_economy_catalog().expect("catalog");
    let grocery_profile_runtime_id = catalog
        .profile_for_id("grocery_basic")
        .expect("grocery starter profile")
        .runtime_id;

    for (asset_id, zone_type, center_x, frontage_t, cell_x, occupancy, profile_id) in [
        (residential_asset, ZoneType::Residential, 10.0, 0.1, 0, 1, 0),
        (
            commercial_asset,
            ZoneType::Commercial,
            40.0,
            0.4,
            4,
            0,
            grocery_profile_runtime_id,
        ),
    ] {
        let mut building = indexed_test_building(asset_id, zone_type, 0);
        building.center_x = center_x;
        building.center_y = 10.0;
        building.width_cells = 2;
        building.depth_cells = 2;
        building.frontage_t = frontage_t;
        building.side_offset = 1.0;
        building.edge_idx = edge_id;
        building.cell_x = cell_x;
        building.occupancy = occupancy;
        building.economy_profile_runtime_id = profile_id;
        allocator.buildings.push(building);
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let tuning =
        crate::simulation::economy::definitions::load_runtime_economy_tuning().expect("tuning");
    let household_id = households.admit_immigrant_household(&catalog, &tuning, 0, 2);
    for _ in 0..2 {
        let idx = agents.spawn_housed_agent(0, 0.0, 0.0);
        agents.household_id[idx] = household_id;
    }
    households.households[household_id].budget = 1_000.0;
    households.households[household_id].stock = 6.0;
    households.households[household_id].stock_days = 3.0;

    let mut demand = DemandSystem::new();
    for _ in 0..4 {
        demand.run_hourly_pass(&allocator, &households, &graph, &zoning, 1_000.0);
        if demand.households_to_admit_today > 0 {
            break;
        }
    }
    assert!(
        demand.households_to_admit_today > 0,
        "hourly demand credit should accumulate into a household-admission output from open-job pull; credit={:.3} residential={:.3}",
        demand.admission_action_credit,
        demand.residential,
    );
    allocator.execute_demand_household_admission(
        demand.households_to_admit_today,
        &mut agents,
        &network,
        &graph,
    );

    assert!(
        agents.pending_household_size.iter().any(|&size| size > 0),
        "player-seeded startup city should launch a pending household carrier through the demand-owned startup output"
    );
}
