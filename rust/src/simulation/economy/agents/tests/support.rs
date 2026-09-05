// SPDX-License-Identifier: GPL-2.0-only

//! Shared agent movement fixtures and network builders.

use super::*;

pub(super) fn register_test_asset(
    allocator: &mut BuildingAllocator,
    pack_id: &str,
    asset_id: &str,
    zone: ZoneClass,
) -> String {
    let (household_capacity, worker_capacity) = match zone {
        ZoneClass::Residential => (Some(6), None),
        ZoneClass::Commercial | ZoneClass::Industrial | ZoneClass::Office => (None, Some(4)),
        ZoneClass::Mixed => (Some(4), Some(2)),
    };
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
            economy_profile: match zone {
                ZoneClass::Commercial => Some("grocery_basic".to_owned()),
                ZoneClass::Industrial => Some("food_processor_basic".to_owned()),
                _ => None,
            },
            extractor: None,
            field: None,
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

pub(super) fn create_test_edge(n0: u32, n1: u32) -> Edge {
    Edge {
        start_node: n0,
        end_node: n1,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 1.0,
        physical_length: 100.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access:
            crate::simulation::network::types::VehicleFrontageAccess::BothSides,
    }
}

pub(super) fn create_test_building(edge_idx: usize, side: i8) -> Building {
    Building {
        center_x: 0.0,
        center_y: 0.0,
        support_height_m: 0.0,
        width_cells: 1,
        depth_cells: 1,
        zone_profile_runtime_id: 0,
        parcel_id: 0,
        zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(1.0, 0.0),
        frontage_t: 0.5, // t=0.5 → depart node = end_node of the edge
        side_offset: 5.0,
        is_deserted: false,
        budget_distress: false,
        edge_idx,
        side,
        cell_x: 0,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        service_funding_override: -1.0,
        asset_id: "test:placeholder".to_owned(),
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
        work_area_scale: 1.0,
        pending_redevelopment: false,
        rezone_grace_days_remaining: 0,
    }
}

// Builds one straight road and returns its directional vehicle lane ids.
pub(super) fn setup_straight_road() -> (TransitNetwork, RegionGraph, usize, usize) {
    let mut network = TransitNetwork::new();
    let mut graph = RegionGraph::new();
    let mut zoning = ZoningSystem::new(&WorldConfig::default());
    let mut allocator = BuildingAllocator::new();
    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(500.0, 0.0, 0.0)],
        1,
        1,
        EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    network.cch_graph = CchGraph::build(&graph);
    let edge_idx = 0;
    let fwd_lane = *network.lane_system.edge_lanes[&edge_idx]
        .iter()
        .find(|&&lid| {
            let l = &network.lane_system.lanes[lid];
            l.is_fwd && l.lane_type == crate::simulation::network::lanes::LaneType::Vehicle
        })
        .expect("forward vehicle lane");
    (network, graph, edge_idx, fwd_lane)
}

/// Place an agent directly on-road on the given lane.
pub(super) fn place_on_lane(
    agents: &mut AgentSystem,
    edge_idx: usize,
    fwd_lane: usize,
    lane_dist: f32,
    speed: f32,
) -> usize {
    let (n0, n1) = (0u32, 1u32);
    let idx = agents.spawn_border_arrival_agent(usize::MAX, n1, 0.0, 0.0, n0, 0.0, 0.0);
    agents.transit[idx] = TRANSIT_NETWORK;
    agents.current_edge[idx] = edge_idx;
    agents.current_lane_id[idx] = fwd_lane;
    agents.lane_distance[idx] = lane_dist;
    agents.speed[idx] = speed;
    agents.current_path[idx] = vec![n0, n1];
    agents.current_path_index[idx] = 1;
    idx
}

