use criterion::{BatchSize, BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use godot::prelude::*;
use metrum_rise::assets::AssetManifest;
use metrum_rise::assets::asset::{Anchor, AnchorType, BuildingData, LodEntry, ZoneClass};
use metrum_rise::simulation::buildings::allocator::BuildingAllocator;
use metrum_rise::simulation::core::config::WorldConfig;
use metrum_rise::simulation::economy::agents::data::Agent;
use metrum_rise::simulation::economy::agents::{
    ACCESS_PLAN_VALID, AgentSystem, MODE_CAR, TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS,
    TRANSIT_IN_BUILDING, TRANSIT_NETWORK, VEHICLE_SEDAN,
};
use metrum_rise::simulation::economy::households::HouseholdSystem;
use metrum_rise::simulation::economy::logistics::ShipmentSystem;
use metrum_rise::simulation::grid::zoning::{ZoneType, ZoningSystem};
use metrum_rise::simulation::network::TransitNetwork;
use metrum_rise::simulation::network::graph::{Edge, RegionGraph};
use metrum_rise::simulation::network::lanes::{Lane, LaneType};
use metrum_rise::simulation::network::types::{
    EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
};
use std::time::Duration;

struct SharedSetup {
    graph: RegionGraph,
    node_a: u32,
    node_b: u32,
    edge_ab: usize,
}

struct AccessSharedSetup {
    graph: RegionGraph,
    zoning: ZoningSystem,
    allocator: BuildingAllocator,
    door_pos: Vector2,
    lane_point: Vector2,
    lane_id: usize,
    attach_node: u32,
    detach_node: u32,
    lane_d: f32,
}

struct AccessBenchState {
    agents: AgentSystem,
    allocator: BuildingAllocator,
    transit: TransitNetwork,
    graph: RegionGraph,
}

fn build_shared() -> SharedSetup {
    let mut graph = RegionGraph::new();

    // Minimal 2-node, 1-edge graph.
    // No TransitNetwork::add_road() — no Voronoi, no zoning, no CCH-dirty marking.
    let node_a = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let node_b = graph.add_node(Vector3::new(500.0, 0.0, 0.0), NodeType::Junction);

    // 1,000 km edge with geometry every 10 km.
    // This keeps the route alive across long Criterion runs without needing a huge per-agent
    // node path buffer.
    let geometry: Vec<Vector3> = (0..=100)
        .map(|i| Vector3::new(i as f32 * 10_000.0, 0.0, 0.0))
        .collect();
    let physical_length = 1_000_000.0;
    let speed_ms = 20.0_f32;
    let edge = Edge {
        start_node: node_a,
        end_node: node_b,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: speed_ms,
        base_cost: physical_length / speed_ms,
        physical_length,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: geometry.clone(),
        physical_geometry: geometry,
        class: EdgeClass::Standard,
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    };
    let edge_ab = graph.add_edge(edge);
    graph.rebuild_adjacency_list();

    // Build a TransitNetwork with a CCH and lane system over this minimal graph.
    let mut transit = TransitNetwork::new();
    transit.cch_graph = metrum_rise::simulation::pathing::cch::CchGraph::build(&graph);
    transit.lane_system.rebuild(&mut graph);

    SharedSetup {
        graph,
        node_a,
        node_b,
        edge_ab,
    }
}

fn register_test_asset(
    allocator: &mut BuildingAllocator,
    pack_id: &str,
    asset_id: &str,
    zone: ZoneClass,
) -> String {
    let manifest = AssetManifest {
        asset_id: asset_id.to_owned(),
        display_name: "Bench".to_owned(),
        asset_set: None,
        tags: vec![],
        thumbnail: None,
        lods: vec![LodEntry {
            file: "lod0.glb".to_owned(),
            distance_min_m: 0.0,
            distance_max_m: None,
        }],
        anchors: vec![Anchor {
            anchor_type: AnchorType::Entrance,
            name: "main".to_owned(),
            position: [0.0, 0.0, 0.5],
            forward: [0.0, 0.0, 1.0],
        }],
        building: Some(BuildingData {
            zone_type: zone,
            density: "low".to_owned(),
            lot_width_cells: 1,
            lot_depth_cells: 1,
            level: 1,
            residents_capacity: Some(6),
            worker_capacity: None,
            service_class: None,
            economy_profile: None,
            preview_scale: Some(1.0),
        }),
        prop: None,
        vehicle: None,
        character: None,
        pivot_offset: None,
    };
    allocator
        .registry
        .register(pack_id, manifest, String::new());
    format!("{pack_id}:{asset_id}")
}

fn create_access_edge(n0: u32, n1: u32) -> Edge {
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
        base_cost: 10.0,
        physical_length: 500.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(500.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(500.0, 0.0, 0.0)],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    }
}

