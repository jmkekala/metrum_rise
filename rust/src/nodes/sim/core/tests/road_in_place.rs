// SPDX-License-Identifier: GPL-2.0-only

//! In-place edits of an existing road: the class change (`ROAD-97`: a spliced publication equals
//! a full emission; R3-k: its undo record) and the lane-count edit (`ROAD-80`): what it builds,
//! the cars it moves, what it refuses, its undo and its publication.

use super::road_crosswalks::{
    arm_towards, fully_emitted_chunks, products, published_chunks, t_junction_with_side,
};
use super::*;
use crate::simulation::network::lanes::LaneType;

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

// The lane edit's verdict and its commit: `""` when applied, else the refusal's reason. The
// preview must give the same verdict, and a refusal must change nothing.
fn edit_lanes(core: &mut SimCore, edge: usize, fwd: u8, bkw: u8) -> String {
    let before = (products(core), core.undo_stack.len());
    let preview = core
        .preview_edge_cross_section_internal(edge as i32, fwd, bkw, None)
        .map(|refusal| refusal.reason)
        .unwrap_or_default();
    assert_eq!(products(core), before.0, "a preview changes nothing");
    let applied = core.set_edge_cross_section_internal(edge as i32, fwd, bkw, None);
    assert_eq!(
        applied,
        preview.is_empty(),
        "the preview is the commit: {preview}"
    );
    if !applied {
        assert_eq!((products(core), core.undo_stack.len()), before, "{preview}");
    }
    preview
}

// `ROAD-80` (1), (4): widening the T's side road from 1+1 to 2+2 in place draws, bitwise, what
// drawing it 2+2 from the start draws (pieces, paint, lanes, clips), publishes the chunks a full
// emission draws, and undo restores the whole road and its chunks.
#[test]
fn a_road_widened_in_place_is_the_road_drawn_wide() {
    let (mut core, _, side, _) = t_junction_with_side(test_core(), 1, 1);
    core.benchmark_mode = false;
    core.finalize_network_render_products();
    let drawn = published_chunks(&core);
    let before = products(&mut core);
    let undo_depth = core.undo_stack.len();

    assert_eq!(edit_lanes(&mut core, side, 2, 2), "");
    assert_eq!(core.undo_stack.len(), undo_depth + 1);
    let edge = core.region_graph.edge(side);
    assert_eq!((edge.fwd_lanes, edge.bkw_lanes, edge.width), (2, 2, 14.0));
    core.finalize_network_render_products();
    let widened = published_chunks(&core);
    assert_ne!(widened, drawn);
    assert_eq!(fully_emitted_chunks(&mut core), widened);
    let (mut wide, _, wide_side, _) = t_junction_with_side(test_core(), 2, 2);
    assert_eq!(wide_side, side);
    assert_eq!(
        products(&mut core),
        products(&mut wide),
        "the road drawn 2+2"
    );

    assert!(core.undo_action_internal());
    core.finalize_network_render_products();
    let edge = core.region_graph.edge(side);
    assert_eq!((edge.fwd_lanes, edge.bkw_lanes, edge.width), (1, 1, 7.0));
    assert_eq!(published_chunks(&core), drawn);
    assert_eq!(fully_emitted_chunks(&mut core), drawn);
    assert_eq!(products(&mut core), before);
    assert_eq!(core.undo_stack.len(), undo_depth);
}

