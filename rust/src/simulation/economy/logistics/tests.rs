// SPDX-License-Identifier: GPL-2.0-only

//! Logistics unit tests.

use super::*;
use crate::assets::AssetManifest;
use crate::assets::asset::{
    Anchor, AnchorType, BuildingData, BuildingFieldData, MeshPart, PlacementMode, ZoneClass,
};
use crate::simulation::buildings::allocator::{
    Building, BuildingAllocator, resolve_building_economy_profile_binding,
};
use crate::simulation::economy::agents::{AgentSystem, TRANSIT_IN_BUILDING};
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
use crate::simulation::zoning::ZoneType;
use godot::prelude::{Vector2, Vector3};

#[test]
#[ignore = "manual matched release timing of resource-keyed supplier indexing"]
fn benchmark_supplier_index() {
    use super::supplier_index::SupplierCandidateIndex;
    use crate::simulation::economy::accessibility::{ModeComponentIndex, ReachableBucketScanEvent};
    use std::{hint::black_box, time::Instant};
    let (graph, network, edge, _, _) = simple_graph_with_border();
    let catalog = load_runtime_economy_catalog().unwrap();
    let resource = catalog.resource_runtime_id_for_id("packaged_food").unwrap();
    let components = ModeComponentIndex::build(&graph, TransitFlags::CAR);
    let reservations = ShipmentSystem::new().build_reservation_views(catalog.resource_count());
    let truck_payload_kg = load_runtime_economy_tuning()
        .unwrap()
        .logistics
        .truck_payload_kg;
    for count in [1_024, 8_192, 65_536] {
        let mut allocator = BuildingAllocator::new();
        let asset = register_test_asset(
            &mut allocator,
            "test",
            "supplier_bench",
            ZoneClass::Industrial,
        );
        let template = make_building(
            &allocator,
            -50.0,
            ZoneType::Industrial,
            edge,
            &asset,
            100.0,
            10_000.0,
        );
        allocator.buildings = (0..count)
            .map(|idx| {
                let mut building = template.clone();
                building.center_x = ((idx * 73) % 4_096) as f32 * 4.0;
                building.center_y = (idx % 9) as f32 * 256.0;
                building
            })
            .collect();
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);
        let reachable = components.building_components(&allocator, &graph, 0, TransitFlags::CAR);
        let mut samples = [0.0; 11];
        for sample in &mut samples {
            let start = Instant::now();
            let index = SupplierCandidateIndex::build(
                black_box(&allocator),
                &graph,
                &catalog,
                &components,
                &reservations,
                truck_payload_kg,
            );
            *sample = start.elapsed().as_secs_f64() * 1_000.0;
            let mut found = 0;
            index.buckets_for_resource(resource).unwrap().scan_nearest(
                reachable,
                0.0,
                0.0,
                |event| {
                    if let ReachableBucketScanEvent::Item { .. } = event {
                        found += 1;
                    }
                    true
                },
            );
            assert_eq!(found, count);
            black_box(index);
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "supplier_index buildings={count} median_ms={:.3}",
            samples[5]
        );
    }
}

#[test]
fn returning_shipments_close_without_changing_settled_cargo_or_unrelated_agents() {
    use crate::simulation::economy::agents::VEHICLE_FREIGHT_DELIVERY;
    // A real returning carrier, an absent carrier, and stale references to either kind of agent.
    for (carrier_id, vehicle_type, shipment_id, expected_agents) in [
        (0, VEHICLE_FREIGHT_DELIVERY, 7, 0),
        (usize::MAX, VEHICLE_FREIGHT_DELIVERY, 99, 1),
        (0, 0, u64::MAX, 1),
        (0, VEHICLE_FREIGHT_DELIVERY, 99, 1),
    ] {
        let mut allocator = BuildingAllocator::new();
        let asset = register_test_asset(
            &mut allocator,
            "test",
            "return_cleanup",
            ZoneClass::Commercial,
        );
        let mut destination = make_building(
            &allocator,
            0.0,
            ZoneType::Commercial,
            usize::MAX,
            &asset,
            0.0,
            800.0,
        );
        let resource = load_runtime_economy_catalog()
            .unwrap()
            .resource_runtime_id_for_id("packaged_food")
            .unwrap();
        destination.set_inventory_units(resource, 40.0);
        allocator.buildings.push(destination);
        let mut agents = AgentSystem::new();
        let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
        agents.vehicle_type[agent] = vehicle_type;
        agents.freight_shipment_id[agent] = shipment_id;
        let seed = agents.schedule_seed[agent];
        let mut shipments = ShipmentSystem::new();
        shipments.shipments.push(Shipment {
            id: 7,
            resource_runtime_id: resource,
            amount: 40.0,
            source: ShipmentEndpoint::OwaBorder(7),
            destination: ShipmentEndpoint::Building(0),
            carrier_class: CarrierClass::Truck,
            status: ShipmentStatus::Returning,
            carrier_agent_id: carrier_id,
            total_cost: 200.0,
            freight_cost: 0.0,
            eta_hours: 1,
            queued_hours: 5,
        });
        for _ in 0..2 {
            progress_shipments_for_test(
                &mut shipments,
                &mut allocator,
                &mut agents,
                &TransitNetwork::new(),
                &RegionGraph::new(),
            );
            assert_eq!(shipments.shipments[0].status, ShipmentStatus::Fulfilled);
            assert_eq!(shipments.shipments[0].carrier_agent_id, usize::MAX);
            assert_eq!(agents.len(), expected_agents);
            assert_eq!(allocator.buildings[0].operating_budget, 800.0);
            assert_eq!(allocator.buildings[0].inventory_units(resource), 40.0);
            if expected_agents > 0 {
                assert_eq!(agents.schedule_seed[0], seed);
            }
        }
    }
}

#[test]
fn stale_shipment_carrier_cannot_remove_an_unrelated_agent() {
    use crate::simulation::economy::agents::VEHICLE_FREIGHT_DELIVERY;
    for removal in [false, true] {
        for vehicle_type in [0, VEHICLE_FREIGHT_DELIVERY] {
            let mut allocator = BuildingAllocator::new();
            let asset = register_test_asset(
                &mut allocator,
                "test",
                "stale_carrier",
                ZoneClass::Commercial,
            );
            allocator.buildings.push(make_building(
                &allocator,
                0.0,
                ZoneType::Commercial,
                usize::MAX,
                &asset,
                0.0,
                800.0,
            ));
            let mut agents = AgentSystem::new();
            let unrelated = agents.spawn_housed_agent(0, 0.0, 0.0);
            agents.vehicle_type[unrelated] = vehicle_type;
            agents.freight_shipment_id[unrelated] = 99;
            let seed = agents.schedule_seed[unrelated];
            let mut shipments = ShipmentSystem::new();
            shipments.shipments.push(Shipment {
                id: 7,
                resource_runtime_id: 1,
                amount: 40.0,
                source: ShipmentEndpoint::OwaBorder(7),
                destination: ShipmentEndpoint::Building(0),
                carrier_class: CarrierClass::Truck,
                status: ShipmentStatus::InTransit,
                carrier_agent_id: unrelated,
                total_cost: 200.0,
                freight_cost: 0.0,
                eta_hours: 1,
                queued_hours: 0,
            });
            if removal {
                shipments.invalidate_building(
                    0,
                    &mut allocator,
                    &mut agents,
                    &mut crate::simulation::economy::households::HouseholdSystem::new(),
                    &mut 0.0,
                );
            } else {
                progress_shipments_for_test(
                    &mut shipments,
                    &mut allocator,
                    &mut agents,
                    &TransitNetwork::new(),
                    &RegionGraph::new(),
                );
                assert_eq!(shipments.shipments[0].status, ShipmentStatus::Failed);
            }
            assert_eq!(agents.len(), 1);
            assert_eq!(agents.schedule_seed[0], seed);
            assert_eq!(allocator.buildings[0].operating_budget, 1_000.0);
        }
    }
}

fn returning_carrier_fixture(count: usize) -> (ShipmentSystem, AgentSystem) {
    use crate::simulation::economy::agents::VEHICLE_FREIGHT_DELIVERY;
    let mut agents = AgentSystem::new();
    let mut shipments = ShipmentSystem::new();
    for idx in 0..count {
        let agent = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
        agents.vehicle_type[agent] = VEHICLE_FREIGHT_DELIVERY;
        agents.freight_shipment_id[agent] = idx as u64 * 3 + 1;
        shipments.shipments.push(Shipment {
            id: idx as u64 * 3 + 1,
            resource_runtime_id: 1,
            amount: 40.0,
            source: ShipmentEndpoint::OwaBorder(7),
            destination: ShipmentEndpoint::Building(0),
            carrier_class: CarrierClass::Truck,
            status: ShipmentStatus::Returning,
            carrier_agent_id: agent,
            total_cost: 200.0,
            freight_cost: 0.0,
            eta_hours: u16::MAX,
            queued_hours: 0,
        });
    }
    (shipments, agents)
}

#[test]
#[ignore = "manual matched release timing of active return-leg progression"]
fn benchmark_freight_return_progress() {
    use std::{hint::black_box, time::Instant};
    for count in [1_024, 8_192, 65_536] {
        let mut allocator = BuildingAllocator::new();
        let (mut shipments, mut agents) = returning_carrier_fixture(count);
        let graph = RegionGraph::new();
        let network = TransitNetwork::new();
        let mut samples = [0.0; 11];
        for sample in &mut samples {
            let start = Instant::now();
            for _ in 0..16 {
                progress_shipments_for_test(
                    black_box(&mut shipments),
                    &mut allocator,
                    &mut agents,
                    &network,
                    &graph,
                );
            }
            *sample = start.elapsed().as_secs_f64() * 1_000.0 / 16.0;
        }
        assert_eq!(agents.len(), count);
        assert!(
            shipments
                .shipments
                .iter()
                .all(|s| s.status == ShipmentStatus::Returning)
        );
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "freight_return_progress shipments={count} median_ms={:.3}",
            samples[5]
        );
    }
}

#[test]
fn carrier_cleanup_preserves_surviving_links_across_swaps() {
    for resident_at_end in [false, true] {
        let (mut shipments, mut agents) = returning_carrier_fixture(64);
        if resident_at_end {
            agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
        }
        for idx in [0, 5, 12, 17, 61] {
            shipments.shipments[idx].queued_hours = u16::MAX;
        }
        progress_shipments_for_test(
            &mut shipments,
            &mut BuildingAllocator::new(),
            &mut agents,
            &TransitNetwork::new(),
            &RegionGraph::new(),
        );
        assert_eq!(agents.len(), 59 + usize::from(resident_at_end));
        for shipment in &shipments.shipments {
            if shipment.status == ShipmentStatus::Returning {
                assert!(agents.freight_carrier_matches(shipment.carrier_agent_id, shipment.id));
            } else {
                assert_eq!(shipment.status, ShipmentStatus::Fulfilled);
                assert_eq!(shipment.carrier_agent_id, usize::MAX);
            }
        }
    }
}

#[test]
#[ignore = "manual matched release timing of carrier reference remapping"]
fn benchmark_carrier_swap_remap() {
    use std::{hint::black_box, time::Instant};
    for count in [1_024, 8_192, 65_536] {
        let (mut shipments, mut agents) = returning_carrier_fixture(count);
        let spare = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
        let last = count - 1;
        let mut samples = [0.0; 11];
        for sample in &mut samples {
            let start = Instant::now();
            for _ in 0..128 {
                // Isolate reference repair from the SoA record-removal/setup cost.
                for (old, new) in [(last, spare), (spare, last)] {
                    agents.freight_shipment_id.swap(old, new);
                    agents.vehicle_type.swap(old, new);
                    shipments.remap_carrier_agent_index(black_box(old), black_box(new), &agents);
                }
            }
            *sample = start.elapsed().as_secs_f64() * 1_000.0;
        }
        assert!(
            shipments
                .shipments
                .iter()
                .enumerate()
                .all(|(idx, shipment)| shipment.carrier_agent_id == idx)
        );
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "carrier_swap_remap shipments={count} swaps=256 median_ms={:.3}",
            samples[5]
        );
    }
}

#[test]
fn restoring_freight_undo_preserves_shipment_id_order_after_other_jobs_close() {
    let (mut shipments, _) = returning_carrier_fixture(8);
    for (idx, shipment) in shipments.shipments.iter_mut().enumerate() {
        shipment.destination = ShipmentEndpoint::Building(idx % 2);
    }
    let undo = shipments.capture_building_undo(1);
    shipments
        .shipments
        .retain(|s| s.destination == ShipmentEndpoint::Building(0) && s.id != 1);
    let mut new_job = shipments.shipments.last().unwrap().clone();
    new_job.id = 25;
    shipments.shipments.push(new_job);
    shipments.restore_building_undo(undo);
    assert_eq!(
        shipments.shipments.iter().map(|s| s.id).collect::<Vec<_>>(),
        [4, 7, 10, 13, 16, 19, 22, 25]
    );
}

macro_rules! logistics_tick {
    ($shipments:expr, $allocator:expr, $network:expr, $graph:expr, $minute:expr) => {{
        let mut agents = AgentSystem::new();
        let mut treasury_balance = 0.0;
        $shipments.hourly_tick(
            $allocator,
            &mut agents,
            &mut crate::simulation::economy::households::HouseholdSystem::new(),
            $network,
            $graph,
            $minute,
            &mut treasury_balance,
            |_, _| {},
        )
    }};
}

fn create_profile_input_shipments_for_test(
    shipments: &mut ShipmentSystem,
    allocator: &mut BuildingAllocator,
    network: &TransitNetwork,
    graph: &RegionGraph,
    minute_of_day: u16,
) {
    let mut treasury_balance = 0.0;
    let mut planning =
        super::planning::FreightPlanningContext::build(shipments, allocator, network, graph);
    shipments.create_profile_input_shipments(
        allocator,
        network,
        graph,
        minute_of_day,
        &mut planning,
        &mut treasury_balance,
    );
    planning.finish(shipments);
}

fn create_profile_output_exports_for_test(
    shipments: &mut ShipmentSystem,
    allocator: &mut BuildingAllocator,
    network: &TransitNetwork,
    graph: &RegionGraph,
    minute_of_day: u16,
) {
    let mut planning =
        super::planning::FreightPlanningContext::build(shipments, allocator, network, graph);
    shipments.create_profile_output_exports(
        allocator,
        network,
        graph,
        minute_of_day,
        &mut planning,
    );
    planning.finish(shipments);
}

fn progress_shipments_for_test(
    shipments: &mut ShipmentSystem,
    allocator: &mut BuildingAllocator,
    agents: &mut AgentSystem,
    network: &TransitNetwork,
    graph: &RegionGraph,
) {
    let mut treasury_balance = 0.0;
    shipments.progress_shipments(
        allocator,
        agents,
        &mut crate::simulation::economy::households::HouseholdSystem::new(),
        network,
        graph,
        &mut treasury_balance,
        |_, _| {},
    )
}

fn mark_carrier_arrived(shipments: &ShipmentSystem, agents: &mut AgentSystem, shipment_idx: usize) {
    let shipment = &shipments.shipments[shipment_idx];
    mark_carrier_at_endpoint(agents, shipment.carrier_agent_id, shipment.destination);
}

fn mark_carrier_returned(
    shipments: &ShipmentSystem,
    agents: &mut AgentSystem,
    shipment_idx: usize,
) {
    let shipment = &shipments.shipments[shipment_idx];
    mark_carrier_at_endpoint(agents, shipment.carrier_agent_id, shipment.source);
}

fn mark_carrier_at_endpoint(
    agents: &mut AgentSystem,
    carrier_idx: usize,
    endpoint: ShipmentEndpoint,
) {
    assert!(carrier_idx < agents.len(), "shipment carrier should exist");
    match endpoint {
        ShipmentEndpoint::Building(building_idx) => {
            agents.transit[carrier_idx] = TRANSIT_IN_BUILDING;
            agents.current_building[carrier_idx] = building_idx;
            agents.current_lane_id[carrier_idx] = usize::MAX;
            agents.current_path[carrier_idx].clear();
        }
        ShipmentEndpoint::OwaBorder(border_node) => {
            agents.current_node[carrier_idx] = border_node;
            agents.current_lane_id[carrier_idx] = usize::MAX;
            agents.current_path[carrier_idx].clear();
        }
    }
}

fn register_test_asset(
    allocator: &mut BuildingAllocator,
    pack_id: &str,
    asset_id: &str,
    zone: ZoneClass,
) -> String {
    let economy_profile = match zone {
        ZoneClass::Commercial => Some("grocery_basic"),
        ZoneClass::Industrial => Some("food_processor_basic"),
        _ => None,
    };
    register_test_asset_with_profile(allocator, pack_id, asset_id, zone, economy_profile)
}

fn register_test_asset_with_profile(
    allocator: &mut BuildingAllocator,
    pack_id: &str,
    asset_id: &str,
    zone: ZoneClass,
    economy_profile: Option<&str>,
) -> String {
    let (household_capacity, worker_capacity) = match zone {
        ZoneClass::Residential => (Some(6), None),
        ZoneClass::Commercial | ZoneClass::Industrial | ZoneClass::Office => (None, Some(4)),
        ZoneClass::Mixed => (Some(4), Some(2)),
    };
    let economy_profile = economy_profile.map(str::to_owned);
    let manifest = AssetManifest {
        asset_id: asset_id.to_owned(),
        display_name: "Test".to_owned(),
        asset_set: None,
        tags: vec![],
        thumbnail: None,
        lods: vec![],
        mesh_parts: vec![MeshPart::single_lod0("main", "lod0.glb")],
        anchors: vec![Anchor {
            anchor_type: AnchorType::Entrance,
            name: "main".to_owned(),
            position: [0.0, 0.0, 0.5],
            forward: [0.0, 0.0, 1.0],
            width_m: None,
            length_m: None,
            vehicle_class: None,
        }],
        site_surfaces: vec![],
        building: Some(BuildingData {
            window_brightness: 3.0,
            appearance: None,
            flat_size_m2: None,
            placement_mode: PlacementMode::ZonedPrivate,
            zone_type: Some(zone),
            density: Some("low".to_owned()),
            lot_width_cells: 1,
            lot_depth_cells: 1,
            frontage_forward: None,
            min_zone_width_cells: None,
            min_zone_depth_cells: None,
            level: 1,
            household_capacity,
            worker_capacity,
            service_class: None,
            economy_profile,
            extractor: None,
            field: None,
            yard_hedge: None,
            yard_planting: Vec::new(),
        }),
        prop: None,
        vehicle: None,
        character: None,
    };
    allocator
        .registry
        .register(pack_id, manifest, String::new());
    format!("{pack_id}:{asset_id}")
}

fn register_test_city_service_asset(
    allocator: &mut BuildingAllocator,
    pack_id: &str,
    asset_id: &str,
    service_class: &str,
    economy_profile: &str,
) -> String {
    let manifest = AssetManifest {
        asset_id: asset_id.to_owned(),
        display_name: "Test Service".to_owned(),
        asset_set: None,
        tags: vec![],
        thumbnail: None,
        lods: vec![],
        mesh_parts: vec![MeshPart::single_lod0("main", "lod0.glb")],
        anchors: vec![Anchor {
            anchor_type: AnchorType::Entrance,
            name: "main".to_owned(),
            position: [0.0, 0.0, 0.5],
            forward: [0.0, 0.0, 1.0],
            width_m: None,
            length_m: None,
            vehicle_class: None,
        }],
        site_surfaces: vec![],
        building: Some(BuildingData {
            window_brightness: 3.0,
            appearance: None,
            flat_size_m2: None,
            placement_mode: PlacementMode::Explicit,
            zone_type: None,
            density: None,
            lot_width_cells: 2,
            lot_depth_cells: 2,
            frontage_forward: None,
            min_zone_width_cells: None,
            min_zone_depth_cells: None,
            level: 1,
            household_capacity: None,
            worker_capacity: None,
            service_class: Some(service_class.to_owned()),
            economy_profile: Some(economy_profile.to_owned()),
            extractor: None,
            field: None,
            yard_hedge: None,
            yard_planting: Vec::new(),
        }),
        prop: None,
        vehicle: None,
        character: None,
    };
    allocator
        .registry
        .register(pack_id, manifest, String::new());
    format!("{pack_id}:{asset_id}")
}

fn register_test_field_asset(
    allocator: &mut BuildingAllocator,
    pack_id: &str,
    asset_id: &str,
) -> String {
    let manifest = AssetManifest {
        asset_id: asset_id.to_owned(),
        display_name: "Test Field".to_owned(),
        asset_set: None,
        tags: vec![],
        thumbnail: None,
        lods: vec![],
        mesh_parts: vec![MeshPart::single_lod0("main", "lod0.glb")],
        anchors: vec![Anchor {
            anchor_type: AnchorType::Entrance,
            name: "main".to_owned(),
            position: [0.0, 0.0, 0.5],
            forward: [0.0, 0.0, 1.0],
            width_m: None,
            length_m: None,
            vehicle_class: None,
        }],
        site_surfaces: vec![],
        building: Some(BuildingData {
            window_brightness: 3.0,
            appearance: None,
            flat_size_m2: None,
            placement_mode: PlacementMode::Explicit,
            zone_type: None,
            density: None,
            lot_width_cells: 2,
            lot_depth_cells: 2,
            frontage_forward: None,
            min_zone_width_cells: None,
            min_zone_depth_cells: None,
            level: 1,
            household_capacity: None,
            worker_capacity: None,
            service_class: None,
            economy_profile: Some("grain_farm_basic".to_owned()),
            extractor: None,
            field: Some(BuildingFieldData {
                resource: "grain".to_owned(),
                area_mode: "player_polygon".to_owned(),
            }),
            yard_hedge: None,
            yard_planting: Vec::new(),
        }),
        prop: None,
        vehicle: None,
        character: None,
    };
    allocator
        .registry
        .register(pack_id, manifest, String::new());
    format!("{pack_id}:{asset_id}")
}