pub(super) fn expected_frontage_delay_penalty_s(
    network: &TransitNetwork,
    graph: &RegionGraph,
    edge_idx: usize,
    lane_id: usize,
    observed_speed: f32,
    update_steps: i32,
) -> f32 {
    let lane = &network.lane_system.lanes[lane_id];
    let speed_limit = graph.edge(edge_idx).speed_limit.max(1.0);
    let free_flow_lane_time_s = lane.length / speed_limit;
    let observed_lane_time_s = lane.length / observed_speed.clamp(1.0, speed_limit);
    let raw_delay_s = (observed_lane_time_s - free_flow_lane_time_s).clamp(0.0, 30.0);
    raw_delay_s * (1.0 - 0.75_f32.powi(update_steps))
}

// Returns all forward vehicle lane ids for an edge.
pub(super) fn fwd_vehicle_lanes(network: &TransitNetwork, edge_idx: usize) -> Vec<usize> {
    network.lane_system.edge_lanes[&edge_idx]
        .iter()
        .filter(|&&lid| {
            let l = &network.lane_system.lanes[lid];
            l.is_fwd && l.lane_type == crate::simulation::network::lanes::LaneType::Vehicle
        })
        .copied()
        .collect()
}

/// Returns all backward vehicle lane IDs for `edge_idx`.
pub(super) fn bkw_vehicle_lanes(network: &TransitNetwork, edge_idx: usize) -> Vec<usize> {
    network.lane_system.edge_lanes[&edge_idx]
        .iter()
        .filter(|&&lid| {
            let l = &network.lane_system.lanes[lid];
            !l.is_fwd && l.lane_type == crate::simulation::network::lanes::LaneType::Vehicle
        })
        .copied()
        .collect()
}

/// Returns all forward foot lane IDs for `edge_idx`.
pub(super) fn fwd_foot_lanes(network: &TransitNetwork, edge_idx: usize) -> Vec<usize> {
    network.lane_system.edge_lanes[&edge_idx]
        .iter()
        .filter(|&&lid| {
            let l = &network.lane_system.lanes[lid];
            l.is_fwd && l.lane_type == crate::simulation::network::lanes::LaneType::Foot
        })
        .copied()
        .collect()
}

pub(super) fn fwd_foot_lane_to_edge(
    network: &TransitNetwork,
    edge_idx: usize,
    target_edge_idx: usize,
) -> usize {
    for lane_id in fwd_foot_lanes(network, edge_idx) {
        let lane = &network.lane_system.lanes[lane_id];
        for &conn_lane_id in &lane.next_lanes {
            let Some(conn_lane) = network.lane_system.lanes.get(conn_lane_id) else {
                continue;
            };
            if conn_lane.edge_id != usize::MAX {
                continue;
            }
            let Some(&target_lane_id) = conn_lane.next_lanes.first() else {
                continue;
            };
            if network
                .lane_system
                .lanes
                .get(target_lane_id)
                .is_some_and(|target_lane| target_lane.edge_id == target_edge_idx)
            {
                return lane_id;
            }
        }
    }
    panic!("expected a forward foot lane from edge {edge_idx} to edge {target_edge_idx}");
}

/// Build a two-edge road n0 → n1 → n2 with the given lane counts.
/// Returns `(network, graph, fwd_vehicle_lanes_on_edge_0)`.
pub(super) fn build_two_edge_road(fwd: u8, bkw: u8) -> (TransitNetwork, RegionGraph, Vec<usize>) {
    let width = (fwd as f32 + bkw as f32) * 3.5;
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let n2 = graph.add_node(Vector3::new(200.0, 0.0, 0.0), NodeType::Junction);
    let make = |s: u32, e: u32, x0: f32, x1: f32| Edge {
        start_node: s,
        end_node: e,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width,
        fwd_lanes: fwd,
        bkw_lanes: bkw,
        speed_limit: 14.0,
        base_cost: 1.0,
        physical_length: 100.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(x0, 0.0, 0.0), Vector3::new(x1, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(x0, 0.0, 0.0), Vector3::new(x1, 0.0, 0.0)],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access:
            crate::simulation::network::types::VehicleFrontageAccess::BothSides,
    };
    graph.add_edge(make(n0, n1, 0.0, 100.0));
    graph.add_edge(make(n1, n2, 100.0, 200.0));
    graph.rebuild_adjacency_list();
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    let lanes = fwd_vehicle_lanes(&network, 0);
    (network, graph, lanes)
}

