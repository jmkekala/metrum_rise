// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-80` divided spans: where a raised median continues through a pass-through node, where
//! it noses out, and the span meshes it builds.

use crate::simulation::network::graph::{Edge, RegionGraph, RoadMedian};
use crate::simulation::network::roadbed::arms::{
    edges_pass_through, median_continues, pass_through_crossings,
};
use crate::simulation::network::roadbed::fixtures::{divided, grass, straight_pair};
use crate::simulation::network::roadbed::layout::EdgeEnd;
use crate::simulation::network::roadbed::span::incident_direction;

// Whether the pair's two edges pass through at the centre node 0 by the one pass-through rule.
fn pass_through(graph: &RegionGraph) -> bool {
    let arm = |edge: usize| {
        let edge = graph.edge(edge);
        let end = if edge.start_node == 0 {
            EdgeEnd::Start
        } else {
            EdgeEnd::End
        };
        (edge, incident_direction(edge, end).unwrap())
    };
    edges_pass_through(arm(0), arm(1))
}

// Whether node 0 crosses on either arm.
fn crosses(graph: &RegionGraph) -> bool {
    pass_through_crossings(graph, 0).is_some_and(|arms| arms.iter().any(|(_, crosses)| *crosses))
}

#[test]
fn a_raised_median_continues_only_where_both_arms_carry_it_in_line() {
    let paving = RoadMedian::from_tool(3, 2.0).unwrap();
    let painted = RoadMedian::from_tool(1, 2.0).unwrap();
    let with = |fwd, bkw, median: Option<RoadMedian>| {
        move |edge: Edge| divided(edge, fwd, bkw, median)
    };
    // (west, east, east reversed, continues). Lane groups line up looking along the road: the
    // reversed east edge must carry the mirrored counts.
    let cases = [
        ("grass both ways", with(2, 2, grass(4.0)), with(2, 2, grass(4.0)), false, true),
        ("reversed", with(2, 2, grass(4.0)), with(2, 2, grass(4.0)), true, true),
        ("uneven, in line", with(2, 1, grass(4.0)), with(2, 1, grass(4.0)), false, true),
        ("uneven, mirrored", with(2, 1, grass(4.0)), with(1, 2, grass(4.0)), true, true),
        ("uneven, offset", with(2, 1, grass(4.0)), with(2, 1, grass(4.0)), true, false),
        ("grass and paving", with(2, 2, grass(2.0)), with(2, 2, paving), false, false),
        ("painted", with(2, 2, painted), with(2, 2, painted), false, false),
    ];
    for (name, west, east, reversed, continues) in cases {
        let graph = straight_pair(west, east, reversed, false);
        assert!(pass_through(&graph), "{name}: one mouth profile passes through");
        assert_eq!(median_continues(&graph, 0), continues, "{name}");
        // Z3: no default crossing where the median continues; elsewhere the D8 default.
        assert_eq!(crosses(&graph), !continues, "{name}: crossing");
    }

    // A divided road and an undivided one of its width pass through (one mouth profile); the
    // median noses out at the node. Of another width they are laid out.
    let wide = |edge: Edge| {
        let mut edge = divided(edge, 2, 2, None);
        edge.width = Edge::cross_section_width_m(2, 2, grass(4.0));
        edge
    };
    let graph = straight_pair(with(2, 2, grass(4.0)), wide, false, false);
    assert!(pass_through(&graph) && !median_continues(&graph, 0) && crosses(&graph));
    let graph = straight_pair(with(2, 2, grass(4.0)), with(2, 2, None), false, false);
    assert!(!pass_through(&graph));

    // An explicit crossing opens a continuing median; turning it off again closes it.
    let mut graph = straight_pair(with(2, 2, grass(4.0)), with(2, 2, grass(4.0)), false, false);
    graph.set_crosswalk_override(0, 1, true);
    assert!(crosses(&graph) && !median_continues(&graph, 0));
    graph.set_crosswalk_override(0, 1, false);
    assert!(!crosses(&graph) && median_continues(&graph, 0));
}
