// SPDX-License-Identifier: GPL-3.0-or-later

//! Node, edge and lane ids as saved state on the populated street grid (`SAVE-11`), where equal
//! cost routes exist and ties break by id: a grid saved after road edits continues across a load
//! as the running game.

use super::PopulatedCity;
use crate::nodes::sim::core::scenario::save_cases::{run_alike, save_and_load};
use crate::simulation::network::types::{TransitFlags, TransitType};

// The 2k grid after a commit across a trunk (a split), the bulldozing of the
// lowest live road, and an in-place class change and its undo: the loaded network equals the
// running one in memory (`save_and_load`), and both games then run a quarter game hour with
// every agent alike each frame and byte-equal saves every 300 frames.
#[test]
fn a_grid_continues_across_a_save_after_road_edits_as_the_running_game() {
    let mut city = PopulatedCity::build(2_000);
    let core = &mut city.core;
    let graph = &core.region_graph;
    // The long street furthest out: a trunk to the world's edge, which no building fronts.
    let mid_of = |edge: usize| {
        let e = graph.edge(edge);
        (graph.node(e.start_node).pos + graph.node(e.end_node).pos) * 0.5
    };
    let street = (0..graph.edge_count())
        .filter(|&edge| {
            let e = graph.edge(edge);
            !e.deleted
                && e.primary_type == TransitType::Road
                && e.allowed_types & TransitFlags::CAR != 0
                && e.physical_length > 80.0
        })
        .max_by(|&a, &b| {
            let (a, b) = (mid_of(a), mid_of(b));
            (a.x.abs() + a.z.abs()).total_cmp(&(b.x.abs() + b.z.abs()))
        })
        .expect("a long street");
    let (a, b) = {
        let e = graph.edge(street);
        (graph.node(e.start_node).pos, graph.node(e.end_node).pos)
    };
    let end = graph.edge(street).end_node;
    let mid = (a + b) * 0.5;
    let along = (b - a).normalized();
    let across = (along.z * 30.0, -along.x * 30.0);
    crate::nodes::sim::core::scenario::commit_road(
        core,
        (mid.x - across.0, mid.z - across.1),
        (mid.x + across.0, mid.z + across.1),
    );
    assert!(
        core.region_graph.edge(street).end_node != end,
        "the commit split the street"
    );
    let lowest = (0..core.region_graph.edge_count())
        .find(|&edge| !core.region_graph.edge(edge).deleted)
        .unwrap();
    assert!(core.bulldoze_road_edge_for_test(lowest));
    let side = (0..core.region_graph.edge_count())
        .rev()
        .find(|&edge| !core.region_graph.edge(edge).deleted)
        .unwrap();
    assert!(core.set_edge_class_internal(side as i32, 1));
    assert!(core.undo_action_internal());

    let path =
        std::env::temp_dir().join(format!("metrum-save11-grid-{}.sqlite", std::process::id()));
    let mut loaded = save_and_load(core, &path);
    std::fs::remove_file(path).unwrap();
    run_alike(core, &mut loaded, 1_800, "grid");
}
