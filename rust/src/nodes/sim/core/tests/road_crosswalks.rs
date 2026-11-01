// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-74`: the per-arm crossing toggle on a city. What turning a zebra off removes and turning
//! it on restores, the preview being the commit, undo, overrides carried through a split and a
//! road removed and drawn again, and the overrides surviving a save file.

use super::*;

// A T junction: a through road from (-100, 0) to (100, 0) and a side road from its middle to
// (0, 100). Returns the core, the T node, the side road's edge and the west arm's edge.
fn t_junction() -> (SimCore, u32, usize, usize) {
    let mut core = test_core();
    core.treasury.balance = 1.0e7;
    for points in [
        vec![
            Vector3::new(-100.0, 0.0, 0.0),
            Vector3::new(100.0, 0.0, 0.0),
        ],
        vec![Vector3::ZERO, Vector3::new(0.0, 0.0, 100.0)],
    ] {
        assert!(core.add_road_internal(points, 1, 1).committed);
    }
    core.precompute_road_mesh_data();
    let node = (0..core.region_graph.node_count() as u32)
        .find(|&node| live_arms(&core, node).len() == 3)
        .expect("T junction");
    let side = arm_towards(&core, node, Vector3::new(0.0, 0.0, 100.0));
    let west = arm_towards(&core, node, Vector3::new(-100.0, 0.0, 0.0));
    (core, node, side, west)
}

fn live_arms(core: &SimCore, node: u32) -> Vec<usize> {
    let graph = &core.region_graph;
    if graph.get_valid_node(node) != node || node as usize >= graph.node_adjacency_count() {
        return Vec::new();
    }
    graph
        .node_adjacency(node)
        .iter()
        .copied()
        .filter(|&edge| !graph.edge(edge).deleted)
        .collect()
}

// The arm of `node` whose far end lies nearest `towards`.
fn arm_towards(core: &SimCore, node: u32, towards: Vector3) -> usize {
    let graph = &core.region_graph;
    let far = |edge: usize| {
        let edge = graph.edge(edge);
        let points = &edge.geometry;
        let point = if graph.get_valid_node(edge.start_node) == node {
            points[points.len() - 1]
        } else {
            points[0]
        };
        point.distance_to(towards)
    };
    live_arms(core, node)
        .into_iter()
        .min_by(|&a, &b| far(a).total_cmp(&far(b)))
        .unwrap()
}

// Everything a crossing decides, independent of lane ids (an incremental lane rebuild renumbers
// them): the road payload (paint, signs, lights, pavement), every edge's clips and profile, and
// each live lane with its crossing and its successors' first points.
fn products(core: &mut SimCore) -> String {
    core.precompute_road_mesh_data();
    assert!(core.transit_network.roadbed.failed_nodes().is_empty());
    let mesh = core
        .transit_network
        .try_generate_mesh_data(&core.region_graph, &core.heightmap)
        .expect("current roadbed");
    let mut out = format!(
        "{:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}\n",
        mesh.road_crossings,
        mesh.road_signs,
        mesh.street_lights,
        mesh.marking_vertices,
        mesh.road_vertices,
        mesh.sidewalk_vertices
    );
    for edge in core.region_graph.edges().iter().filter(|edge| !edge.deleted) {
        out += &format!(
            "{} {} {:?}\n",
            edge.start_clip.to_bits(),
            edge.end_clip.to_bits(),
            edge.physical_geometry
        );
    }
    let lanes = &core.transit_network.lane_system;
    let mut live: Vec<String> = lanes
        .edge_lanes
        .values()
        .chain(lanes.node_lanes.values())
        .flatten()
        .map(|&id| {
            let lane = &lanes.lanes[id];
            let mut next: Vec<String> = lane
                .next_lanes
                .iter()
                .map(|&next| format!("{:?}", lanes.lanes[next].geometry.first()))
                .collect();
            next.sort();
            format!(
                "{} {:?} {:?} {:?} {:?}",
                lane.edge_id,
                lane.lane_type,
                lane.geometry,
                lanes.crosswalk(id),
                next
            )
        })
        .collect();
    live.sort();
    out + &live.join("\n")
}

fn crossing_lanes(core: &SimCore, node: u32, edge: usize) -> usize {
    let lanes = &core.transit_network.lane_system;
    lanes.node_lanes.get(&(node as usize)).map_or(0, |ids| {
        ids.iter()
            .filter(|&&id| lanes.crosswalk(id).is_some_and(|c| c.edge_id == edge))
            .count()
    })
}

fn crossing_signs(core: &mut SimCore) -> usize {
    core.precompute_road_mesh_data();
    let mesh = core
        .transit_network
        .try_generate_mesh_data(&core.region_graph, &core.heightmap)
        .expect("current roadbed");
    format!("{:?}", mesh.road_signs).matches("Crossing").count()
}

