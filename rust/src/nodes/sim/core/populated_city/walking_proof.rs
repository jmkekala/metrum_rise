// SPDX-License-Identifier: GPL-2.0-only

//! The walking planner's whole-network proof on the benchmark street grid (`ROAD-68-C`, D8):
//! every node lets every arriving sidewalk reach every leaving one, its four corner bends
//! included, so walk forecasts quote per node context.

use super::street_grid;
use crate::simulation::economy::agents::tick::TripQueryScratch;
use crate::simulation::network::types::TransitType;

#[test]
fn street_grid_walking_proof_holds_at_its_corner_bends() {
    let (core, _, _) = street_grid(400.0);
    let graph = &core.region_graph;
    // Its only degree-two nodes are the four corner bends of the 400 m grid.
    let corners: Vec<_> = (0..graph.node_count() as u32)
        .filter(|&node| {
            graph.get_valid_node(node) == node
                && graph
                    .node_adjacency(node)
                    .iter()
                    .filter(|&&edge| !graph.edge(edge).deleted)
                    .count()
                    == 2
        })
        .map(|node| graph.node(node).pos)
        .collect();
    assert_eq!(corners.len(), 4, "{corners:?}");
    assert!(
        corners
            .iter()
            .all(|pos| (pos.x.abs() - 200.0).abs() < 1e-3 && (pos.z.abs() - 200.0).abs() < 1e-3),
        "{corners:?}"
    );
    let mut scratch = TripQueryScratch::default();
    scratch.prepare(&core.transit_network, graph);
    assert!(scratch.walking_proof_complete_for_test());
}

// `ROAD-68` open question 14: the proof on the economy performance-contract cities, from their
// prepared snapshots (`METRUM_PERF_RESIDENTS`, `METRUM_PERF_CACHE`; `./run.sh --perf-contract
// --prepare` writes them). Their street grids hold no footpath, so footpath junctions cannot
// change it.
#[test]
#[ignore = "needs the prepared performance-contract snapshots"]
fn perf_contract_city_walking_proof_holds() {
    let residents = super::contract::residents();
    let city = super::snapshot::load_snapshot(residents, &super::contract::cache_dir());
    let graph = &city.core.region_graph;
    let footpaths = graph
        .edges()
        .iter()
        .filter(|edge| !edge.deleted && edge.primary_type == TransitType::Foot)
        .count();
    let mut scratch = TripQueryScratch::default();
    scratch.prepare(&city.core.transit_network, graph);
    let complete = scratch.walking_proof_complete_for_test();
    eprintln!("WALKING_PROOF {residents} residents: complete {complete}, footpaths {footpaths}");
    assert!(complete);
}