fn world_door_pos(center_x: f32, center_y: f32, facing_dir: Vector2) -> Vector2 {
    let basis_z = if facing_dir.length_squared() > 1e-12 {
        facing_dir.normalized()
    } else {
        Vector2::new(0.0, 1.0)
    };
    Vector2::new(center_x, center_y) + basis_z * 0.5
}

fn sample_pos_on_polyline(points: &[Vector3], total_len: f32, s_m: f32) -> Vector2 {
    if points.is_empty() {
        return Vector2::ZERO;
    }
    if points.len() == 1 || total_len <= 1e-6 {
        return Vector2::new(points[0].x, points[0].z);
    }

    let target_s = s_m.clamp(0.0, total_len);
    let mut acc_len = 0.0;
    for i in 0..points.len() - 1 {
        let seg_len = points[i].distance_to(points[i + 1]);
        if seg_len <= 1e-6 {
            continue;
        }
        if acc_len + seg_len >= target_s {
            let local_t = ((target_s - acc_len) / seg_len).clamp(0.0, 1.0);
            let p0 = Vector2::new(points[i].x, points[i].z);
            let p1 = Vector2::new(points[i + 1].x, points[i + 1].z);
            return p0.lerp(p1, local_t);
        }
        acc_len += seg_len;
    }

    Vector2::new(points.last().unwrap().x, points.last().unwrap().z)
}

fn sample_pos_on_edge(graph: &RegionGraph, edge_idx: usize, t: f32) -> Vector2 {
    let edge = graph.edge(edge_idx);
    sample_pos_on_polyline(
        &edge.physical_geometry,
        edge.physical_length,
        t.clamp(0.0, 1.0) * edge.physical_length,
    )
}

fn sample_pos_on_lane(lane: &Lane, lane_d: f32) -> Vector2 {
    sample_pos_on_polyline(&lane.geometry, lane.length, lane_d.clamp(0.0, lane.length))
}