/// Build a 4-way cross junction with the given lane counts on each arm.
/// Returns `(network, graph, [fwd_lanes_arm0..arm3])` — arm order: W, E, N, S.
pub(super) fn build_4way_junction(
    fwd: u8,
    bkw: u8,
) -> (TransitNetwork, RegionGraph, [Vec<usize>; 4]) {
    let width = (fwd as f32 + bkw as f32) * 3.5;
    let mut graph = RegionGraph::new();
    let nc = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let nw = graph.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Junction);
    let ne = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let nn = graph.add_node(Vector3::new(0.0, 0.0, -100.0), NodeType::Junction);
    let ns = graph.add_node(Vector3::new(0.0, 0.0, 100.0), NodeType::Junction);
    let arm = |s: u32, e: u32, sx: f32, sz: f32, ex: f32, ez: f32| Edge {
        start_node: s,
        end_node: e,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width,
        fwd_lanes: fwd,
        bkw_lanes: bkw,
        speed_limit: 14.0,
        base_cost: 1.0,
        physical_length: 100.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(sx, 0.0, sz), Vector3::new(ex, 0.0, ez)],
        physical_geometry: vec![Vector3::new(sx, 0.0, sz), Vector3::new(ex, 0.0, ez)],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access:
            crate::simulation::network::types::VehicleFrontageAccess::BothSides,
    };
    let ew = graph.add_edge(arm(nw, nc, -100.0, 0.0, 0.0, 0.0));
    let ee = graph.add_edge(arm(ne, nc, 100.0, 0.0, 0.0, 0.0));
    let en = graph.add_edge(arm(nn, nc, 0.0, -100.0, 0.0, 0.0));
    let es = graph.add_edge(arm(ns, nc, 0.0, 100.0, 0.0, 0.0));
    graph.rebuild_adjacency_list();
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    let arm_lanes = [
        fwd_vehicle_lanes(&network, ew),
        fwd_vehicle_lanes(&network, ee),
        fwd_vehicle_lanes(&network, en),
        fwd_vehicle_lanes(&network, es),
    ];
    (network, graph, arm_lanes)
}

// ── Scenario helpers ──────────────────────────────────────────────────────────

pub(super) fn assert_connection_lane_spacing(
    agents: &AgentSystem,
    network: &TransitNetwork,
    tick: usize,
    label: &str,
) {
    use crate::config::{CAR_LENGTH, IDM_S_MIN};

    let min_sep = CAR_LENGTH + IDM_S_MIN;
    for i in 0..agents.len() {
        if agents.transit[i] != TRANSIT_INTERSECTION {
            continue;
        }
        let lane_i = agents.current_lane_id[i];
        if lane_i == usize::MAX
            || lane_i >= network.lane_system.lanes.len()
            || network.lane_system.lanes[lane_i].edge_id != usize::MAX
        {
            continue;
        }
        for j in (i + 1)..agents.len() {
            if agents.transit[j] != TRANSIT_INTERSECTION || agents.current_lane_id[j] != lane_i {
                continue;
            }
            let gap = (agents.lane_distance[i] - agents.lane_distance[j]).abs();
            assert!(
                gap >= min_sep - 0.01,
                "[{label}] tick {tick}: cars {i} and {j} overlap on connection lane {lane_i}; gap {gap:.3} < {min_sep:.3}"
            );
        }
    }
}