#[test]
fn a_zebra_turned_off_removes_its_crossing_and_on_restores_it() {
    let (mut core, node, side, _) = t_junction();
    let before = products(&mut core);
    let signs_before = crossing_signs(&mut core);
    assert!(core.can_toggle_crosswalk_internal(node, side as i32));
    assert!(core.has_crosswalk_internal(node, side as i32));
    assert_eq!(crossing_lanes(&core, node, side), 2, "both walking directions");
    let undo_depth = core.undo_stack.len();

    // The preview is the commit's check and changes nothing.
    assert!(
        core.preview_crosswalk_override_internal(node, side as i32, false)
            .is_none()
    );
    assert_eq!(products(&mut core), before, "a preview leaves the city as it was");
    assert_eq!(core.undo_stack.len(), undo_depth);

    assert!(core.set_crosswalk_override_internal(node, side as i32, false));
    let off = products(&mut core);
    assert!(!core.has_crosswalk_internal(node, side as i32));
    assert_eq!(crossing_lanes(&core, node, side), 0, "no crossing lanes");
    assert!(
        crossing_signs(&mut core) < signs_before,
        "the crossing's own signs go with it"
    );
    assert_ne!(off, before);
    // The other arms keep theirs.
    for arm in live_arms(&core, node).into_iter().filter(|&arm| arm != side) {
        assert!(core.has_crosswalk_internal(node, arm as i32));
    }

    assert!(core.set_crosswalk_override_internal(node, side as i32, true));
    assert_eq!(products(&mut core), before, "on restores every product");
    assert_eq!(crossing_signs(&mut core), signs_before);

    // Undo steps back through both toggles to the city without an override.
    assert!(core.undo_action_internal());
    assert_eq!(products(&mut core), off);
    assert!(core.undo_action_internal());
    assert_eq!(products(&mut core), before);
    assert!(core.region_graph.node(node).crosswalk_overrides.is_empty());
    assert_eq!(core.undo_stack.len(), undo_depth);
}

#[test]
fn an_arm_that_cannot_cross_is_refused_by_preview_and_commit_alike() {
    let (mut core, _, side, _) = t_junction();
    // The side road's far end is a dead end: its sidewalk wraps the cap, nothing crosses.
    let graph = &core.region_graph;
    let end = graph.get_valid_node(graph.edge(side).end_node);
    let end = if live_arms(&core, end).len() == 1 {
        end
    } else {
        graph.get_valid_node(graph.edge(side).start_node)
    };
    let before = products(&mut core);
    let undo_depth = core.undo_stack.len();
    assert!(!core.can_toggle_crosswalk_internal(end, side as i32));
    let refusal = core.preview_crosswalk_override_internal(end, side as i32, true);
    assert_eq!(
        refusal.map(|refusal| refusal.reason).as_deref(),
        Some("crosswalk_arm_not_eligible")
    );
    assert!(!core.set_crosswalk_override_internal(end, side as i32, true));
    assert!(!core.set_crosswalk_override_internal(end, -1, true));
    assert!(!core.set_crosswalk_override_internal(u32::MAX, side as i32, true));
    assert_eq!(products(&mut core), before);
    assert_eq!(core.undo_stack.len(), undo_depth);
    assert!(core.region_graph.node(end).crosswalk_overrides.is_empty());
}

#[test]
fn a_split_through_an_authored_arm_keeps_its_crossing_off() {
    let (mut core, node, side, west) = t_junction();
    assert!(core.set_crosswalk_override_internal(node, west as i32, false));
    let before = products(&mut core);
    let overrides = core.region_graph.node(node).crosswalk_overrides.clone();
    // A game commit, with its undo record.
    core.benchmark_mode = false;
    let undo_depth = core.undo_stack.len();
    // A road across the west arm splits it; the half at the T is a new edge whose start is the
    // new junction, so the T's override moves onto it.
    assert!(
        core.add_road_internal(
            vec![
                Vector3::new(-50.0, 0.0, -60.0),
                Vector3::new(-50.0, 0.0, 60.0)
            ],
            1,
            1
        )
        .committed
    );
    core.precompute_road_mesh_data();
    let arm = arm_towards(&core, node, Vector3::new(-100.0, 0.0, 0.0));
    assert_ne!(arm, west, "the T's west arm is the split's new half");
    assert_eq!(
        core.region_graph.node(node).crosswalk_overrides.get(&arm),
        Some(&false)
    );
    assert!(!core.has_crosswalk_internal(node, arm as i32));
    assert!(core.has_crosswalk_internal(node, side as i32));
    // Every override names an arm of its node; the new junction crosses by default.
    let arms = live_arms(&core, node);
    assert!(
        core.region_graph
            .node(node)
            .crosswalk_overrides
            .keys()
            .all(|edge| arms.contains(edge))
    );
    let new = (0..core.region_graph.node_count() as u32)
        .find(|&n| {
            live_arms(&core, n).len() == 4
                && core.region_graph.node(n).pos.distance_to(Vector3::new(-50.0, 0.0, 0.0)) < 1.0
        })
        .expect("the new junction");
    assert!(core.region_graph.node(new).crosswalk_overrides.is_empty());
    for arm in live_arms(&core, new) {
        assert!(core.has_crosswalk_internal(new, arm as i32), "arm {arm}");
    }
    // Undoing the road puts the override back on the original arm.
    assert_eq!(core.undo_stack.len(), undo_depth + 1);
    assert!(core.undo_action_internal());
    assert_eq!(core.region_graph.node(node).crosswalk_overrides, overrides);
    assert_eq!(products(&mut core), before);
}