// `ROAD-80` (2): narrowing 3+3 to 1+1 removes the outer lanes; the cars on them reattach to the
// nearest rebuilt lane of the road (the lane system's 30 m rule), at most 7 m sideways, and none
// is lost.
#[test]
fn narrowing_moves_cars_off_the_removed_lanes() {
    use crate::nodes::sim::render::lane_pose::sample_lane_pose;
    let (mut core, node, side, _) = t_junction_with_side(test_core(), 3, 3);
    let far = core.region_graph.edge(side).end_node;
    let lanes = &core.transit_network.lane_system;
    let outer: Vec<usize> = lanes.edge_lanes[&side]
        .iter()
        .copied()
        .filter(|&id| {
            let lane = &lanes.lanes[id];
            lane.lane_type == LaneType::Vehicle && (lane.lane_idx == 2 || lane.lane_idx == -3)
        })
        .collect();
    assert_eq!(outer.len(), 2);
    let mut cars = Vec::new();
    for lane_id in outer {
        let lane = &core.transit_network.lane_system.lanes[lane_id];
        let path = if lane.is_fwd {
            vec![node, far]
        } else {
            vec![far, node]
        };
        let at = lane.geometry[0];
        let core_ref = &mut core;
        let car = core_ref.agents.spawn_debug_car(
            path,
            &core_ref.transit_network.lane_system,
            lane_id,
            godot::prelude::Vector2::new(at.x, at.z),
        );
        let lane = &core.transit_network.lane_system.lanes[lane_id];
        let (pose, _) = sample_lane_pose(lane, 0.0).unwrap();
        cars.push((car, pose));
    }
    let agents = core.agents.agents.current_lane_id.len();

    assert_eq!(edit_lanes(&mut core, side, 1, 1), "");
    assert_eq!(
        core.agents.agents.current_lane_id.len(),
        agents,
        "no car is lost"
    );
    for (car, was) in cars {
        let lane_id = core.agents.agents.current_lane_id[car];
        let lane = &core.transit_network.lane_system.lanes[lane_id];
        assert_eq!(lane.lane_type, LaneType::Vehicle);
        let (pose, _) = sample_lane_pose(lane, core.agents.agents.lane_distance[car]).unwrap();
        assert!(
            pose.distance_to(was) <= 7.0 + 1e-3,
            "car {car} moved {} m to lane {lane_id}",
            pose.distance_to(was)
        );
    }
}

// `ROAD-80` (3): refusals, each the preview's and the commit's, each changing nothing: not a
// live road, no lanes (a road stays a road), too many lanes, no change, and a tunnel widened in
// place (R3-j); narrowing the tunnel applies.
#[test]
fn lane_edits_are_refused_with_a_reason() {
    let (mut core, node, side, west) = t_junction_with_side(test_core(), 1, 1);
    assert_eq!(edit_lanes(&mut core, usize::MAX >> 1, 1, 1), "invalid_road");
    assert_eq!(
        edit_lanes(&mut core, side, 0, 0),
        "cross_section_cannot_change_transit_mode"
    );
    assert_eq!(edit_lanes(&mut core, side, 5, 1), "too_many_lanes");
    assert_eq!(edit_lanes(&mut core, side, 1, 1), "cross_section_unchanged");
    core.set_edge_class_internal(west as i32, 2);
    assert_eq!(core.region_graph.edge(west).class, EdgeClass::Tunnel);
    assert_eq!(edit_lanes(&mut core, west, 2, 1), "edit_tunnel_width");
    // The side road at the tunnel's portal widens.
    assert_eq!(edit_lanes(&mut core, side, 2, 2), "");
    let west = arm_towards(&core, node, Vector3::new(-100.0, 0.0, 0.0));
    assert_eq!(edit_lanes(&mut core, west, 1, 0), "");
}

