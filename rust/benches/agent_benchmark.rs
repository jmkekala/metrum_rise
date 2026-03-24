use criterion::{black_box, criterion_group, criterion_main, Criterion};
use metrum_rise::simulation::economy::agents::AgentSystem;
use metrum_rise::simulation::network::graph::TransitGraph;
use metrum_rise::simulation::pathing::hpa::HpaGraph;
use metrum_rise::simulation::buildings::allocator::BuildingAllocator;
use godot::prelude::Vector3;

fn setup_benchmark(_agent_count: usize) -> (AgentSystem, TransitGraph, HpaGraph, BuildingAllocator) {
    let mut graph = TransitGraph::new();
    let hpa = HpaGraph::new(); // Empty for now, simplified
    let allocator = BuildingAllocator::new(100, 100);
    let agents = AgentSystem::new();

    // Add at least one node and building so agents can spawn
    let _n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), metrum_rise::simulation::network::types::NodeType::Junction);
    // (Manual building creation for benchmark if needed, but let's assume we just need the system to not crash)
    
    // For a pure tick benchmark, we can just manually push agents into the system if we want to bypass spawning logic
    // but using spawn_random_agents is more realistic.
    // However, allocator needs buildings for spawn_random_agents.
    
    (agents, graph, hpa, allocator)
}

fn bench_agent_tick(c: &mut Criterion) {
    let mut group = c.benchmark_group("AgentSystem::tick");
    
    for count in [1_000, 10_000, 100_000, 1_000_000].iter() {
        group.bench_with_input(criterion::BenchmarkId::from_parameter(count), count, |b, &count| {
            let (mut agents, mut graph, hpa, allocator) = setup_benchmark(count);
            // Manually populate agents to avoid needing a complex building setup in this isolated bench
            for _ in 0..count {
                agents.pos_x.push(0.0);
                agents.pos_y.push(0.0);
                agents.is_visible.push(true);
                agents.activity.push(0);
                agents.transit.push(0); 
                agents.happiness.push(50.0);
                agents.money.push(100.0);
                agents.current_building.push(0);
                agents.target_building.push(0);
                agents.current_node.push(0);
                agents.target_node.push(0);
                agents.current_edge.push(usize::MAX);
                agents.edge_progression.push(0);
                agents.current_lane.push(0);
                agents.is_driving.push(true);
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
                agents.parked_edge.push(usize::MAX);
                agents.parked_progression.push(0);
                agents.home_building.push(0);
                agents.work_building.push(0);
                agents.pathfind_count = 0;
            }
            agents.count = count;

            b.iter(|| {
                agents.tick(black_box(&allocator), black_box(&hpa), black_box(&mut graph), black_box(0.016));
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_agent_tick);
criterion_main!(benches);
