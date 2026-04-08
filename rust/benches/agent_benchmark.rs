use criterion::{Criterion, black_box, criterion_group, criterion_main};
use godot::prelude::*;
use metrum_rise::simulation::buildings::allocator::BuildingAllocator;
use metrum_rise::simulation::economy::agents::data::Agent;
use metrum_rise::simulation::economy::agents::{
    AgentSystem, MODE_CAR, TRANSIT_IN_BUILDING, TRANSIT_NETWORK, VEHICLE_SEDAN,
};
use metrum_rise::simulation::network::TransitNetwork;
use metrum_rise::simulation::network::graph::{Edge, RegionGraph};
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
                transit.cch_graph =
                    metrum_rise::simulation::pathing::cch::CchGraph::build(&graph);
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
                transit.cch_graph =
                    metrum_rise::simulation::pathing::cch::CchGraph::build(&graph);
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
}

criterion_group!(benches, bench_agent_tick);
criterion_main!(benches);
