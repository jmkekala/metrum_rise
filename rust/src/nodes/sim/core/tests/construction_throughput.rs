// SPDX-License-Identifier: GPL-2.0-only

//! Successful queued entry throughput, including physical placement and owner funding.

use super::*;
use std::time::Instant;

fn fixture(districts: usize) -> (SimCore, Vec<DemandSpawnAction>) {
    let mut core = test_core();
    add_test_border_road(&mut core);
    let template = core.region_graph.edges()[0].clone();
    // Each district has its own real outside connection. Increasing this workload grows
    // both placed homes and border routing; it is not a fixed-neighborhood locality test.
    for district in 1..districts {
        let x = (district % 32) as f32 * 256.0;
        let z = (district / 32) as f32 * 64.0;
        let start = Vector3::new(x, 0.0, z);
        let end = Vector3::new(x + 180.0, 0.0, z);
        let mut edge = template.clone();
        edge.start_node = core.region_graph.add_node(start, NodeType::Border);
        edge.end_node = core.region_graph.add_node(end, NodeType::Junction);
        edge.geometry = vec![start, end];
        edge.physical_geometry.clone_from(&edge.geometry);
        core.region_graph.add_edge(edge);
    }
    core.transit_network
        .lane_system
        .rebuild(&mut core.region_graph);
    core.transit_network
        .rebuild_cch_and_check(&core.region_graph);
    let home = register_test_asset(
        &mut core.allocator,
        "throughput_home",
        ZoneType::Residential,
    );
    let factory = register_test_asset(
        &mut core.allocator,
        "throughput_factory",
        ZoneType::Industrial,
    );
    for district in 0..districts {
        let x = (district % 32) as f32 * 256.0;
        let z = (district / 32) as f32 * 64.0 - 20.0;
        for (zone, start, end) in [
            (ZoneType::Residential, 10.0, 90.0),
            (ZoneType::Industrial, 110.0, 170.0),
        ] {
            let profile = core
                .zoning
                .profiles
                .default_runtime_id_for_zone_type(zone)
                .unwrap();
            core.zoning
                .place_parcel_run_at(
                    start + x,
                    z,
                    end + x,
                    z,
                    profile,
                    20.0,
                    30.0,
                    0.0,
                    &core.region_graph,
                )
                .unwrap();
        }
    }
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    let mut homes = Vec::new();
    let mut factories = Vec::new();
    let mut per_district = vec![0; districts];
    for parcel in core.zoning.parcels() {
        let zone = core
            .zoning
            .profiles
            .profile_by_runtime_id(parcel.zone_profile_runtime_id())
            .unwrap()
            .zone_type;
        let (actions, asset) = if zone == ZoneType::Residential {
            (&mut homes, &home)
        } else {
            let count = &mut per_district[parcel.edge_idx()];
            if *count == 2 {
                continue;
            }
            *count += 1;
            (&mut factories, &factory)
        };
        actions.push(DemandSpawnAction {
            parcel_id: parcel.id().raw(),
            asset_id: asset.clone(),
        });
    }
    assert_eq!(factories.len(), districts * 2);
    for action in &homes {
        let before = core.allocator.buildings.len();
        execute_test_construction(&mut core, action, ZoneType::Residential, 1);
        assert_eq!(core.allocator.buildings.len(), before + 1);
    }
    (core, factories)
}

#[test]
#[ignore = "manual release throughput of successful physical projects; setup and planning slot separate"]
fn benchmark_successful_project_placement() {
    for districts in [1, 8, 64, 512] {
        let mut samples = [[0.0_f64; 3]; 5];
        let mut owner_equity = 0.0;
        for sample in &mut samples {
            let start = Instant::now();
            let (mut core, factories) = fixture(districts);
            sample[0] = start.elapsed().as_secs_f64() * 1_000.0;
            let homes = core.allocator.buildings.len();
            // One planning slot proves every queued factory's permit; each start then places.
            for (index, action) in factories.iter().enumerate() {
                core.pending_demand_spawns.push_back(
                    super::super::state::PendingDemandSpawnAction {
                        id: 0,
                        due_minute: 2 + index as u64,
                        zone_type: ZoneType::Industrial,
                        action: action.clone(),
                        planned_day_index: 1,
                        planned_minute_of_day: 0,
                        permit: Default::default(),
                        counted: false,
                    },
                );
            }
            let start = Instant::now();
            core.prove_construction_permits_for_test(1, 1);
            sample[1] = start.elapsed().as_secs_f64() * 1_000.0;
            assert!(
                core.pending_demand_spawns
                    .iter()
                    .all(|pending| pending.permit.granted().is_some())
            );
            let start = Instant::now();
            for index in 0..factories.len() {
                let minute = 2 + index as u16;
                assert_eq!(core.execute_pending_demand_spawns_for_minute(1, minute), 1);
                assert_eq!(core.allocator.buildings.len(), homes + index + 1);
            }
            sample[2] = start.elapsed().as_secs_f64() * 1_000.0;
            let placed = &core.allocator.buildings[homes..];
            assert!(placed.iter().all(|b| b.is_under_construction()
                && b.worker_count == 0
                && b.revenue == 0.0
                && b.operating_budget > 0.0));
            let catalog = core.demand.runtime_catalog();
            let profile = catalog.profile_for_id("food_processor_basic").unwrap();
            owner_equity = placed.iter().map(|b| f64::from(b.operating_budget)).sum();
            assert_eq!(
                owner_equity,
                placed.len() as f64
                    * f64::from(profile.zoned_startup_budget(
                        catalog,
                        core.demand.runtime_tuning().owa_import_price_multiplier
                    ))
            );
        }
        let median = |column| {
            let mut values = samples.map(|sample| sample[column]);
            values.sort_by(f64::total_cmp);
            values[2]
        };
        eprintln!(
            "successful_project_placement districts={districts} placed={} owner_equity={owner_equity:.2} setup_ms={:.3} planning_slot_ms={:.3} placement_ms={:.3}",
            districts * 2,
            median(0),
            median(1),
            median(2)
        );
    }
}
