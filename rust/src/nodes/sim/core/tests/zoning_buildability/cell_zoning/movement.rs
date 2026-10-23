// SPDX-License-Identifier: GPL-2.0-only

//! Node movement must preserve cell reservations before publishing road or dependent state.

use super::*;
use crate::simulation::network::roadbed::{PieceId, visual_roadbed_half_width_m};

#[test]
fn unrelated_cell_paint_does_not_block_a_clear_bridge_terminal_move() {
    let mut core = fixture(0.0, false, false);
    let added = core.add_road_internal(
        vec![
            Vector3::new(-96.0, 12.0, 160.0),
            Vector3::new(96.0, 12.0, 160.0),
        ],
        1,
        1,
    );
    assert!(added.committed);
    core.precompute_road_mesh_data();
    let edge_idx = core.region_graph.edge_count() - 1;
    let edge = core.region_graph.edge(edge_idx);
    assert_eq!(
        edge.class,
        crate::simulation::network::types::EdgeClass::Bridge
    );
    let node = edge.end_node;
    let original = core.region_graph.node(node).pos;
    let paint = core.zoning.cells.saved_cells();
    assert!(core.zoning.cells.has_reservations());
    assert!(
        !core
            .transit_network
            .roadbed
            .pieces_in_bounds([f64::MIN, f64::MIN, f64::MAX, f64::MAX])
            .any(|(piece, _)| piece == PieceId::Node(node)),
        "the bridge span owns its terminal rather than requiring a separate cap"
    );
    let span_end = core
        .transit_network
        .roadbed
        .sections(edge_idx)
        .unwrap()
        .last()
        .unwrap();
    assert!(
        (span_end.center_xz - glam::DVec2::new(f64::from(original.x), f64::from(original.z)))
            .length()
            < 1e-3,
        "the bridge span reaches its terminal node"
    );
    let target = Vector3::new(120.0, 12.0, 160.0);
    assert!(core.move_network_node_internal(node as i32, target));
    assert_eq!(core.region_graph.node(node).pos, target);
    assert_eq!(core.zoning.cells.saved_cells(), paint);
    assert!(core.undo_action_internal());
    assert_eq!(core.region_graph.node(node).pos, original);
    assert_eq!(core.zoning.cells.saved_cells(), paint);
}

#[test]
#[ignore = "ROAD-68-F: node-move reservation check against moved pieces"]
fn node_move_rejects_terminal_cap_overlap_outside_the_straight_corridor() {
    let mut core = test_core();
    for endpoints in [
        [Vector3::new(-96.0, 0.0, 0.0), Vector3::new(96.0, 0.0, 0.0)],
        [
            Vector3::new(120.0, 0.0, 0.0),
            Vector3::new(120.0, 0.0, 160.0),
        ],
    ] {
        road_terrain_plan::commit_ready(&mut core, endpoints.to_vec());
    }
    let preview =
        core.preview_cell_selection_internal(CellSelectionShape::Cell, &[DVec2::new(110.0, 2.0)]);
    assert_eq!(preview.selection.cells.len(), 1);
    assert!(core.apply_cell_selection_internal(&preview, 1));
    let node = core.region_graph.edge(0).end_node;
    // End curb/sidewalk bands project 1.5 m beyond the carriageway endpoint.
    let target = Vector3::new(104.5, 0.0, 0.0);
    let mut candidate = core.region_graph.clone();
    candidate.move_node(node, target);
    assert!(
        !core.zoning.cells_overlap_road_corridor(
            &candidate.edge(0).physical_geometry,
            visual_roadbed_half_width_m(candidate.edge(0)),
        ),
        "the straight corridor alone does not reach the reserved cell"
    );
    let dependencies = core.cell_tool_dependencies();
    let undo_len = core.undo_stack.len();
    assert!(!core.move_network_node_internal(node as i32, target));
    assert_eq!(
        core.region_graph.node(node).pos,
        Vector3::new(96.0, 0.0, 0.0)
    );
    assert_eq!(core.cell_tool_dependencies(), dependencies);
    assert_eq!(core.undo_stack.len(), undo_len);
}

fn fixture(angle: f64, occupied: bool, erased: bool) -> SimCore {
    let mut core = painted_fixture(angle);
    core.benchmark_mode = false;
    if occupied {
        core.prepare_cell_lots_internal();
        let candidates = core.allocator.collect_demand_spawn_candidates_by_use(
            &core.zoning,
            &core.region_graph,
            core.demand.runtime_catalog(),
            &[],
            environment(&core),
        );
        // All candidates are on the painted side, where moving the end toward the site
        // must be refused even when only the building's erased claim remains.
        let action = &candidates.residential[candidates.residential.len() - 1].action;
        core.allocator
            .execute_demand_spawn_action(
                action,
                &mut core.zoning,
                &core.region_graph,
                &core.transit_network.roadbed,
                &core.heightmap,
                core.demand.runtime_catalog(),
                core.demand.runtime_tuning(),
            )
            .unwrap();
    }
    if erased {
        let preview = core.preview_cell_selection_internal(
            CellSelectionShape::Brush { radius_m: 300.0 },
            &[DVec2::ZERO],
        );
        assert!(core.apply_cell_selection_internal(&preview, 0));
        assert!(!core.zoning.cells.has_paint());
    }
    core
}

