// SPDX-License-Identifier: GPL-2.0-only

//! Demand spawn, upgrade, and lifecycle action tests.

use super::support::*;
use super::*;

#[test]
#[ignore = "manual matched release locality timing of selected building action lookup"]
fn benchmark_demand_action_lookup() {
    use crate::simulation::economy::demand::{
        DemandBuildingActionPlan, DemandSystem, demand_building_action_key,
    };
    use crate::simulation::zoning::{ZoningSystem, parcels::ParcelGeometry};
    use std::hash::{DefaultHasher, Hash, Hasher};
    use std::hint::black_box;
    use std::time::Instant;

    let graph = RegionGraph::new();
    let network = TransitNetwork::new();
    let terrain = flat_test_terrain();
    let demand = DemandSystem::new();
    for count in [1, 1_024, 65_536, 262_144] {
        let mut allocator = BuildingAllocator::new();
        let mut zoning = ZoningSystem::new(&WorldConfig::default());
        let mut agents = AgentSystem::new();
        let mut households = HouseholdSystem::new();
        let mut logistics = ShipmentSystem::new();
        let mut treasury = 0.0;
        for idx in 0..count {
            // Isolated indexed records: lookup rejection does not consume routing/site geometry.
            // Keep the selected parcel fixed and place background records in distant chunks.
            let center = if idx == 0 {
                Vector2::ZERO
            } else {
                Vector2::new(
                    1_024.0 + (idx % 512) as f32 * 16.0,
                    1_024.0 + (idx / 512) as f32 * 16.0,
                )
            };
            let min = center - Vector2::new(2.5, 2.5);
            let max = center + Vector2::new(2.5, 2.5);
            let parcel = zoning.parcels.insert_new(
                ParcelGeometry {
                    edge_idx: 0,
                    side: 1,
                    frontage_center_t: 0.5,
                    frontage_m: 5.0,
                    depth_m: 5.0,
                    front_center: Vector2::new(center.x, min.y),
                    center,
                    tangent: Vector2::new(1.0, 0.0),
                    normal: Vector2::new(0.0, 1.0),
                    corners: [
                        min,
                        Vector2::new(max.x, min.y),
                        max,
                        Vector2::new(min.x, max.y),
                    ],
                    aabb_min: min,
                    aabb_max: max,
                },
                0,
            );
            let mut building =
                indexed_test_building("benchmark.house".to_owned(), ZoneType::Residential, 0);
            building.parcel_id = parcel.raw();
            building.center_x = center.x;
            building.center_y = center.y;
            building.occupancy = 1;
            assert!(zoning.occupy_parcel(parcel.raw(), idx));
            allocator.buildings.push(building);
        }
        // Admission can occupy a selected empty home before immediate building actions execute.
        // Use a nonempty plan, as the core intentionally skips empty action plans altogether.
        let mut plan = DemandBuildingActionPlan::default();
        plan.residential
            .despawns
            .push(demand_building_action_key(&allocator.buildings[0]));
        let checksum = |allocator: &BuildingAllocator, zoning: &ZoningSystem| {
            let mut hash = DefaultHasher::new();
            for (idx, building) in allocator.buildings.iter().enumerate() {
                (
                    idx,
                    building.parcel_id,
                    &building.asset_id,
                    building.level,
                    building.occupancy,
                    building.center_x.to_bits(),
                    building.center_y.to_bits(),
                    zoning
                        .parcel_by_raw_id(building.parcel_id)
                        .unwrap()
                        .occupied_building(),
                )
                    .hash(&mut hash);
            }
            hash.finish()
        };
        let expected = checksum(&allocator, &zoning);
        let revision = allocator.building_ref_revision();
        let occupancy_revision = zoning.overlay_occupancy_revision();
        let mut execute = || {
            black_box(allocator.execute_demand_building_actions(
                black_box(&plan),
                &mut zoning,
                &mut agents,
                &mut households,
                &mut logistics,
                &mut treasury,
                &graph,
                &network.lane_system,
                &network.road_surface,
                &terrain,
                demand.runtime_catalog(),
                demand.runtime_tuning(),
            ));
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
        assert_eq!(checksum(&allocator, &zoning), expected);
        assert_eq!(allocator.building_ref_revision(), revision);
        assert_eq!(zoning.overlay_occupancy_revision(), occupancy_revision);
        assert_eq!(treasury, 0.0);
        assert!(agents.is_empty());
        assert!(households.households.is_empty());
        eprintln!(
            "demand_action_lookup buildings={count} median_ms={:.9} checksum={expected}",
            samples[10]
        );
    }
}

#[test]
fn test_demand_building_spawn_plan_executes_from_hourly_budget() {
    use crate::simulation::economy::demand::DemandSystem;
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "base",
        "b.res.house",
        ZoneClass::Residential,
    );
    let mut zoning = crate::simulation::zoning::ZoningSystem::new(&WorldConfig::default());
    let mut graph = RegionGraph::new();
    let mut network = crate::simulation::network::TransitNetwork::new();
    let mut agents = AgentSystem::new();
    let mut logistics = ShipmentSystem::new();
    let mut households = HouseholdSystem::new();

    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(120.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    graph.set_node_type(0, crate::simulation::network::types::NodeType::Border);
    paint_zone_rect(
        &mut zoning,
        &graph,
        -40.0,
        -40.0,
        80.0,
        40.0,
        ZoneType::Residential,
    );

    let mut demand = DemandSystem::new();
    for _ in 0..24 {
        demand.run_hourly_pass(&allocator, &households, &graph, &zoning, 1_000.0);
        if !demand.building_actions.residential.spawns.is_empty() {
            break;
        }
    }
    assert!(
        !demand.building_actions.residential.spawns.is_empty(),
        "pioneer demand and legal zoning should produce at least one residential spawn action across hourly demand credit"
    );

    let terrain = compiled_flat_test_terrain(&mut network, &graph);
    allocator.execute_demand_building_actions(
        &demand.building_actions,
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut 0.0,
        &graph,
        &network.lane_system,
        &network.road_surface,
        &terrain,
        demand.runtime_catalog(),
        demand.runtime_tuning(),
    );

    assert!(
        allocator
            .buildings
            .iter()
            .any(|building| building.asset_id == residential_asset),
        "the demand-owned residential spawn plan should place a real building"
    );
}

#[test]
fn test_execute_demand_building_actions_applies_despawn_downgrade_and_upgrade() {
    use crate::simulation::economy::demand::{
        DemandBuildingActionKey, DemandBuildingActionPlan, DemandLevelChangeAction,
        DemandSpawnAction, DemandSystem,
    };
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    let residential_level_1 = register_test_asset_with_family_level(
        &mut allocator,
        "base",
        "b.res.family_l1",
        ZoneClass::Residential,
        Some("res_family"),
        1,
    );
    let residential_level_2 = register_test_asset_with_family_level(
        &mut allocator,
        "base",
        "b.res.family_l2",
        ZoneClass::Residential,
        Some("res_family"),
        2,
    );
    let mut zoning = crate::simulation::zoning::ZoningSystem::new(&WorldConfig::default());
    let mut graph = RegionGraph::new();
    let mut network = crate::simulation::network::TransitNetwork::new();
    let mut agents = AgentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut logistics = ShipmentSystem::new();

    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(120.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    paint_zone_rect(
        &mut zoning,
        &graph,
        -40.0,
        -40.0,
        120.0,
        40.0,
        ZoneType::Residential,
    );
    let occupied_parcels = {
        let mut parcels = zoning
            .parcels()
            .iter()
            .filter(|parcel| {
                parcel.edge_idx() == 0
                    && parcel.side() == 1
                    && parcel.is_available()
                    && zoning
                        .profiles
                        .zone_type_for_runtime_id(parcel.zone_profile_runtime_id())
                        == ZoneType::Residential
            })
            .map(|parcel| (parcel.frontage_center_t(), parcel.id().raw()))
            .collect::<Vec<_>>();
        parcels.sort_by(|left, right| left.0.total_cmp(&right.0).then(left.1.cmp(&right.1)));
        [parcels[0].1, parcels[2].1, parcels[4].1]
    };

    // Remove index 0 first so the later downgrade must follow the swapped building.
    for (parcel_slot, asset_id, level, occupancy) in [
        (2, &residential_level_1, 1, 0),
        (0, &residential_level_1, 1, 6),
        (1, &residential_level_2, 2, 0),
    ] {
        let mut building = indexed_test_building(asset_id.clone(), ZoneType::Residential, 0);
        building.width_cells = 1;
        building.depth_cells = 1;
        building.parcel_id = occupied_parcels[parcel_slot];
        building.facing_dir = Vector2::new(0.0, -1.0);
        building.frontage_t = 0.0;
        building.side_offset = 1.0;
        building.level = level;
        building.occupancy = occupancy;
        building.operating_budget = 0.0;
        building.profit_tax_budget_baseline = 0.0;
        assert!(zoning.occupy_parcel(building.parcel_id, allocator.buildings.len()));
        allocator.buildings.push(building);
    }
    allocator
        .recompute_derived_transforms(&graph, &zoning)
        .expect("building transforms should rebuild for test fixtures");
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let spawn_parcel_id = zoning
        .parcels()
        .iter()
        .find(|parcel| {
            parcel.id().raw() != 0
                && parcel.is_available()
                && zoning
                    .profiles
                    .zone_type_for_runtime_id(parcel.zone_profile_runtime_id())
                    == ZoneType::Residential
        })
        .expect("residential spawn parcel")
        .id()
        .raw();

    let mut plan = DemandBuildingActionPlan::default();
    plan.residential.despawns.push(DemandBuildingActionKey {
        parcel_id: occupied_parcels[2],
        edge_idx: 0,
        side: 1,
        cell_x: 0,
        width_cells: 1,
        depth_cells: 1,
        level: 1,
        asset_id: residential_level_1.clone(),
    });
    plan.residential.downgrades.push(DemandLevelChangeAction {
        building: DemandBuildingActionKey {
            parcel_id: occupied_parcels[1],
            edge_idx: 0,
            side: 1,
            cell_x: 0,
            width_cells: 1,
            depth_cells: 1,
            level: 2,
            asset_id: residential_level_2.clone(),
        },
        target_asset_id: residential_level_1.clone(),
    });
    plan.residential.upgrades.push(DemandLevelChangeAction {
        building: DemandBuildingActionKey {
            parcel_id: occupied_parcels[0],
            edge_idx: 0,
            side: 1,
            cell_x: 0,
            width_cells: 1,
            depth_cells: 1,
            level: 1,
            asset_id: residential_level_1.clone(),
        },
        target_asset_id: residential_level_2.clone(),
    });
    plan.residential.spawns.push(DemandSpawnAction {
        parcel_id: spawn_parcel_id,
        asset_id: residential_level_1.clone(),
    });

    // The same parcel now has a different asset: a stale action must not remove it.
    let mut stale = plan.residential.downgrades[0].building.clone();
    stale.asset_id = "building.replaced".to_owned();
    plan.residential.despawns.insert(0, stale);

    let demand = DemandSystem::new();
    let terrain = compiled_flat_test_terrain(&mut network, &graph);
    allocator.execute_demand_building_actions(
        &plan,
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut 0.0,
        &graph,
        &network.lane_system,
        &network.road_surface,
        &terrain,
        demand.runtime_catalog(),
        demand.runtime_tuning(),
    );

    let spawned_exists = allocator
        .buildings
        .iter()
        .any(|building| building.parcel_id == spawn_parcel_id);
    assert!(
        spawned_exists,
        "building_count={} spawn_parcel={} spawned_exists={}",
        allocator.buildings.len(),
        spawn_parcel_id,
        spawned_exists
    );

    assert_eq!(allocator.buildings.len(), 3);
    assert!(
        allocator.buildings.iter().any(|building| {
            building.parcel_id == occupied_parcels[0]
                && building.asset_id == residential_level_2
                && building.level == 2
        }),
        "upgrade action should replace the first building with the next family level"
    );
    assert!(
        allocator.buildings.iter().any(|building| {
            building.parcel_id == occupied_parcels[1]
                && building.asset_id == residential_level_1
                && building.level == 1
        }),
        "downgrade action should replace the second building with the previous family level"
    );
    assert!(
        allocator
            .buildings
            .iter()
            .all(|building| building.parcel_id != occupied_parcels[2]),
        "despawn action should remove the empty third building"
    );
    let spawned = allocator
        .buildings
        .iter()
        .find(|building| building.parcel_id == spawn_parcel_id)
        .expect("spawn action should place a building on the selected parcel");
    assert!(spawned.operating_budget.abs() <= f32::EPSILON);

    // A later terrain edit cannot authorize a new asset footprint at an unsupported fixed pad.
    let mut blocked_terrain = terrain.clone();
    for z in 0..blocked_terrain.height {
        for x in 0..blocked_terrain.width {
            blocked_terrain.set_height(x, z, 20.0);
        }
    }
    let building = allocator
        .buildings
        .iter()
        .find(|b| b.parcel_id == occupied_parcels[0])
        .unwrap();
    let support_height = building.support_height_m;
    let mut blocked = DemandBuildingActionPlan::default();
    blocked
        .residential
        .downgrades
        .push(DemandLevelChangeAction {
            building: crate::simulation::economy::demand::demand_building_action_key(building),
            target_asset_id: residential_level_1,
        });
    let revision = allocator.building_ref_revision();
    allocator.execute_demand_building_actions(
        &blocked,
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut 0.0,
        &graph,
        &network.lane_system,
        &network.road_surface,
        &blocked_terrain,
        demand.runtime_catalog(),
        demand.runtime_tuning(),
    );
    let building = allocator
        .buildings
        .iter()
        .find(|b| b.parcel_id == occupied_parcels[0])
        .unwrap();
    assert_eq!(building.asset_id, residential_level_2);
    assert_eq!(building.support_height_m, support_height);
    assert_eq!(allocator.building_ref_revision(), revision);
}

#[test]
fn test_commercial_demand_spawn_startup_budget_includes_first_import_cost() {
    use crate::simulation::economy::demand::{
        DemandBuildingActionPlan, DemandSpawnAction, DemandSystem,
    };
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    let commercial_asset =
        register_test_asset(&mut allocator, "base", "b.com.shop", ZoneClass::Commercial);
    let mut zoning = crate::simulation::zoning::ZoningSystem::new(&WorldConfig::default());
    let mut graph = RegionGraph::new();
    let mut network = crate::simulation::network::TransitNetwork::new();
    let mut agents = AgentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut logistics = ShipmentSystem::new();

    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(120.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    paint_zone_rect(
        &mut zoning,
        &graph,
        -40.0,
        -40.0,
        120.0,
        40.0,
        ZoneType::Commercial,
    );
    let parcel_id = zoning
        .parcels()
        .iter()
        .find(|parcel| {
            parcel.is_available()
                && zoning
                    .profiles
                    .zone_type_for_runtime_id(parcel.zone_profile_runtime_id())
                    == ZoneType::Commercial
        })
        .expect("commercial spawn parcel")
        .id()
        .raw();

    let mut plan = DemandBuildingActionPlan::default();
    plan.commercial.spawns.push(DemandSpawnAction {
        parcel_id,
        asset_id: commercial_asset,
    });

    let demand = DemandSystem::new();
    let terrain = compiled_flat_test_terrain(&mut network, &graph);
    allocator.execute_demand_building_actions(
        &plan,
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut 0.0,
        &graph,
        &network.lane_system,
        &network.road_surface,
        &terrain,
        demand.runtime_catalog(),
        demand.runtime_tuning(),
    );

    let building = allocator
        .buildings
        .iter()
        .find(|building| building.parcel_id == parcel_id)
        .expect("commercial demand spawn should create a building");
    let profile = demand
        .runtime_catalog()
        .profile_by_runtime_id(building.economy_profile_runtime_id)
        .expect("spawned commercial building should have a profile");
    let first_import_base_cost = profile
        .inputs
        .iter()
        .map(|port| {
            profile.inventory_target_units_for(port)
                * demand
                    .runtime_catalog()
                    .unit_price_for_resource(port.resource_runtime_id)
                    .expect("input resource should have a unit price")
                * demand.runtime_tuning().owa_import_price_multiplier
        })
        .sum::<f32>();
    assert!(first_import_base_cost > 0.0);
    let expected_startup_budget =
        (profile.worker_capacity as f32 * profile.average_daily_wage() * 7.0
            + first_import_base_cost)
            .max(500.0);
    assert!((building.operating_budget - expected_startup_budget).abs() <= 0.01);
}