#[test]
fn a_road_removed_and_drawn_again_gets_a_new_arm_with_a_zebra() {
    let (mut core, node, side, _) = t_junction();
    assert!(core.set_crosswalk_override_internal(node, side as i32, false));
    let off = products(&mut core);
    assert!(core.bulldoze_road_edge_for_test(side));
    core.precompute_road_mesh_data();
    // Undo brings the arm back with its crossing still off.
    assert!(core.undo_action_internal());
    assert_eq!(products(&mut core), off);
    assert!(!core.has_crosswalk_internal(node, side as i32));
    // Removed and drawn again, it is a new arm: it crosses by default (R6-a).
    assert!(core.bulldoze_road_edge_for_test(side));
    assert!(
        core.add_road_internal(vec![Vector3::ZERO, Vector3::new(0.0, 0.0, 100.0)], 1, 1)
            .committed
    );
    core.precompute_road_mesh_data();
    let node = core.region_graph.get_valid_node(node);
    let again = arm_towards(&core, node, Vector3::new(0.0, 0.0, 100.0));
    assert_ne!(again, side);
    assert!(core.has_crosswalk_internal(node, again as i32));
}

#[test]
fn crosswalk_overrides_survive_a_save_file() {
    let (mut core, node, side, west) = t_junction();
    assert!(core.set_crosswalk_override_internal(node, side as i32, false));
    assert!(core.set_crosswalk_override_internal(node, west as i32, true));
    let arms = live_arms(&core, node);
    let crossings = |core: &SimCore| -> Vec<bool> {
        arms.iter()
            .map(|&arm| core.has_crosswalk_internal(node, arm as i32))
            .collect()
    };
    let saved = crossings(&core);
    let path = temp_save_path("road_crosswalks");
    core.save_game_settled_at(path.to_str().unwrap(), None, 0)
        .unwrap();
    let rows: Vec<(i64, i64, i64)> = {
        let connection = rusqlite::Connection::open(&path).unwrap();
        let mut statement = connection
            .prepare("SELECT node_id, edge_id, marked FROM road_crosswalks ORDER BY node_id, edge_id")
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(rows.len(), 2, "{rows:?}");
    let mut loaded = test_core();
    loaded.load_game_internal(path.to_str().unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    loaded.precompute_road_mesh_data();
    // This city saves every node and edge, so the ids are the live ones.
    assert_eq!(
        loaded.region_graph.node(node).crosswalk_overrides,
        core.region_graph.node(node).crosswalk_overrides
    );
    assert_eq!(crossings(&loaded), saved);
    assert_eq!(saved.iter().filter(|&&on| on).count(), 2);
}

// The renderer's path (the user's play-test of round R6): a toggle marks a new render generation,
// and once its pieces publish (`finalize_network_render_products`, which the bridge runs after a
// toggle and the simulation thread after an undo) the emitted road chunks equal a full emission
// of every chunk, bitwise; so does the undo's. Before the fix the toggle marked no generation and
// the drawn zebra stayed.
#[test]
fn a_toggle_and_its_undo_publish_the_chunks_a_full_emission_draws() {
    let (mut core, node, side, _) = t_junction();
    core.benchmark_mode = false;
    core.finalize_network_render_products();
    // Every emitted field, less `current_owner`, the emitter's cursor, which a chunk keeps from
    // its last write.
    let chunks = |core: &SimCore| {
        let text = format!("{:?}", core.cached_road_mesh_chunks);
        let mut kept = String::with_capacity(text.len());
        let mut rest = text.as_str();
        while let Some(at) = rest.find("current_owner: ") {
            kept.push_str(&rest[..at]);
            rest = &rest[at..];
            rest = &rest[rest.find(", crosswalks: ").expect("field order")..];
        }
        kept + rest
    };
    let full = |core: &mut SimCore| {
        core.mark_network_render_dirty();
        core.precompute_road_mesh_data();
        chunks(core)
    };
    let drawn = chunks(&core);
    assert_eq!(full(&mut core), drawn);

    let generation = core.road_tool_surface_generation;
    assert!(core.set_crosswalk_override_internal(node, side as i32, false));
    assert!(core.network_dirty, "the toggle tells the renderer");
    assert_ne!(core.road_tool_surface_generation, generation);
    core.finalize_network_render_products();
    assert!(core.transit_network.roadbed.is_current());
    let off = chunks(&core);
    assert_ne!(off, drawn, "the zebra's chunks are emitted again");
    assert_eq!(full(&mut core), off);

    let generation = core.road_tool_surface_generation;
    assert!(core.undo_action_internal());
    assert_ne!(core.road_tool_surface_generation, generation);
    core.finalize_network_render_products();
    let undone = chunks(&core);
    assert_eq!(undone, drawn);
    assert_eq!(full(&mut core), undone);
}
