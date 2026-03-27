use criterion::{Criterion, black_box, criterion_group, criterion_main};
use godot::prelude::*;
use metrum_rise::simulation::buildings::allocator::BuildingAllocator;
use metrum_rise::simulation::economy::agents::AgentSystem;
use metrum_rise::simulation::network::graph::{Edge, RegionGraph};
use metrum_rise::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
use metrum_rise::simulation::pathing::cch::CchGraph;
use std::time::Duration;

struct SharedSetup {
    graph: RegionGraph,
    cch: CchGraph,
    node_a: u32,
    node_b: u32,
    edge_ab: usize,
}

fn build_shared() -> SharedSetup {
    let mut graph = RegionGraph::new();

    // Minimal 2-node, 1-edge graph.
    // No TransitNetwork, no ZoningSystem, no Voronoi — zero expensive setup.
    let node_a = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let node_b = graph.add_node(Vector3::new(500.0, 0.0, 0.0), NodeType::Junction);

    // 500 m edge with geometry every 10 m (50 segments).
    // At 20 m/s and delta=0.016 s, one segment takes ~3 ticks to traverse.
    // Total traversal: 50 segments × 3 ticks = 150 ticks per one-way pass.
    let geometry: Vec<Vector3> = (0..=50)
        .map(|i| Vector3::new(i as f32 * 10.0, 0.0, 0.0))
        .collect();
    let speed_ms = 50.0_f32 / 3.6; // 50 km/h → m/s
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
        zoning_left: false,
        zoning_right: false,
        class: EdgeClass::Standard,
        deleted: false,
    };
    let edge_ab = graph.add_edge(edge);
    graph.rebuild_adjacency_list();

    let cch = CchGraph::build(&graph);
    SharedSetup {
        graph,
        cch,
        node_a,
        node_b,
        edge_ab,
    }
}

fn push_idle_agent(agents: &mut AgentSystem, shared: &SharedSetup) {
    agents.home_building.push(0); // zeroed by safety scrub (empty allocator → 0 >= 0)
    agents.work_building.push(0);
    agents.pos_x.push(0.0);
    agents.pos_y.push(0.0);
    agents.is_visible.push(true);
    agents.activity.push(0);
    agents
        .transit
        .push(metrum_rise::simulation::economy::agents::TRANSIT_IDLE);
    agents.happiness.push(50.0);
    agents.money.push(100.0);
    agents.journey_start_time.push(0.0);
    agents.current_building.push(0);
    agents.target_building.push(0);
    agents.current_node.push(shared.node_a);
    agents.target_node.push(shared.node_b);
    agents.current_edge.push(usize::MAX);
    agents.edge_progression.push(0);
    agents.current_lane.push(0);
    agents
        .transit_mode
        .push(metrum_rise::simulation::economy::agents::MODE_CAR);
    agents.bezier_p0_x.push(0.0);
    agents.bezier_p0_y.push(0.0);
    agents.bezier_p1_x.push(0.0);
    agents.bezier_p1_y.push(0.0);
    agents.bezier_p2_x.push(0.0);
    agents.bezier_p2_y.push(0.0);
    agents.bezier_p3_x.push(0.0);
    agents.bezier_p3_y.push(0.0);
    agents.bezier_t.push(0.0);
    agents.current_path.push(Vec::new());
    agents.current_path_index.push(0);
    agents.has_car.push(true);
    agents.count += 1;
}

/// ON_ROAD agent with a pre-computed A↔B bounce path long enough to never be
/// exhausted during the benchmark window.
///
/// One A→B traversal = 150 ticks. Bounce path of 200 entries = 100 round trips
/// = 15 000 ticks. With warm_up_time=500 ms and 100k agents at ~0.5 ms/tick,
/// warm-up runs ~1 000 ticks — well within the 15 000-tick budget.
///
/// `target_building = usize::MAX` so the arrival check is skipped every tick
/// and CCH is never called → zero heap allocation in the hot path.
fn push_on_road_agent(
    agents: &mut AgentSystem,
    shared: &SharedSetup,
    bounce_path: &[u32],
    progression: isize,
) {
    agents.home_building.push(usize::MAX);
    agents.work_building.push(usize::MAX);
    agents.pos_x.push(0.0);
    agents.pos_y.push(0.0);
    agents.is_visible.push(true);
    agents.activity.push(0);
    agents
        .transit
        .push(metrum_rise::simulation::economy::agents::TRANSIT_ON_ROAD);
    agents.happiness.push(50.0);
    agents.money.push(100.0);
    agents.journey_start_time.push(0.0);
    agents.current_building.push(usize::MAX);
    agents.target_building.push(usize::MAX); // no arrival check → no CCH calls
    agents.current_node.push(shared.node_a);
    agents.target_node.push(shared.node_b);
    agents.current_edge.push(shared.edge_ab);
    agents.edge_progression.push(progression);
    agents.current_lane.push(0);
    agents
        .transit_mode
        .push(metrum_rise::simulation::economy::agents::MODE_CAR);
    agents.bezier_p0_x.push(0.0);
    agents.bezier_p0_y.push(0.0);
    agents.bezier_p1_x.push(0.0);
    agents.bezier_p1_y.push(0.0);
    agents.bezier_p2_x.push(0.0);
    agents.bezier_p2_y.push(0.0);
    agents.bezier_p3_x.push(0.0);
    agents.bezier_p3_y.push(0.0);
    agents.bezier_t.push(0.0);
    agents.current_path.push(bounce_path.to_vec());
    agents.current_path_index.push(0);
    agents.has_car.push(true);
    agents.count += 1;
}

fn bench_agent_tick(c: &mut Criterion) {
    let shared = build_shared();

    let mut group = c.benchmark_group("AgentSystem::tick");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));

    // --- ON_ROAD: measures polyline traversal and lane-offset maths. ---
    // 200-entry bounce path, empty allocator → CCH never called, no arena bloat.
    // Memory: 200 × 4 bytes × 100k agents = 80 MB peak.
    let bounce_path: Vec<u32> = (0..200)
        .map(|i| {
            if i % 2 == 0 {
                shared.node_a
            } else {
                shared.node_b
            }
        })
        .collect();

    for &count in &[1_000usize, 10_000, 100_000] {
        group.bench_with_input(
            criterion::BenchmarkId::new("on_road", count),
            &count,
            |b, &count| {
                let mut agents = AgentSystem::new();
                let mut graph = shared.graph.clone();
                let mut allocator = BuildingAllocator::new();

                let seg_count = shared.graph.edges[shared.edge_ab].physical_geometry.len() as isize;
                for i in 0..count {
                    // Spread agents across the first half of the edge so they don't all
                    // hit the node boundary simultaneously.
                    let prog = (i as isize) % (seg_count / 2).max(1);
                    push_on_road_agent(&mut agents, &shared, &bounce_path, prog);
                }

                b.iter(|| {
                    agents.tick(
                        black_box(&mut allocator),
                        black_box(&shared.cch),
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

                for _ in 0..count {
                    push_idle_agent(&mut agents, &shared);
                }

                b.iter(|| {
                    agents.tick(
                        black_box(&mut allocator),
                        black_box(&shared.cch),
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