fn make_building(
    allocator: &BuildingAllocator,
    center_x: f32,
    zone_type: ZoneType,
    edge_idx: usize,
    asset_id: &str,
    stock: f32,
    budget: f32,
) -> Building {
    let economy_binding = resolve_building_economy_profile_binding(&allocator.registry, asset_id);
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let mut resource_inventory = vec![0.0; catalog.resource_count()];
    if stock > 0.0
        && let Some(profile) = catalog.profile_by_runtime_id(economy_binding.runtime_id)
        && let Some(output_port) = profile.outputs.first()
    {
        resource_inventory[output_port.resource_runtime_id as usize - 1] = stock;
    }
    Building {
        build_generation: 0,
        center_x,
        center_y: 10.0,
        support_height_m: 0.0,
        width_cells: 2,
        depth_cells: 2,
        zone_profile_runtime_id: 0,
        parcel_id: 0,
        zone_type,
        facing_dir: Vector2::new(0.0, 1.0),
        frontage_t: 0.5,
        side_offset: 1.0,
        is_deserted: false,
        budget_distress: false,
        edge_idx,
        side: 1,
        cell_x: 0,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        service_funding_override: -1.0,
        asset_id: asset_id.to_owned(),
        level: 1,
        construction_total_hours: 0,
        construction_remaining_hours: 0,
        broken: false,
        economy_profile_runtime_id: economy_binding.runtime_id,
        economy_broken: economy_binding.economy_broken,
        resource_inventory,
        revenue: 0.0,
        operating_budget: budget,
        profit_tax_budget_baseline: budget,
        last_day_profit: 0.0,
        shipment_cooldown_hours: 0,
        daily_owa_input_value: 0.0,
        daily_local_input_value: 0.0,
        daily_city_funded_input_cost: 0.0,
        daily_household_sales_value: 0.0,
        daily_power_service_units: 0.0,
        daily_electricity: Default::default(),
        recent_electricity: Default::default(),
        hourly_economy_activity: 0.0,
        daily_power_served_units: 0.0,
        recent_power_service_units: 0.0,
        recent_power_served_units: 0.0,
        recent_household_sales_value: 0.0,
        // Freight fixtures model a store serving enough demand for one worker.
        commercial_activity_floor_scale: catalog
            .profile_by_runtime_id(economy_binding.runtime_id)
            .filter(|profile| {
                profile.kind
                    == crate::simulation::economy::definitions::EconomyProfileRuntimeKind::Store
            })
            .map_or(0.0, |profile| 1.0 / profile.worker_capacity as f32),
        work_area_scale: 1.0,
        pending_redevelopment: false,
        rezone_grace_days_remaining: 0,
    }
}

/// Minimal connected freight graph shared with cache replacement regressions.
pub(super) fn simple_graph_with_border() -> (RegionGraph, TransitNetwork, usize, usize, u32) {
    graph_with_border_to(100.0)
}

#[test]
fn prepared_border_minima_preserve_each_border_quote_bits() {
    let (mut graph, mut network, first, second, border) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "border_minimum",
        ZoneClass::Commercial,
    );
    for (x, edge) in [(-50.0, first), (50.0, second)] {
        allocator.buildings.push(make_building(
            &allocator,
            x,
            ZoneType::Commercial,
            edge,
            &asset,
            0.0,
            100_000.0,
        ));
    }
    let mut cache = super::route_cache::FreightRouteCache::default();
    for one_way in [false, true] {
        if one_way {
            graph.edge_mut(first).bkw_lanes = 0;
            graph.edge_mut(second).bkw_lanes = 0;
        }
        network.lane_system.rebuild(&mut graph);
        network.cch_graph = crate::simulation::pathing::cch::CchGraph::build(&graph);
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);
        for metric in [0.137, 5.0, 23.333334] {
            graph.edge_mut(first).base_cost = metric;
            network.cch_graph.customize(&graph);
            cache.clear();
            for borders in [
                vec![],
                vec![border],
                vec![2],
                vec![0, 2],
                vec![2, 0],
                vec![1, 1],
            ] {
                cache.clear_buildings();
                cache.prepare_border_costs(&borders, allocator.buildings.len(), &network, &graph);
                let costs = network
                    .cch_graph
                    .costs_from_each(&borders, &graph, TransitFlags::CAR);
                for prepared in [false, true] {
                    if prepared {
                        cache.prepare_buildings(allocator.buildings.len());
                        cache.prepare_access_for(&allocator, &network, &graph, |_| true);
                    }
                    for penalty in [0.0, 0.00001, 31.7] {
                        for (i, lane) in network.lane_system.lanes.iter_mut().enumerate() {
                            lane.live_frontage_delay_s = penalty * (i + 1) as f32;
                        }
                        network.publish_frontage_prices();
                        for (index, entrance) in allocator.entrances.iter().enumerate() {
                            let expected = borders
                                .iter()
                                .zip(&costs)
                                .filter_map(|(&border, costs)| {
                                    BuildingAllocator::freight_car_eta_to_entrance(
                                        border, entrance, costs, &network, &graph,
                                    )
                                })
                                .min_by(f32::total_cmp)
                                .map(f32::to_bits);
                            assert_eq!(
                                cache
                                    .minimum_border_eta(index, &allocator, &network, &graph)
                                    .map(f32::to_bits),
                                expected
                            );
                            assert_eq!(
                                cache
                                    .minimum_site_border_eta(&borders, entrance, &network, &graph)
                                    .map(f32::to_bits),
                                expected
                            );
                        }
                    }
                }
                // A proposed-door caller can select a previously prepared subset of borders.
                if borders.len() > 1 {
                    let subset = &borders[..1];
                    let entrance = &allocator.entrances[1];
                    assert_eq!(
                        cache
                            .minimum_site_border_eta(subset, entrance, &network, &graph)
                            .map(f32::to_bits),
                        BuildingAllocator::freight_car_eta_to_entrance(
                            subset[0], entrance, &costs[0], &network, &graph
                        )
                        .map(f32::to_bits)
                    );
                }
            }
        }
    }
}

#[test]
fn recurring_receiving_windows_price_local_and_imported_goods_without_markup_on_carriers() {
    use crate::simulation::economy::accessibility::{ModeComponentIndex, max_speed_for_modes};
    use crate::simulation::economy::operation::InputRequirement;
    let (graph, network, source_edge, buyer_edge, border) = simple_graph_with_border();
    let catalog = load_runtime_economy_catalog().unwrap();
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 2.0;
    let source_profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let buyer_profile = catalog.profile_for_id("grocery_basic").unwrap();
    let resource = source_profile.outputs[0].resource_runtime_id;
    let mut allocator = BuildingAllocator::new();
    let source_asset = register_test_asset(
        &mut allocator,
        "test",
        "window_source",
        ZoneClass::Industrial,
    );
    let buyer_asset = register_test_asset(
        &mut allocator,
        "test",
        "window_buyer",
        ZoneClass::Commercial,
    );
    let mut source = make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        source_edge,
        &source_asset,
        1_000.0,
        100_000.0,
    );
    source.worker_count = source_profile.worker_capacity;
    allocator.buildings.push(source);
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        buyer_edge,
        &buyer_asset,
        0.0,
        100_000.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut logistics = ShipmentSystem::new();
    let mut outside = [None; 2];
    logistics.write_border_carrier_quotes(
        &allocator,
        &network,
        &graph,
        &[border],
        &catalog,
        &tuning,
        &mut outside,
    );
    assert!(outside[1].is_some());
    let reservations = logistics.build_reservation_views(catalog.resource_count());
    let components = ModeComponentIndex::build(&graph, TransitFlags::CAR);
    let index = super::supplier_index::SupplierCandidateIndex::build(
        &allocator,
        &graph,
        &catalog,
        &components,
        &reservations,
        tuning.logistics.truck_payload_kg,
    );
    let mut routes = super::route_cache::FreightRouteCache::default();
    let mut supply = RecurringSupply::default();
    let mut carrier = None;
    for (minute, factor) in [(419, 1.1), (420, 1.0), (1079, 1.0), (1080, 1.1)] {
        let multiplier =
            freight_goods_multiplier(buyer_profile, ZoneType::Commercial, &tuning, minute);
        assert_eq!(multiplier, factor);
        let freight_profile = super::resource::freight_profile_for_building(
            &catalog,
            &tuning,
            &allocator.buildings[1],
        )
        .unwrap();
        let physical = super::local_supplier::find_local_supplier(
            1,
            200.0,
            false,
            source_profile.outputs[0].shipment_batch_kg,
            resource,
            100_000.0,
            &allocator,
            &network,
            &graph,
            &reservations,
            &index,
            &components,
            &mut routes,
            freight_profile,
            minute,
            &catalog,
            &tuning.logistics,
            max_speed_for_modes(&graph, TransitFlags::CAR),
            tuning.operational_clock.seconds_per_day,
        )
        .unwrap();
        let quote = |supply: &mut RecurringSupply, logistics: &mut ShipmentSystem, units| {
            logistics.quote_recurring_input(
                supply,
                InputRequirement {
                    resource,
                    daily_units: units,
                    buffer_units: units * 4.0,
                    replenishment_batch_units: Some(200.0),
                },
                Some(1),
                &allocator.entrances[1],
                outside[1],
                &allocator,
                &catalog,
                &tuning,
                &network,
                &graph,
                multiplier,
            )
        };
        supply.reset(&allocator, &graph, &network, &catalog, &[1.0, 0.0]);
        let local = quote(&mut supply, &mut logistics, 200.0);
        assert_eq!(
            local.goods as f32 + local.freight as f32,
            physical.total_cost
        );
        if let Some(previous) = carrier {
            assert_eq!(local.freight, previous);
        }
        carrier = Some(local.freight);
        supply.retry(&allocator, &graph);
        let mixed = quote(&mut supply, &mut logistics, 250.0);
        assert!((mixed.goods - 3_600.0 * f64::from(factor)).abs() < 0.001);
        assert!((mixed.buffer - 14_400.0 * f64::from(factor)).abs() < 0.01);
        assert!((supply.input_price(resource) - 14.4 * f64::from(factor)).abs() < 0.001);
        supply.reject(&allocator, &graph);
        assert_eq!(quote(&mut supply, &mut logistics, 250.0).goods, mixed.goods);
        supply.reject(&allocator, &graph);
        supply.reset(&allocator, &graph, &network, &catalog, &[0.0, 0.0]);
        let imported = quote(&mut supply, &mut logistics, 250.0);
        assert!((imported.goods - 6_000.0 * f64::from(factor)).abs() < 0.001);
        supply.reject(&allocator, &graph);
    }
}

#[test]
fn recurring_publication_preserves_claims_and_caps_new_supply_by_actual_staff() {
    use crate::simulation::economy::operation::InputRequirement;
    let (graph, network, source_edge, buyer_edge, _) = simple_graph_with_border();
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let resource = profile.outputs[0].resource_runtime_id;
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "published_food",
        ZoneClass::Industrial,
    );
    for (x, edge) in [(-50.0, source_edge), (25.0, buyer_edge), (75.0, buyer_edge)] {
        allocator.buildings.push(make_building(
            &allocator,
            x,
            ZoneType::Industrial,
            edge,
            &asset,
            0.0,
            100_000.0,
        ));
    }
    allocator.buildings[0].worker_count = 5;
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut logistics = ShipmentSystem::new();
    let quote = |book: &mut RecurringSupply, logistics: &mut ShipmentSystem, buyer, units| {
        logistics.quote_recurring_input(
            book,
            InputRequirement {
                resource,
                daily_units: units,
                buffer_units: units * 4.0,
                replenishment_batch_units: Some(200.0),
            },
            Some(buyer),
            &allocator.entrances[buyer],
            None,
            &allocator,
            &catalog,
            &tuning,
            &network,
            &graph,
            1.0,
        )
    };
    let mut book = RecurringSupply::default();
    book.reset(&allocator, &graph, &network, &catalog, &[0.1, 0.0, 0.0]);
    assert_eq!(quote(&mut book, &mut logistics, 1, 20.0).goods, 240.0);
    book.accept_for(1);
    assert!(!book.has_offers());
    book.publish_funded_activity(0, 0.1, 1.0, &allocator, &catalog, &graph);
    assert!(
        quote(&mut book, &mut logistics, 2, 81.0)
            .goods
            .is_infinite()
    );
    book.reject(&allocator, &graph);
    assert_eq!(quote(&mut book, &mut logistics, 2, 80.0).goods, 960.0);
    book.accept_for(2);
    assert!(!book.has_offers());
    let mut fork = RecurringSupply::default();
    fork.copy_reservations_from(&book, &allocator, &catalog, &graph);
    for book in [&mut book, &mut fork] {
        book.release_owner(1, &allocator, &graph);
        assert_eq!(quote(book, &mut logistics, 1, 20.0).goods, 240.0);
        book.retry(&allocator, &graph);
        assert!(quote(book, &mut logistics, 1, 21.0).goods.is_infinite());
        book.reject(&allocator, &graph);
        assert!(!book.has_offers());
        book.release_owner(2, &allocator, &graph);
        assert_eq!(quote(book, &mut logistics, 2, 80.0).goods, 960.0);
        book.reject(&allocator, &graph);
        assert!(!book.has_offers());
    }
}