fn project_point_to_polyline_s(points: &[Vector3], world_pos: Vector2) -> f32 {
    if points.len() < 2 {
        return 0.0;
    }

    let mut acc_len = 0.0;
    let mut best_dist2 = f32::INFINITY;
    let mut best_idx = usize::MAX;
    let mut best_t = f32::INFINITY;
    let mut best_s = 0.0;

    for i in 0..points.len() - 1 {
        let p0 = Vector2::new(points[i].x, points[i].z);
        let p1 = Vector2::new(points[i + 1].x, points[i + 1].z);
        let seg = p1 - p0;
        let seg_len2 = seg.length_squared();
        let local_t = if seg_len2 > 1e-12 {
            ((world_pos - p0).dot(seg) / seg_len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let closest = p0 + seg * local_t;
        let dist2 = (world_pos - closest).length_squared();
        let seg_len = points[i].distance_to(points[i + 1]);
        let better = dist2 < best_dist2
            || (dist2 == best_dist2 && (i < best_idx || (i == best_idx && local_t < best_t)));
        if better {
            best_dist2 = dist2;
            best_idx = i;
            best_t = local_t;
            best_s = acc_len + seg_len * local_t;
        }
        acc_len += seg_len;
    }

    best_s
}

fn lane_origin_node(lane: &Lane, graph: &RegionGraph) -> u32 {
    let edge = graph.edge(lane.edge_id);
    if lane.is_fwd {
        edge.start_node
    } else {
        edge.end_node
    }
}

fn lane_terminal_node(lane: &Lane, graph: &RegionGraph) -> u32 {
    let edge = graph.edge(lane.edge_id);
    if lane.is_fwd {
        edge.end_node
    } else {
        edge.start_node
    }
}

fn build_access_shared() -> AccessSharedSetup {
    let mut graph = RegionGraph::new();
    let node_a = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let node_b = graph.add_node(Vector3::new(500.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(create_access_edge(node_a, node_b));
    graph.rebuild_adjacency_list();

    let map_cfg = WorldConfig::default();
    let mut zoning = ZoningSystem::new(&map_cfg);
    zoning.set_zone_rect(-50.0, -50.0, 550.0, 50.0, ZoneType::Commercial);
    zoning.update_distance_to_road(&graph);

    let mut allocator = BuildingAllocator::new();
    let asset_id = register_test_asset(
        &mut allocator,
        "bench",
        "access_building",
        ZoneClass::Commercial,
    );
    allocator
        .buildings
        .push(metrum_rise::simulation::buildings::allocator::Building {
            center_x: 250.0,
            center_y: -10.0,
            width_cells: 1,
            depth_cells: 1,
            zone_type: ZoneType::Commercial,
            facing_dir: Vector2::new(0.0, -1.0),
            frontage_t: 0.5,
            side_offset: 5.0,
            abandoned_timer: 0,
            edge_idx,
            side: 1,
            cell_x: 0,
            cell_y: 0,
            occupancy: 0,
            worker_count: 0,
            asset_id,
            level: 1,
            broken: false,
            stock: 0.0,
            revenue: 0.0,
            operating_budget: 500.0,
            utility_service_available: false,
            shipment_cooldown_days: 0,
        });

    let building = &allocator.buildings[0];
    let door_pos = world_door_pos(building.center_x, building.center_y, building.facing_dir);
    let entrance_s_m =
        project_point_to_polyline_s(&graph.edge(edge_idx).physical_geometry, door_pos);

    let mut transit = TransitNetwork::new();
    transit.lane_system.rebuild(&mut graph);
    let lane_id = transit
        .lane_system
        .edge_lanes
        .get(&edge_idx)
        .and_then(|lanes| {
            lanes.iter().copied().find(|&lane_id| {
                let lane = &transit.lane_system.lanes[lane_id];
                lane.lane_type == LaneType::Vehicle && lane.is_fwd
            })
        })
        .expect("benchmark access setup requires one forward vehicle lane");
    let lane = &transit.lane_system.lanes[lane_id];
    let edge_pos = sample_pos_on_edge(
        &graph,
        edge_idx,
        entrance_s_m / graph.edge(edge_idx).physical_length,
    );
    let lane_d = project_point_to_polyline_s(&lane.geometry, edge_pos);
    let lane_point = sample_pos_on_lane(lane, lane_d);
    let attach_node = lane_terminal_node(lane, &graph);
    let detach_node = lane_origin_node(lane, &graph);

    AccessSharedSetup {
        graph,
        zoning,
        allocator,
        door_pos,
        lane_point,
        lane_id,
        attach_node,
        detach_node,
        lane_d,
    }
}

fn make_access_egress_agent(shared: &AccessSharedSetup) -> Agent {
    Agent {
        home_building: 0,
        household_id: usize::MAX,
        work_building: usize::MAX,
        pos_x: shared.door_pos.x,
        pos_y: shared.door_pos.y,
        activity: 0,
        transit: TRANSIT_ACCESS_EGRESS,
        happiness: 50.0,
        money: 100.0,
        journey_start_time: 0.0,
        current_building: 0,
        target_building: usize::MAX,
        planned_target_building: usize::MAX,
        current_node: u32::MAX,
        planned_attach_node: shared.attach_node,
        planned_detach_node: u32::MAX,
        planned_attach_lane_id: shared.lane_id as u32,
        planned_detach_lane_id: u32::MAX,
        planned_attach_lane_d: shared.lane_d,
        planned_detach_lane_d: 0.0,
        access_flags: ACCESS_PLAN_VALID,
        next_replan_time: 0.0,
        current_edge: usize::MAX,
        current_lane_id: usize::MAX,
        lane_distance: 0.0,
        speed: 0.0,
        transit_mode: MODE_CAR,
        planned_activity: 0,
        current_path: Vec::new(),
        current_path_index: 0,
        has_car: true,
        vehicle_type: VEHICLE_SEDAN,
        pedestrian_type: 0,
        walk_phase: 0.0,
    }
}

fn make_access_ingress_agent(shared: &AccessSharedSetup) -> Agent {
    Agent {
        home_building: 0,
        household_id: usize::MAX,
        work_building: usize::MAX,
        pos_x: shared.lane_point.x,
        pos_y: shared.lane_point.y,
        activity: 1,
        transit: TRANSIT_ACCESS_INGRESS,
        happiness: 50.0,
        money: 100.0,
        journey_start_time: 0.0,
        current_building: usize::MAX,
        target_building: 0,
        planned_target_building: usize::MAX,
        current_node: u32::MAX,
        planned_attach_node: u32::MAX,
        planned_detach_node: shared.detach_node,
        planned_attach_lane_id: u32::MAX,
        planned_detach_lane_id: shared.lane_id as u32,
        planned_attach_lane_d: 0.0,
        planned_detach_lane_d: shared.lane_d,
        access_flags: ACCESS_PLAN_VALID,
        next_replan_time: 0.0,
        current_edge: usize::MAX,
        current_lane_id: usize::MAX,
        lane_distance: 0.0,
        speed: 0.0,
        transit_mode: MODE_CAR,
        planned_activity: 0,
        current_path: Vec::new(),
        current_path_index: 0,
        has_car: true,
        vehicle_type: VEHICLE_SEDAN,
        pedestrian_type: 0,
        walk_phase: 0.0,
    }
}

fn build_access_state(shared: &AccessSharedSetup, count: usize, phase: u8) -> AccessBenchState {
    let mut graph = shared.graph.clone();
    let mut zoning = shared.zoning.clone();
    let mut allocator = shared.allocator.clone();
    let mut transit = TransitNetwork::new();
    transit.lane_system.rebuild(&mut graph);

    let mut allocator_agents = AgentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut logistics = ShipmentSystem::new();
    allocator.tick(
        &mut zoning,
        &mut allocator_agents,
        &mut households,
        &mut logistics,
        &mut transit,
        &mut graph,
    );

    let mut agents = AgentSystem::new();
    for _ in 0..count {
        let agent = if phase == TRANSIT_ACCESS_EGRESS {
            make_access_egress_agent(shared)
        } else {
            make_access_ingress_agent(shared)
        };
        agents.agents.push(agent);
    }

    AccessBenchState {
        agents,
        allocator,
        transit,
        graph,
    }
}

fn make_idle_agent(shared: &SharedSetup) -> Agent {
    Agent {
        home_building: usize::MAX,
        household_id: usize::MAX,
        work_building: usize::MAX,
        pos_x: 0.0,
        pos_y: 0.0,
        activity: 0,
        transit: TRANSIT_IN_BUILDING,
        happiness: 50.0,
        money: 100.0,
        journey_start_time: 0.0,
        current_building: usize::MAX,
        target_building: usize::MAX,
        planned_target_building: usize::MAX,
        current_node: shared.node_a,
        planned_attach_node: u32::MAX,
        planned_detach_node: u32::MAX,
        planned_attach_lane_id: u32::MAX,
        planned_detach_lane_id: u32::MAX,
        planned_attach_lane_d: 0.0,
        planned_detach_lane_d: 0.0,
        access_flags: 0,
        next_replan_time: 0.0,
        current_edge: usize::MAX,
        current_lane_id: usize::MAX,
        lane_distance: 0.0,
        speed: 0.0,
        transit_mode: MODE_CAR,
        planned_activity: 0,
        current_path: Vec::new(),
        current_path_index: 0,
        has_car: true,
        vehicle_type: VEHICLE_SEDAN,
        pedestrian_type: 0,
        walk_phase: 0.0,
    }
}

/// NETWORK agent with a long single-edge route that will not be exhausted during a Criterion run.
///
/// `target_building = usize::MAX` and `access_flags = 0` keep the benchmark on the pure
/// lane-traversal path with no destination-side entrance replanning.
fn make_on_road_agent(shared: &SharedSetup, route: Vec<u32>, progression: f32) -> Agent {
    Agent {
        home_building: usize::MAX,
        household_id: usize::MAX,
        work_building: usize::MAX,
        pos_x: 0.0,
        pos_y: 0.0,
        activity: 0,
        transit: TRANSIT_NETWORK,
        happiness: 50.0,
        money: 100.0,
        journey_start_time: 0.0,
        current_building: usize::MAX,
        target_building: usize::MAX, // no arrival check → no CCH calls
        planned_target_building: usize::MAX,
        current_node: shared.node_a,
        planned_attach_node: u32::MAX,
        planned_detach_node: u32::MAX,
        planned_attach_lane_id: u32::MAX,
        planned_detach_lane_id: u32::MAX,
        planned_attach_lane_d: 0.0,
        planned_detach_lane_d: 0.0,
        access_flags: 0,
        next_replan_time: 0.0,
        current_edge: shared.edge_ab,
        current_lane_id: usize::MAX,
        lane_distance: progression,
        speed: 20.0,
        transit_mode: MODE_CAR,
        planned_activity: 0,
        current_path: route,
        current_path_index: 1, // [0] = origin node, [1] = live target
        has_car: true,
        vehicle_type: VEHICLE_SEDAN,
        pedestrian_type: 0,
        walk_phase: 0.0,
    }
}

fn bench_agent_tick(c: &mut Criterion) {
    let shared = build_shared();
    let access_shared = build_access_shared();

    let mut group = c.benchmark_group("AgentSystem::tick");
    group.sample_size(30);
    group.warm_up_time(Duration::from_secs(2));
    group.measurement_time(Duration::from_secs(8));

    // --- ON_ROAD: measures lane traversal and movement maths. ---
    // Single long route, empty allocator, and no exact access plan → no CCH calls in the hot path.
    let route = vec![shared.node_a, shared.node_b];

    for &count in &[1_000usize, 10_000, 100_000, 1_000_000] {
        group.bench_with_input(
            criterion::BenchmarkId::new("on_road", count),
            &count,
            |b, &count| {
                let mut agents = AgentSystem::new();
                let mut graph = shared.graph.clone();
                let allocator = BuildingAllocator::new();
                let mut transit = TransitNetwork::new();
                transit.cch_graph = metrum_rise::simulation::pathing::cch::CchGraph::build(&graph);
                transit.lane_system.rebuild(&mut graph);

                let seg_count = shared.graph.edges()[shared.edge_ab].physical_length;
                for i in 0..count {
                    // Spread agents across the first half of the edge.
                    let prog = i as f32 % (seg_count / 2.0).max(1.0);
                    agents
                        .agents
                        .push(make_on_road_agent(&shared, route.clone(), prog));
                }

                b.iter(|| {
                    agents.tick(
                        black_box(&allocator),
                        black_box(&mut transit),
                        black_box(&mut graph),
                        black_box(0.016),
                    );
                });
            },
        );
    }

    // --- IDLE SCALING: pure SoA iteration cost, no pathfinding. ---
    // Empty allocator → safety scrub sets all building refs to usize::MAX on tick 1
    // → IDLE branch never finds a destination → zero activations, zero CCH calls.
    for &count in &[1_000usize, 10_000, 100_000, 1_000_000] {
        group.bench_with_input(
            criterion::BenchmarkId::new("idle_scaling", count),
            &count,
            |b, &count| {
                let mut agents = AgentSystem::new();
                let mut graph = shared.graph.clone();
                let allocator = BuildingAllocator::new();
                let mut transit = TransitNetwork::new();
                transit.cch_graph = metrum_rise::simulation::pathing::cch::CchGraph::build(&graph);
                transit.lane_system.rebuild(&mut graph);

                for _ in 0..count {
                    agents.agents.push(make_idle_agent(&shared));
                }

                b.iter(|| {
                    agents.tick(
                        black_box(&allocator),
                        black_box(&mut transit),
                        black_box(&mut graph),
                        black_box(0.016),
                    );
                });
            },
        );
    }

    group.finish();

    // ACCESS benchmark intentionally uses the richer opposite-side car frontage path so we
    // measure the full local-access polyline/crossover logic rather than the trivial same-side
    // two-point path. Each Criterion iteration starts from a fresh legal access state.
    let mut access_group = c.benchmark_group("AgentSystem::tick_access");
    access_group.sample_size(20);
    access_group.warm_up_time(Duration::from_secs(1));
    access_group.measurement_time(Duration::from_secs(5));

    for &count in &[1_000usize, 10_000, 100_000] {
        access_group.bench_with_input(
            BenchmarkId::new("access_egress_car", count),
            &count,
            |b, &count| {
                b.iter_batched_ref(
                    || build_access_state(&access_shared, count, TRANSIT_ACCESS_EGRESS),
                    |state| {
                        state.agents.tick(
                            black_box(&state.allocator),
                            black_box(&mut state.transit),
                            black_box(&mut state.graph),
                            black_box(0.016),
                        );
                    },
                    BatchSize::LargeInput,
                );
            },
        );
    }

    for &count in &[1_000usize, 10_000, 100_000] {
        access_group.bench_with_input(
            BenchmarkId::new("access_ingress_car", count),
            &count,
            |b, &count| {
                b.iter_batched_ref(
                    || build_access_state(&access_shared, count, TRANSIT_ACCESS_INGRESS),
                    |state| {
                        state.agents.tick(
                            black_box(&state.allocator),
                            black_box(&mut state.transit),
                            black_box(&mut state.graph),
                            black_box(0.016),
                        );
                    },
                    BatchSize::LargeInput,
                );
            },
        );
    }

    access_group.finish();
}

criterion_group!(benches, bench_agent_tick);
criterion_main!(benches);
