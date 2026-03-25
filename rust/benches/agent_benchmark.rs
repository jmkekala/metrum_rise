use criterion::{black_box, criterion_group, criterion_main, Criterion};
use metrum_rise::simulation::economy::agents::AgentSystem;
use metrum_rise::simulation::network::graph::RegionGraph;
use metrum_rise::simulation::pathing::cch::CchGraph;
use metrum_rise::simulation::buildings::allocator::BuildingAllocator;
use godot::prelude::Vector3;

fn setup_benchmark(agent_count: usize) -> (AgentSystem, RegionGraph, CchGraph, BuildingAllocator) {
    let mut graph = RegionGraph::new();
    let allocator = BuildingAllocator::new();
    let mut agents = AgentSystem::new();

    // Add at least one node so agents have a valid current_node
    let _n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), metrum_rise::simulation::network::types::NodeType::Junction);
    let cch = CchGraph::build(&graph);

    // Manually populate agents to avoid needing a complex building setup in this isolated bench
    for _ in 0..agent_count {
        agents.home_building.push(0);
        agents.work_building.push(0);
        agents.pos_x.push(0.0);
        agents.pos_y.push(0.0);
        agents.is_visible.push(true);
        agents.activity.push(0);
        agents.transit.push(0);
        agents.happiness.push(50.0);
        agents.money.push(100.0);
        agents.journey_start_time.push(0.0);
        agents.current_building.push(0);
        agents.target_building.push(0);
        agents.current_node.push(0);
        agents.target_node.push(0);
        agents.current_edge.push(usize::MAX);
        agents.edge_progression.push(0);
        agents.current_lane.push(0);
        agents.transit_mode.push(1); // MODE_CAR
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

    (agents, graph, cch, allocator)
}

fn bench_agent_tick(c: &mut Criterion) {
    let mut group = c.benchmark_group("AgentSystem::tick");

    for count in [1_000, 10_000, 100_000, 1_000_000].iter() {
        group.bench_with_input(criterion::BenchmarkId::from_parameter(count), count, |b, &count| {
            let (mut agents, mut graph, cch, mut allocator) = setup_benchmark(count);
            b.iter(|| {
                agents.tick(black_box(&mut allocator), black_box(&cch), black_box(&mut graph), black_box(0.016));
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_agent_tick);
criterion_main!(benches);