#[test]
fn recurring_equal_route_quotes_match_dispatch_price_order_after_cached_search_and_retry() {
    use crate::simulation::economy::accessibility::{ModeComponentIndex, max_speed_for_modes};
    use crate::simulation::economy::definitions::compile_test_catalog;
    use crate::simulation::economy::operation::InputRequirement;
    let authored = include_str!("../../../../../economy/profiles.toml");
    let first = authored
        .find("[[profiles]]\nid = \"food_processor_basic\"")
        .unwrap();
    let end = first + authored[first + 1..].find("\n[[profiles]]").unwrap() + 1;
    let recipe = &authored[first..end];
    let cheap = recipe
        .replace("food_processor_basic", "cheaper_food")
        .replace("unit_price_currency = 12.0", "unit_price_currency = 6.0");
    let middle = recipe
        .replace("food_processor_basic", "middle_food")
        .replace("unit_price_currency = 12.0", "unit_price_currency = 8.0");
    let catalog = compile_test_catalog(&format!("{authored}\n{cheap}\n{middle}")).unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let resource = profile.outputs[0].resource_runtime_id;
    let (graph, network, source_edge, buyer_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(&mut allocator, "test", "price_ties", ZoneClass::Industrial);
    for id in ["food_processor_basic", "cheaper_food", "middle_food"] {
        // Identical doors guarantee identical directed routes and carrier costs.
        let mut source = make_building(
            &allocator,
            -50.0,
            ZoneType::Industrial,
            source_edge,
            &asset,
            1_000.0,
            100_000.0,
        );
        source.economy_profile_runtime_id = catalog.profile_for_id(id).unwrap().runtime_id;
        source.worker_count = profile.worker_capacity;
        allocator.buildings.push(source);
    }
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::Industrial,
        buyer_edge,
        &asset,
        0.0,
        100_000.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut logistics = ShipmentSystem::new();
    let reservations = logistics.build_reservation_views(catalog.resource_count());
    let components = ModeComponentIndex::build(&graph, TransitFlags::CAR);
    let index = super::supplier_index::SupplierCandidateIndex::build(
        &allocator,
        &graph,
        &catalog,
        &components,
        &reservations,
        tuning.logistics.truck_payload_kg,
    );
    let mut routes = super::route_cache::FreightRouteCache::default();
    let physical = super::local_supplier::find_local_supplier(
        3,
        200.0,
        false,
        profile.outputs[0].shipment_batch_kg,
        resource,
        100_000.0,
        &allocator,
        &network,
        &graph,
        &reservations,
        &index,
        &components,
        &mut routes,
        &tuning.operational_clock.freight_profiles[0],
        0,
        &catalog,
        &tuning.logistics,
        max_speed_for_modes(&graph, TransitFlags::CAR),
        tuning.operational_clock.seconds_per_day,
    )
    .unwrap();
    assert_eq!(physical.supplier_idx, 1);
    let mut supply = RecurringSupply::default();
    for buyer in [Some(3), None] {
        let quote = |supply: &mut RecurringSupply, logistics: &mut ShipmentSystem, units| {
            logistics.quote_recurring_input(
                supply,
                InputRequirement {
                    resource,
                    daily_units: units,
                    buffer_units: units * 2.0,
                    replenishment_batch_units: Some(200.0),
                },
                buyer,
                &allocator.entrances[3],
                None,
                &allocator,
                &catalog,
                &tuning,
                &network,
                &graph,
                1.0,
            )
        };
        supply.reset(
            &allocator,
            &graph,
            &network,
            &catalog,
            &[1.0, 0.0, 0.0, 0.0],
        );
        assert_eq!(quote(&mut supply, &mut logistics, 200.0).goods, 2_400.0);
        supply.reject(&allocator, &graph);
        // The cached incumbent is the expensive supplier. Equal-time candidates must
        // still be visited and compared by price, even when their index is larger.
        supply.reset(
            &allocator,
            &graph,
            &network,
            &catalog,
            &[1.0, 1.0, 1.0, 0.0],
        );
        let cheapest = quote(&mut supply, &mut logistics, 200.0);
        assert_eq!(cheapest.goods, 1_200.0);
        assert_eq!(
            cheapest.goods as f32 + cheapest.freight as f32,
            physical.total_cost
        );
        supply.retry(&allocator, &graph);
        let mixed = quote(&mut supply, &mut logistics, 250.0);
        assert_eq!(mixed.goods, 1_600.0);
        supply.reject(&allocator, &graph);
        assert_eq!(quote(&mut supply, &mut logistics, 250.0).goods, mixed.goods);
        supply.accept();
        assert_eq!(quote(&mut supply, &mut logistics, 150.0).goods, 1_200.0);
        supply.accept();
        assert_eq!(quote(&mut supply, &mut logistics, 200.0).goods, 2_400.0);
        supply.reject(&allocator, &graph);
    }
}

#[test]
fn projected_supplier_uses_its_recipe_door_and_price_and_restores_all_commitments() {
    use crate::simulation::buildings::allocator::BuildingEntrance;
    use crate::simulation::economy::definitions::compile_test_catalog;
    use crate::simulation::economy::operation::InputRequirement;
    let authored = include_str!("../../../../../economy/profiles.toml");
    let first = authored
        .find("[[profiles]]\nid = \"food_processor_basic\"")
        .unwrap();
    let end = first + authored[first + 1..].find("\n[[profiles]]").unwrap() + 1;
    let cheap = authored[first..end]
        .replace("food_processor_basic", "replacement_food")
        .replace("unit_price_currency = 12.0", "unit_price_currency = 6.0");
    let catalog = compile_test_catalog(&format!("{authored}\n{cheap}")).unwrap();
    let original = catalog.profile_for_id("food_processor_basic").unwrap();
    let replacement = catalog.profile_for_id("replacement_food").unwrap();
    let farm = catalog.profile_for_id("grain_farm_basic").unwrap();
    let food = original.outputs[0].resource_runtime_id;
    let grain = farm.outputs[0].resource_runtime_id;
    let tuning = load_runtime_economy_tuning().unwrap();
    let (graph, mut network, edge, buyer_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "projected_source",
        ZoneClass::Industrial,
    );
    for (x, edge) in [(-50.0, edge), (25.0, buyer_edge), (75.0, buyer_edge)] {
        let mut building = make_building(
            &allocator,
            x,
            ZoneType::Industrial,
            edge,
            &asset,
            0.0,
            100_000.0,
        );
        building.economy_profile_runtime_id = original.runtime_id;
        building.worker_count = original.worker_capacity;
        allocator.buildings.push(building);
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let proposed_door = allocator.entrances[2].clone();
    let input = |resource, units| InputRequirement {
        resource,
        daily_units: units,
        buffer_units: units * 2.0,
        replenishment_batch_units: Some(40.0),
    };
    let quote = |book: &mut RecurringSupply,
                 logistics: &mut ShipmentSystem,
                 owner: usize,
                 resource,
                 units,
                 network: &TransitNetwork| {
        logistics.quote_recurring_input(
            book,
            input(resource, units),
            Some(owner),
            &allocator.entrances[owner],
            None,
            &allocator,
            &catalog,
            &tuning,
            network,
            &graph,
            1.0,
        )
    };
    let mut logistics = ShipmentSystem::new();
    logistics.prepare_site_carrier_quotes(&allocator, &network, &graph, &[]);
    let mut live = RecurringSupply::default();
    live.reset(&allocator, &graph, &network, &catalog, &[1.0, 0.0, 0.0]);
    let old_cost = quote(&mut live, &mut logistics, 1, food, 80.0, &network);
    live.accept_for(1);
    let mut book = RecurringSupply::default();
    for pass in 0..2 {
        if pass > 0 {
            network.cch_graph.metric_revision += 1;
            assert!(!live.matches_layout(&allocator, &network));
            for lane in &mut network.lane_system.lanes {
                lane.frontage_delay_penalty_s = 0.0;
            }
            network.frontage_delay_revision += 1;
            logistics.prepare_site_carrier_quotes(&allocator, &network, &graph, &[]);
            live.reset(&allocator, &graph, &network, &catalog, &[1.0, 0.0, 0.0]);
            assert!(live.matches_layout(&allocator, &network));
            assert_eq!(
                quote(&mut live, &mut logistics, 1, food, 80.0, &network).goods,
                old_cost.goods
            );
            live.accept_for(1);
        }
        book.copy_reservations_from(&live, &allocator, &catalog, &graph);
        book.prepare_projected_sources(&catalog, &graph, &[0; 3], |_, _| {
            assert_eq!(
                pass, 0,
                "warm passes must retain their prepared variant geometry"
            );
            vec![
                ProjectedSupplySource::new(
                    0,
                    "cheap",
                    replacement.runtime_id,
                    proposed_door.clone(),
                ),
                ProjectedSupplySource::new(0, "farm", farm.runtime_id, proposed_door.clone()),
                ProjectedSupplySource::new(
                    0,
                    "unreachable",
                    replacement.runtime_id,
                    BuildingEntrance::default(),
                ),
            ]
        });
        book.prepare_owners(3, 3);
        let cheap = book.projected_source(0, "cheap").unwrap();
        let farm = book.projected_source(0, "farm").unwrap();
        let unreachable = book.projected_source(0, "unreachable").unwrap();
        assert!(cheap >= allocator.buildings.len());
        let storage = (
            book.projected_storage(),
            book.receipt_storage(),
            book.revalidation_storage(),
        );
        for round in 0..32 {
            for lane in &mut network.lane_system.lanes {
                lane.frontage_delay_penalty_s = if round % 2 == 0 { 0.0 } else { 40.0 };
            }
            network.frontage_delay_revision += 1;
            book.begin_revalidation();
            book.replace_source_output(0, 1.0, 0.0, |_| 0.0, &allocator, &catalog, &graph);
            assert!(
                book.has_overclaims(),
                "withdrawing production preserves the incumbent's buyer commitments"
            );
            book.replace_source_output(cheap, 1.0, 1.0, |_| 20.0, &allocator, &catalog, &graph);
            book.release_owner(1, &allocator, &graph);
            let revised = quote(&mut book, &mut logistics, 1, food, 80.0, &network);
            let carrier = logistics
                .customer_carrier_quote(
                    &proposed_door,
                    &allocator.entrances[1],
                    None,
                    Some(1),
                    &allocator,
                    &network,
                    &graph,
                    &tuning,
                )
                .unwrap();
            let expected = input(food, 80.0).at_prices(6.0, carrier);
            if round == 0 {
                assert_ne!(
                    expected.freight, old_cost.freight,
                    "the proposed door must distinguish this route"
                );
            }
            assert_eq!(
                [revised.goods, revised.freight, revised.buffer],
                [expected.goods, expected.freight, expected.buffer]
            );
            book.accept_for(1);
            assert!(!book.has_overclaims());
            let mut received = Vec::new();
            assert!(book.visit_input_sources(
                1,
                input(food, 80.0),
                99.0,
                None,
                |source, units, _, _| received.push((source, units))
            ));
            assert_eq!(
                received,
                [(Some(0), 80.0)],
                "financing retains the real owner, not a variant handle"
            );
            assert!(
                quote(&mut book, &mut logistics, 0, food, 1.0, &network)
                    .goods
                    .is_infinite(),
                "an owner cannot purchase its own projected output"
            );
            book.reject_revalidation(&allocator, &graph);
            let restored = book.owner_input_costs(1, input(food, 80.0), 99.0, None);
            assert_eq!(
                [
                    restored.goods.to_bits(),
                    restored.freight.to_bits(),
                    restored.buffer.to_bits()
                ],
                [
                    old_cost.goods.to_bits(),
                    old_cost.freight.to_bits(),
                    old_cost.buffer.to_bits()
                ]
            );
            assert_eq!(
                (
                    book.projected_storage(),
                    book.receipt_storage(),
                    book.revalidation_storage()
                ),
                storage
            );
        }
        book.begin_revalidation();
        book.replace_source_output(0, 1.0, 0.0, |_| 0.0, &allocator, &catalog, &graph);
        book.replace_source_output(unreachable, 1.0, 1.0, |_| 0.0, &allocator, &catalog, &graph);
        assert!(
            quote(&mut book, &mut logistics, 1, food, 10.0, &network)
                .goods
                .is_infinite()
        );
        book.reject(&allocator, &graph);
        book.replace_source_output(farm, 1.0, 1.0, |_| 0.0, &allocator, &catalog, &graph);
        assert_eq!(
            quote(&mut book, &mut logistics, 2, grain, 10.0, &network).goods,
            f64::from(
                catalog
                    .profile_for_id("grain_farm_basic")
                    .unwrap()
                    .unit_price_currency
            ) * 10.0
        );
        book.accept_for(2);
        assert!(
            book.has_overclaims(),
            "a changed recipe cannot erase an unsupported food commitment"
        );
        book.reject_revalidation(&allocator, &graph);
        assert!(!book.has_overclaims());
        assert_eq!(
            (
                book.projected_storage(),
                book.receipt_storage(),
                book.revalidation_storage()
            ),
            storage
        );
        book.begin_revalidation();
        book.replace_source_output(0, 1.0, 0.0, |_| 0.0, &allocator, &catalog, &graph);
        book.replace_source_output(cheap, 1.0, 1.0, |_| 20.0, &allocator, &catalog, &graph);
        book.release_owner(1, &allocator, &graph);
        assert_eq!(
            quote(&mut book, &mut logistics, 1, food, 80.0, &network).goods,
            480.0
        );
        book.accept_for(1);
        book.accept_revalidation();
        let mut owners = Vec::new();
        book.visit_dependents(cheap, |owner, resource, units| {
            owners.push((owner, resource, units))
        });
        assert_eq!(owners, [(1, food, 80.0)]);
        book.visit_dependents(0, |_, _, _| {
            panic!("retired source cannot retain accepted buyer receipts")
        });
        assert_eq!(
            quote(&mut book, &mut logistics, 1, food, 100.0, &network).goods,
            600.0
        );
        book.reject(&allocator, &graph);
        assert!(
            quote(&mut book, &mut logistics, 1, food, 101.0, &network)
                .goods
                .is_infinite()
        );
        book.reject(&allocator, &graph);
        assert_eq!(
            allocator.buildings[0].economy_profile_runtime_id,
            original.runtime_id
        );
    }
    // The previous grain quote retains a projected supplier as its search bound. A
    // subsequent asset/layout rebuild removes that handle while retaining the live fork.
    book.prepare_owners(16_384, 16_384);
    let retained_capacity = (book.receipt_storage().1, book.revalidation_storage());
    network.rebuild_cch_and_check(&graph);
    live.reset(&allocator, &graph, &network, &catalog, &[1.0, 0.0, 0.0]);
    book.copy_reservations_from(&live, &allocator, &catalog, &graph);
    assert!(book.receipt_storage().1 >= retained_capacity.0);
    assert!(
        book.revalidation_storage()
            .into_iter()
            .zip(retained_capacity.1)
            .all(|(after, before)| after >= before)
    );
    book.prepare_projected_sources(&catalog, &graph, &[1; 3], |previous, _| {
        assert_eq!(
            previous.len(),
            3,
            "a changed live layout retains only geometry as factory input"
        );
        Vec::new()
    });
    assert!(
        quote(&mut book, &mut logistics, 2, grain, 1.0, &network)
            .goods
            .is_infinite()
    );
    book.reject(&allocator, &graph);
}

#[test]
fn physical_receipt_capacity_rejects_publication_before_mutation_and_replays() {
    use crate::simulation::economy::operation::InputRequirement;
    let (graph, network, source_edge, buyer_edge, _) = simple_graph_with_border();
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let resource = profile.outputs[0].resource_runtime_id;
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(&mut allocator, "test", "capacity", ZoneClass::Industrial);
    for (x, edge) in [(-25.0, source_edge), (50.0, buyer_edge)] {
        let mut building = make_building(
            &allocator,
            x,
            ZoneType::Industrial,
            edge,
            &asset,
            0.0,
            100_000.0,
        );
        building.economy_profile_runtime_id = profile.runtime_id;
        building.worker_count = profile.worker_capacity;
        allocator.buildings.push(building);
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut book = RecurringSupply::default();
    book.reset(&allocator, &graph, &network, &catalog, &[1.0, 1.0]);
    book.commit_customer_delivery(
        0,
        1,
        resource,
        5.0,
        [12.0, 2.0],
        &allocator,
        &catalog,
        &graph,
    )
    .unwrap();
    book.shrink_receipt_storage_for_test();
    assert_eq!(book.receipt_storage(), (1, 1));
    let received = |book: &RecurringSupply| {
        let mut flows = Vec::new();
        book.visit_dependents(0, |buyer, resource, units| {
            flows.push((buyer, resource, units))
        });
        flows
    };
    let original = received(&book);
    book.begin_revalidation();
    book.replace_source_output(0, 1.0, 0.5, |_| 0.0, &allocator, &catalog, &graph);
    assert_eq!(
        book.commit_customer_delivery(
            0,
            1,
            resource,
            20.0,
            [12.0, 2.0],
            &allocator,
            &catalog,
            &graph
        ),
        Err(2)
    );
    assert_eq!(received(&book), original);
    assert_eq!(book.receipt_storage(), (1, 1));
    let mut logistics = ShipmentSystem::new();
    let remaining = logistics.quote_recurring_input(
        &mut book,
        InputRequirement {
            resource,
            daily_units: 95.0,
            buffer_units: 95.0,
            replenishment_batch_units: Some(40.0),
        },
        Some(1),
        &allocator.entrances[1],
        None,
        &allocator,
        &catalog,
        &tuning,
        &network,
        &graph,
        1.0,
    );
    assert_eq!(
        remaining.goods,
        95.0 * 12.0,
        "capacity failure must not consume source output"
    );
    book.reject_revalidation(&allocator, &graph);
    assert_eq!(received(&book), original);
    book.reserve_receipts(2);
    let capacity = (book.receipt_storage().1, book.revalidation_storage());
    for round in 0..65 {
        book.begin_revalidation();
        book.replace_source_output(0, 1.0, 0.5, |_| 0.0, &allocator, &catalog, &graph);
        book.commit_customer_delivery(
            0,
            1,
            resource,
            20.0,
            [12.0, 2.0],
            &allocator,
            &catalog,
            &graph,
        )
        .unwrap();
        assert_eq!(received(&book), [(1, resource, 20.0), (1, resource, 5.0)]);
        if round == 64 {
            book.accept_revalidation();
        } else {
            book.reject_revalidation(&allocator, &graph);
            assert_eq!(received(&book), original);
        }
        assert_eq!(
            (book.receipt_storage().1, book.revalidation_storage()),
            capacity
        );
    }
}

#[test]
fn excess_input_withdrawal_keeps_earlier_buyers_and_restores_exact_receipts() {
    use crate::simulation::economy::operation::InputRequirement;
    let (graph, network, edge, buyer_edge, _) = simple_graph_with_border();
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let resource = profile.outputs[0].resource_runtime_id;
    let rate = f64::from(profile.net_output_units_per_day(&profile.outputs[0]));
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "withdraw_excess",
        ZoneClass::Industrial,
    );
    for (x, edge) in [
        (-50.0, edge),
        (-10.0, edge),
        (25.0, buyer_edge),
        (75.0, buyer_edge),
    ] {
        let mut building = make_building(
            &allocator,
            x,
            ZoneType::Industrial,
            edge,
            &asset,
            0.0,
            100_000.0,
        );
        building.economy_profile_runtime_id = profile.runtime_id;
        building.worker_count = profile.worker_capacity;
        allocator.buildings.push(building);
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut book = RecurringSupply::default();
    book.reset(&allocator, &graph, &network, &catalog, &[1.0; 4]);
    for (source, owner, share) in [
        (0, 2, 0.25),
        (1, 3, 0.125),
        (0, 3, 0.25),
        (1, 2, 0.125),
        (0, 3, 0.25),
    ] {
        book.commit_customer_delivery(
            source,
            owner,
            resource,
            rate * share,
            [11.0 + source as f32, 2.0],
            &allocator,
            &catalog,
            &graph,
        )
        .unwrap();
    }
    book.prepare_owners(4, 4);
    let input = InputRequirement {
        resource,
        daily_units: rate as f32,
        buffer_units: 2.0 * rate as f32,
        replenishment_batch_units: Some(40.0),
    };
    let snapshot = |book: &RecurringSupply| {
        let mut deliveries = Vec::new();
        for source in 0..2 {
            book.visit_dependents(source, |owner, resource, units| {
                deliveries.push((source, owner, resource, units.to_bits()))
            });
        }
        let mut purchases = Vec::new();
        for owner in 2..4 {
            assert!(book.visit_input_sources(
                owner,
                input,
                20.0,
                Some(100.0),
                |source, units, goods, carrier| purchases.push((
                    owner,
                    source,
                    units.to_bits(),
                    goods.to_bits(),
                    carrier.to_bits()
                ))
            ));
        }
        (deliveries, purchases)
    };
    let original = snapshot(&book);
    let storage = (book.receipt_storage(), book.revalidation_storage());
    let mut affected = Vec::with_capacity(4);
    let mut seen = vec![false; 4];
    for activity in [0.625, 0.375, 0.0] {
        for _ in 0..16 {
            book.begin_revalidation();
            book.publish_funded_activity(0, 1.0, activity, &allocator, &catalog, &graph);
            assert!(book.has_overclaims());
            book.withdraw_overclaims(&mut affected, &mut seen, &allocator, &graph);
            assert!(!book.has_overclaims());
            assert_eq!(affected, if activity > 0.0 { vec![3] } else { vec![3, 2] });
            let mut units = [0.0; 4];
            book.visit_dependents(0, |owner, _, quantity| units[owner] += quantity);
            assert_eq!(units[2], if activity > 0.0 { rate * 0.25 } else { 0.0 });
            assert_eq!(
                units[3],
                if activity > 0.0 {
                    rate * (f64::from(activity) - 0.25)
                } else {
                    0.0
                }
            );
            assert_eq!(
                snapshot(&book)
                    .0
                    .into_iter()
                    .filter(|row| row.0 == 1)
                    .collect::<Vec<_>>(),
                original
                    .0
                    .iter()
                    .copied()
                    .filter(|row| row.0 == 1)
                    .collect::<Vec<_>>()
            );
            // Reuse retired cells inside the outer proof, then reject its changed owner
            // endpoints, partially trimmed receipts and source reverse links together.
            book.commit_customer_delivery(
                1,
                3,
                resource,
                rate * 0.125,
                [12.0, 2.0],
                &allocator,
                &catalog,
                &graph,
            )
            .unwrap();
            book.reject_revalidation(&allocator, &graph);
            assert_eq!(snapshot(&book), original);
            assert_eq!(
                (book.receipt_storage(), book.revalidation_storage()),
                storage
            );
            assert!(!book.has_overclaims());
            for owner in affected.drain(..) {
                seen[owner] = false;
            }
        }
    }
    book.begin_revalidation();
    book.publish_funded_activity(0, 1.0, 0.375, &allocator, &catalog, &graph);
    book.withdraw_overclaims(&mut affected, &mut seen, &allocator, &graph);
    book.accept_revalidation();
    let accepted = snapshot(&book);
    // A later rejected full-owner replacement must retain the accepted partial withdrawal.
    book.release_owner(3, &allocator, &graph);
    book.reject(&allocator, &graph);
    assert_eq!(snapshot(&book), accepted);
}

#[test]
fn downstream_input_gap_preserves_unaffected_tranches_and_exact_rollback() {
    use crate::simulation::economy::operation::InputRequirement;
    let (graph, network, edge, buyer_edge, _) = simple_graph_with_border();
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let resource = profile.outputs[0].resource_runtime_id;
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(&mut allocator, "test", "input_gap", ZoneClass::Industrial);
    for (x, edge) in [
        (-50.0, edge),
        (-10.0, edge),
        (25.0, buyer_edge),
        (75.0, buyer_edge),
    ] {
        let mut building = make_building(
            &allocator,
            x,
            ZoneType::Industrial,
            edge,
            &asset,
            0.0,
            100_000.0,
        );
        building.economy_profile_runtime_id = profile.runtime_id;
        building.worker_count = profile.worker_capacity;
        allocator.buildings.push(building);
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let input = InputRequirement {
        resource,
        daily_units: 500.0,
        buffer_units: 1_000.0,
        replenishment_batch_units: Some(40.0),
    };
    let imported = imported_input_unit_price(&catalog, &tuning, resource, 1.0);
    let mut logistics = ShipmentSystem::new();
    logistics.prepare_site_carrier_quotes(&allocator, &network, &graph, &[]);
    let mut book = RecurringSupply::default();
    book.reset(
        &allocator,
        &graph,
        &network,
        &catalog,
        &[1.0, 1.0, 1.0, 1.0],
    );
    // Removed cells occupy the owner's head, middle and tail. Retained fractional
    // quantities deliberately leave a gap that cannot be represented exactly in f32.
    for (source, units, price) in [
        (0, 0.1, 12.0),
        (1, 0.1, 21.0),
        (0, 0.2, 12.0),
        (2, 0.2, 27.0),
        (0, 0.3, 12.0),
    ] {
        book.commit_customer_delivery(
            source,
            3,
            resource,
            units,
            [price, 2.0],
            &allocator,
            &catalog,
            &graph,
        )
        .unwrap();
    }
    book.prepare_owners(4, 4);
    let storage = (book.receipt_storage(), book.revalidation_storage());
    let received = |book: &RecurringSupply| {
        let mut tranches = Vec::new();
        assert!(book.visit_input_sources(
            3,
            input,
            imported,
            Some(1_000_000.0),
            |source, units, goods, carrier| tranches.push((source, units, goods, carrier))
        ));
        tranches
    };
    let dependents = |book: &RecurringSupply, source| {
        let mut values = Vec::new();
        book.visit_dependents(source, |owner, resource, units| {
            values.push((owner, resource, units))
        });
        values
    };
    let costs = |book: &RecurringSupply| {
        let costs = book.owner_input_costs(3, input, imported, Some(1_000_000.0));
        [
            costs.goods.to_bits(),
            costs.freight.to_bits(),
            costs.buffer.to_bits(),
        ]
    };
    let original = (
        received(&book),
        costs(&book),
        [
            dependents(&book, 0),
            dependents(&book, 1),
            dependents(&book, 2),
        ],
    );
    for round in 0..65 {
        book.begin_revalidation();
        book.replace_source_output(0, 1.0, 0.0, |_| 0.0, &allocator, &catalog, &graph);
        book.release_source_from_owner(3, 0, &allocator, &graph);
        assert!(dependents(&book, 0).is_empty());
        assert_eq!(dependents(&book, 1), original.2[1]);
        assert_eq!(dependents(&book, 2), original.2[2]);
        assert!(
            !book.has_overclaims(),
            "last removed receipt must restore exact zero production"
        );
        let outside = (round % 2 == 0).then_some(1_000_000.0);
        // A revised door has no cached live buyer route, but must still exclude its own output.
        let buyer = (round % 4 < 2).then_some(3);
        let revised = logistics.quote_missing_recurring_input(
            &mut book,
            input,
            3,
            buyer,
            &allocator.entrances[3],
            outside,
            &allocator,
            &catalog,
            &tuning,
            &network,
            &graph,
            1.0,
        );
        assert_eq!(revised.goods.is_finite(), outside.is_some());
        if outside.is_some() {
            book.extend_inputs_for(3);
            let tranches = received(&book);
            assert_eq!(
                &tranches[..2],
                &[(Some(1), 0.1, 21.0, 2.0), (Some(2), 0.2, 27.0, 2.0)]
            );
            assert_eq!(
                tranches.last().unwrap().0,
                None,
                "only the actual remaining gap is imported"
            );
            assert_eq!(
                costs(&book),
                [
                    revised.goods.to_bits(),
                    revised.freight.to_bits(),
                    revised.buffer.to_bits()
                ]
            );
            assert!(!book.has_overclaims());
            if round == 64 {
                book.accept_revalidation();
                assert!(dependents(&book, 0).is_empty());
                break;
            }
        }
        // Alternate rollback after a failed staged quote and after an accepted inner repair.
        book.reject_revalidation(&allocator, &graph);
        assert_eq!(
            (
                received(&book),
                costs(&book),
                [
                    dependents(&book, 0),
                    dependents(&book, 1),
                    dependents(&book, 2)
                ]
            ),
            original
        );
        assert_eq!(
            (book.receipt_storage(), book.revalidation_storage()),
            storage
        );
    }
    assert_eq!(
        (book.receipt_storage(), book.revalidation_storage()),
        storage
    );
    // An accepted repair must also leave valid owner links for subsequent full replacement.
    book.begin_revalidation();
    book.release_owner(3, &allocator, &graph);
    let revised = logistics.quote_recurring_input(
        &mut book,
        input,
        Some(3),
        &allocator.entrances[3],
        Some(1_000_000.0),
        &allocator,
        &catalog,
        &tuning,
        &network,
        &graph,
        1.0,
    );
    assert!(revised.goods.is_finite());
    book.accept_for(3);
    book.accept_revalidation();
    assert!(dependents(&book, 0).is_empty());
    assert!(!book.has_overclaims());
}

#[test]
fn downstream_revalidation_restores_accepted_and_pending_owner_quotes_together() {
    use crate::simulation::economy::operation::InputRequirement;
    let (graph, network, source_edge, buyer_edge, _) = simple_graph_with_border();
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let resource = profile.outputs[0].resource_runtime_id;
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "atomic_supply",
        ZoneClass::Industrial,
    );
    for (x, edge) in [
        (-50.0, source_edge),
        (-10.0, source_edge),
        (25.0, buyer_edge),
        (75.0, buyer_edge),
    ] {
        let mut building = make_building(
            &allocator,
            x,
            ZoneType::Industrial,
            edge,
            &asset,
            0.0,
            100_000.0,
        );
        building.worker_count = profile.worker_capacity;
        allocator.buildings.push(building);
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let requirement = |units| InputRequirement {
        resource,
        daily_units: units,
        buffer_units: units * 2.0,
        replenishment_batch_units: Some(40.0),
    };
    let mut logistics = ShipmentSystem::new();
    logistics.prepare_site_carrier_quotes(&allocator, &network, &graph, &[]);
    let quote = |book: &mut RecurringSupply, logistics: &mut ShipmentSystem, owner, units| {
        logistics.quote_recurring_input(
            book,
            requirement(units),
            Some(owner),
            &allocator.entrances[owner],
            None,
            &allocator,
            &catalog,
            &tuning,
            &network,
            &graph,
            1.0,
        )
    };
    let dependents = |book: &RecurringSupply, source| {
        let mut values = Vec::new();
        book.visit_dependents(source, |owner, resource, units| {
            values.push((owner, resource, units))
        });
        values.sort_by_key(|&(owner, resource, _)| (owner, resource));
        values
    };
    let mut live = RecurringSupply::default();
    live.reset(
        &allocator,
        &graph,
        &network,
        &catalog,
        &[1.0, 0.0, 0.0, 0.0],
    );
    let mut costs = Vec::new();
    for (owner, units) in [(2, 120.0), (3, 80.0)] {
        let original = quote(&mut live, &mut logistics, owner, units);
        assert!(original.goods.is_finite());
        costs.push([
            original.goods.to_bits(),
            original.freight.to_bits(),
            original.buffer.to_bits(),
        ]);
        live.accept_for(owner);
    }
    let expected = dependents(&live, 0);
    let mut book = RecurringSupply::default();
    for _ in 0..2 {
        book.copy_reservations_from(&live, &allocator, &catalog, &graph);
        book.prepare_owners(5, 6);
        let storage = (book.receipt_storage(), book.revalidation_storage());
        for round in 0..128 {
            book.begin_revalidation();
            book.publish_funded_activity(0, 1.0, 0.0, &allocator, &catalog, &graph);
            book.publish_funded_activity(1, 0.0, 1.0, &allocator, &catalog, &graph);
            book.reserve_household_output(1, resource, 0.0, 0.1, &allocator, &catalog, &graph);
            // A newly accepted owner grows the arena before old owners recycle their cells.
            book.commit_customer_delivery(
                1,
                4,
                resource,
                10.0,
                [12.0, 3.0],
                &allocator,
                &catalog,
                &graph,
            )
            .unwrap();
            let owners = if round % 2 == 0 {
                [(2, 120.0), (3, 80.0)]
            } else {
                [(3, 80.0), (2, 120.0)]
            };
            for (position, (owner, units)) in owners.into_iter().enumerate() {
                book.release_owner(owner, &allocator, &graph);
                let revised = quote(&mut book, &mut logistics, owner, units);
                assert_eq!(revised.goods.is_finite(), position == 0);
                if position == 0 {
                    book.accept_for(owner);
                }
            }
            // One earlier owner committed, the final failed quote is still staged, and
            // both sources changed. The enclosing rejection must undo all three kinds.
            book.reject_revalidation(&allocator, &graph);
            assert_eq!(dependents(&book, 0), expected);
            assert!(dependents(&book, 1).is_empty());
            assert!(!book.has_offers());
            assert_eq!(
                (book.receipt_storage(), book.revalidation_storage()),
                storage
            );
            for (index, (owner, units)) in [(2, 120.0), (3, 80.0)].into_iter().enumerate() {
                let retained = book.owner_input_costs(owner, requirement(units), 1_000.0, None);
                assert_eq!(
                    [
                        retained.goods.to_bits(),
                        retained.freight.to_bits(),
                        retained.buffer.to_bits()
                    ],
                    costs[index]
                );
            }
        }
        book.begin_revalidation();
        book.publish_funded_activity(0, 1.0, 0.0, &allocator, &catalog, &graph);
        book.publish_funded_activity(1, 0.0, 1.0, &allocator, &catalog, &graph);
        book.commit_customer_delivery(
            1,
            4,
            resource,
            10.0,
            [12.0, 3.0],
            &allocator,
            &catalog,
            &graph,
        )
        .unwrap();
        for (owner, units) in [(2, 110.0), (3, 70.0)] {
            book.release_owner(owner, &allocator, &graph);
            assert!(
                quote(&mut book, &mut logistics, owner, units)
                    .goods
                    .is_finite()
            );
            book.accept_for(owner);
        }
        assert!(!book.has_overclaims());
        book.accept_revalidation();
        assert!(dependents(&book, 0).is_empty());
        assert_eq!(
            dependents(&book, 1),
            [
                (2, resource, 110.0),
                (3, resource, 70.0),
                (4, resource, 10.0)
            ]
        );
        assert!(quote(&mut book, &mut logistics, 2, 10.0).goods.is_finite());
        book.reject(&allocator, &graph);
        assert!(
            quote(&mut book, &mut logistics, 2, 11.0)
                .goods
                .is_infinite()
        );
        book.reject(&allocator, &graph);
    }
    assert_eq!(
        dependents(&live, 0),
        expected,
        "the fork cannot alter live receipts"
    );
}

#[test]
fn recurring_owner_replacement_restores_rejection_and_retires_only_its_own_claims() {
    use crate::simulation::economy::operation::InputRequirement;
    let (graph, network, source_edge, buyer_edge, _) = simple_graph_with_border();
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let resource = profile.outputs[0].resource_runtime_id;
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "owner_inputs",
        ZoneClass::Industrial,
    );
    for (x, edge) in [(-50.0, source_edge), (25.0, buyer_edge), (75.0, buyer_edge)] {
        allocator.buildings.push(make_building(
            &allocator,
            x,
            ZoneType::Industrial,
            edge,
            &asset,
            0.0,
            100_000.0,
        ));
    }
    allocator.buildings[0].worker_count = profile.worker_capacity;
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut live = RecurringSupply::default();
    live.reset(&allocator, &graph, &network, &catalog, &[1.0, 0.0, 0.0]);
    let mut logistics = ShipmentSystem::new();
    let quote = |book: &mut RecurringSupply, logistics: &mut ShipmentSystem, units| {
        logistics.quote_recurring_input(
            book,
            InputRequirement {
                resource,
                daily_units: units,
                buffer_units: units * 2.0,
                replenishment_batch_units: Some(40.0),
            },
            Some(1),
            &allocator.entrances[1],
            None,
            &allocator,
            &catalog,
            &tuning,
            &network,
            &graph,
            1.0,
        )
    };
    for (owner, units) in [(1, 120.0), (2, 80.0)] {
        let original = quote(&mut live, &mut logistics, units);
        assert!(original.goods.is_finite());
        live.accept_for(owner);
        let retained = live.owner_input_costs(
            owner,
            InputRequirement {
                resource,
                daily_units: units,
                buffer_units: units * 2.0,
                replenishment_batch_units: Some(40.0),
            },
            1_000.0,
            None,
        );
        assert_eq!(
            (retained.goods, retained.freight, retained.buffer),
            (original.goods, original.freight, original.buffer)
        );
    }
    let dependents = |book: &RecurringSupply| {
        let mut result = Vec::new();
        book.visit_dependents(0, |owner, resource, units| {
            result.push((owner, resource, units))
        });
        result.sort_unstable_by_key(|&(owner, resource, _)| (owner, resource));
        result
    };
    let expected = vec![(1, resource, 120.0), (2, resource, 80.0)];
    assert_eq!(dependents(&live), expected);
    assert!(!live.has_offers());
    let mut projected = RecurringSupply::default();
    for _ in 0..2 {
        // Exercise both initial geometry copying and the warm receipt/quantity copy.
        projected.copy_reservations_from(&live, &allocator, &catalog, &graph);
        assert_eq!(dependents(&projected), expected);
        let storage = projected.receipt_storage();
        for round in 0..128 {
            let owners = if round % 2 == 0 {
                [(1, 120.0), (2, 80.0)]
            } else {
                [(2, 80.0), (1, 120.0)]
            };
            for (owner, units) in owners {
                projected.release_owner(owner, &allocator, &graph);
                let original = quote(&mut projected, &mut logistics, units);
                assert!(original.goods.is_finite());
                projected.accept_for(owner);
                assert_eq!(dependents(&projected), expected);
                assert_eq!(
                    projected.receipt_storage(),
                    storage,
                    "accepted dependency revalidation must recycle source/owner receipt links"
                );
                let retained = projected.owner_input_costs(
                    owner,
                    InputRequirement {
                        resource,
                        daily_units: units,
                        buffer_units: units * 2.0,
                        replenishment_batch_units: Some(40.0),
                    },
                    1_000.0,
                    None,
                );
                assert_eq!(
                    (retained.goods, retained.freight, retained.buffer),
                    (original.goods, original.freight, original.buffer)
                );
            }
        }
        for _ in 0..32 {
            projected.release_owner(1, &allocator, &graph);
            assert_eq!(dependents(&projected), [(2, resource, 80.0)]);
            assert!(
                !quote(&mut projected, &mut logistics, 121.0)
                    .goods
                    .is_finite(),
                "an incumbent cannot reclaim the other owner's 80 units"
            );
            projected.reject(&allocator, &graph);
            assert_eq!(dependents(&projected), expected);
            assert!(
                !projected.has_offers(),
                "rollback must restore depleted index entries too"
            );
        }
        projected.release_owner(1, &allocator, &graph);
        assert!(
            quote(&mut projected, &mut logistics, 100.0)
                .goods
                .is_finite()
        );
        projected.retry(&allocator, &graph);
        assert!(
            quote(&mut projected, &mut logistics, 100.0)
                .goods
                .is_finite()
        );
        projected.accept();
        assert_eq!(dependents(&projected), [(2, resource, 80.0)]);
        projected.release_owner(1, &allocator, &graph);
        assert!(
            !quote(&mut projected, &mut logistics, 21.0)
                .goods
                .is_finite(),
            "an accepted replacement retires the old receipt exactly once"
        );
        projected.reject(&allocator, &graph);
        assert!(
            quote(&mut projected, &mut logistics, 20.0)
                .goods
                .is_finite()
        );
        projected.reject(&allocator, &graph);
    }
    assert!(
        !live.has_offers(),
        "project probes cannot alter live commitments"
    );
}

#[test]
fn recurring_input_quotes_reserve_production_not_stock_and_roll_back_rejection() {
    use crate::simulation::economy::operation::InputRequirement;
    let (graph, network, source_edge, buyer_edge, _) = simple_graph_with_border();
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let resource = profile.outputs[0].resource_runtime_id;
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(&mut allocator, "test", "recurring", ZoneClass::Industrial);
    for (x, edge) in [(-50.0, source_edge), (50.0, buyer_edge)] {
        allocator.buildings.push(make_building(
            &allocator,
            x,
            ZoneType::Industrial,
            edge,
            &asset,
            1_000_000.0,
            100_000.0,
        ));
    }
    allocator.buildings[0].worker_count = profile.worker_capacity;
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut supply = RecurringSupply::default();
    supply.reset(&allocator, &graph, &network, &catalog, &[1.0, 0.0]);
    let mut logistics = ShipmentSystem::new();
    let quote = |supply: &mut RecurringSupply, logistics: &mut ShipmentSystem, units| {
        logistics.quote_recurring_input(
            supply,
            InputRequirement {
                resource,
                daily_units: units,
                buffer_units: units * 2.0,
                replenishment_batch_units: Some(40.0),
            },
            Some(1),
            &allocator.entrances[1],
            None,
            &allocator,
            &catalog,
            &tuning,
            &network,
            &graph,
            1.0,
        )
    };
    let first = quote(&mut supply, &mut logistics, 150.0);
    assert_eq!(first.goods, 150.0 * f64::from(profile.unit_price_currency));
    assert!(first.freight.is_finite() && first.freight > 0.0);
    assert_eq!(first.buffer, 300.0 * f64::from(profile.unit_price_currency));
    supply.retry(&allocator, &graph);
    let smaller = quote(&mut supply, &mut logistics, 100.0);
    assert_eq!(
        smaller.goods,
        100.0 * f64::from(profile.unit_price_currency)
    );
    assert_eq!(
        smaller.buffer,
        200.0 * f64::from(profile.unit_price_currency)
    );
    supply.reject(&allocator, &graph);
    assert_eq!(
        quote(&mut supply, &mut logistics, 150.0).goods.to_bits(),
        first.goods.to_bits()
    );
    supply.accept();
    assert!(!quote(&mut supply, &mut logistics, 100.0).goods.is_finite());
    supply.retry(&allocator, &graph);
    assert!(quote(&mut supply, &mut logistics, 50.0).goods.is_finite());
    supply.accept();
    assert!(!quote(&mut supply, &mut logistics, 1.0).goods.is_finite());
    supply.reject(&allocator, &graph);
    supply.reset(&allocator, &graph, &network, &catalog, &[0.0, 0.0]);
    let searches = supply.search_counts_for_test()[0];
    assert!(
        !quote(&mut supply, &mut logistics, 1.0).goods.is_finite(),
        "large physical stock cannot manufacture funded recurring production"
    );
    assert_eq!(supply.search_counts_for_test()[0], searches);
    supply.reject(&allocator, &graph);
    supply.reset(&allocator, &graph, &network, &catalog, &[1.0, 0.0]);
    assert!(quote(&mut supply, &mut logistics, 1.0).goods.is_finite());
    assert!(supply.search_counts_for_test()[0] > searches);
}

#[test]
fn prepared_business_sites_use_candidate_recipes_doors_and_restore_live_accounts() {
    use crate::simulation::economy::accessibility::ModeComponentIndex;
    use crate::simulation::economy::customers::business::{BusinessMarket, PreparedBusinessSite};
    use crate::simulation::economy::operation::InputRequirement;
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let old = catalog.profile_for_id("food_processor_basic").unwrap();
    let target = catalog.profile_for_id("power_plant_basic").unwrap();
    let grain = catalog.resource_runtime_id_for_id("grain").unwrap();
    let coal = catalog.resource_runtime_id_for_id("coal").unwrap();
    let (mut graph, mut network, west, east, border) = simple_graph_with_border();
    for edge in [west, east] {
        graph.edge_mut(edge).bkw_lanes = 0;
    }
    network.lane_system.rebuild(&mut graph);
    network.rebuild_cch_and_check(&graph);
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "prepared_buyers",
        ZoneClass::Industrial,
    );
    for (x, edge) in [(-50.0, west), (50.0, east), (10.0, east)] {
        let mut site = make_building(
            &allocator,
            x,
            ZoneType::Industrial,
            edge,
            &asset,
            1_000.0,
            100_000.0,
        );
        site.economy_profile_runtime_id = old.runtime_id;
        allocator.buildings.push(site);
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let components = ModeComponentIndex::build(&graph, TransitFlags::CAR);
    let mut logistics = ShipmentSystem::new();
    logistics.prepare_site_carrier_quotes(&allocator, &network, &graph, &[border]);
    let carrier = logistics
        .customer_carrier_quote(
            &allocator.entrances[2],
            &allocator.entrances[1],
            Some(2),
            Some(1),
            &allocator,
            &network,
            &graph,
            &tuning,
        )
        .unwrap();
    assert!(
        logistics
            .customer_carrier_quote(
                &allocator.entrances[2],
                &allocator.entrances[0],
                Some(2),
                Some(0),
                &allocator,
                &network,
                &graph,
                &tuning
            )
            .is_none()
    );
    let input = InputRequirement {
        resource: grain,
        daily_units: 100.0,
        buffer_units: 400.0,
        replenishment_batch_units: Some(50.0),
    };
    let quote = |market: &mut BusinessMarket,
                 logistics: &mut ShipmentSystem,
                 network: &TransitNetwork,
                 resource| {
        market
            .claim_reachable(
                &allocator.entrances[2],
                Some(2),
                resource,
                100.0,
                10.0,
                &components,
                &allocator,
                logistics,
                network,
                &graph,
                &tuning,
            )
            .units
    };
    let mut live = BusinessMarket::default();
    live.reset(
        &allocator,
        &network,
        &graph,
        &components,
        &catalog,
        2,
        2,
        |book| {
            book.set_finance(1, [100.0, 10_000.0]);
            for _ in 0..2 {
                book.push_input(1, input, 10.0, carrier, 0.0, 50.0, 1.0)
                    .unwrap();
            }
        },
    );
    assert_eq!(quote(&mut live, &mut logistics, &network, grain), 100.0);
    assert!(live.accept_for(2));
    let mut market = BusinessMarket::default();
    market.copy_reservations_from(&live);
    market.prepare_sites(&[17, 0, 0], &catalog, &components, &graph, |_, _| {
        vec![
            PreparedBusinessSite::new(1, "west", target.runtime_id, allocator.entrances[0].clone()),
            PreparedBusinessSite::new(1, "east", target.runtime_id, allocator.entrances[1].clone()),
            PreparedBusinessSite::new(
                3,
                "new-project",
                target.runtime_id,
                allocator.entrances[1].clone(),
            ),
        ]
    });
    let layout = market.prepared_layout();
    assert_eq!(layout, (3 * old.inputs.len() + 3 * target.inputs.len(), 2));
    for round in 0..16 {
        market.copy_reservations_from(&live);
        network.cch_graph.metric_revision += 1;
        market.prepare_sites(&[17, 0, 0], &catalog, &components, &graph, |_, _| {
            panic!("warm geometry must survive a live fork")
        });
        market.prepare_owners(4, 4);
        market.reserve_accounts(8);
        let original = market.financing(1).map(f64::to_bits);
        for (key, expected) in [("west", 0.0), ("east", 60.0)] {
            market.begin_revalidation();
            let site = market.prepared_site(1, key).unwrap();
            let mut withdrawn = 0.0;
            market
                .replace_buyer_at_site(
                    site,
                    [100.0, 10_000.0],
                    1,
                    |book| {
                        book.push_input(
                            1,
                            InputRequirement {
                                resource: coal,
                                ..input
                            },
                            10.0,
                            carrier,
                            0.0,
                            60.0,
                            1.0,
                        )
                        .unwrap();
                    },
                    |seller, resource, units, _| {
                        assert_eq!((seller, resource), (2, grain));
                        withdrawn += units;
                    },
                )
                .unwrap();
            assert_eq!(withdrawn, 100.0);
            assert_eq!(quote(&mut market, &mut logistics, &network, grain), 0.0);
            assert_eq!(quote(&mut market, &mut logistics, &network, coal), expected);
            if round % 2 == 0 {
                assert!(market.accept_for(2));
            }
            market.reject_revalidation();
            assert_eq!(market.financing(1).map(f64::to_bits), original);
            assert_eq!(market.sales(2, grain, 100.0).units, 100.0);
            assert_eq!(quote(&mut market, &mut logistics, &network, coal), 0.0);
            market.release_owner(2);
            assert_eq!(quote(&mut market, &mut logistics, &network, grain), 100.0);
            market.reject();
        }
        for delay in [0.0, 10_000.0, 0.0] {
            for lane in &mut network.lane_system.lanes {
                lane.frontage_delay_penalty_s = delay;
            }
            network.frontage_delay_revision += 1;
            market.begin_revalidation();
            let site = market.prepared_site(3, "new-project").unwrap();
            market
                .replace_buyer_at_site(
                    site,
                    [100.0, 10_000.0],
                    1,
                    |book| {
                        book.push_input(
                            3,
                            InputRequirement {
                                resource: coal,
                                ..input
                            },
                            10.0,
                            carrier,
                            0.0,
                            60.0,
                            1.0,
                        )
                        .unwrap();
                    },
                    |_, _, _, _| panic!("unplaced buyer has no previous purchases"),
                )
                .unwrap();
            let units = quote(&mut market, &mut logistics, &network, coal);
            if delay == 0.0 {
                assert_eq!(units, 60.0);
            } else {
                assert!(units > 0.0 && units < 60.0);
            }
            market.try_extend_for(2).unwrap();
            market.reject_revalidation();
            assert_eq!(market.financing(3), [0.0; 2]);
            assert_eq!(quote(&mut market, &mut logistics, &network, coal), 0.0);
        }
        assert_eq!(market.prepared_layout(), layout);
    }
    market.prepare_owners(16_384, 16_384);
    market.reserve_accounts(16_384);
    let retained_capacity = market.account_storage().1;
    network.rebuild_cch_and_check(&graph);
    live.reset(
        &allocator,
        &network,
        &graph,
        &components,
        &catalog,
        2,
        2,
        |book| {
            book.set_finance(1, [100.0, 10_000.0]);
            for _ in 0..2 {
                book.push_input(1, input, 10.0, carrier, 0.0, 50.0, 1.0)
                    .unwrap();
            }
        },
    );
    market.copy_reservations_from(&live);
    assert!(
        market
            .account_storage()
            .1
            .into_iter()
            .zip(retained_capacity)
            .all(|(after, before)| after >= before)
    );
    // An unchanged caller revision re-derives the changed live layout from retained rows.
    market.prepare_sites(&[17, 0, 0], &catalog, &components, &graph, |_, _| {
        panic!("rows survive a live layout change")
    });
    assert_eq!(market.financing(1), live.financing(1));
    assert_eq!(
        quote(&mut market, &mut logistics, &network, grain),
        quote(&mut live, &mut logistics, &network, grain)
    );
    assert_eq!(market.prepared_layout().0, layout.0);
}

#[test]
fn business_price_masks_match_unfiltered_claims_and_financial_rollbacks() {
    use crate::simulation::economy::accessibility::ModeComponentIndex;
    use crate::simulation::economy::customers::business::BusinessMarket;
    use crate::simulation::economy::operation::InputRequirement;
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let grain = catalog.resource_runtime_id_for_id("grain").unwrap();
    let buyer_profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let price = catalog
        .all_profiles()
        .iter()
        .find(|profile| {
            profile
                .outputs
                .iter()
                .any(|output| output.resource_runtime_id == grain)
        })
        .unwrap()
        .unit_price_currency;
    let (graph, network, west, east, border) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "masked_buyers",
        ZoneClass::Industrial,
    );
    for (x, edge) in [(-50.0, west), (10.0, east), (30.0, east), (50.0, east)] {
        let mut building = make_building(
            &allocator,
            x,
            ZoneType::Industrial,
            edge,
            &asset,
            1_000.0,
            100_000.0,
        );
        building.economy_profile_runtime_id = buyer_profile.runtime_id;
        allocator.buildings.push(building);
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let components = ModeComponentIndex::build(&graph, TransitFlags::CAR);
    let mut logistics = ShipmentSystem::new();
    logistics.prepare_site_carrier_quotes(&allocator, &network, &graph, &[border]);
    let input = InputRequirement {
        resource: grain,
        daily_units: 100.0,
        buffer_units: 400.0,
        replenishment_batch_units: Some(50.0),
    };
    let mut filtered = BusinessMarket::default();
    filtered.reset(
        &allocator,
        &network,
        &graph,
        &components,
        &catalog,
        6,
        6,
        |book| {
            for buyer in 1..4 {
                book.set_finance(buyer, if buyer == 2 { [10_000.0; 2] } else { [0.0; 2] });
                for inventory in [0.0, 1_000.0] {
                    book.push_input(buyer, input, price, 50.0, inventory, 50.0, 1.0)
                        .unwrap();
                }
            }
        },
    );
    filtered.prepare_owners(4, 12);
    filtered.reserve_accounts(12);
    let mut reference = filtered.clone();
    reference.disable_finance_masks_for_test();
    let check = |a: &BusinessMarket, b: &BusinessMarket| {
        for buyer in 0..4 {
            assert_eq!(
                a.financing(buyer).map(f64::to_bits),
                b.financing(buyer).map(f64::to_bits)
            );
        }
        let receipts = |market: &BusinessMarket| {
            let mut rows = Vec::new();
            market.visit_additional_sales(|s, b, r, u, q| {
                rows.push((s, b, r, u.to_bits(), q.map(f32::to_bits)))
            });
            rows
        };
        assert_eq!(receipts(a), receipts(b));
    };
    let mut quote = |a: &mut BusinessMarket, b: &mut BusinessMarket, price, joint| {
        let mut run = |market: &mut BusinessMarket| {
            if joint {
                market.claim_reachable_joint(
                    &allocator.entrances[0],
                    0,
                    grain,
                    150.0,
                    price,
                    &components,
                    &allocator,
                    &mut logistics,
                    &network,
                    &graph,
                    &tuning,
                )
            } else {
                market.claim_reachable(
                    &allocator.entrances[0],
                    Some(0),
                    grain,
                    150.0,
                    price,
                    &components,
                    &allocator,
                    &mut logistics,
                    &network,
                    &graph,
                    &tuning,
                )
            }
        };
        let actual = run(a);
        let expected = run(b);
        assert_eq!(
            (actual.units.to_bits(), actual.revenue.to_bits()),
            (expected.units.to_bits(), expected.revenue.to_bits())
        );
        check(a, b);
    };
    quote(&mut filtered, &mut reference, price, false);
    assert!(filtered.quote_counts_for_test()[0] < reference.quote_counts_for_test()[0]);
    filtered.reject();
    reference.reject();
    for round in 0..8 {
        for market in [&mut filtered, &mut reference] {
            // Retire the previous round's committed purchases; commits require an unowned seller.
            market.release_owner(0);
            assert!(market.accept_for(0));
            market.begin_revalidation();
            market.set_financing(1, [1_000.0; 2]);
            market.set_financing(2, [0.0; 2]);
        }
        quote(&mut filtered, &mut reference, price, false);
        for market in [&mut filtered, &mut reference] {
            assert!(market.accept_for(0));
        }
        for market in [&mut filtered, &mut reference] {
            market.withdraw_buyer(1, |_, _, _, _| {});
            market
                .replace_buyer_inputs(
                    3,
                    [20_000.0; 2],
                    1,
                    |book| {
                        book.push_input(3, input, price, 50.0, 1_000.0, 80.0, 1.0)
                            .unwrap();
                    },
                    |_, _, _, _| {},
                )
                .unwrap();
        }
        quote(&mut filtered, &mut reference, price, false);
        filtered.reject();
        reference.reject();
        quote(&mut filtered, &mut reference, price.next_up(), false);
        filtered.reject();
        reference.reject();
        quote(&mut filtered, &mut reference, price, true);
        for market in [&mut filtered, &mut reference] {
            if round % 2 == 0 {
                market.commit_joint_for(0).unwrap();
            }
            market.reject_revalidation();
        }
        check(&filtered, &reference);
        quote(&mut filtered, &mut reference, price, false);
        for market in [&mut filtered, &mut reference] {
            assert!(market.accept_for(0));
            market.release_owner(0);
        }
        quote(&mut filtered, &mut reference, price, false);
        filtered.reject();
        reference.reject();
        check(&filtered, &reference);
    }
}

#[test]
fn financed_business_customers_require_directed_routes_and_reprice_frontage() {
    use crate::simulation::economy::accessibility::ModeComponentIndex;
    use crate::simulation::economy::customers::business::BusinessMarket;
    use crate::simulation::economy::operation::InputRequirement;
    let catalog = load_runtime_economy_catalog().unwrap();
    let grain = catalog.resource_runtime_id_for_id("grain").unwrap();
    let profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let (mut graph, mut network, west, east, border) = simple_graph_with_border();
    for edge in [west, east] {
        graph.edge_mut(edge).bkw_lanes = 0;
    }
    network.lane_system.rebuild(&mut graph);
    network.rebuild_cch_and_check(&graph);
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "business_customers",
        ZoneClass::Industrial,
    );
    for (x, edge) in [(-50.0, west), (50.0, east)] {
        allocator.buildings.push(make_building(
            &allocator,
            x,
            ZoneType::Industrial,
            edge,
            &asset,
            1_000.0,
            100_000.0,
        ));
    }
    for building in &mut allocator.buildings {
        building.economy_profile_runtime_id = profile.runtime_id;
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let tuning = load_runtime_economy_tuning().unwrap();
    let components = ModeComponentIndex::build(&graph, TransitFlags::CAR);
    let mut logistics = ShipmentSystem::new();
    logistics.prepare_site_carrier_quotes(&allocator, &network, &graph, &[border]);
    let original_carrier = logistics
        .customer_carrier_quote(
            &allocator.entrances[0],
            &allocator.entrances[1],
            Some(0),
            Some(1),
            &allocator,
            &network,
            &graph,
            &tuning,
        )
        .unwrap();
    assert!(
        logistics
            .customer_carrier_quote(
                &allocator.entrances[1],
                &allocator.entrances[0],
                Some(1),
                Some(0),
                &allocator,
                &network,
                &graph,
                &tuning
            )
            .is_none()
    );
    let mut customers = BusinessMarket::default();
    let input = InputRequirement {
        resource: grain,
        daily_units: 100.0,
        buffer_units: 400.0,
        replenishment_batch_units: Some(50.0),
    };
    let quote = |customers: &mut BusinessMarket,
                 logistics: &mut ShipmentSystem,
                 seller,
                 network: &TransitNetwork| {
        customers
            .claim_reachable(
                &allocator.entrances[seller],
                Some(seller),
                grain,
                100.0,
                10.0,
                &components,
                &allocator,
                logistics,
                network,
                &graph,
                &tuning,
            )
            .units
    };
    for buyer in [0, 1] {
        customers.reset(
            &allocator,
            &network,
            &graph,
            &components,
            &catalog,
            1,
            1,
            |book| {
                book.set_finance(buyer, [100.0, 10_000.0]);
                book.push_input(buyer, input, 10.0, original_carrier, 0.0, 100.0, 1.0)
                    .unwrap();
            },
        );
        assert_eq!(
            quote(&mut customers, &mut logistics, buyer, &network),
            0.0,
            "a firm cannot manufacture revenue by buying its own net output"
        );
        assert_eq!(
            quote(&mut customers, &mut logistics, 1 - buyer, &network),
            if buyer == 0 { 0.0 } else { 100.0 }
        );
        customers.reject();
    }
    assert_eq!(quote(&mut customers, &mut logistics, 0, &network), 100.0);
    assert!(customers.accept_for(0));
    assert_eq!(quote(&mut customers, &mut logistics, 0, &network), 0.0);
    for lane in &mut network.lane_system.lanes {
        lane.frontage_delay_penalty_s = 10_000.0;
    }
    network.frontage_delay_revision += 1;
    assert!(
        logistics
            .customer_carrier_quote(
                &allocator.entrances[0],
                &allocator.entrances[1],
                Some(0),
                Some(1),
                &allocator,
                &network,
                &graph,
                &tuning
            )
            .unwrap()
            > original_carrier
    );
    customers.prepare_owners(2, 2);
    let original_financing = customers.financing(1).map(f64::to_bits);
    for round in 0..16 {
        customers.begin_revalidation();
        customers.release_owner(0);
        let units = quote(&mut customers, &mut logistics, 0, &network);
        assert!(
            units > 0.0 && units < 100.0,
            "current freight must consume shared financing: {units}"
        );
        if round % 2 == 0 {
            assert!(customers.accept_for(0));
        }
        customers.reject_revalidation();
        assert_eq!(customers.financing(1).map(f64::to_bits), original_financing);
        assert_eq!(quote(&mut customers, &mut logistics, 0, &network), 0.0);
    }
    for lane in &mut network.lane_system.lanes {
        lane.frontage_delay_penalty_s = 0.0;
    }
    network.frontage_delay_revision += 1;
    customers.release_owner(0);
    assert_eq!(quote(&mut customers, &mut logistics, 0, &network), 100.0);
    assert!(customers.accept_for(0));
    customers.release_owner(0);
    assert_eq!(quote(&mut customers, &mut logistics, 0, &network), 100.0);
    customers.reject();
    customers.reset(
        &allocator,
        &network,
        &graph,
        &components,
        &catalog,
        1,
        1,
        |book| {
            book.set_finance(1, [100.0, 10_000.0]);
            book.push_input(1, input, 15.0, original_carrier, 0.0, 100.0, 1.5)
                .unwrap();
        },
    );
    let sale = customers.claim_reachable(
        &allocator.entrances[0],
        Some(0),
        grain,
        100.0,
        10.0,
        &components,
        &allocator,
        &mut logistics,
        &network,
        &graph,
        &tuning,
    );
    assert_eq!(sale.units, 100.0);
    assert_eq!(
        sale.revenue, 1_500.0,
        "seller receipts follow the actual goods invoice, excluding freight"
    );
    assert!(customers.accept_for(0));
    customers.prepare_owners(2, 2);
    for _ in 0..8 {
        customers.begin_revalidation();
        let mut withdrawn = Vec::new();
        customers.withdraw_buyer(1, |seller, resource, units, revenue| {
            withdrawn.push((seller, resource, units, revenue));
        });
        assert_eq!(withdrawn, [(0, grain, 100.0, 1_500.0)]);
        assert_eq!(customers.sales(0, grain, 100.0).units, 0.0);
        assert_eq!(quote(&mut customers, &mut logistics, 0, &network), 0.0);
        customers.reject_revalidation();
        assert_eq!(customers.sales(0, grain, 100.0).revenue, 1_500.0);
    }
    customers.release_owner(0);
    assert!(customers.accept_for(0));
    customers.begin_revalidation();
    customers.withdraw_buyer(1, |_, _, _, _| panic!("no remaining seller commitments"));
    assert_eq!(quote(&mut customers, &mut logistics, 0, &network), 0.0);
    customers.reject_revalidation();
    assert_eq!(
        quote(&mut customers, &mut logistics, 0, &network),
        100.0,
        "rejected buyer withdrawal restores the previously available spatial account"
    );
    customers.reject();
    let layout = customers.prepared_layout();
    assert_eq!(layout, (2 * profile.inputs.len(), 1));
    customers.reserve_accounts(8);
    let storage = customers.account_storage();
    let other = profile
        .inputs
        .iter()
        .find(|port| port.resource_runtime_id != grain)
        .unwrap()
        .resource_runtime_id;
    for round in 0..16 {
        let original = customers.financing(1).map(f64::to_bits);
        let resource = if round % 2 == 0 { grain } else { other };
        customers.begin_revalidation();
        customers
            .replace_buyer_inputs(
                1,
                [100.0, 10_000.0],
                3,
                |book| {
                    for (price, units) in [(9.0, 20.0), (10.0, 30.0), (11.0, 10.0)] {
                        book.push_input(
                            1,
                            InputRequirement { resource, ..input },
                            price,
                            original_carrier,
                            0.0,
                            units,
                            1.0,
                        )
                        .unwrap();
                    }
                },
                |_, _, _, _| panic!("original demand is unclaimed"),
            )
            .unwrap();
        assert_eq!(
            customers
                .claim_reachable(
                    &allocator.entrances[0],
                    Some(0),
                    resource,
                    100.0,
                    10.0,
                    &components,
                    &allocator,
                    &mut logistics,
                    &network,
                    &graph,
                    &tuning
                )
                .units,
            60.0
        );
        if round % 2 == 0 {
            assert!(customers.accept_for(0));
        }
        customers.reject_revalidation();
        assert_eq!(customers.financing(1).map(f64::to_bits), original);
        assert_eq!(quote(&mut customers, &mut logistics, 0, &network), 100.0);
        customers.reject();
        assert_eq!(customers.prepared_layout(), layout);
        assert_eq!(customers.account_storage(), storage);
        assert_eq!(
            customers
                .claim_reachable(
                    &allocator.entrances[0],
                    Some(0),
                    other,
                    100.0,
                    10.0,
                    &components,
                    &allocator,
                    &mut logistics,
                    &network,
                    &graph,
                    &tuning
                )
                .units,
            0.0
        );
    }
    // Capacity is a retry request, never an economic rejection or a partial withdrawal.
    customers.begin_revalidation();
    let required = customers
        .replace_buyer_inputs(
            1,
            [0.0; 2],
            1_024,
            |_| panic!("insufficient prepared storage must precede seeding"),
            |_, _, _, _| panic!("insufficient prepared storage must precede withdrawal"),
        )
        .unwrap_err();
    assert_eq!(required, 1_024);
    assert_eq!(quote(&mut customers, &mut logistics, 0, &network), 100.0);
    customers.reject_revalidation();
    for round in 0..64 {
        customers.begin_revalidation();
        let tranches = 1 + round % 3;
        customers
            .replace_buyer_inputs(
                1,
                [100.0, 10_000.0],
                tranches,
                |book| {
                    for _ in 0..tranches {
                        book.push_input(1, input, 10.0, original_carrier, 0.0, 20.0, 1.0)
                            .unwrap();
                    }
                },
                |_, _, _, _| {},
            )
            .unwrap();
        assert_eq!(
            quote(&mut customers, &mut logistics, 0, &network),
            20.0 * tranches as f64
        );
        assert!(customers.accept_for(0));
        customers.accept_revalidation();
        let (accounts, capacities) = customers.account_storage();
        assert!(
            accounts <= 3,
            "accepted reseeding must reuse withdrawn input slots"
        );
        assert_eq!(capacities, storage.1);
        assert_eq!(customers.prepared_layout(), layout);
    }
    for (tranches, funded) in [(1, true), (4, true), (2, false), (0, true), (3, true)] {
        use crate::simulation::economy::customers::business::BusinessBudgets;
        network.cch_graph.metric_revision += 1;
        let seed = |book: &mut BusinessBudgets| {
            book.set_finance(1, if funded { [100.0, 10_000.0] } else { [0.0; 2] });
            for tranche in 0..tranches {
                book.push_input(
                    1,
                    input,
                    8.0 + tranche as f32,
                    original_carrier,
                    0.0,
                    100.0 / f64::from(tranches),
                    1.0,
                )
                .unwrap();
            }
        };
        customers.reset(
            &allocator,
            &network,
            &graph,
            &components,
            &catalog,
            4,
            1,
            seed,
        );
        assert_eq!(
            customers.prepared_layout(),
            layout,
            "financial fragmentation, buyer funding and route metrics do not change recipe-port geometry"
        );
        let mut reference = BusinessBudgets::default();
        reference.reset(2, 4, 2, 1);
        seed(&mut reference);
        let mut expected = 0.0;
        for slot in 0..tranches as usize {
            expected += reference.claim_opportunity(slot, 100.0 - expected, 10.0, original_carrier);
        }
        let before = customers.financing(1).map(f64::to_bits);
        assert_eq!(
            quote(&mut customers, &mut logistics, 0, &network).to_bits(),
            expected.to_bits(),
            "one directed quote per buyer must retain the original financial tranche order"
        );
        customers.reject();
        assert_eq!(customers.financing(1).map(f64::to_bits), before);
    }
}

#[test]
fn claims_unfundable_with_free_freight_stay_unfundable_at_any_carrier_quote() {
    use crate::simulation::economy::customers::business::BusinessBudgets;
    use crate::simulation::economy::operation::InputRequirement;
    let input = InputRequirement {
        resource: 1,
        daily_units: 100.0,
        buffer_units: 400.0,
        replenishment_batch_units: Some(50.0),
    };
    // Funded, margin-limited and infeasible buyers against cheaper and dearer sellers.
    for finance in [
        [100.0, 10_000.0],
        [0.5, 10_000.0],
        [100.0, 0.0],
        [0.0, 0.0],
        [-1.0, 5.0],
    ] {
        for price in [6.0, 8.0, 9.5, 12.0] {
            let mut book = BusinessBudgets::default();
            book.reset(1, 1, 1, 1);
            book.set_finance(0, finance);
            book.push_input(0, input, 8.0, 40.0, 0.0, 100.0, 1.0)
                .unwrap();
            let claimable = book.claimable_at_carrier_lower_bound(0, 100.0, price, 0.0, true);
            let mut free = book.clone();
            assert_eq!(
                claimable,
                free.claim_opportunity(0, 100.0, price, 0.0) > 0.0
            );
            for carrier in [0.0, 10.0, 40.0, 400.0] {
                let mut quoted = book.clone();
                let units = quoted.claim_opportunity(0, 100.0, price, carrier);
                assert!(
                    claimable || units == 0.0,
                    "finance={finance:?} price={price} carrier={carrier}: skipped quote would claim {units}"
                );
            }
        }
    }
}

#[test]
fn recurring_supply_requires_directed_route_even_inside_one_weak_component() {
    use crate::simulation::economy::operation::InputRequirement;
    let (mut graph, mut network, first, second, _) = simple_graph_with_border();
    // Both roads point east. A source on the eastern road cannot supply a western buyer.
    graph.edge_mut(first).bkw_lanes = 0;
    graph.edge_mut(second).bkw_lanes = 0;
    network.lane_system.rebuild(&mut graph);
    network.rebuild_cch_and_check(&graph);
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "directed_recurring",
        ZoneClass::Industrial,
    );
    for (x, edge) in [(50.0, second), (-50.0, first)] {
        allocator.buildings.push(make_building(
            &allocator,
            x,
            ZoneType::Industrial,
            edge,
            &asset,
            1_000.0,
            100_000.0,
        ));
    }
    allocator.buildings[0].worker_count = profile.worker_capacity;
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut supply = RecurringSupply::default();
    supply.reset(&allocator, &graph, &network, &catalog, &[1.0, 0.0]);
    let mut logistics = ShipmentSystem::new();
    let input = InputRequirement {
        resource: profile.outputs[0].resource_runtime_id,
        daily_units: 1.0,
        buffer_units: 2.0,
        replenishment_batch_units: Some(40.0),
    };
    let quote = logistics.quote_recurring_input(
        &mut supply,
        input,
        Some(1),
        &allocator.entrances[1],
        None,
        &allocator,
        &catalog,
        &tuning,
        &network,
        &graph,
        1.0,
    );
    assert!(!quote.goods.is_finite());
    supply.reject(&allocator, &graph);
    let proposed = logistics.quote_recurring_input(
        &mut supply,
        input,
        None,
        &allocator.entrances[1],
        None,
        &allocator,
        &catalog,
        &tuning,
        &network,
        &graph,
        1.0,
    );
    assert!(!proposed.goods.is_finite());
}

#[test]
fn prepared_freight_quotes_stay_exact_after_cache_saturation_and_invalidation() {
    use super::route_cache::FreightRouteCache;
    use crate::simulation::pathing::cch::CchQueryScratch;
    let (graph, mut network, west, east, border) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "bounded_quotes",
        ZoneClass::Industrial,
    );
    for index in 0..32 {
        let offset = (index / 2) as f32 * 4.0;
        let (x, edge) = if index % 2 == 0 {
            (-85.0 + offset, west)
        } else {
            (15.0 + offset, east)
        };
        allocator.buildings.push(make_building(
            &allocator,
            x,
            ZoneType::Industrial,
            edge,
            &asset,
            0.0,
            1_000.0,
        ));
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut cache = FreightRouteCache::default();
    // A town can add buildings after initial preparation. Saturation must affect only
    // retention, never route answers or query allocation, even before the next preparation.
    cache.prepare_border_costs(&[border], 0, &network, &graph);
    let capacities = cache.storage().map(|(_, capacity)| capacity);
    assert!(capacities[2] < allocator.buildings.len());
    let costs = network
        .cch_graph
        .costs_from_each(&[border], &graph, TransitFlags::CAR);
    let mut scratch = CchQueryScratch::default();
    scratch.prepare(&network.cch_graph, &graph);
    for delay in [0.0, 20.0, 0.0] {
        for lane in &mut network.lane_system.lanes {
            lane.frontage_delay_penalty_s = delay;
        }
        network.frontage_delay_revision += 1;
        for prepared in [false, true] {
            let entries = cache.storage().map(|(len, _)| len);
            cache.clear_buildings();
            assert_eq!(
                cache.storage().map(|(len, _)| len),
                entries,
                "entrance invalidation must not sweep city-sized caches"
            );
            if prepared {
                cache.prepare_buildings(allocator.buildings.len());
            }
            for destination in 0..allocator.buildings.len() {
                let expected = allocator.freight_car_eta_from_border_costs(
                    border,
                    destination,
                    &costs[0],
                    &network,
                    &graph,
                );
                assert_eq!(
                    cache.from_border(border, destination, &allocator, &network, &graph),
                    expected
                );
                for source in 0..allocator.buildings.len() {
                    let expected = allocator.freight_car_eta_between_buildings(
                        source,
                        destination,
                        &network,
                        &graph,
                        |from, to| {
                            network.cch_graph.find_cost(
                                from,
                                to,
                                usize::MAX,
                                usize::MAX,
                                &graph,
                                TransitFlags::CAR,
                                &mut scratch,
                            )
                        },
                    );
                    if let Some(seconds) = expected {
                        let bound = cache
                            .minimum_entrance_network_cost(
                                &allocator.entrances[source],
                                &allocator.entrances[destination],
                                &network,
                                &graph,
                            )
                            .unwrap();
                        assert!(
                            bound <= seconds,
                            "{source} -> {destination}: {bound} > {seconds}"
                        );
                    }
                    assert_eq!(
                        cache
                            .between_buildings(source, destination, &allocator, &network, &graph)
                            .map(f32::to_bits),
                        expected.map(f32::to_bits)
                    );
                    let hits = cache.building_cache_counts[1];
                    let node_requests = cache.node_cache_counts[0];
                    assert_eq!(
                        cache
                            .between_buildings(source, destination, &allocator, &network, &graph)
                            .map(f32::to_bits),
                        expected.map(f32::to_bits)
                    );
                    assert_eq!(cache.building_cache_counts[1], hits + 1);
                    assert_eq!(cache.node_cache_counts[0], node_requests);
                    assert_eq!(cache.storage().map(|(_, capacity)| capacity), capacities);
                }
            }
        }
    }
    let storage = cache.storage();
    assert_eq!(storage[1].0, storage[1].1);
    assert_eq!(storage[2].0, storage[2].1);
}

#[test]
fn complete_freight_quotes_survive_unrelated_building_edits_but_recheck_doors_and_lanes() {
    use super::route_cache::FreightRouteCache;
    let (mut graph, mut network, west, east, border) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "quote_dependencies",
        ZoneClass::Industrial,
    );
    for (x, edge) in [(-80.0, west), (40.0, east), (20.0, east)] {
        allocator.buildings.push(make_building(
            &allocator,
            x,
            ZoneType::Industrial,
            edge,
            &asset,
            0.0,
            1_000.0,
        ));
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut cache = FreightRouteCache::default();
    cache.prepare_border_costs(&[border], 3, &network, &graph);
    cache.prepare_buildings(3);
    let original = cache
        .between_buildings(0, 1, &allocator, &network, &graph)
        .unwrap();
    for prepared in [false, true] {
        allocator.entrances[2].door_pos.x += 1.0;
        cache.clear_buildings();
        if prepared {
            cache.prepare_access_for(&allocator, &network, &graph, |_| true);
        }
        let hits = cache.building_cache_counts[1];
        let nodes = cache.node_cache_counts[0];
        assert_eq!(
            cache
                .between_buildings(0, 1, &allocator, &network, &graph)
                .unwrap()
                .to_bits(),
            original.to_bits()
        );
        assert_eq!(cache.building_cache_counts[1], hits + 1);
        assert_eq!(cache.node_cache_counts[0], nodes);
    }
    for change in 0..5 {
        match change {
            0 => {
                allocator.entrances[0].door_pos.x += 10.0;
            }
            1 => {
                network.lane_system.rebuild(&mut graph);
            }
            2 => {
                graph.edge_mut(west).base_cost *= 1.75;
                network.cch_graph.customize(&graph);
                cache.clear();
            }
            3 => {
                graph.edge_mut(west).speed_limit *= 0.5;
            }
            _ => {
                allocator.entrances.swap(0, 1);
            }
        }
        cache.clear_buildings();
        let hits = cache.building_cache_counts[1];
        let actual = cache.between_buildings(0, 1, &allocator, &network, &graph);
        let mut fresh = FreightRouteCache::default();
        fresh.prepare_border_costs(&[border], 3, &network, &graph);
        assert_eq!(
            actual.map(f32::to_bits),
            fresh
                .between_buildings(0, 1, &allocator, &network, &graph)
                .map(f32::to_bits)
        );
        assert_eq!(
            cache.building_cache_counts[1], hits,
            "changed dependency {change} reused a quote"
        );
    }
}

#[test]
fn prepared_freight_access_follows_door_and_building_index_changes() {
    use super::route_cache::FreightRouteCache;
    let (graph, mut network, west, east, border) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "access_edits",
        ZoneClass::Industrial,
    );
    for (x, edge) in [(-85.0, west), (-20.0, west), (30.0, east)] {
        allocator.buildings.push(make_building(
            &allocator,
            x,
            ZoneType::Industrial,
            edge,
            &asset,
            0.0,
            1_000.0,
        ));
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut warm = FreightRouteCache::default();
    warm.prepare_border_costs(&[border], allocator.buildings.len(), &network, &graph);
    warm.prepare_buildings(allocator.buildings.len());
    let mut states = Vec::new();
    for edit in 0..5 {
        match edit {
            1 => {
                allocator.entrances[0].entrance_s_m += 12.0;
                allocator.entrances[0].door_pos.x += 12.0;
                warm.clear_buildings();
            }
            2 => {
                allocator.buildings.swap(0, 2);
                allocator.entrances.swap(0, 2);
                warm.clear_buildings();
            }
            3 => {
                for lane in &mut network.lane_system.lanes {
                    lane.frontage_delay_penalty_s = 40.0;
                }
                network.frontage_delay_revision += 1;
            }
            4 => {
                allocator.entrances[0].car_lane_fwd = usize::MAX;
                allocator.entrances[0].car_lane_bkw = usize::MAX;
                warm.clear_buildings();
            }
            _ => {}
        }
        let mut expected = FreightRouteCache::default();
        expected.prepare_border_costs(&[border], allocator.buildings.len(), &network, &graph);
        let mut values = Vec::new();
        // First use the exact uncached fallback after invalidation, then rebuilt geometry.
        for prepared in [false, true] {
            if prepared {
                warm.prepare_buildings(allocator.buildings.len());
            }
            for source in 0..3 {
                for destination in 0..3 {
                    let value =
                        warm.between_buildings(source, destination, &allocator, &network, &graph);
                    assert_eq!(
                        value.map(f32::to_bits),
                        expected
                            .between_buildings(source, destination, &allocator, &network, &graph)
                            .map(f32::to_bits)
                    );
                    let hits = warm.building_cache_counts[1];
                    assert_eq!(
                        warm.between_buildings(source, destination, &allocator, &network, &graph)
                            .map(f32::to_bits),
                        value.map(f32::to_bits)
                    );
                    assert_eq!(warm.building_cache_counts[1], hits + 1);
                    assert_eq!(
                        warm.to_site(
                            source,
                            &allocator.entrances[destination],
                            &allocator,
                            &network,
                            &graph
                        )
                        .map(f32::to_bits),
                        value.map(f32::to_bits)
                    );
                    if !prepared {
                        values.push(value.map(f32::to_bits));
                    }
                }
            }
        }
        if let Some(previous) = states.last() {
            assert_ne!(previous, &values);
        }
        states.push(values);
    }
}

#[test]
fn customer_route_cache_does_not_alias_replacement_doors_to_live_owners() {
    let (mut graph, mut network, west, east, border) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "customer_access",
        ZoneClass::Industrial,
    );
    for (x, edge) in [(-80.0, west), (40.0, east)] {
        allocator.buildings.push(make_building(
            &allocator,
            x,
            ZoneType::Industrial,
            edge,
            &asset,
            0.0,
            1_000.0,
        ));
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut moved_source = allocator.entrances[0].clone();
    moved_source.door_pos.x += 21.0;
    moved_source.entrance_s_m += 21.0;
    let mut moved_destination = allocator.entrances[1].clone();
    moved_destination.door_pos.x += 13.0;
    moved_destination.entrance_s_m += 13.0;
    let tuning = load_runtime_economy_tuning().unwrap();
    let mut logistics = ShipmentSystem::new();
    logistics.prepare_site_carrier_quotes(&allocator, &network, &graph, &[border]);
    let mut oracle = crate::simulation::pathing::cch::CchQueryScratch::default();
    oracle.prepare(&network.cch_graph, &graph);
    for delay in [0.0, 40.0, 0.0] {
        for lane in &mut network.lane_system.lanes {
            lane.frontage_delay_penalty_s = delay;
        }
        network.frontage_delay_revision += 1;
        let mut quotes = Vec::new();
        for from in [&allocator.entrances[0], &moved_source] {
            for to in [&allocator.entrances[1], &moved_destination] {
                let expected = BuildingAllocator::freight_car_eta_between_entrances(
                    from,
                    to,
                    &network,
                    &graph,
                    |from, to| {
                        network.cch_graph.find_cost(
                            from,
                            to,
                            usize::MAX,
                            usize::MAX,
                            &graph,
                            TransitFlags::CAR,
                            &mut oracle,
                        )
                    },
                )
                .map(|seconds| {
                    super::timing::freight_cost(
                        seconds,
                        tuning.operational_clock.seconds_per_day,
                        &tuning.logistics,
                    )
                });
                for source in [None, Some(0)] {
                    for destination in [None, Some(1)] {
                        assert_eq!(
                            logistics
                                .customer_carrier_quote(
                                    from,
                                    to,
                                    source,
                                    destination,
                                    &allocator,
                                    &network,
                                    &graph,
                                    &tuning
                                )
                                .map(f32::to_bits),
                            expected.map(f32::to_bits)
                        );
                    }
                }
                quotes.push(expected.unwrap().to_bits());
            }
        }
        assert_ne!(quotes[0], quotes[1]);
        assert_ne!(quotes[0], quotes[2]);
    }
    let before = logistics
        .customer_carrier_quote(
            &moved_source,
            &moved_destination,
            None,
            None,
            &allocator,
            &network,
            &graph,
            &tuning,
        )
        .unwrap();
    // A fresh lane epoch with unchanged door keys must discard proposal projections.
    // Translate the isolated fixture's physical lanes to make stale local distances visible.
    network.lane_system.rebuild(&mut graph);
    for lane in &mut network.lane_system.lanes {
        for point in &mut lane.geometry {
            point.z += 100.0;
        }
    }
    let expected = BuildingAllocator::freight_car_eta_between_entrances(
        &moved_source,
        &moved_destination,
        &network,
        &graph,
        |from, to| {
            network.cch_graph.find_cost(
                from,
                to,
                usize::MAX,
                usize::MAX,
                &graph,
                TransitFlags::CAR,
                &mut oracle,
            )
        },
    )
    .map(|seconds| {
        super::timing::freight_cost(
            seconds,
            tuning.operational_clock.seconds_per_day,
            &tuning.logistics,
        )
    })
    .unwrap();
    assert_ne!(before.to_bits(), expected.to_bits());
    assert_eq!(
        logistics
            .customer_carrier_quote(
                &moved_source,
                &moved_destination,
                None,
                None,
                &allocator,
                &network,
                &graph,
                &tuning
            )
            .unwrap()
            .to_bits(),
        expected.to_bits()
    );
}

#[test]
fn warm_freight_quotes_follow_live_frontage_and_metric_edits() {
    use super::route_cache::FreightRouteCache;

    let (mut graph, mut network, source_edge, destination_edge, border) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(&mut allocator, "test", "quote", ZoneClass::Industrial);
    for (x, edge) in [(-50.0, source_edge), (50.0, destination_edge)] {
        allocator.buildings.push(make_building(
            &allocator,
            x,
            ZoneType::Industrial,
            edge,
            &asset,
            100.0,
            1_000.0,
        ));
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let quote = |cache: &mut FreightRouteCache, network: &TransitNetwork, graph: &RegionGraph| {
        cache.prepare_buildings(allocator.buildings.len());
        cache.prepare_border_costs(&[border], allocator.buildings.len(), network, graph);
        (
            cache
                .between_buildings(0, 1, &allocator, network, graph)
                .unwrap(),
            cache
                .from_border(border, 1, &allocator, network, graph)
                .unwrap(),
        )
    };
    let mut warm = FreightRouteCache::default();
    let before = quote(&mut warm, &network, &graph);
    let used = [&allocator.entrances[0], &allocator.entrances[1]]
        .map(|entrance| [entrance.car_lane_fwd, entrance.car_lane_bkw]);
    let mut edited = 0;
    for (lane_id, lane) in network.lane_system.lanes.iter_mut().enumerate() {
        if !used.iter().flatten().any(|&used| used == lane_id) {
            lane.frontage_delay_penalty_s = 137.0;
            edited += 1;
        }
    }
    assert!(edited > 0);
    network.frontage_delay_revision += 1;
    let requests = warm.node_cache_counts[0];
    let hits = warm.building_cache_counts[1];
    assert_eq!(before, quote(&mut warm, &network, &graph));
    assert_eq!(
        warm.node_cache_counts[0], requests,
        "unrelated lane-price edits must retain the complete freight quote"
    );
    assert_eq!(warm.building_cache_counts[1], hits + 1);
    for lane in &mut network.lane_system.lanes {
        lane.frontage_delay_penalty_s = 20.0;
    }
    // Publish the direct fixture edit just as the live traffic update does.
    network.frontage_delay_revision += 1;
    let after = quote(&mut warm, &network, &graph);
    assert!(after.0 > before.0 && after.1 > before.1);
    assert_eq!(
        after,
        quote(&mut FreightRouteCache::default(), &network, &graph)
    );
    for lane in &mut network.lane_system.lanes {
        lane.frontage_delay_penalty_s = 0.0;
    }
    network.frontage_delay_revision += 1;
    assert_eq!(before, quote(&mut warm, &network, &graph));

    for lane_id in used
        .into_iter()
        .flatten()
        .filter(|&lane| lane != usize::MAX)
    {
        for price in [100_000.0, 0.0] {
            network.lane_system.lanes[lane_id].frontage_delay_penalty_s = price;
            network.frontage_delay_revision += 1;
            let actual = quote(&mut warm, &network, &graph);
            let expected = quote(&mut FreightRouteCache::default(), &network, &graph);
            assert_eq!(
                (actual.0.to_bits(), actual.1.to_bits()),
                (expected.0.to_bits(), expected.1.to_bits()),
                "every candidate lane must invalidate pricing, including a previously losing one"
            );
        }
    }

    let mut logistics = ShipmentSystem::new();
    logistics.refresh_freight_route_cache(&allocator, &network);
    logistics.freight_route_cache = warm;
    graph.edge_mut(source_edge).base_cost += 30.0;
    network.cch_graph.customize(&graph);
    logistics.refresh_freight_route_cache(&allocator, &network);
    logistics.freight_route_cache.prepare_border_costs(
        &[border],
        allocator.buildings.len(),
        &network,
        &graph,
    );
    let changed = logistics
        .freight_route_cache
        .from_border(border, 1, &allocator, &network, &graph)
        .unwrap();
    assert!(changed > before.1);
    assert_eq!(
        changed,
        quote(&mut FreightRouteCache::default(), &network, &graph).1
    );
}

fn graph_with_border_to(end_x: f32) -> (RegionGraph, TransitNetwork, usize, usize, u32) {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Border);
    let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n2 = graph.add_node(Vector3::new(end_x, 0.0, 0.0), NodeType::Junction);
    let e0 = graph.add_edge(Edge {
        start_node: n0,
        end_node: n1,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 20.0,
        base_cost: 5.0,
        physical_length: end_x.max(1.0),
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(-100.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(-100.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access:
            crate::simulation::network::types::VehicleFrontageAccess::BothSides,
    });
    let e1 = graph.add_edge(Edge {
        start_node: n1,
        end_node: n2,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 20.0,
        base_cost: 5.0,
        physical_length: 100.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(end_x, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(end_x, 0.0, 0.0)],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access:
            crate::simulation::network::types::VehicleFrontageAccess::BothSides,
    });
    graph.rebuild_adjacency_list();

    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = crate::simulation::pathing::cch::CchGraph::build(&graph);
    (graph, network, e0, e1, n0)
}

#[test]
fn payload_limits_local_import_and_export_cargo_without_losing_stock_or_money() {
    // A small test payload makes all three planners reach the limit with minimal setup.
    // The shipped 20,000 kg limit is checked separately.
    for (local_supplier, export) in [(true, false), (false, false), (true, true)] {
        let (graph, network, source_edge, destination_edge, border) = simple_graph_with_border();
        let mut allocator = BuildingAllocator::new();
        if local_supplier {
            let asset = register_test_asset(
                &mut allocator,
                "test",
                "payload_source",
                ZoneClass::Industrial,
            );
            allocator.buildings.push(make_building(
                &allocator,
                -50.0,
                ZoneType::Industrial,
                source_edge,
                &asset,
                600.0,
                0.0,
            ));
        }
        let destination_idx = allocator.buildings.len();
        if !export {
            let asset = register_test_asset(
                &mut allocator,
                "test",
                "payload_buyer",
                ZoneClass::Commercial,
            );
            let mut buyer = make_building(
                &allocator,
                50.0,
                ZoneType::Commercial,
                destination_edge,
                &asset,
                0.0,
                10_000.0,
            );
            buyer.commercial_activity_floor_scale = 1.0;
            allocator.buildings.push(buyer);
        }
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);
        let mut shipments = ShipmentSystem::new();
        let mut planning = super::planning::FreightPlanningContext::build(
            &mut shipments,
            &allocator,
            &network,
            &graph,
        );
        std::sync::Arc::make_mut(&mut planning.tuning)
            .logistics
            .truck_payload_kg = 100.0;
        let food = planning
            .catalog
            .resource_runtime_id_for_id("packaged_food")
            .unwrap();
        let price = planning.catalog.unit_price_for_resource(food).unwrap();
        let expected_payment = 100.0
            * price
            * if !local_supplier {
                planning.tuning.owa_import_price_multiplier
            } else {
                1.0
            };
        let mut treasury = 0.0;
        if export {
            shipments.create_profile_output_exports(
                &mut allocator,
                &network,
                &graph,
                480,
                &mut planning,
            );
        } else {
            shipments.create_profile_input_shipments(
                &mut allocator,
                &network,
                &graph,
                480,
                &mut planning,
                &mut treasury,
            );
            assert_eq!(
                allocator.buildings[destination_idx].operating_budget,
                10_000.0 - shipments.shipments[0].total_cost
            );
        }
        planning.finish(&mut shipments);
        assert_eq!(shipments.shipments.len(), 1);
        assert_eq!(shipments.shipments[0].amount, 100.0);
        let fee = shipments.shipments[0].freight_cost;
        assert!(fee > 0.0);
        assert_eq!(
            shipments.shipments[0].total_cost,
            expected_payment + if export { 0.0 } else { fee }
        );
        assert_eq!(
            shipments.shipments[0].source,
            if local_supplier {
                ShipmentEndpoint::Building(0)
            } else {
                ShipmentEndpoint::OwaBorder(border)
            }
        );
        assert_eq!(
            shipments.shipments[0].destination,
            if export {
                ShipmentEndpoint::OwaBorder(border)
            } else {
                ShipmentEndpoint::Building(destination_idx)
            }
        );
        let mut agents = AgentSystem::new();
        progress_shipments_for_test(
            &mut shipments,
            &mut allocator,
            &mut agents,
            &network,
            &graph,
        );
        assert_eq!(agents.len(), 1, "one capped shipment creates one carrier");
        if local_supplier {
            assert_eq!(allocator.buildings[0].inventory_units(food), 500.0);
            assert_eq!(
                allocator.buildings[0].operating_budget, 0.0,
                "seller paid only on arrival"
            );
        }
        mark_carrier_arrived(&shipments, &mut agents, 0);
        progress_shipments_for_test(
            &mut shipments,
            &mut allocator,
            &mut agents,
            &network,
            &graph,
        );
        if !export {
            assert_eq!(
                allocator.buildings[destination_idx].inventory_units(food),
                100.0
            );
            assert_eq!(
                allocator.buildings[destination_idx].operating_budget,
                10_000.0 - shipments.shipments[0].total_cost
            );
        }
        if local_supplier {
            assert_eq!(
                allocator.buildings[0].operating_budget,
                expected_payment - if export { fee } else { 0.0 }
            );
            assert_eq!(
                allocator.buildings[0].inventory_units(food),
                500.0,
                "excess cargo stays at source"
            );
        }
        assert_eq!(treasury, 0.0);
    }
}

#[test]
fn minimum_input_order_must_afford_the_carrier_as_well_as_the_goods() {
    for local in [false, true] {
        let (graph, network, source_edge, dest_edge, _) = simple_graph_with_border();
        let mut allocator = BuildingAllocator::new();
        if local {
            let asset =
                register_test_asset(&mut allocator, "test", "fee_source", ZoneClass::Industrial);
            allocator.buildings.push(make_building(
                &allocator,
                -50.0,
                ZoneType::Industrial,
                source_edge,
                &asset,
                500.0,
                0.0,
            ));
        }
        let asset = register_test_asset(&mut allocator, "test", "fee_buyer", ZoneClass::Commercial);
        let goods_budget = 50.0 * 12.0 * if local { 1.0 } else { 1.25 };
        let buyer = allocator.buildings.len();
        allocator.buildings.push(make_building(
            &allocator,
            50.0,
            ZoneType::Commercial,
            dest_edge,
            &asset,
            0.0,
            goods_budget,
        ));
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);
        let mut shipments = ShipmentSystem::new();
        create_profile_input_shipments_for_test(
            &mut shipments,
            &mut allocator,
            &network,
            &graph,
            480,
        );
        assert!(shipments.shipments.is_empty());
        assert_eq!(allocator.buildings[buyer].operating_budget, goods_budget);
        allocator.buildings[buyer].operating_budget += 100.0;
        allocator.buildings[buyer].shipment_cooldown_hours = 0;
        create_profile_input_shipments_for_test(
            &mut shipments,
            &mut allocator,
            &network,
            &graph,
            480,
        );
        assert_eq!(shipments.shipments.len(), 1);
        let job = &shipments.shipments[0];
        assert_eq!(job.amount, 50.0);
        assert_eq!(job.total_cost, goods_budget + job.freight_cost);
        assert!(
            (allocator.buildings[buyer].operating_budget - (100.0 - job.freight_cost)).abs()
                < 0.001
        );
    }
}

#[test]
fn local_supplier_creates_and_delivers_shipment() {
    let (graph, network, industrial_edge, commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "logistics_industrial",
        ZoneClass::Industrial,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "logistics_commercial",
        ZoneClass::Commercial,
    );
    let supplier = make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        industrial_edge,
        &industrial_asset,
        300.0,
        0.0,
    );
    let destination = make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        100.0,
        2_000.0,
    );
    allocator.buildings.push(supplier);
    allocator.buildings.push(destination);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    let mut agents = AgentSystem::new();
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 480);
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );

    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(shipments.shipments[0].source, ShipmentEndpoint::Building(0));
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let household_supplies = catalog
        .resource_runtime_id_for_id("household_supplies")
        .expect("household supplies resource");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    assert_eq!(
        allocator.buildings[1].inventory_units(household_supplies),
        100.0
    );

    mark_carrier_arrived(&shipments, &mut agents, 0);
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );

    assert_eq!(shipments.shipments[0].status, ShipmentStatus::Returning);
    assert_eq!(agents.len(), 1);
    assert!(allocator.buildings[1].inventory_units(packaged_food) > 0.0);
    assert!(allocator.buildings[0].inventory_units(packaged_food) < 300.0);
    assert!(allocator.buildings[0].revenue > 0.0);
    assert!(
        shipments
            .shipments
            .iter()
            .all(|shipment| shipment.resource_runtime_id == packaged_food)
    );

    mark_carrier_returned(&shipments, &mut agents, 0);
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );
    assert_eq!(shipments.shipments[0].status, ShipmentStatus::Fulfilled);
    assert_eq!(agents.len(), 0);
}