/// Assert connector-lane queues stay separated while 5 cars/lane pass through
/// the n1 junction of a two-edge road.
pub(super) fn check_connection_spacing_two_edge(fwd: u8, bkw: u8, label: &str) {
    let (mut network, mut graph, fwd_lanes) = build_two_edge_road(fwd, bkw);
    let mut agents = AgentSystem::new();
    let mut allocator = BuildingAllocator::new();
    let (n0, n1, n2) = (0u32, 1u32, 2u32);

    for (li, &lane_id) in fwd_lanes.iter().enumerate() {
        let lane_len = network.lane_system.lanes[lane_id].length;
        for k in 0..5 {
            let dist = (lane_len - 10.0 - (li * 5 + k) as f32 * 8.0).max(0.0);
            let idx = agents.spawn_border_arrival_agent(usize::MAX, n2, 0.0, 0.0, n0, 0.0, 0.0);
            agents.transit[idx] = TRANSIT_NETWORK;
            agents.transit_mode[idx] = MODE_CAR;
            agents.current_node[idx] = n0;
            agents.current_edge[idx] = 0;
            agents.current_lane_id[idx] = lane_id;
            agents.lane_distance[idx] = dist;
            agents.speed[idx] = 14.0;
            agents.current_path[idx] = vec![n0, n1, n2];
            agents.current_path_index[idx] = 1;
        }
    }

    for tick in 0..100 {
        agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);
        assert_connection_lane_spacing(&agents, &network, tick, label);
    }
}

/// Assert connector-lane queues stay separated while one car/lane approaches
/// the center of a 4-way junction from all four arms.
pub(super) fn check_connection_spacing_4way(fwd: u8, bkw: u8, label: &str) {
    let (mut network, mut graph, arm_lanes) = build_4way_junction(fwd, bkw);
    let mut agents = AgentSystem::new();
    let mut allocator = BuildingAllocator::new();
    let nc = 0u32;
    let arm_nodes = [1u32, 2u32, 3u32, 4u32];
    let arm_edges = [0usize, 1usize, 2usize, 3usize];

    for (k, lanes) in arm_lanes.iter().enumerate() {
        for &lane_id in lanes {
            let lane_len = network.lane_system.lanes[lane_id].length;
            let idx =
                agents.spawn_border_arrival_agent(usize::MAX, nc, 0.0, 0.0, arm_nodes[k], 0.0, 0.0);
            agents.transit[idx] = TRANSIT_NETWORK;
            agents.transit_mode[idx] = MODE_CAR;
            agents.current_node[idx] = arm_nodes[k];
            agents.current_edge[idx] = arm_edges[k];
            agents.current_lane_id[idx] = lane_id;
            agents.lane_distance[idx] = (lane_len - 5.0).max(0.0);
            agents.speed[idx] = 14.0;
            agents.current_path[idx] = vec![arm_nodes[k], nc];
            agents.current_path_index[idx] = 1;
        }
    }

    for tick in 0..60 {
        agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);
        assert_connection_lane_spacing(&agents, &network, tick, label);
    }
}

// Assert no car loops back to edge 0 after passing through the degree-2 node.
pub(super) fn check_no_uturn_at_frontage(fwd: u8, bkw: u8, label: &str) {
    let (mut network, mut graph, fwd_lanes) = build_two_edge_road(fwd, bkw);
    let (n0, n1, n2) = (0u32, 1u32, 2u32);
    let mut allocator = BuildingAllocator::new();
    let mut agents = AgentSystem::new();

    for (li, &lane_id) in fwd_lanes.iter().enumerate() {
        let lane_len = network.lane_system.lanes[lane_id].length;
        for k in 0..3 {
            let dist = (lane_len - 5.0 - (li * 3 + k) as f32 * 8.0).max(0.0);
            let idx = agents.spawn_border_arrival_agent(usize::MAX, n2, 0.0, 0.0, n0, 0.0, 0.0);
            agents.transit[idx] = TRANSIT_NETWORK;
            agents.transit_mode[idx] = MODE_CAR;
            agents.current_node[idx] = n0;
            agents.current_edge[idx] = 0;
            agents.current_lane_id[idx] = lane_id;
            agents.lane_distance[idx] = dist;
            agents.speed[idx] = 14.0;
            agents.current_path[idx] = vec![n0, n1, n2];
            agents.current_path_index[idx] = 1;
        }
    }

    for _ in 0..200 {
        agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);
    }

    for i in 0..agents.len() {
        assert_ne!(
            agents.current_edge[i], 0,
            "[{label}] car {i} still on edge 0 after 200 ticks — stuck or U-turning at degree-2 node"
        );
    }
}

// ── Parametrized test entry points ───────────────────────────────────────────
