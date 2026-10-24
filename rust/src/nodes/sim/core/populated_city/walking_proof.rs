// SPDX-License-Identifier: GPL-2.0-only

//! The walking planner's whole-network proof on the benchmark street grid (`ROAD-68-C`, D8):
//! every node lets every arriving sidewalk reach every leaving one, its four corner bends
//! included, so walk forecasts quote per node context.

use super::street_grid;
use crate::simulation::economy::agents::tick::TripQueryScratch;

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
