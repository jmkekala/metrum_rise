use criterion::{Criterion, black_box, criterion_group, criterion_main};
use godot::prelude::*;
use metrum_rise::simulation::buildings::allocator::BuildingAllocator;
use metrum_rise::simulation::economy::agents::{
    AgentSystem, MODE_CAR, TRANSIT_IDLE, TRANSIT_ON_ROAD, VEHICLE_SEDAN,
};
use metrum_rise::simulation::economy::agents::data::Agent;
use metrum_rise::simulation::network::graph::{Edge, RegionGraph};
use metrum_rise::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
use metrum_rise::simulation::network::TransitNetwork;
use std::time::Duration;

struct SharedSetup {
    graph: RegionGraph,
    transit: TransitNetwork,
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

    // 500 m edge with geometry every 10 m (50 segments).
    // At 50 km/h (~14 m/s) and delta=0.016 s, each segment (~10 m) takes ~0.7 s / 44 ticks.
    let geometry: Vec<Vector3> = (0..=50)
        .map(|i| Vector3::new(i as f32 * 10.0, 0.0, 0.0))
        .collect();
    let speed_ms = 50.0_f32 / 3.6;
    let edge = Edge {
        start_node: node_a,
        end_node: node_b,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 500.0 / speed_ms,
        physical_length: 500.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: geometry.clone(),
        physical_geometry: geometry,
        class: EdgeClass::Standard,
        deleted: false,
    };
    let edge_ab = graph.add_edge(edge);
    graph.rebuild_adjacency_list();

    // Build a TransitNetwork with a CCH and lane system over this minimal graph.
    let mut transit = TransitNetwork::new();
    transit.cch_graph = metrum_rise::simulation::pathing::cch::CchGraph::build(&graph);
    transit.lane_system.rebuild(&mut graph);

    SharedSetup { graph, transit, node_a, node_b, edge_ab }
}

fn make_idle_agent(shared: &SharedSetup) -> Agent {
    Agent {
        home_building: usize::MAX, // empty allocator → safety scrub blocks activation
        work_building: usize::MAX,
        pos_x: 0.0,
        pos_y: 0.0,
        is_visible: true,
        activity: 0,
        transit: TRANSIT_IDLE,
        happiness: 50.0,
        money: 100.0,
        journey_start_time: 0.0,
        current_building: usize::MAX,
        target_building: usize::MAX,
        current_node: shared.node_a,
        target_node: shared.node_b,
        current_edge: usize::MAX,
        current_lane_id: usize::MAX,
        lane_distance: 0.0,
        transit_mode: MODE_CAR,
        current_path: Vec::new(),
        current_path_index: 0,
        has_car: true,
        vehicle_type: VEHICLE_SEDAN,
        pedestrian_type: 0,
        speed: 4.0,
        walk_phase: 0.0,
    }
}

/// ON_ROAD agent with a pre-computed A↔B bounce path long enough to never be
/// exhausted during the benchmark window.
///
/// `target_building = usize::MAX` so the arrival check is skipped every tick
/// and CCH is never called → zero heap allocation in the hot path.
fn make_on_road_agent(shared: &SharedSetup, bounce_path: Vec<u32>, progression: f32) -> Agent {
    Agent {
        home_building: usize::MAX,
        work_building: usize::MAX,
        pos_x: 0.0,
        pos_y: 0.0,
        is_visible: true,
        activity: 0,
        transit: TRANSIT_ON_ROAD,
        happiness: 50.0,
        money: 100.0,
        journey_start_time: 0.0,
        current_building: usize::MAX,
        target_building: usize::MAX, // no arrival check → no CCH calls
        current_node: shared.node_a,
        target_node: shared.node_b,
        current_edge: shared.edge_ab,
        current_lane_id: usize::MAX,
        lane_distance: progression,
        transit_mode: MODE_CAR,
        current_path: bounce_path,
        current_path_index: 1, // [0] = origin node, [1] = first target
        has_car: true,
        vehicle_type: VEHICLE_SEDAN,
        pedestrian_type: 0,
        speed: 20.0,
        walk_phase: 0.0,
    }
}

fn bench_agent_tick(c: &mut Criterion) {
    let shared = build_shared();

    let mut group = c.benchmark_group("AgentSystem::tick");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));

    // --- ON_ROAD: measures lane traversal and movement maths. ---
    // 200-entry bounce path, empty allocator → CCH never called, no arena bloat.
    let bounce_path: Vec<u32> = (0..200)
        .map(|i| if i % 2 == 0 { shared.node_a } else { shared.node_b })
        .collect();

    for &count in &[1_000usize, 10_000, 100_000, 1_000_000] {
        group.bench_with_input(
            criterion::BenchmarkId::new("on_road", count),
            &count,
            |b, &count| {
                let mut agents = AgentSystem::new();
                let mut graph = shared.graph.clone();
                let mut allocator = BuildingAllocator::new();
                let transit = &shared.transit;

                let seg_count = shared.graph.edges()[shared.edge_ab].physical_length;
                for i in 0..count {
                    // Spread agents across the first half of the edge.
                    let prog = i as f32 % (seg_count / 2.0).max(1.0);
                    agents.agents.push(make_on_road_agent(&shared, bounce_path.clone(), prog));
                }

                b.iter(|| {
                    agents.tick(
                        black_box(&mut allocator),
                        black_box(transit),
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
                let mut allocator = BuildingAllocator::new();
                let transit = &shared.transit;

                for _ in 0..count {
                    agents.agents.push(make_idle_agent(&shared));
                }

                b.iter(|| {
                    agents.tick(
                        black_box(&mut allocator),
                        black_box(transit),
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