#[test]
fn dispatch_uses_cheaper_import() {
    let (graph, network, source_edge, buyer_edge, border) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let source_asset = register_test_asset(
        &mut allocator,
        "test",
        "price_source",
        ZoneClass::Industrial,
    );
    let buyer_asset =
        register_test_asset(&mut allocator, "test", "price_buyer", ZoneClass::Commercial);
    allocator.buildings.push(make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        source_edge,
        &source_asset,
        300.0,
        0.0,
    ));
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        buyer_edge,
        &buyer_asset,
        100.0,
        2_000.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();
    let mut shipments = ShipmentSystem::new();
    let mut planning = super::planning::FreightPlanningContext::build(
        &mut shipments,
        &allocator,
        &network,
        &graph,
    );
    let tuning = std::sync::Arc::make_mut(&mut planning.tuning);
    tuning.owa_import_price_multiplier = 0.1;
    shipments.create_profile_input_shipments(
        &mut allocator,
        &network,
        &graph,
        480,
        &mut planning,
        &mut 0.0,
    );
    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(
        shipments.shipments[0].source,
        ShipmentEndpoint::OwaBorder(border)
    );
}

#[test]
fn dispatched_local_shipment_no_longer_reserves_source_inventory() {
    let (graph, network, industrial_edge, commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "dispatched_reservation_industrial",
        ZoneClass::Industrial,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "dispatched_reservation_commercial",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        industrial_edge,
        &industrial_asset,
        300.0,
        0.0,
    ));
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        100.0,
        2_000.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let mut shipments = ShipmentSystem::new();
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 480);
    let queued_source_reservation = shipments
        .build_reservation_views(catalog.resource_count())
        .reserved_outbound_amount(0, packaged_food);
    assert!(queued_source_reservation > 0.0);

    let mut agents = AgentSystem::new();
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );

    let reservations = shipments.build_reservation_views(catalog.resource_count());
    assert_eq!(reservations.reserved_outbound_amount(0, packaged_food), 0.0);
    assert!(reservations.has_open_inbound(1, packaged_food));
}

