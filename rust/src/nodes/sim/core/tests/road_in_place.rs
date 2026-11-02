// SPDX-License-Identifier: GPL-2.0-only

//! In-place edits of an existing road: the class change (`ROAD-97`: a spliced publication equals
//! a full emission; R3-k: its undo record).

use super::road_crosswalks::{fully_emitted_chunks, products, published_chunks};
use super::*;

// Four roads (round R8f's vegetation repro, on flat ground): a through road, a side road from
// it, a later road ending 10 m short of the side road, and a fourth from the through road. The
// side road is older than the later road's terminal, so a rebuild of the side road's pieces keeps
// a node piece of a higher id in their chunks. Returns the core and the side road's edge.
fn older_side_road() -> (SimCore, usize) {
    let mut core = test_core();
    core.treasury.balance = 1.0e7;
    for [a, b] in [
        [(-120.0, 10.0), (120.0, 10.0)],
        [(0.0, 10.0), (0.0, 130.0)],
        [(-120.0, 70.0), (-10.0, 70.0)],
        [(60.0, 10.0), (60.0, 80.0)],
    ] {
        let points = vec![Vector3::new(a.0, 0.0, a.1), Vector3::new(b.0, 0.0, b.1)];
        assert!(core.add_road_internal(points, 1, 1).committed);
    }
    core.precompute_road_mesh_data();
    let side = core.get_hovered_edge_internal(0.0, 100.0);
    assert!(side >= 0);
    (core, side as usize)
}

// `ROAD-97`: a grounded road made a bridge emits its abutment faces with its own piece, so the
// skirt layer's owner runs stay in piece order and the spliced chunks equal a full emission;
// before the fix the abutments followed every piece and the kept later terminal sorted ahead of
// them in the splice. R3-k: the class change has an undo record that restores the chunks.
#[test]
fn a_class_change_publishes_the_chunks_a_full_emission_draws_and_undoes() {
    let (mut core, side) = older_side_road();
    core.benchmark_mode = false;
    core.finalize_network_render_products();
    let drawn = published_chunks(&core);
    assert_eq!(fully_emitted_chunks(&mut core), drawn);
    let before = products(&mut core);
    let undo_depth = core.undo_stack.len();

    core.set_edge_class_internal(side as i32, 1);
    assert_eq!(core.region_graph.edge(side).class, EdgeClass::Bridge);
    core.precompute_road_mesh_data();
    let bridge = published_chunks(&core);
    assert_ne!(bridge, drawn, "the bridge's chunks are emitted again");
    assert_eq!(
        fully_emitted_chunks(&mut core),
        bridge,
        "the spliced bridge chunks equal a full emission"
    );

    assert_eq!(
        core.undo_stack.len(),
        undo_depth + 1,
        "the class change has an undo record"
    );
    assert!(core.undo_action_internal());
    assert_eq!(core.region_graph.edge(side).class, EdgeClass::Standard);
    core.finalize_network_render_products();
    assert_eq!(published_chunks(&core), drawn);
    assert_eq!(fully_emitted_chunks(&mut core), drawn);
    assert_eq!(products(&mut core), before);
}
