// SPDX-License-Identifier: GPL-2.0-only

//! The load's node check on the economy performance contract's cities (`ROAD-68-E`).

// The load's node check (`RoadbedSystem::unpinned_nodes`, `ROAD-68-E`) on the economy
// performance contract's 10k and 30k cities, against the load's own piece compile
// (`rebuild_roadbed`) in the same process, alternated five times; ms medians. The city build
// is not timed.
#[test]
#[ignore = "release timing: METRUM_LOAD_CHECK_TIERS=10000,30000 $TEST_BINARY benchmark_load_node_check --exact --ignored --nocapture"]
fn benchmark_load_node_check() {
    use std::time::Instant;
    let tiers: Vec<u32> = std::env::var("METRUM_LOAD_CHECK_TIERS").map_or_else(
        |_| vec![10_000, 30_000],
        |value| {
            value
                .split(',')
                .map(|tier| tier.trim().parse().unwrap())
                .collect()
        },
    );
    let median = |values: &mut Vec<f64>| {
        values.sort_by(f64::total_cmp);
        values[values.len() / 2]
    };
    for residents in tiers {
        let mut core = super::PopulatedCity::build(residents).core;
        let (mut compile, mut check) = (Vec::new(), Vec::new());
        let mut unpinned = 0;
        for _ in 0..5 {
            let begin = Instant::now();
            core.transit_network.rebuild_roadbed(&core.region_graph);
            compile.push(begin.elapsed().as_secs_f64() * 1e3);
            let begin = Instant::now();
            let nodes = core
                .transit_network
                .roadbed
                .unpinned_nodes(&core.region_graph);
            check.push(begin.elapsed().as_secs_f64() * 1e3);
            unpinned = std::hint::black_box(nodes).len();
        }
        eprintln!(
            "LOAD_NODE_CHECK {}",
            serde_json::json!({
                "residents": residents,
                "workers": crate::thread_pools::install(rayon::current_num_threads),
                "nodes": core.region_graph.node_count(),
                "edges": core.region_graph.edge_count(),
                "unpinned": unpinned,
                "rebuild_roadbed_ms": median(&mut compile),
                "unpinned_nodes_ms": median(&mut check),
            })
        );
    }
}