#[test]
fn local_supplier_can_serve_far_reachable_destination() {
    let (graph, network, industrial_edge, commercial_edge, _) = graph_with_border_to(6_500.0);
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "far_logistics_industrial",
        ZoneClass::Industrial,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "far_logistics_commercial",
        ZoneClass::Commercial,
    );
    let supplier = make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        industrial_edge,
        &industrial_asset,
        300.0,
        0.0,
    );
    let destination = make_building(
        &allocator,
        6_000.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        100.0,
        2_000.0,
    );
    allocator.buildings.push(supplier);
    allocator.buildings.push(destination);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 480);

    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(shipments.shipments[0].source, ShipmentEndpoint::Building(0));
}

#[test]
fn owa_border_fallback_creates_import_shipment() {
    let (graph, network, _industrial_edge, commercial_edge, border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "owa_commercial",
        ZoneClass::Commercial,
    );
    let destination = make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        50.0,
        5_000.0,
    );
    allocator.buildings.push(destination);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    let mut agents = AgentSystem::new();
    let mut treasury_balance = 0.0;
    shipments.hourly_tick(
        &mut allocator,
        &mut agents,
        &mut crate::simulation::economy::households::HouseholdSystem::new(),
        &network,
        &graph,
        480,
        &mut treasury_balance,
        |_, _| {},
    );

    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(
        shipments.shipments[0].source,
        ShipmentEndpoint::OwaBorder(border_node)
    );

    shipments.shipments[0].queued_hours = 4;
    mark_carrier_arrived(&shipments, &mut agents, 0);
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );
    assert_eq!(shipments.shipments[0].status, ShipmentStatus::Returning);
    assert_eq!(shipments.shipments[0].queued_hours, 0);
    mark_carrier_returned(&shipments, &mut agents, 0);
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );
    shipments
        .shipments
        .retain(|shipment| shipment.status.is_open());
    assert!(shipments.shipments.is_empty());
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    assert!(allocator.buildings[0].inventory_units(packaged_food) > 0.0);
}