#[test]
#[ignore = "ROAD-68-F: node-move reservation check against moved pieces"]
fn node_move_into_paint_or_erased_occupied_claim_is_atomic() {
    for angle in [0.0_f64, 0.35] {
        for (occupied, erased) in [(false, false), (true, false), (true, true)] {
            let mut core = fixture(angle, occupied, erased);
            let edge = core.region_graph.edge(0).clone();
            let node = edge.end_node;
            let position = core.region_graph.node(node).pos;
            let paint = core.zoning.cells.saved_cells();
            let choices = core.zoning.cells.saved_road_alignments();
            let dependencies = core.cell_tool_dependencies();
            let undo_len = core.undo_stack.len();
            let v = DVec2::new(-angle.sin(), angle.cos());
            let target = position + Vector3::new((v.x * 20.0) as f32, 0.0, (v.y * 20.0) as f32);
            let mut candidate = core.region_graph.clone();
            candidate.move_node(node, target);
            assert!(
                core.zoning.cells_overlap_road_corridor(
                    &candidate.edge(0).physical_geometry,
                    visual_roadbed_half_width_m(candidate.edge(0)),
                ),
                "the final road must actually intersect a reserved cell"
            );
            assert!(!core.move_network_node_internal(node as i32, target));
            assert_eq!(
                core.region_graph.node(node).pos,
                position,
                "angle={angle}, occupied={occupied}, erased={erased}"
            );
            assert_eq!(core.region_graph.edge(0).geometry, edge.geometry);
            assert_eq!(
                core.region_graph.edge(0).physical_geometry,
                edge.physical_geometry
            );
            assert_eq!(core.zoning.cells.saved_cells(), paint);
            assert_eq!(core.zoning.cells.saved_road_alignments(), choices);
            assert_eq!(core.cell_tool_dependencies(), dependencies);
            assert_eq!(core.undo_stack.len(), undo_len);
            assert_eq!(core.allocator.buildings.len(), usize::from(occupied));
        }
    }
}

#[test]
fn clear_node_extension_preserves_occupied_cells_and_undo() {
    for angle in [0.0_f64, 0.35] {
        for erased in [false, true] {
            let mut core = fixture(angle, true, erased);
            let before = core.allocator.buildings[0].clone();
            let lot = core
                .zoning
                .parcel_by_raw_id(before.parcel_id)
                .unwrap()
                .clone();
            let edge = core.region_graph.edge(before.edge_idx).clone();
            let node = edge.end_node;
            let position = core.region_graph.node(node).pos;
            let u = DVec2::new(angle.cos(), angle.sin());
            let target = position + Vector3::new((u.x * 24.0) as f32, 0.0, (u.y * 24.0) as f32);
            let paint = core.zoning.cells.saved_cells();
            let choices = core.zoning.cells.saved_road_alignments();
            assert!(core.move_network_node_internal(node as i32, target));
            assert_eq!(core.region_graph.node(node).pos, target);
            for undo in [false, true] {
                if undo {
                    assert!(core.undo_action_internal());
                    core.rebuild_network_surface_terrain_internal();
                }
                core.preview_cell_selection_internal(
                    CellSelectionShape::Brush { radius_m: 300.0 },
                    &[DVec2::ZERO],
                );
                core.prepare_cell_lots_internal();
                let building = &core.allocator.buildings[0];
                let current = core.zoning.parcel_by_raw_id(before.parcel_id).unwrap();
                assert_eq!(
                    (building.center_x, building.center_y, building.facing_dir),
                    (before.center_x, before.center_y, before.facing_dir)
                );
                assert_eq!(current.corners(), lot.corners());
                assert_eq!(current.cell_lot(), lot.cell_lot());
                assert_eq!(current.occupied_building(), Some(0));
                assert_eq!(building.edge_idx, current.edge_idx());
                assert_eq!(building.frontage_t, current.frontage_center_t());
                assert_eq!(core.allocator.entrances[0].edge_idx, building.edge_idx);
                assert_eq!(core.zoning.cells.saved_cells(), paint);
                for key in current.cell_lot().unwrap().cells() {
                    assert_eq!(core.zoning.cells.lot(key), Some(before.parcel_id));
                }
                if undo {
                    assert_eq!(core.region_graph.node(node).pos, position);
                    assert_eq!(
                        core.region_graph.edge(before.edge_idx).geometry,
                        edge.geometry
                    );
                    assert_eq!(core.zoning.cells.saved_road_alignments(), choices);
                }
            }
        }
    }
}