// `ROAD-80` (3): a widening reaches a parcel fronting the road, or painted cells beside it, or
// water beside it, or turns too tightly for its new width: refused, nothing changed.
#[test]
fn a_widening_keeps_clear_of_parcels_cells_water_and_tight_curves() {
    use crate::simulation::zoning::cells::{CellBounds, CellSelectionShape};
    use glam::DVec2;
    // A parcel east of the side road.
    let (mut core, _, side, _) = t_junction_with_side(test_core(), 1, 1);
    let residential = core
        .zoning
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();
    core.zoning
        .place_or_rezone_default_parcel_at(12.0, 60.0, residential, &core.region_graph)
        .expect("a parcel fronting the side road");
    assert_eq!(edit_lanes(&mut core, side, 3, 3), "parcel_overlap");

    // Painted cells east of the side road.
    let (mut core, _, side, _) = t_junction_with_side(test_core(), 1, 1);
    core.zoning.generate_cells(
        &core.region_graph,
        CellBounds {
            min: DVec2::splat(-100.0),
            max: DVec2::splat(100.0),
        },
        |_| false,
    );
    let selection = core
        .zoning
        .cells
        .select(CellSelectionShape::Cell, &[DVec2::new(9.0, 60.0)]);
    assert!(!selection.cells.is_empty());
    core.zoning.paint_cells(&selection, 1, |_| false).unwrap();
    assert_eq!(edit_lanes(&mut core, side, 3, 3), "zoning_cell_overlap");

    // A pond 7-12 m east of the side road's centreline, 40-60 m up it.
    let mut core = test_core();
    core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
    core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
    let mut water = WaterSystem::with_chunking(257, 257, 1.0, 129);
    let mut depth = vec![0.0; 257 * 257];
    for z in 168..=188 {
        for x in 135..=140 {
            depth[x + z * 257] = 2.0;
        }
    }
    water.replace_baseline_depth_from_dense(&depth).unwrap();
    core.watermap = water;
    let (mut core, _, side, _) = t_junction_with_side(core, 1, 1);
    assert_eq!(edit_lanes(&mut core, side, 3, 3), "water_requires_bridge");
    assert_eq!(
        edit_lanes(&mut core, side, 1, 0),
        "",
        "narrowing keeps clear of it"
    );

    // A quarter circle of 16 m radius: legal at 1+1, too tight at 4+4.
    let mut core = test_core();
    core.treasury.balance = 1.0e7;
    let arc: Vec<Vector3> = (0..=16)
        .map(|step| {
            let angle = std::f32::consts::FRAC_PI_2 * step as f32 / 16.0;
            Vector3::new(16.0 * angle.cos(), 0.0, 16.0 * angle.sin())
        })
        .collect();
    assert!(core.add_road_internal(arc, 1, 1).committed);
    core.precompute_road_mesh_data();
    let curve = (0..core.region_graph.edge_count())
        .find(|&edge| !core.region_graph.edge(edge).deleted)
        .unwrap();
    assert_eq!(
        edit_lanes(&mut core, curve, 4, 4),
        "cross_section_too_wide_for_curve"
    );
    assert_eq!(edit_lanes(&mut core, curve, 2, 1), "");
}

// `ROAD-80` (5) after `ROAD-97`: a lane edit beside an older bridge publishes the chunks a full
// emission draws; its undo too.
#[test]
fn a_lane_edit_beside_a_bridge_publishes_the_chunks_a_full_emission_draws() {
    let (mut core, side) = older_side_road();
    core.benchmark_mode = false;
    core.set_edge_class_internal(side as i32, 1);
    assert_eq!(core.region_graph.edge(side).class, EdgeClass::Bridge);
    core.finalize_network_render_products();
    let drawn = published_chunks(&core);
    assert_eq!(fully_emitted_chunks(&mut core), drawn);
    let through = core.get_hovered_edge_internal(-60.0, 10.0) as usize;
    assert_eq!(edit_lanes(&mut core, through, 2, 1), "");
    core.finalize_network_render_products();
    let edited = published_chunks(&core);
    assert_ne!(edited, drawn);
    assert_eq!(fully_emitted_chunks(&mut core), edited);
    assert!(core.undo_action_internal());
    core.finalize_network_render_products();
    assert_eq!(published_chunks(&core), drawn);
    assert_eq!(fully_emitted_chunks(&mut core), drawn);
}

// `ROAD-80` (6): the edit writes the existing columns, so a save round trip keeps it.
#[test]
fn a_lane_edit_survives_a_save_file() {
    let (mut core, _, side, _) = t_junction_with_side(test_core(), 1, 1);
    assert_eq!(edit_lanes(&mut core, side, 2, 1), "");
    let path = temp_save_path("road_lane_edit");
    core.save_game_settled_at(path.to_str().unwrap(), None, 0)
        .unwrap();
    let mut loaded = test_core();
    loaded.load_game_internal(path.to_str().unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    let edge = loaded.region_graph.edge(side);
    assert_eq!((edge.fwd_lanes, edge.bkw_lanes, edge.width), (2, 1, 10.5));
    assert_eq!(products(&mut loaded), products(&mut core));
}