#[test]
fn city_service_owa_fuel_import_debits_treasury_not_building_budget() {
    let (graph, network, _industrial_edge, service_edge, border_node) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let power_asset = register_test_city_service_asset(
        &mut allocator,
        "test",
        "municipal_power_with_fuel",
        "power",
        "power_plant_basic",
    );
    let mut service_building = make_building(
        &allocator,
        50.0,
        ZoneType::None,
        service_edge,
        &power_asset,
        0.0,
        0.0,
    );
    service_building.worker_count = 1;
    allocator.buildings.push(service_building);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let coal = catalog
        .resource_runtime_id_for_id("coal")
        .expect("coal resource");
    let mut shipments = ShipmentSystem::new();
    let mut agents = AgentSystem::new();
    let mut treasury_balance = 1_000.0;
    shipments.hourly_tick(
        &mut allocator,
        &mut agents,
        &mut crate::simulation::economy::households::HouseholdSystem::new(),
        &network,
        &graph,
        480,
        &mut treasury_balance,
        |_, _| {},
    );

    assert_eq!(shipments.shipments.len(), 2);
    let machinery = catalog.resource_runtime_id_for_id("machinery").unwrap();
    assert_eq!(shipments.shipments[1].resource_runtime_id, machinery);
    let expected_cost: f32 = shipments
        .shipments
        .iter()
        .map(|shipment| {
            shipment.amount
                * catalog
                    .unit_price_for_resource(shipment.resource_runtime_id)
                    .unwrap()
                * load_runtime_economy_tuning()
                    .unwrap()
                    .owa_import_price_multiplier
                + shipment.freight_cost
        })
        .sum();
    assert!((treasury_balance - (1_000.0 - f64::from(expected_cost))).abs() < 0.01);
    assert!((allocator.buildings[0].daily_city_funded_input_cost - expected_cost).abs() < 0.01);
    assert_eq!(shipments.shipments[0].resource_runtime_id, coal);
    assert_eq!(
        shipments.shipments[0].source,
        ShipmentEndpoint::OwaBorder(border_node)
    );
    assert!(treasury_balance < 1_000.0);
    assert_eq!(allocator.buildings[0].operating_budget, 0.0);
    assert!(allocator.buildings[0].daily_city_funded_input_cost > 0.0);

    mark_carrier_arrived(&shipments, &mut agents, 0);
    shipments.progress_shipments(
        &mut allocator,
        &mut agents,
        &mut crate::simulation::economy::households::HouseholdSystem::new(),
        &network,
        &graph,
        &mut treasury_balance,
        |_, _| {},
    );
    assert!(allocator.buildings[0].inventory_units(coal) > 0.0);
    assert!(allocator.buildings[0].daily_owa_input_value > 0.0);
}

