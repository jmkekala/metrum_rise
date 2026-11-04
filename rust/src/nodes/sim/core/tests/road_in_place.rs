// SPDX-License-Identifier: GPL-2.0-only

//! In-place edits of an existing road: the class change (`ROAD-97`: a spliced publication equals
//! a full emission; R3-k: its undo record), the lane-count edit (`ROAD-80`): what it builds,
//! the cars it moves, what it refuses, its undo and its publication, and a road type's
//! attributes given in place (`ROAD-101`): sidewalks with their walkers and entrances, speed and
//! routing, zoning off with its demolitions, the corridor check and the charge.

use super::road_crosswalks::{
    arm_towards, fully_emitted_chunks, products, published_chunks, t_junction_with_side,
};
use super::*;
use crate::simulation::network::graph::RoadMedian;
use crate::simulation::network::lanes::LaneType;
use crate::simulation::network::road_types::RoadAttributes;

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

// The in-place edit's verdict and its commit on the full spec: `""` when applied, else the
// refusal's reason. The preview must give the same verdict, and a refusal must change nothing.
fn edit(
    core: &mut SimCore,
    edge: usize,
    (fwd, bkw, median): (u8, u8, Option<RoadMedian>),
    attributes: RoadAttributes,
) -> String {
    let before = (products(core), core.undo_stack.len());
    let preview = core
        .preview_edge_cross_section_internal(edge as i32, fwd, bkw, median, attributes)
        .err()
        .map(|refusal| refusal.reason)
        .unwrap_or_default();
    assert_eq!(products(core), before.0, "a preview changes nothing");
    let applied = core.set_edge_cross_section_internal(edge as i32, fwd, bkw, median, attributes);
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

// The lane edit: `edit` keeping the road's median-less layout's attributes.
fn edit_lanes(core: &mut SimCore, edge: usize, fwd: u8, bkw: u8) -> String {
    let attributes = core
        .region_graph
        .get_edge(edge)
        .map_or(RoadAttributes::DEFAULT, RoadAttributes::of_edge);
    edit(core, edge, (fwd, bkw, None), attributes)
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

// A one-cell house on a default residential parcel fronting `edge` near `(x, z)`, with the zone
// index, sites and entrance cache current, as the `editing` tests build one. Returns the
// building's index and its parcel's id.
fn house_fronting(core: &mut SimCore, edge: usize, (x, z): (f32, f32)) -> (usize, u64) {
    use crate::assets::AssetManifest;
    if core.allocator.registry.get("test:house").is_none() {
        let manifest: AssetManifest = serde_json::from_value(serde_json::json!({
            "asset_id": "house", "display_name": "House",
            "building": {
                "placement_mode": "zoned_private", "zone_type": "residential", "density": "low",
                "lot_width_cells": 1, "lot_depth_cells": 1,
                "flat_size_m2": 80.0, "household_capacity": 1
            },
            "mesh_parts": [{"name": "main", "lods": [{"file": "lod0.glb", "distance_min_m": 0.0}]}],
            "anchors": [{"type": "entrance", "name": "main", "position": [0.0, 0.0, 0.5], "forward": [0.0, 0.0, 1.0]}]
        }))
        .unwrap();
        manifest.validate().unwrap();
        core.allocator
            .registry
            .register("test", manifest, String::new());
    }
    let profile = core
        .zoning
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();
    let id = core
        .zoning
        .place_or_rezone_default_parcel_at(x, z, profile, &core.region_graph)
        .unwrap();
    let parcel = core.zoning.parcel_by_raw_id(id.raw()).unwrap();
    assert_eq!(parcel.edge_idx(), edge, "the parcel fronts the intended road");
    let center = parcel.front_center() + parcel.normal() * (core.config.zone_cell_m * 0.5);
    let mut building = crate::simulation::buildings::allocator::indexed_test_building(
        "test:house".to_owned(),
        ZoneType::Residential,
        0,
    );
    building.center_x = center.x;
    building.center_y = center.y;
    building.width_cells = 1;
    building.depth_cells = 1;
    building.edge_idx = edge;
    building.parcel_id = id.raw();
    building.zone_profile_runtime_id = profile;
    building.frontage_t = parcel.frontage_center_t();
    building.facing_dir = -parcel.normal();
    building.side = parcel.side();
    building.side_offset = core.region_graph.edge(edge).width * 0.5 + crate::config::SIDEWALK_WIDTH;
    let index = core.allocator.buildings.len();
    core.allocator.buildings.push(building);
    assert!(core.zoning.occupy_parcel(id.raw(), index));
    core.allocator
        .rebuild_building_site_clients(core.config.zone_cell_m);
    core.allocator.rebuild_zone_index();
    core.allocator
        .rebuild_entrance_cache(&core.region_graph, &core.transit_network.lane_system);
    (index, id.raw())
}

// The side road's lanes of `kind`.
fn lanes_of(core: &SimCore, edge: usize, kind: LaneType) -> Vec<usize> {
    let lanes = &core.transit_network.lane_system;
    lanes.edge_lanes[&edge]
        .iter()
        .copied()
        .filter(|&id| lanes.lanes[id].lane_type == kind)
        .collect()
}

// `ROAD-101` (D3): a road type without sidewalks at 80 km/h given in place. The sidewalk lanes go
// with `FOOT`; the walkers on them stay where they stand (laneless on their road, `TRAFFIC-19`'s
// case, or on a sidewalk of their own kind nearby), none moved or lost; the house fronting the
// road gets a car-only entrance; the speed and its routing cost are written; undo restores the
// lanes, `FOOT`, the speed, the cost and the walking entrance.
#[test]
fn sidewalks_removed_in_place_keep_their_walkers_and_undo() {
    use crate::nodes::sim::render::lane_pose::sample_lane_pose;
    use crate::simulation::economy::agents::MODE_WALK;
    use crate::simulation::network::types::TransitFlags;
    let (mut core, _, side, _) = t_junction_with_side(test_core(), 1, 1);
    let (house, _) = house_fronting(&mut core, side, (12.0, 60.0));
    let entrance = &core.allocator.entrances[house];
    assert_ne!(entrance.foot_lane_fwd.min(entrance.foot_lane_bkw), usize::MAX);
    let sidewalks = lanes_of(&core, side, LaneType::Foot);
    let sidewalk_lanes = sidewalks.len();
    assert!(sidewalk_lanes >= 2, "a lane each way on each sidewalk");
    let road = core.region_graph.edge(side);
    let (start, end) = (road.start_node, road.end_node);
    let mut walkers = Vec::new();
    for lane_id in sidewalks {
        let lane = &core.transit_network.lane_system.lanes[lane_id];
        let (at, _) = sample_lane_pose(lane, 40.0).unwrap();
        let from = if lane.is_fwd { start } else { end };
        let walker = core.agents.spawn_housed_agent(usize::MAX, at.x, at.z);
        core.agents.agents.transit_mode[walker] = MODE_WALK;
        core.agents
            .agents
            .enter_lane(walker, &core.transit_network.lane_system, lane_id, 40.0, from);
        walkers.push(walker);
    }
    let agents = core.agents.agents.current_lane_id.len();
    let positions: Vec<_> = walkers
        .iter()
        .map(|&w| (core.agents.agents.pos_x[w].to_bits(), core.agents.agents.pos_y[w].to_bits()))
        .collect();
    let before = core.region_graph.edge(side).clone();

    let rural = RoadAttributes::new(80, false, true);
    assert_eq!(edit(&mut core, side, (1, 1, None), rural), "");
    let road = core.region_graph.edge(side);
    assert_eq!(road.allowed_types & TransitFlags::FOOT, 0);
    assert_eq!(road.speed_limit.to_bits(), rural.speed_ms.to_bits());
    assert_eq!(
        road.base_cost.to_bits(),
        crate::simulation::pathing::cost::CostCalculator::calculate_costs(road)
            .0
            .to_bits()
    );
    assert!(road.base_cost < before.base_cost, "faster is cheaper to route");
    assert!(lanes_of(&core, side, LaneType::Foot).is_empty());
    assert_eq!(lanes_of(&core, side, LaneType::Vehicle).len(), 2);
    assert_eq!(core.agents.agents.current_lane_id.len(), agents, "no walker is lost");
    let lanes = &core.transit_network.lane_system;
    for (&walker, &position) in walkers.iter().zip(&positions) {
        let a = &core.agents.agents;
        assert_eq!((a.pos_x[walker].to_bits(), a.pos_y[walker].to_bits()), position);
        let lane = a.current_lane_id[walker];
        assert!(
            (lane == usize::MAX && a.current_edge[walker] == side)
                || lanes.lanes[lane].lane_type == LaneType::Foot,
            "walker {walker} on lane {lane}"
        );
    }
    let entrance = &core.allocator.entrances[house];
    assert_eq!((entrance.foot_lane_fwd, entrance.foot_lane_bkw), (usize::MAX, usize::MAX));
    assert_eq!(entrance.curb_pos, entrance.door_pos, "a car-only entrance");
    assert_ne!(entrance.car_lane_fwd.min(entrance.car_lane_bkw), usize::MAX);

    assert!(core.undo_action_internal());
    let road = core.region_graph.edge(side);
    assert_eq!(road.allowed_types, before.allowed_types);
    assert_eq!(road.speed_limit.to_bits(), before.speed_limit.to_bits());
    assert_eq!(road.base_cost.to_bits(), before.base_cost.to_bits());
    assert_eq!(lanes_of(&core, side, LaneType::Foot).len(), sidewalk_lanes);
    let entrance = &core.allocator.entrances[house];
    assert_ne!(entrance.foot_lane_fwd.min(entrance.foot_lane_bkw), usize::MAX);
}

// `ROAD-101` (C): a speed change in place recomputes the road's routing cost, and the routing
// hierarchy the edit schedules routes by it: from the T to the side road's far end costs half as
// much at 100 km/h as at 50.
#[test]
fn a_speed_change_in_place_reroutes_by_the_new_cost() {
    use crate::simulation::network::types::TransitFlags;
    let (mut core, node, side, _) = t_junction_with_side(test_core(), 1, 1);
    core.transit_network
        .rebuild_pathing(&mut core.region_graph);
    let far = core
        .region_graph
        .get_valid_node(core.region_graph.edge(side).end_node);
    let cost = |core: &SimCore| {
        core.transit_network
            .cch_graph
            .find_path(
                node,
                far,
                usize::MAX,
                usize::MAX,
                &core.region_graph,
                TransitFlags::CAR,
            )
            .expect("a car route to the far end")
            .0
    };
    let slow = cost(&core);
    assert_eq!(
        edit(&mut core, side, (1, 1, None), RoadAttributes::new(100, true, true)),
        ""
    );
    // The rebuild is deferred (D7): the routing in use keeps the old cost until it is adopted.
    assert_eq!(cost(&core).to_bits(), slow.to_bits());
    core.transit_network.install_pending_cch();
    let fast = cost(&core);
    assert!((fast / slow - 0.5).abs() < 0.02, "{slow} -> {fast}");
    assert_eq!(edit_lanes(&mut core, side, 1, 1), "cross_section_unchanged");
}

// `ROAD-101` (D4, B.2): a road type with zoning off given in place does what the "No buildings"
// checkbox does (`editing`'s `set_no_building_spawn_preserves_unrelated_rezone_grace`): the
// road's parcels go and their buildings are demolished, the verdict counted them first, and a
// house on another road keeps its building. Undo restores the flag and the (empty) parcel; zoning
// back on demolishes nothing.
#[test]
fn zoning_off_in_place_removes_the_roads_parcels_and_buildings() {
    let (mut core, _, side, west) = t_junction_with_side(test_core(), 1, 1);
    let (_, side_parcel) = house_fronting(&mut core, side, (12.0, 60.0));
    let (_, west_parcel) = house_fronting(&mut core, west, (-50.0, 12.0));
    let off = RoadAttributes::new(50, true, false);
    // The count comes from the road's 512 m chunks, or from the whole city while the chunk
    // index is stale.
    for stale in [false, true] {
        core.allocator.rebuild_zone_index();
        core.allocator.dirty_index = stale;
        let verdict = core
            .preview_edge_cross_section_internal(side as i32, 1, 1, None, off)
            .unwrap();
        assert_eq!(verdict.buildings_removed, 1, "stale index: {stale}");
    }
    core.allocator.rebuild_zone_index();
    let verdict = core
        .preview_edge_cross_section_internal(side as i32, 1, 1, None, off)
        .unwrap();
    assert_eq!(verdict.buildings_removed, 1);
    assert!(verdict.demolition_permanent, "no undo restores a road's buildings");
    assert_eq!(verdict.cost_delta, 0.0, "zoning costs nothing");
    assert_eq!(edit(&mut core, side, (1, 1, None), off), "");
    assert!(core.region_graph.edge(side).no_building_spawn);
    assert!(core.zoning.parcel_by_raw_id(side_parcel).is_none());
    assert_eq!(core.allocator.buildings.len(), 1);
    assert_eq!(core.allocator.buildings[0].parcel_id, west_parcel);
    assert_eq!(
        core.zoning
            .parcel_by_raw_id(west_parcel)
            .unwrap()
            .occupied_building(),
        Some(0)
    );
    assert_eq!(
        core.preview_edge_cross_section_internal(west as i32, 1, 1, None, off)
            .unwrap()
            .buildings_removed,
        1
    );

    assert!(core.undo_action_internal());
    assert!(!core.region_graph.edge(side).no_building_spawn);
    let restored = core.zoning.parcel_by_raw_id(side_parcel).expect("the parcel is back");
    assert_eq!(restored.occupied_building(), None, "its building stays demolished");
    assert_eq!(core.allocator.buildings.len(), 1);

    // Zoning back on (from the checkbox's state) demolishes nothing.
    core.set_no_building_spawn_internal(west as i32, true);
    let on = RoadAttributes::DEFAULT;
    let verdict = core
        .preview_edge_cross_section_internal(west as i32, 1, 1, None, on)
        .unwrap();
    assert_eq!(verdict.buildings_removed, 0);
    assert!(!verdict.demolition_permanent);
    assert_eq!(edit(&mut core, west, (1, 1, None), on), "");
    assert!(!core.region_graph.edge(west).no_building_spawn);
}

// `ROAD-101` (A.4): adding sidewalks widens the roadbed though not the carriageway, so the
// widening checks run on the roadbed half-width: a parcel inside the 1.5 m sidewalk band, clear
// of the 0.75 m shoulder, refuses it; removing them is a narrowing and never checked.
#[test]
fn sidewalks_added_next_to_a_parcel_are_refused() {
    use crate::simulation::zoning::ParcelGeometry;
    let (mut core, _, side, _) = t_junction_with_side(test_core(), 1, 1);
    let shoulders = RoadAttributes::new(50, false, true);
    assert_eq!(edit(&mut core, side, (1, 1, None), shoulders), "");
    // A 4 m parcel from 4.6 m east of the side road's centreline: past its 4.25 m shoulder edge,
    // inside its 5.0 m sidewalk edge. Attached to the side road, as a frontage lot.
    let (x0, z0, size) = (4.6, 58.0, 4.0);
    let at = |dx: f32, dz: f32| godot::prelude::Vector2::new(x0 + dx, z0 + dz);
    let profile = core
        .zoning
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();
    core.zoning.parcels.insert_new(
        ParcelGeometry {
            edge_idx: side,
            side: 1,
            frontage_center_t: 0.6,
            frontage_m: size,
            depth_m: size,
            front_center: at(0.0, size * 0.5),
            center: at(size * 0.5, size * 0.5),
            tangent: godot::prelude::Vector2::new(0.0, 1.0),
            normal: godot::prelude::Vector2::new(1.0, 0.0),
            corners: [at(0.0, 0.0), at(0.0, size), at(size, size), at(size, 0.0)],
            aabb_min: at(0.0, 0.0),
            aabb_max: at(size, size),
        },
        profile,
    );
    assert_eq!(
        edit(&mut core, side, (1, 1, None), RoadAttributes::DEFAULT),
        "parcel_overlap"
    );
    assert_eq!(
        edit(&mut core, side, (1, 1, None), RoadAttributes::new(70, false, true)),
        ""
    );
}

// `ROAD-101` (D10): Apply charges the build-cost difference over the road's length outside
// `benchmark_mode`, exactly the verdict's; a cheaper type is free and refunds nothing.
#[test]
fn an_in_place_type_change_charges_the_cost_difference_only() {
    let (mut core, _, side, _) = t_junction_with_side(test_core(), 1, 1);
    core.benchmark_mode = false;
    let length = f64::from(core.region_graph.edge(side).physical_length);
    let built = core.treasury.lifetime_build_cost;
    // 1+1 (weight 1.0) to 2+2 (1.8): 0.8 x 15 a metre.
    let verdict = core
        .preview_edge_cross_section_internal(side as i32, 2, 2, None, RoadAttributes::DEFAULT)
        .unwrap();
    assert!((verdict.cost_delta - 12.0 * length).abs() < 1e-6 * length, "{verdict:?}");
    assert_eq!(edit_lanes(&mut core, side, 2, 2), "");
    assert_eq!(
        (core.treasury.lifetime_build_cost - built).to_bits(),
        verdict.cost_delta.to_bits()
    );
    let built = core.treasury.lifetime_build_cost;
    let verdict = core
        .preview_edge_cross_section_internal(side as i32, 1, 1, None, RoadAttributes::DEFAULT)
        .unwrap();
    assert_eq!(verdict.cost_delta, 0.0);
    assert_eq!(edit_lanes(&mut core, side, 1, 1), "");
    assert_eq!(core.treasury.lifetime_build_cost, built, "no refund");
    // A benchmark city builds for free.
    core.benchmark_mode = true;
    assert_eq!(edit_lanes(&mut core, side, 2, 2), "");
    assert_eq!(core.treasury.lifetime_build_cost, built);
}

// A residential street of six 150 m edges through degree-two nodes, across three render chunks:
// its lights are spaced along the whole street. Published and acknowledged, so the next
// publication is local. Returns the core and the edge holding x = 75 (the fourth).
fn long_street() -> (SimCore, usize) {
    let mut core = test_core();
    core.treasury.balance = 1.0e7;
    core.benchmark_mode = false;
    for i in 0..6 {
        let x = -450.0 + 150.0 * i as f32;
        let points = vec![Vector3::new(x, 0.0, 0.0), Vector3::new(x + 150.0, 0.0, 0.0)];
        assert!(core.add_road_internal(points, 1, 1).committed);
    }
    core.finalize_network_render_products();
    assert_eq!(core.cached_road_mesh_chunks.len(), 3);
    acknowledge(&mut core);
    let edge = core.get_hovered_edge_internal(75.0, 0.0);
    assert!(edge >= 0);
    (core, edge as usize)
}

// Godot's acknowledgement of the last publication: the next one is local (only its changed
// chunks), as in the game.
fn acknowledge(core: &mut SimCore) {
    let generation = core.road_tool_surface_generation;
    assert!(core.acknowledge_network_render_generation(generation));
    assert!(!core.road_mesh_full_replace);
}

// Each chunk's light positions, to tell a re-spaced street from a kept one.
fn chunk_lights(
    core: &SimCore,
) -> Vec<(crate::simulation::network::roadbed::SurfaceChunkKey, Vec<[u32; 3]>)> {
    core.cached_road_mesh_chunks
        .iter()
        .map(|(&chunk, mesh)| {
            let poles = mesh
                .street_lights
                .iter()
                .map(|light| light.position.to_array().map(f32::to_bits))
                .collect();
            (chunk, poles)
        })
        .collect()
}

// `ROAD-59`: widening one edge of a street from 1 + 1 to 2 + 2 in place makes it a main road and
// splits the residential street in two, so both halves are lit anew along their whole length,
// two chunks away from the edit. The local publication re-emits every chunk those streets cross:
// it equals a full emission. Before, only the chunks of the rebuilt pieces were published, and
// the far chunk kept the old spacing.
#[test]
fn a_family_change_in_place_republishes_its_whole_street() {
    let (mut core, edge) = long_street();
    let before = chunk_lights(&core);
    assert_eq!(edit_lanes(&mut core, edge, 2, 2), "");
    core.finalize_network_render_products();
    let after = chunk_lights(&core);
    assert!(
        before.iter().zip(&after).all(|(old, new)| old != new),
        "every chunk's lights move: {before:?} / {after:?}"
    );
    let published = published_chunks(&core);
    assert_eq!(fully_emitted_chunks(&mut core), published);
}

// The street's lights in world space along x (it runs along z = 0), sorted, from a full emission.
fn street_light_xs(core: &mut SimCore) -> Vec<f32> {
    let mesh = core
        .transit_network
        .try_generate_mesh_data(&core.region_graph, &core.heightmap)
        .expect("current roadbed");
    let mut xs: Vec<f32> = mesh.street_lights.iter().map(|light| light.position.x).collect();
    xs.sort_by(f32::total_cmp);
    xs
}

// What a road's street lights must never change: every product but the road payload's first
// six lines (crossings, signs, lights and the three vertex layers): clips, profiles and lanes.
fn simulation_products(core: &mut SimCore) -> String {
    products(core).lines().skip(6).collect::<Vec<_>>().join("\n")
}

// `ROAD-101` RB3e: a road type's street lights given in place. Off: the road has none, and the
// street around it ends at its two nodes, so each half is lit evenly on its own; the local
// publication re-emits both halves wherever they run (a full emission draws the same chunks).
// Lanes, clips, profiles and cost do not change. On again: the street is lit as before, bitwise.
// Undo restores the lights.
#[test]
fn street_lights_turned_off_in_place_relight_the_street_and_undo() {
    let (mut core, edge) = long_street();
    let lit_xs = street_light_xs(&mut core);
    let lit_chunks = published_chunks(&core);
    let simulation = simulation_products(&mut core);
    let lit = RoadAttributes::of_edge(core.region_graph.edge(edge));
    assert!(lit.street_lights);
    let verdict = core
        .preview_edge_cross_section_internal(edge as i32, 1, 1, None, lit.lit(false))
        .expect("lights off is a valid change");
    assert_eq!(verdict.cost_delta, 0.0, "street lights cost nothing");
    assert_eq!(verdict.buildings_removed, 0);

    acknowledge(&mut core);
    let undo_depth = core.undo_stack.len();
    assert_eq!(edit(&mut core, edge, (1, 1, None), lit.lit(false)), "");
    assert!(core.region_graph.edge(edge).no_street_lights);
    core.finalize_network_render_products();
    let unlit_chunks = published_chunks(&core);
    assert_ne!(unlit_chunks, lit_chunks);
    assert_eq!(fully_emitted_chunks(&mut core), unlit_chunks);
    assert_eq!(simulation_products(&mut core), simulation);
    let xs = street_light_xs(&mut core);
    assert!(
        xs.iter().all(|&x| !(0.0..=150.0).contains(&x)),
        "none on the unlit road: {xs:?}"
    );
    for half in [
        xs.iter().copied().filter(|&x| x < 0.0).collect::<Vec<_>>(),
        xs.iter().copied().filter(|&x| x > 150.0).collect(),
    ] {
        assert!(half.len() >= 4, "each half stays lit: {xs:?}");
        let gaps: Vec<f32> = half.windows(2).map(|pair| pair[1] - pair[0]).collect();
        assert!(
            gaps.iter().all(|gap| (gap - gaps[0]).abs() < 1e-3),
            "evenly spaced on its own: {gaps:?}"
        );
    }

    // On again, through a local publication: the street as it was lit.
    acknowledge(&mut core);
    assert_eq!(edit(&mut core, edge, (1, 1, None), lit), "");
    core.finalize_network_render_products();
    assert_eq!(published_chunks(&core), lit_chunks);
    assert_eq!(street_light_xs(&mut core), lit_xs);
    assert_eq!(simulation_products(&mut core), simulation);

    // Undo the "on": dark again; undo the "off": lit.
    acknowledge(&mut core);
    assert_eq!(core.undo_stack.len(), undo_depth + 2);
    assert!(core.undo_action_internal());
    assert!(core.region_graph.edge(edge).no_street_lights);
    core.finalize_network_render_products();
    assert_eq!(published_chunks(&core), unlit_chunks);
    acknowledge(&mut core);
    assert!(core.undo_action_internal());
    assert!(!core.region_graph.edge(edge).no_street_lights);
    core.finalize_network_render_products();
    assert_eq!(published_chunks(&core), lit_chunks);
    assert_eq!(fully_emitted_chunks(&mut core), lit_chunks);
}