#[test]
fn owa_border_fallback_scales_import_to_affordable_amount() {
    let (graph, network, _industrial_edge, commercial_edge, _border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "owa_affordable_commercial",
        ZoneClass::Commercial,
    );
    let destination = make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        50.0,
        1_500.0,
    );
    allocator.buildings.push(destination);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 480);

    assert_eq!(shipments.shipments.len(), 1);
    assert!(matches!(
        shipments.shipments[0].source,
        ShipmentEndpoint::OwaBorder(_)
    ));
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let grocery = catalog
        .profile_for_id("grocery_basic")
        .expect("grocery starter profile");
    let truck_payload_kg = load_runtime_economy_tuning()
        .expect("runtime economy tuning")
        .logistics
        .truck_payload_kg;
    assert!(shipments.shipments[0].amount >= grocery.inputs[0].shipment_batch_kg);
    assert_eq!(
        shipments.shipments[0].amount % grocery.inputs[0].shipment_batch_kg,
        0.0
    );
    assert!(shipments.shipments[0].amount <= truck_payload_kg);
    let grocery_input = grocery.inputs.first().expect("grocery input port");
    assert!(shipments.shipments[0].amount < grocery.inventory_target_units_for(grocery_input));
    assert!(allocator.buildings[0].operating_budget >= 0.0);
    assert!(allocator.buildings[0].operating_budget < 1_500.0);
}

#[test]
fn industrial_processor_requests_input_imports() {
    let (graph, network, industrial_edge, _commercial_edge, _border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "owa_industrial_inputs",
        ZoneClass::Industrial,
    );
    let destination = make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        industrial_edge,
        &industrial_asset,
        0.0,
        5_000.0,
    );
    allocator.buildings.push(destination);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 480);

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let grain = catalog
        .resource_runtime_id_for_id("grain")
        .expect("grain resource");
    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(shipments.shipments[0].resource_runtime_id, grain);
    assert_eq!(
        shipments.shipments[0].destination,
        ShipmentEndpoint::Building(0)
    );
    assert_eq!(allocator.buildings[0].inventory_units(grain), 0.0);
}

#[test]
fn factory_batches_input_orders_and_daily_exports_instead_of_hourly_trucks() {
    let (graph, network, edge, _, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "batched_factory",
        ZoneClass::Industrial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        edge,
        &asset,
        200.0,
        20_000.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let catalog = load_runtime_economy_catalog().unwrap();
    let grain = catalog.resource_runtime_id_for_id("grain").unwrap();
    let food = catalog.resource_runtime_id_for_id("packaged_food").unwrap();
    let machinery = catalog.resource_runtime_id_for_id("machinery").unwrap();
    allocator.buildings[0].set_inventory_units(grain, 750.0);
    allocator.buildings[0].set_inventory_units(machinery, 80.0);
    let mut shipments = ShipmentSystem::new();
    for hour in 0..24 {
        // Six hours of output fit one 50 kg minimum batch. Wait for a daily lot.
        if hour % 6 == 5 {
            allocator.buildings[0].add_inventory_units(food, 50.0);
        }
        create_profile_input_shipments_for_test(
            &mut shipments,
            &mut allocator,
            &network,
            &graph,
            hour * 60,
        );
        create_profile_output_exports_for_test(
            &mut shipments,
            &mut allocator,
            &network,
            &graph,
            hour * 60,
        );
        assert_eq!(shipments.shipments.len(), usize::from(hour == 23));
    }
    assert_eq!(shipments.shipments[0].amount, 200.0);
    assert_eq!(shipments.shipments[0].resource_runtime_id, food);
    allocator.buildings[0].shipment_cooldown_hours = 0;
    allocator.buildings[0].set_inventory_units(grain, 399.0);
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 0);
    assert_eq!(shipments.shipments.len(), 2);
    assert_eq!(shipments.shipments[1].resource_runtime_id, grain);
    assert_eq!(shipments.shipments[1].amount, 400.0);
}

#[test]
fn explicit_field_producer_supplies_processor_input() {
    let (graph, network, industrial_edge, _commercial_edge, _border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let field_asset = register_test_field_asset(&mut allocator, "test", "grain_field_supplier");
    let processor_asset = register_test_asset(
        &mut allocator,
        "test",
        "grain_processor_destination",
        ZoneClass::Industrial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        -70.0,
        ZoneType::None,
        industrial_edge,
        &field_asset,
        300.0,
        0.0,
    ));
    allocator.buildings.push(make_building(
        &allocator,
        -40.0,
        ZoneType::Industrial,
        industrial_edge,
        &processor_asset,
        0.0,
        5_000.0,
    ));
    let machinery = load_runtime_economy_catalog()
        .unwrap()
        .resource_runtime_id_for_id("machinery")
        .unwrap();
    for building in &mut allocator.buildings {
        building.set_inventory_units(machinery, 80.0);
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 480);

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let grain = catalog
        .resource_runtime_id_for_id("grain")
        .expect("grain resource");
    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(shipments.shipments[0].resource_runtime_id, grain);
    assert_eq!(shipments.shipments[0].source, ShipmentEndpoint::Building(0));
    assert_eq!(
        shipments.shipments[0].destination,
        ShipmentEndpoint::Building(1)
    );
}

#[test]
fn owa_export_buffer_uses_active_explicit_work_area_output() {
    let (graph, network, industrial_edge, _commercial_edge, border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let field_asset = register_test_field_asset(&mut allocator, "test", "scaled_export_field");
    allocator.buildings.push(make_building(
        &allocator,
        -70.0,
        ZoneType::None,
        industrial_edge,
        &field_asset,
        425.0,
        0.0,
    ));
    // One of two workers produces 200 kg/day, leaving 225 kg above the reserve.
    allocator.buildings[0].work_area_scale = 20.0;
    allocator.buildings[0].commercial_activity_floor_scale = 1.0;
    allocator.buildings[0].worker_count = 1;
    let machinery = load_runtime_economy_catalog()
        .unwrap()
        .resource_runtime_id_for_id("machinery")
        .unwrap();
    for building in &mut allocator.buildings {
        building.set_inventory_units(machinery, 80.0);
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    create_profile_output_exports_for_test(&mut shipments, &mut allocator, &network, &graph, 480);

    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(
        shipments.shipments[0].destination,
        ShipmentEndpoint::OwaBorder(border_node)
    );
    assert_eq!(shipments.shipments[0].amount, 200.0);
}

#[test]
fn owa_export_buffer_ignores_output_headroom_throttle() {
    let (graph, network, industrial_edge, _commercial_edge, border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let field_asset = register_test_field_asset(&mut allocator, "test", "headroom_export_field");
    allocator.buildings.push(make_building(
        &allocator,
        -70.0,
        ZoneType::None,
        industrial_edge,
        &field_asset,
        200.0,
        0.0,
    ));
    allocator.buildings[0].work_area_scale = 0.25;
    allocator.buildings[0].commercial_activity_floor_scale = 1.0;
    allocator.buildings[0].worker_count = 2;
    let machinery = load_runtime_economy_catalog()
        .unwrap()
        .resource_runtime_id_for_id("machinery")
        .unwrap();
    for building in &mut allocator.buildings {
        building.set_inventory_units(machinery, 80.0);
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    create_profile_output_exports_for_test(&mut shipments, &mut allocator, &network, &graph, 480);

    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(
        shipments.shipments[0].destination,
        ShipmentEndpoint::OwaBorder(border_node)
    );
    // Two workers fully staff this small field: reserve 50 kg/day despite its full buffer.
    assert_eq!(shipments.shipments[0].amount, 150.0);
}

#[test]
fn local_supplier_reservations_prevent_same_pass_overpromise() {
    let (graph, network, industrial_edge, commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "reservation_industrial",
        ZoneClass::Industrial,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "reservation_commercial",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        industrial_edge,
        &industrial_asset,
        300.0,
        0.0,
    ));
    allocator.buildings.push(make_building(
        &allocator,
        40.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        0.0,
        20_000.0,
    ));
    allocator.buildings.push(make_building(
        &allocator,
        60.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        0.0,
        20_000.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let mut shipments = ShipmentSystem::new();
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 480);

    let local_reserved: f32 = shipments
        .shipments
        .iter()
        .filter(|shipment| {
            shipment.source == ShipmentEndpoint::Building(0)
                && shipment.resource_runtime_id == packaged_food
        })
        .map(|shipment| shipment.amount)
        .sum();
    assert!(local_reserved <= 300.0 + f32::EPSILON);
    assert_eq!(
        shipments
            .shipments
            .iter()
            .filter(|shipment| {
                shipment.source == ShipmentEndpoint::Building(0)
                    && shipment.resource_runtime_id == packaged_food
            })
            .count(),
        2
    );
}

#[test]
fn supplier_index_lists_only_suppliers_with_an_unreserved_batch() {
    use crate::simulation::economy::accessibility::ReachableBucketScanEvent;
    let (graph, network, industrial_edge, commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "index_industrial",
        ZoneClass::Industrial,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "index_commercial",
        ZoneClass::Commercial,
    );
    // A full stock, one unit short of a truckload, and a full stock mostly reserved.
    for (x, stock) in [(-50.0, 300.0), (-40.0, 39.0), (-30.0, 300.0)] {
        allocator.buildings.push(make_building(
            &allocator,
            x,
            ZoneType::Industrial,
            industrial_edge,
            &industrial_asset,
            stock,
            0.0,
        ));
    }
    allocator.buildings.push(make_building(
        &allocator,
        40.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        0.0,
        20_000.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let mut shipments = ShipmentSystem::new();
    shipments.shipments.push(Shipment {
        id: 1,
        resource_runtime_id: packaged_food,
        amount: 280.0,
        source: ShipmentEndpoint::Building(2),
        destination: ShipmentEndpoint::Building(3),
        carrier_class: CarrierClass::Truck,
        status: ShipmentStatus::InTransit,
        carrier_agent_id: usize::MAX,
        total_cost: 0.0,
        freight_cost: 0.0,
        eta_hours: 1,
        queued_hours: 0,
    });
    let planning = super::planning::FreightPlanningContext::build(
        &mut shipments,
        &allocator,
        &network,
        &graph,
    );
    let buyer =
        planning
            .freight_components
            .building_components(&allocator, &graph, 3, TransitFlags::CAR);
    let mut listed = Vec::new();
    planning
        .supplier_index
        .buckets_for_resource(packaged_food)
        .expect("stocked supplier indexed")
        .scan_nearest(buyer, 40.0, 10.0, |event| {
            if let ReachableBucketScanEvent::Item { item_idx } = event {
                listed.push(item_idx);
            }
            true
        });
    assert_eq!(listed, [0]);
}

#[test]
fn owa_export_holds_affordable_local_input_need_first() {
    let (graph, network, industrial_edge, commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "local_priority_export_industrial",
        ZoneClass::Industrial,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "local_priority_export_commercial",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        industrial_edge,
        &industrial_asset,
        400.0,
        0.0,
    ));
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        0.0,
        20_000.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    create_profile_output_exports_for_test(&mut shipments, &mut allocator, &network, &graph, 480);

    assert!(
        shipments.shipments.is_empty(),
        "industrial surplus should be held for affordable local commercial input demand"
    );
}

#[test]
fn unreachable_local_input_need_does_not_hold_owa_export() {
    let (mut graph, mut network, industrial_edge, commercial_edge, _) = simple_graph_with_border();
    graph.edge_mut(commercial_edge).allowed_types = TransitFlags::FOOT;
    graph.rebuild_adjacency_list();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = crate::simulation::pathing::cch::CchGraph::build(&graph);

    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "unreachable_local_priority_industrial",
        ZoneClass::Industrial,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "unreachable_local_priority_commercial",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        industrial_edge,
        &industrial_asset,
        400.0,
        0.0,
    ));
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        0.0,
        20_000.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    create_profile_output_exports_for_test(&mut shipments, &mut allocator, &network, &graph, 480);

    assert_eq!(shipments.shipments.len(), 1);
    assert!(matches!(
        shipments.shipments[0].destination,
        ShipmentEndpoint::OwaBorder(_)
    ));
}

#[test]
fn zero_sales_commercial_input_target_reorders_before_empty() {
    let (graph, network, _industrial_edge, commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "scaled_input_commercial",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        0.0,
        20_000.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    logistics_tick!(&mut shipments, &mut allocator, &network, &graph, 480);

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let grocery = catalog
        .profile_for_id("grocery_basic")
        .expect("grocery starter profile");
    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(
        shipments.shipments[0].amount,
        2.0 * grocery.inputs[0].shipment_batch_kg
    );
    allocator.buildings[0].set_inventory_units(grocery.inputs[0].resource_runtime_id, 39.5);
    let mut restock = ShipmentSystem::new();
    create_profile_input_shipments_for_test(&mut restock, &mut allocator, &network, &graph, 480);
    assert_eq!(restock.shipments.len(), 1);
    assert_eq!(
        restock.shipments[0].amount,
        grocery.inputs[0].shipment_batch_kg
    );
}

#[test]
fn deserted_destination_rejects_inbound_delivery() {
    let (graph, network, _industrial_edge, commercial_edge, _border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "deserted_delivery_commercial",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        0.0,
        100.0,
    ));
    allocator.buildings[0].is_deserted = true;
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let mut shipments = ShipmentSystem::new();
    shipments.shipments.push(Shipment {
        id: 0,
        resource_runtime_id: packaged_food,
        amount: 40.0,
        source: ShipmentEndpoint::OwaBorder(0),
        destination: ShipmentEndpoint::Building(0),
        carrier_class: CarrierClass::Truck,
        status: ShipmentStatus::InTransit,
        carrier_agent_id: usize::MAX,
        total_cost: 600.0,
        freight_cost: 0.0,
        eta_hours: 1,
        queued_hours: 0,
    });

    logistics_tick!(&mut shipments, &mut allocator, &network, &graph, 480);

    assert!(shipments.shipments.is_empty());
    assert_eq!(allocator.buildings[0].inventory_units(packaged_food), 0.0);
    assert!((allocator.buildings[0].operating_budget - 700.0).abs() < f32::EPSILON);
}

#[test]
fn under_construction_destination_rejects_inbound_delivery() {
    let (graph, network, _industrial_edge, commercial_edge, _border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "construction_delivery_commercial",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        0.0,
        100.0,
    ));
    allocator.buildings[0].construction_total_hours = 3;
    allocator.buildings[0].construction_remaining_hours = 2;
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let mut shipments = ShipmentSystem::new();
    shipments.shipments.push(Shipment {
        id: 0,
        resource_runtime_id: packaged_food,
        amount: 40.0,
        source: ShipmentEndpoint::OwaBorder(0),
        destination: ShipmentEndpoint::Building(0),
        carrier_class: CarrierClass::Truck,
        status: ShipmentStatus::InTransit,
        carrier_agent_id: usize::MAX,
        total_cost: 600.0,
        freight_cost: 0.0,
        eta_hours: 1,
        queued_hours: 0,
    });

    logistics_tick!(&mut shipments, &mut allocator, &network, &graph, 480);

    assert!(shipments.shipments.is_empty());
    assert_eq!(allocator.buildings[0].inventory_units(packaged_food), 0.0);
    assert!((allocator.buildings[0].operating_budget - 700.0).abs() < f32::EPSILON);
}

#[test]
fn deserted_local_source_releases_reserved_delivery() {
    let (graph, network, industrial_edge, commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "deserted_source_industrial",
        ZoneClass::Industrial,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "deserted_source_commercial",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        industrial_edge,
        &industrial_asset,
        300.0,
        0.0,
    ));
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        0.0,
        100.0,
    ));
    allocator.buildings[0].is_deserted = true;
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let mut shipments = ShipmentSystem::new();
    shipments.shipments.push(Shipment {
        id: 0,
        resource_runtime_id: packaged_food,
        amount: 40.0,
        source: ShipmentEndpoint::Building(0),
        destination: ShipmentEndpoint::Building(1),
        carrier_class: CarrierClass::Truck,
        status: ShipmentStatus::InTransit,
        carrier_agent_id: usize::MAX,
        total_cost: 600.0,
        freight_cost: 0.0,
        eta_hours: 1,
        queued_hours: 0,
    });

    logistics_tick!(&mut shipments, &mut allocator, &network, &graph, 480);

    assert!(shipments.shipments.is_empty());
    assert_eq!(allocator.buildings[0].inventory_units(packaged_food), 300.0);
    assert_eq!(allocator.buildings[1].inventory_units(packaged_food), 0.0);
    assert!((allocator.buildings[1].operating_budget - 700.0).abs() < f32::EPSILON);
}

#[test]
fn stuck_in_transit_local_delivery_expires_and_restores_source_stock() {
    let (graph, network, industrial_edge, commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "stuck_delivery_industrial",
        ZoneClass::Industrial,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "stuck_delivery_commercial",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        industrial_edge,
        &industrial_asset,
        300.0,
        0.0,
    ));
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        100.0,
        2_000.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let mut shipments = ShipmentSystem::new();
    let mut agents = AgentSystem::new();
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 480);
    assert_eq!(shipments.shipments.len(), 1);
    let total_cost = shipments.shipments[0].total_cost;
    assert!(shipments.shipments[0].freight_cost > 0.0);

    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );
    assert_eq!(agents.len(), 1);
    assert!(allocator.buildings[0].inventory_units(packaged_food) < 300.0);
    assert!((allocator.buildings[1].operating_budget - (2_000.0 - total_cost)).abs() < 0.01);

    for _ in 0..6 {
        progress_shipments_for_test(
            &mut shipments,
            &mut allocator,
            &mut agents,
            &network,
            &graph,
        );
    }

    assert_eq!(shipments.shipments[0].status, ShipmentStatus::Expired);
    assert_eq!(agents.len(), 0);
    assert!((allocator.buildings[0].inventory_units(packaged_food) - 300.0).abs() < 0.01);
    assert!((allocator.buildings[1].operating_budget - 2_000.0).abs() < 0.01);
}

#[test]
fn destination_removal_restores_dispatched_local_source_stock() {
    let (graph, network, industrial_edge, commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "removed_destination_source",
        ZoneClass::Industrial,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "removed_destination_target",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        industrial_edge,
        &industrial_asset,
        300.0,
        0.0,
    ));
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        100.0,
        2_000.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let tuning = load_runtime_economy_tuning().expect("runtime economy tuning");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let mut shipments = ShipmentSystem::new();
    let mut agents = AgentSystem::new();
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 480);
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );
    assert_eq!(agents.len(), 1);
    assert!(allocator.buildings[0].inventory_units(packaged_food) < 300.0);

    shipments.invalidate_building(
        1,
        &mut allocator,
        &mut agents,
        &mut crate::simulation::economy::households::HouseholdSystem::new(),
        &mut 0.0,
    );

    assert!(shipments.shipments.is_empty());
    assert_eq!(agents.len(), 0);
    assert!((allocator.buildings[0].inventory_units(packaged_food) - 300.0).abs() < 0.01);
    assert_eq!(
        allocator.buildings[0].shipment_cooldown_hours,
        tuning.operational_clock.shipment_retry_cooldown_hours
    );
}

#[test]
fn endpoint_removal_does_not_reverse_already_settled_sale() {
    for removed in [0, 1] {
        let (graph, network, industrial_edge, commercial_edge, _) = simple_graph_with_border();
        let mut allocator = BuildingAllocator::new();
        let industrial_asset = register_test_asset(
            &mut allocator,
            "test",
            "removed_returning_source",
            ZoneClass::Industrial,
        );
        let commercial_asset = register_test_asset(
            &mut allocator,
            "test",
            "removed_returning_target",
            ZoneClass::Commercial,
        );
        allocator.buildings.push(make_building(
            &allocator,
            -50.0,
            ZoneType::Industrial,
            industrial_edge,
            &industrial_asset,
            300.0,
            0.0,
        ));
        allocator.buildings.push(make_building(
            &allocator,
            50.0,
            ZoneType::Commercial,
            commercial_edge,
            &commercial_asset,
            100.0,
            2_000.0,
        ));
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);
        allocator.rebuild_zone_index();

        let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
        let packaged_food = catalog
            .resource_runtime_id_for_id("packaged_food")
            .expect("packaged food resource");
        let mut shipments = ShipmentSystem::new();
        let mut agents = AgentSystem::new();
        create_profile_input_shipments_for_test(
            &mut shipments,
            &mut allocator,
            &network,
            &graph,
            480,
        );
        progress_shipments_for_test(
            &mut shipments,
            &mut allocator,
            &mut agents,
            &network,
            &graph,
        );
        mark_carrier_arrived(&shipments, &mut agents, 0);
        progress_shipments_for_test(
            &mut shipments,
            &mut allocator,
            &mut agents,
            &network,
            &graph,
        );
        assert_eq!(shipments.shipments[0].status, ShipmentStatus::Returning);
        let source_inventory_after_sale = allocator.buildings[0].inventory_units(packaged_food);
        assert!(source_inventory_after_sale < 300.0);

        let buyer_budget = allocator.buildings[1].operating_budget;
        let seller_revenue = allocator.buildings[0].revenue;
        shipments.invalidate_building(
            removed,
            &mut allocator,
            &mut agents,
            &mut crate::simulation::economy::households::HouseholdSystem::new(),
            &mut 0.0,
        );
        assert_eq!(allocator.buildings[1].operating_budget, buyer_budget);
        assert_eq!(allocator.buildings[0].revenue, seller_revenue);

        assert!(shipments.shipments.is_empty());
        assert_eq!(agents.len(), 0);
        assert!(
            (allocator.buildings[0].inventory_units(packaged_food) - source_inventory_after_sale)
                .abs()
                < 0.01
        );
    }
}

#[test]
fn building_removal_refunds_unsettled_city_freight_to_its_payer() {
    let (_, _, source_edge, service_edge, border) = simple_graph_with_border();
    let catalog = load_runtime_economy_catalog().unwrap();
    let machinery = catalog.resource_runtime_id_for_id("machinery").unwrap();
    for (imported, removed) in [(false, 0), (false, 1), (true, 1)] {
        let mut allocator = BuildingAllocator::new();
        let factory = register_test_asset_with_profile(
            &mut allocator,
            "test",
            "refund_factory",
            ZoneClass::Industrial,
            Some("machinery_factory_basic"),
        );
        let water = register_test_city_service_asset(
            &mut allocator,
            "test",
            "refund_water",
            "water",
            "water_plant_basic",
        );
        allocator.buildings.push(make_building(
            &allocator,
            -50.0,
            ZoneType::Industrial,
            source_edge,
            &factory,
            200.0,
            0.0,
        ));
        allocator.buildings.push(make_building(
            &allocator,
            50.0,
            ZoneType::None,
            service_edge,
            &water,
            0.0,
            123.0,
        ));
        let mut treasury = 2_000.0;
        super::resource::reserve_input_payment(&mut allocator, &mut treasury, 1, 800.0);
        let mut shipments = ShipmentSystem::new();
        shipments.shipments.push(Shipment {
            id: 0,
            resource_runtime_id: machinery,
            amount: 40.0,
            source: if imported {
                ShipmentEndpoint::OwaBorder(border)
            } else {
                ShipmentEndpoint::Building(0)
            },
            destination: ShipmentEndpoint::Building(1),
            carrier_class: CarrierClass::Truck,
            status: ShipmentStatus::InTransit,
            carrier_agent_id: usize::MAX,
            total_cost: 800.0,
            freight_cost: 0.0,
            eta_hours: 1,
            queued_hours: 0,
        });
        assert_eq!(treasury, 1_200.0);
        assert_eq!(allocator.buildings[1].daily_city_funded_input_cost, 800.0);
        let mut agents = AgentSystem::new();

        shipments.invalidate_building(
            removed,
            &mut allocator,
            &mut agents,
            &mut crate::simulation::economy::households::HouseholdSystem::new(),
            &mut treasury,
        );

        assert_eq!(treasury, 2_000.0);
        assert_eq!(allocator.buildings[1].operating_budget, 123.0);
        assert_eq!(allocator.buildings[1].daily_city_funded_input_cost, 0.0);
        assert!(shipments.shipments.is_empty());
        shipments.invalidate_building(
            removed,
            &mut allocator,
            &mut agents,
            &mut crate::simulation::economy::households::HouseholdSystem::new(),
            &mut treasury,
        );
        assert_eq!(treasury, 2_000.0);
    }
}

#[test]
fn unresolved_input_request_escalates_to_terminal_failure() {
    let (graph, network, _industrial_edge, commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "terminal_failure_commercial",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        0.0,
        0.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let tuning = load_runtime_economy_tuning().expect("runtime economy tuning");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let mut shipments = ShipmentSystem::new();
    let ticks = usize::from(tuning.logistics.terminal_failure_attempts)
        * (usize::from(
            tuning
                .operational_clock
                .shipment_retry_cooldown_hours
                .max(1),
        ) + 1);
    for _ in 0..ticks {
        logistics_tick!(&mut shipments, &mut allocator, &network, &graph, 480);
    }

    let failure = shipments
        .request_failures
        .get(&FreightRequestKey {
            destination_building_id: 0,
            resource_runtime_id: packaged_food,
        })
        .expect("request failure");
    assert!(failure.terminal);
    assert_eq!(failure.failures, tuning.logistics.terminal_failure_attempts);
    assert!(shipments.shipments.is_empty());
}

#[test]
fn terminal_input_request_retries_after_budget_recovers() {
    let (graph, network, _industrial_edge, commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "terminal_retry_commercial",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        0.0,
        0.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let tuning = load_runtime_economy_tuning().expect("runtime economy tuning");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let request_key = FreightRequestKey {
        destination_building_id: 0,
        resource_runtime_id: packaged_food,
    };
    let mut shipments = ShipmentSystem::new();
    let ticks = usize::from(tuning.logistics.terminal_failure_attempts)
        * (usize::from(
            tuning
                .operational_clock
                .shipment_retry_cooldown_hours
                .max(1),
        ) + 1);
    for _ in 0..ticks {
        logistics_tick!(&mut shipments, &mut allocator, &network, &graph, 480);
    }
    assert!(
        shipments
            .request_failures
            .get(&request_key)
            .is_some_and(|failure| failure.terminal)
    );

    allocator.buildings[0].operating_budget = 5_000.0;
    allocator.buildings[0].shipment_cooldown_hours = 0;
    logistics_tick!(&mut shipments, &mut allocator, &network, &graph, 480);

    assert!(!shipments.shipments.is_empty());
    assert!(!shipments.request_failures.contains_key(&request_key));
}

#[test]
fn owa_imports_are_not_capped_per_border() {
    let (graph, network, _industrial_edge, commercial_edge, border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "cap_import_commercial",
        ZoneClass::Commercial,
    );
    // More buyers than the retired four active and four queued border jobs.
    for idx in 0..9 {
        allocator.buildings.push(make_building(
            &allocator,
            30.0 + idx as f32 * 5.0,
            ZoneType::Commercial,
            commercial_edge,
            &commercial_asset,
            0.0,
            20_000.0,
        ));
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    logistics_tick!(&mut shipments, &mut allocator, &network, &graph, 480);

    let border_imports: Vec<_> = shipments
        .shipments
        .iter()
        .filter(|shipment| shipment.source == ShipmentEndpoint::OwaBorder(border_node))
        .collect();
    assert_eq!(border_imports.len(), 9);
    assert!(
        border_imports
            .iter()
            .all(|shipment| shipment.status == ShipmentStatus::InTransit)
    );
}

#[test]
fn owa_exports_are_not_capped_per_border() {
    let (graph, network, industrial_edge, _commercial_edge, border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset_with_profile(
        &mut allocator,
        "test",
        "cap_export_industrial",
        ZoneClass::Industrial,
        Some("grain_farm_basic"),
    );
    // More sellers than the retired four active and four queued border jobs.
    for idx in 0..9 {
        allocator.buildings.push(make_building(
            &allocator,
            -70.0 + idx as f32 * 5.0,
            ZoneType::Industrial,
            industrial_edge,
            &industrial_asset,
            50.0,
            0.0,
        ));
    }
    let machinery = load_runtime_economy_catalog()
        .unwrap()
        .resource_runtime_id_for_id("machinery")
        .unwrap();
    for building in &mut allocator.buildings {
        building.set_inventory_units(machinery, 80.0);
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    logistics_tick!(&mut shipments, &mut allocator, &network, &graph, 480);

    let border_exports: Vec<_> = shipments
        .shipments
        .iter()
        .filter(|shipment| shipment.destination == ShipmentEndpoint::OwaBorder(border_node))
        .collect();
    assert_eq!(border_exports.len(), 9);
    assert!(
        border_exports
            .iter()
            .all(|shipment| shipment.status == ShipmentStatus::InTransit)
    );
}

#[test]
fn delivered_exports_pay_freight_without_depressing_the_bid() {
    let (graph, network, industrial_edge, _commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "saturated_export_industrial",
        ZoneClass::Industrial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        industrial_edge,
        &industrial_asset,
        500.0,
        0.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    create_profile_output_exports_for_test(&mut shipments, &mut allocator, &network, &graph, 480);
    assert_eq!(shipments.shipments.len(), 1);
    let first_unit_revenue = shipments.shipments[0].total_cost / shipments.shipments[0].amount;
    let first = shipments.shipments[0].clone();
    let cash_before = allocator.buildings[0].operating_budget;
    assert!(first.freight_cost > 0.0);

    let resource_runtime_id = shipments.shipments[0].resource_runtime_id;
    let mut agents = AgentSystem::new();
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );
    mark_carrier_arrived(&shipments, &mut agents, 0);
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );
    assert!(
        (allocator.buildings[0].operating_budget
            - cash_before
            - (first.total_cost - first.freight_cost))
            .abs()
            < 0.001
    );

    shipments.shipments.clear();
    allocator.buildings[0].shipment_cooldown_hours = 0;
    allocator.buildings[0].add_inventory_units(resource_runtime_id, 500.0);
    create_profile_output_exports_for_test(&mut shipments, &mut allocator, &network, &graph, 480);

    assert_eq!(shipments.shipments.len(), 1);
    let second_unit_revenue = shipments.shipments[0].total_cost / shipments.shipments[0].amount;
    assert_eq!(second_unit_revenue, first_unit_revenue);
}

#[test]
fn exports_have_no_shared_buying_quota_but_reserve_real_stock() {
    let (graph, network, edge, _, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "export_factory",
        ZoneClass::Industrial,
    );
    for x in [-60.0, -40.0] {
        allocator.buildings.push(make_building(
            &allocator,
            x,
            ZoneType::Industrial,
            edge,
            &asset,
            1_000.0,
            0.0,
        ));
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut shipments = ShipmentSystem::new();
    create_profile_output_exports_for_test(&mut shipments, &mut allocator, &network, &graph, 480);
    let reserved: f32 = shipments.shipments.iter().map(|s| s.amount).sum();
    assert_eq!(
        reserved, 1600.0,
        "both factories export their 800 kg surplus, above the former 600 kg quota"
    );
    assert!(
        shipments
            .shipments
            .iter()
            .all(|s| s.freight_cost > 0.0 && s.total_cost > s.freight_cost)
    );
    for building in &mut allocator.buildings {
        building.shipment_cooldown_hours = 0;
    }
    create_profile_output_exports_for_test(&mut shipments, &mut allocator, &network, &graph, 480);
    assert_eq!(
        shipments.shipments.iter().map(|s| s.amount).sum::<f32>(),
        reserved,
        "already reserved cargo cannot be sold twice"
    );
}

#[test]
fn export_waits_until_its_cargo_pays_the_carrier_quote() {
    let (graph, network, edge, _, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "costly_export",
        ZoneClass::Industrial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        edge,
        &asset,
        400.0,
        0.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut shipments = ShipmentSystem::new();
    let mut planning = super::planning::FreightPlanningContext::build(
        &mut shipments,
        &allocator,
        &network,
        &graph,
    );
    std::sync::Arc::make_mut(&mut planning.tuning)
        .logistics
        .dispatch_fee_currency = 3_000.0;
    shipments.create_profile_output_exports(&mut allocator, &network, &graph, 480, &mut planning);
    assert!(
        shipments.shipments.is_empty(),
        "2400 of cargo cannot pay 3000+ freight"
    );
    let food = planning
        .catalog
        .resource_runtime_id_for_id("packaged_food")
        .unwrap();
    assert_eq!(allocator.buildings[0].inventory_units(food), 400.0);
    allocator.buildings[0].add_inventory_units(food, 200.0);
    shipments.create_profile_output_exports(&mut allocator, &network, &graph, 480, &mut planning);
    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(shipments.shipments[0].amount, 400.0);
    assert!(shipments.shipments[0].total_cost > shipments.shipments[0].freight_cost);
}

#[test]
fn owa_export_eta_uses_freight_timing_window() {
    let (graph, network, industrial_edge, _commercial_edge, border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_store_asset = register_test_asset_with_profile(
        &mut allocator,
        "test",
        "timed_export_industrial",
        ZoneClass::Industrial,
        Some("food_processor_basic"),
    );
    let mut building = make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        industrial_edge,
        &industrial_store_asset,
        500.0,
        20_000.0,
    );
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    building.add_inventory_units(packaged_food, 500.0);
    allocator.buildings.push(building);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    let mut agents = AgentSystem::new();
    let mut planning = super::planning::FreightPlanningContext::build(
        &mut shipments,
        &allocator,
        &network,
        &graph,
    );
    // Give this factory a receive window without changing the global catalog.
    let tuning = std::sync::Arc::make_mut(&mut planning.tuning);
    let timing = tuning
        .operational_clock
        .freight_profiles
        .iter_mut()
        .find(|profile| profile.id == "always_open")
        .unwrap();
    timing.preferred_windows = vec![crate::simulation::economy::definitions::MinuteWindow {
        start_minute: 420,
        end_minute: 1080,
    }];
    timing.outside_window_eta_penalty_minutes = 60;
    shipments.create_profile_output_exports(&mut allocator, &network, &graph, 0, &mut planning);
    planning.finish(&mut shipments);
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );

    let export_idx = shipments
        .shipments
        .iter()
        .position(|shipment| shipment.destination == ShipmentEndpoint::OwaBorder(border_node))
        .expect("OWA export shipment");
    assert!(shipments.shipments[export_idx].eta_hours >= 2);

    mark_carrier_arrived(&shipments, &mut agents, export_idx);
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );
    assert_eq!(
        shipments.shipments[export_idx].status,
        ShipmentStatus::Returning
    );
    assert!(allocator.buildings[0].revenue > 0.0);

    mark_carrier_returned(&shipments, &mut agents, export_idx);
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );
    assert_eq!(
        shipments.shipments[export_idx].status,
        ShipmentStatus::Fulfilled
    );
    assert_eq!(agents.len(), 0);
}

#[test]
fn machinery_factory_supplies_city_service_while_retaining_own_upkeep() {
    let (graph, network, industrial_edge, service_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let factory_asset = register_test_asset_with_profile(
        &mut allocator,
        "test",
        "machinery_supplier",
        ZoneClass::Industrial,
        Some("machinery_factory_basic"),
    );
    let water_asset = register_test_city_service_asset(
        &mut allocator,
        "test",
        "water_machinery",
        "water",
        "water_plant_basic",
    );
    let catalog = load_runtime_economy_catalog().unwrap();
    let machinery = catalog.resource_runtime_id_for_id("machinery").unwrap();
    let profile = catalog.profile_for_id("machinery_factory_basic").unwrap();
    let mut factory = make_building(
        &allocator,
        -70.0,
        ZoneType::Industrial,
        industrial_edge,
        &factory_asset,
        120.0,
        10_000.0,
    );
    for port in &profile.inputs {
        if port.resource_runtime_id != machinery {
            factory.set_inventory_units(port.resource_runtime_id, 2.0 * port.shipment_batch_kg);
        }
    }
    allocator.buildings.push(factory);
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::None,
        service_edge,
        &water_asset,
        0.0,
        0.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();
    let mut shipments = ShipmentSystem::new();
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 480);
    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(shipments.shipments[0].resource_runtime_id, machinery);
    assert_eq!(shipments.shipments[0].source, ShipmentEndpoint::Building(0));
    assert_eq!(
        shipments.shipments[0].destination,
        ShipmentEndpoint::Building(1)
    );
    assert_eq!(shipments.shipments[0].amount, 40.0);
    assert!(allocator.buildings[0].inventory_units(machinery) >= 80.0);
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 480);
    assert_eq!(
        shipments.shipments.len(),
        1,
        "reserved shipments must not be ordered again"
    );
}

#[test]
fn small_water_plant_imports_machinery_in_batches_before_running_empty() {
    let (graph, network, _, service_edge, border) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_city_service_asset(
        &mut allocator,
        "test",
        "water_import",
        "water",
        "water_plant_basic",
    );
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::None,
        service_edge,
        &asset,
        0.0,
        0.0,
    ));
    let catalog = load_runtime_economy_catalog().unwrap();
    let machinery = catalog.resource_runtime_id_for_id("machinery").unwrap();
    allocator.buildings[0].set_inventory_units(machinery, 39.5);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();
    let mut shipments = ShipmentSystem::new();
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 480);
    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(shipments.shipments[0].resource_runtime_id, machinery);
    assert_eq!(
        shipments.shipments[0].source,
        ShipmentEndpoint::OwaBorder(border)
    );
    assert_eq!(shipments.shipments[0].amount, 40.0);
    assert_eq!(allocator.buildings[0].inventory_units(machinery), 39.5);
}

#[test]
fn incoming_machinery_factory_can_start_on_paid_owa_deliveries() {
    let (graph, network, industrial_edge, _, border) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset_with_profile(
        &mut allocator,
        "test",
        "machinery_import",
        ZoneClass::Industrial,
        Some("machinery_factory_basic"),
    );
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("machinery_factory_basic").unwrap();
    let import_cost = profile.initial_input_import_cost(
        &catalog,
        1.0,
        load_runtime_economy_tuning()
            .unwrap()
            .owa_import_price_multiplier,
    );
    let payroll = 7.0 * profile.worker_capacity as f32 * profile.average_daily_wage();
    let mut factory = make_building(
        &allocator,
        -70.0,
        ZoneType::Industrial,
        industrial_edge,
        &asset,
        0.0,
        import_cost + payroll,
    );
    factory.worker_count = profile.worker_capacity;
    allocator.buildings.push(factory);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();
    let mut shipments = ShipmentSystem::new();
    let mut agents = AgentSystem::new();
    let mut treasury = 10_000.0;
    shipments.hourly_tick(
        &mut allocator,
        &mut agents,
        &mut crate::simulation::economy::households::HouseholdSystem::new(),
        &network,
        &graph,
        480,
        &mut treasury,
        |_, _| {},
    );
    assert_eq!(shipments.shipments.len(), 3);
    assert_eq!(treasury, 10_000.0, "private imports use the factory budget");
    let fees: f32 = shipments.shipments.iter().map(|job| job.freight_cost).sum();
    assert!(fees > 0.0);
    assert!((allocator.buildings[0].operating_budget - (payroll - fees)).abs() < 0.01);
    for idx in 0..3 {
        assert_eq!(
            shipments.shipments[idx].source,
            ShipmentEndpoint::OwaBorder(border)
        );
        mark_carrier_arrived(&shipments, &mut agents, idx);
    }
    shipments.progress_shipments(
        &mut allocator,
        &mut agents,
        &mut crate::simulation::economy::households::HouseholdSystem::new(),
        &network,
        &graph,
        &mut treasury,
        |_, _| {},
    );
    for port in &profile.inputs {
        assert_eq!(
            allocator.buildings[0].inventory_units(port.resource_runtime_id),
            2.0 * port.shipment_batch_kg
        );
    }
    let factors = crate::simulation::economy::households::building_operation_factors(
        &catalog,
        &allocator.buildings[0],
        profile,
    );
    assert_eq!(
        factors.throughput_factor, 1.0,
        "all delivered ingredients enable production"
    );
}

#[test]
fn local_export_holds_share_the_buyers_budget_across_recipe_inputs() {
    let (graph, network, supply_edge, buyer_edge, _) = simple_graph_with_border();
    let catalog = load_runtime_economy_catalog().unwrap();
    let mut allocator = BuildingAllocator::new();
    let farm = register_test_field_asset(&mut allocator, "test", "hold_grain");
    let machinery = register_test_asset_with_profile(
        &mut allocator,
        "test",
        "hold_machinery",
        ZoneClass::Industrial,
        Some("machinery_factory_basic"),
    );
    let food = register_test_asset(&mut allocator, "test", "hold_food", ZoneClass::Industrial);
    let grain_price = catalog
        .profile_for_id("grain_farm_basic")
        .unwrap()
        .unit_price_currency;
    let buyer_budget = 50.0 * grain_price + 10.0; // Enough for grain freight, not a second input batch.
    for (asset, x, zone, edge, stock, budget) in [
        (farm, -70.0, ZoneType::None, supply_edge, 500.0, 0.0),
        (
            machinery,
            -40.0,
            ZoneType::Industrial,
            supply_edge,
            240.0,
            0.0,
        ),
        (
            food,
            50.0,
            ZoneType::Industrial,
            buyer_edge,
            0.0,
            buyer_budget,
        ),
    ] {
        let mut building = make_building(&allocator, x, zone, edge, &asset, stock, budget);
        if budget == 0.0 {
            let profile = catalog
                .profile_by_runtime_id(building.economy_profile_runtime_id)
                .unwrap();
            for input in &profile.inputs {
                if profile.output_port(input.resource_runtime_id).is_none() {
                    building.set_inventory_units(input.resource_runtime_id, 80.0);
                }
            }
        }
        allocator.buildings.push(building);
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();
    let mut shipments = ShipmentSystem::new();
    let mut planning = super::planning::FreightPlanningContext::build(
        &mut shipments,
        &allocator,
        &network,
        &graph,
    );
    shipments.create_profile_output_exports(&mut allocator, &network, &graph, 480, &mut planning);
    let grain = catalog.resource_runtime_id_for_id("grain").unwrap();
    let machinery = catalog.resource_runtime_id_for_id("machinery").unwrap();
    assert_eq!(
        planning.reservations.reserved_inbound_amount(2, grain),
        50.0
    );
    assert_eq!(
        planning.reservations.reserved_inbound_amount(2, machinery),
        0.0
    );
    assert_eq!(allocator.buildings[2].operating_budget, buyer_budget);
}

#[test]
fn city_input_refund_on_a_later_day_remains_in_net_spending() {
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_city_service_asset(
        &mut allocator,
        "test",
        "refund_water",
        "water",
        "water_plant_basic",
    );
    allocator.buildings.push(make_building(
        &allocator,
        0.0,
        ZoneType::None,
        0,
        &asset,
        0.0,
        0.0,
    ));
    let mut treasury = 10_000.0;
    super::resource::reserve_input_payment(&mut allocator, &mut treasury, 0, 2_800.0);
    allocator.reset_daily_input_accumulators();
    super::resource::refund_input_payment(&mut allocator, &mut treasury, 0, 2_800.0);
    assert_eq!(treasury, 10_000.0);
    assert_eq!(
        allocator.buildings[0].daily_city_funded_input_cost,
        -2_800.0
    );
}
