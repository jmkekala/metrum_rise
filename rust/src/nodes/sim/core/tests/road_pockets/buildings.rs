// SPDX-License-Identifier: GPL-2.0-only

//! Buildings fronting a turn pocket's curbs (`ROAD-95` round R5d3): a building grown on a lot
//! fronting either curb of a pocket edge reads that curb, the carriageway's own edge (D17), for
//! its side offset, its lot's front gaps, its road tie-in and its entrance, as a building on a
//! plain road reads its road's; the pocket edit's dependents migration keeps the buildings it
//! moves on their curb through an add, a resize, a removal and undo; a pocket that would widen
//! into a lot is still refused.

use super::cells::four_way;
use super::*;
use crate::simulation::buildings::allocator::BuildingSiteEnvironment;
use crate::simulation::network::roadbed::RoadProfile;
use crate::simulation::zoning::cells::CellSelectionShape;
use glam::DVec2;
use godot::prelude::Vector2;

const RIGHT: (bool, bool) = (false, true);
const LEFT: (bool, bool) = (true, false);
const BOTH: (bool, bool) = (true, true);

// The Kuopio buildability fixture's houses (2 x 2 cell lots, an entrance anchor), as the
// growth-path tests register them.
fn register_houses(core: &mut SimCore) {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../benchmarks/fixtures/kuopio-terrain/residential-buildability.json"
    )))
    .unwrap();
    for asset in fixture["assets"].as_array().unwrap() {
        let mut manifest: crate::assets::AssetManifest =
            toml::from_str(asset.as_str().unwrap()).unwrap();
        for part in &mut manifest.mesh_parts {
            part.imported_bounds = Some([[-0.75, 0.0, -0.75], [0.75, 1.0, 0.75]]);
        }
        core.allocator
            .registry
            .register("kenney", manifest, String::new());
    }
}

// Where a point lies across `edge`: its signed offset along the lanes' normal (positive right of
// the forward direction), and the edge's tangent there. The oracle side, independent of the
// building's own side code.
fn lateral(edge: &Edge, point: Vector2) -> (f32, Vector2) {
    let (a, b) = (edge.physical_geometry[0], edge.physical_geometry[1]);
    let (a, b) = (Vector2::new(a.x, a.z), Vector2::new(b.x, b.z));
    let tangent = (b - a).normalized();
    let normal = Vector2::new(-tangent.y, tangent.x);
    ((point - a).dot(normal), tangent)
}

// The curb a frontage on the side of `point` reads, from the roadbed's own profile (the bands it
// builds): the carriageway's edge and the sidewalk's outer edge, from the centreline.
fn curb(edge: &Edge, point: Vector2) -> (f32, f32) {
    let positive = lateral(edge, point).0 > 0.0;
    RoadProfile::for_edge(edge).side_half_widths_m(positive)
}

// Paints the frontage row along `edge`'s curb on the side `sign` (+1 the positive lateral side)
// over `xs` (world x; the arms run west to east), prepares the lots and grows a house on each one
// that fronts that curb. Returns the buildings grown, in growth order.
fn grow(core: &mut SimCore, edge: usize, sign: f32, xs: [f32; 2]) -> Vec<usize> {
    let road = core.region_graph.edge(edge).clone();
    let probe = Vector2::new(xs[0], sign * 10.0);
    let (_, outer) = curb(&road, probe);
    // The first two rows of cells (a house's lot is two deep), by their centres.
    let cell_m = core.config.zone_cell_m;
    let row = f64::from(sign * (outer + cell_m));
    let path = [
        DVec2::new(f64::from(xs[0]), row),
        DVec2::new(f64::from(xs[1]), row),
    ];
    // The cell tool's gesture: its preview and commit, which keep off the road's pavement.
    let brush = CellSelectionShape::Brush {
        radius_m: f64::from(0.6 * cell_m),
    };
    let preview = core.preview_cell_selection_internal(brush, &path);
    assert!(!preview.selection.cells.is_empty(), "cells beside the curb");
    assert!(core.apply_cell_selection_internal(&preview, 1));
    core.prepare_cell_lots_internal();
    let candidates = core.allocator.collect_demand_spawn_candidates_by_use(
        &core.zoning,
        &core.region_graph,
        core.demand.runtime_catalog(),
        &[],
        BuildingSiteEnvironment {
            road_surface: &core.transit_network.roadbed,
            terrain: &core.heightmap,
        },
    );
    let mut grown = Vec::new();
    for candidate in &candidates.residential {
        let action = &candidate.action;
        let Some(parcel) = core.zoning.parcel_by_raw_id(action.parcel_id) else {
            continue;
        };
        let front = parcel.front_center();
        if parcel.edge_idx() != edge
            || !parcel.is_available()
            || lateral(&road, front).0 * sign <= 0.0
            || front.x < xs[0]
            || front.x > xs[1]
        {
            continue;
        }
        if let Ok(index) = core.allocator.execute_demand_spawn_action(
            action,
            &mut core.zoning,
            &core.region_graph,
            &core.transit_network.roadbed,
            &core.heightmap,
            core.demand.runtime_catalog(),
            core.demand.runtime_tuning(),
        ) {
            grown.push(index);
        }
    }
    core.rebuild_building_entrances_internal();
    grown
}

// A building's frontage products: what the curb sets (side offset, front gaps, support height)
// and its entrance (door, curb point, lanes), every bit. Lanes by their road, index and
// direction: a local lane rebuild renumbers them.
fn frontage_products(core: &SimCore, building: usize) -> Vec<u32> {
    let b = &core.allocator.buildings[building];
    let e = &core.allocator.entrances[building];
    let mut bits = vec![
        b.edge_idx as u32,
        b.side as u32,
        b.frontage_t.to_bits(),
        b.center_x.to_bits(),
        b.center_y.to_bits(),
        b.side_offset.to_bits(),
        b.support_height_m.to_bits(),
    ];
    bits.extend(b.front_gap_m.map(f32::to_bits));
    bits.extend(
        [
            e.door_pos.x,
            e.door_pos.y,
            e.curb_pos.x,
            e.curb_pos.y,
            e.entrance_s_m,
        ]
        .map(f32::to_bits),
    );
    let lanes = &core.transit_network.lane_system.lanes;
    for lane in [
        e.foot_lane_fwd,
        e.foot_lane_bkw,
        e.car_lane_fwd,
        e.car_lane_bkw,
    ] {
        bits.extend(lanes.get(lane).map_or([u32::MAX; 3], |lane| {
            [
                lane.edge_id as u32,
                lane.lane_idx as u32,
                u32::from(lane.is_fwd),
            ]
        }));
    }
    bits
}

// The frontage contract of `building` on its curb, read from the road profile: its side offset is
// the sidewalk's outer edge, its lot's front line stands on that edge (a plain road's setback,
// the sidewalk's width from the kerb), a straight road leaves no front gap, its pad ties in at
// the sidewalk's top, its entrance meets its own side's sidewalk lane on the sidewalk's centre
// line, and its car lanes are its road's.
fn assert_on_its_curb(core: &SimCore, building: usize, label: &str) {
    let b = &core.allocator.buildings[building];
    let e = &core.allocator.entrances[building];
    let edge = core.region_graph.edge(b.edge_idx);
    let center = Vector2::new(b.center_x, b.center_y);
    let (kerb, outer) = curb(edge, center);
    let half_depth = f32::from(b.depth_cells) * core.config.zone_cell_m * 0.5;
    let front = center + b.facing_dir * half_depth;
    let (front_offset, _) = lateral(edge, front);
    let label = format!(
        "{label}: building {building} on edge {} side {}",
        b.edge_idx, b.side
    );
    assert!(
        (b.side_offset - outer).abs() < 1e-4,
        "{label}: side offset {} on the curb's sidewalk edge {outer}",
        b.side_offset
    );
    assert!(
        (front_offset.abs() - outer).abs() < 1e-3,
        "{label}: lot front {front_offset} on the sidewalk edge {outer}"
    );
    assert!(
        (front_offset.abs() - kerb - crate::config::SIDEWALK_WIDTH).abs() < 1e-3,
        "{label}: a plain road's setback from the kerb {kerb}"
    );
    assert_eq!(
        b.front_gap_m, [0.0; 3],
        "{label}: front gaps on a straight road"
    );
    // The pad ties in at the sidewalk's top, just inside its outer edge (the support's road
    // probe), not on the carriageway.
    let tie_in = front + b.facing_dir * 0.05;
    let sidewalk_top = core.transit_network.roadbed.height_at(tie_in.x, tie_in.y);
    assert!(
        sidewalk_top.is_some_and(|top| (b.support_height_m - top).abs() < 1e-4),
        "{label}: support {} at the sidewalk's top {sidewalk_top:?}",
        b.support_height_m
    );
    assert!(e.foot_lane_fwd != usize::MAX, "{label}: a sidewalk lane");
    let sidewalk = &core.transit_network.lane_system.lanes[e.foot_lane_fwd];
    assert_eq!(
        sidewalk.edge_id, b.edge_idx,
        "{label}: its own road's sidewalk"
    );
    let (curb_offset, _) = lateral(edge, e.curb_pos);
    assert!(
        curb_offset * front_offset > 0.0,
        "{label}: the sidewalk on the lot's side"
    );
    assert!(
        (curb_offset.abs() - 0.5 * (kerb + outer)).abs() < 1e-3,
        "{label}: entrance {curb_offset} on the sidewalk's line {}",
        0.5 * (kerb + outer)
    );
    let (door_offset, _) = lateral(edge, e.door_pos);
    assert!(
        door_offset * front_offset > 0.0 && door_offset.abs() >= outer - 1e-3,
        "{label}: door {door_offset} on the lot, off the sidewalk"
    );
    assert!(
        e.car_lane_fwd != usize::MAX || e.car_lane_bkw != usize::MAX,
        "{label}: a car lane"
    );
    for lane in [e.car_lane_fwd, e.car_lane_bkw] {
        if lane != usize::MAX {
            assert_eq!(
                core.transit_network.lane_system.lanes[lane].edge_id, b.edge_idx,
                "{label}: its own road's lane"
            );
        }
    }
}

// A west-arm pocket of `roles` on the `lanes` + `lanes` four-way, 60 m body; returns the core and
// the pocket edge.
fn pocketed(lanes: i32, roles: (bool, bool)) -> (SimCore, usize) {
    let (mut core, node) = four_way(lanes, 0.0);
    core.benchmark_mode = false;
    register_houses(&mut core);
    let (west, end) = arm_end(&core, node, Vector3::new(-200.0, 0.0, 0.0));
    assert_eq!(pocket_edit(&mut core, request(west, end, roles, 60.0)), "");
    let pocket = core
        .get_road_pocket_internal(west as i32, end)
        .unwrap()
        .edge;
    (core, pocket)
}

// A house on a lot fronting each curb of a right, left and both pocket edge (1+1 and 2+2): side
// offset, front line, front gaps, pad and entrance on the curb's true line, as on a plain road.
// Before round R5d3 the readers took the symmetric half-width (`width / 2`): on a widened curb
// the side offset 1.75 m short, a 1.75 m front gap (paving drawn across the sidewalk) and the
// pad's road tie-in probe on the carriageway (the pad at road level, 0.12 m under the sidewalk);
// on the unwidened curb of a one-side pocket the probe 1.75 m past the road, so no house grew.
// A both-sided pocket's kerbs are symmetric and were right.
#[test]
fn buildings_on_a_pocket_edge_front_their_true_curbs() {
    for lanes in [1, 2] {
        for (name, roles) in [("right", RIGHT), ("left", LEFT), ("both", BOTH)] {
            let (mut core, pocket) = pocketed(lanes, roles);
            for sign in [1.0, -1.0] {
                let label = format!("{lanes}+{lanes} {name} pocket, side {sign}");
                let grown = grow(&mut core, pocket, sign, [-55.0, -25.0]);
                assert!(
                    !grown.is_empty(),
                    "{label}: a house grows on the pocket edge"
                );
                for &building in &grown {
                    assert_eq!(core.allocator.buildings[building].edge_idx, pocket);
                    assert_on_its_curb(&core, building, &label);
                }
            }
        }
    }
}

// The plain road's frontage products, for the bitwise comparison with a build before round
// R5d3 (`R5D3_PLAIN` lines: the same digest on both trees), and the curb contract they meet.
#[test]
fn buildings_on_a_plain_road_keep_their_products() {
    for lanes in [1, 2] {
        let (mut core, node) = four_way(lanes, 0.0);
        core.benchmark_mode = false;
        register_houses(&mut core);
        let (west, _) = arm_end(&core, node, Vector3::new(-200.0, 0.0, 0.0));
        let mut digest = Vec::new();
        for sign in [1.0, -1.0] {
            for building in grow(&mut core, west, sign, [-55.0, -25.0]) {
                assert_on_its_curb(&core, building, &format!("{lanes}+{lanes} plain"));
                digest.extend(frontage_products(&core, building));
            }
        }
        assert!(!digest.is_empty());
        let hash = digest
            .iter()
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, &word| {
                (hash ^ u64::from(word)).wrapping_mul(0x0100_0000_01b3)
            });
        eprintln!(
            "R5D3_PLAIN lanes {lanes} words {} digest {hash:016x}",
            digest.len()
        );
    }
}

// The pocket edit's dependents migration (R5a2) moves the buildings fronting the pair: houses on
// the curb a pocket does not widen keep that curb when an add moves them onto the pocket edge, a
// resize moves more, a removal joins them back, and each undo restores them bit for bit. A pocket
// that would widen into a house's lot is refused (`parcel_overlap`).
#[test]
fn the_pocket_edit_keeps_the_buildings_it_moves_on_their_curb() {
    for lanes in [1, 2] {
        for (name, roles, widened) in [("right", RIGHT, 1.0), ("left", LEFT, -1.0)] {
            let label = format!("{lanes}+{lanes} {name}");
            let (mut core, node) = four_way(lanes, 0.0);
            core.benchmark_mode = false;
            register_houses(&mut core);
            let (west, end) = arm_end(&core, node, Vector3::new(-200.0, 0.0, 0.0));
            // The curb the pocket keeps, out past a 100 m pocket's transition.
            let houses = grow(&mut core, west, -widened, [-150.0, -25.0]);
            let check = |core: &mut SimCore, step: &str| {
                core.prepare_cell_lots_internal();
                core.rebuild_building_entrances_internal();
                for &building in &houses {
                    assert_on_its_curb(core, building, &format!("{label} {step}"));
                }
                houses
                    .iter()
                    .map(|&b| core.allocator.buildings[b].edge_idx)
                    .collect::<Vec<_>>()
            };
            let edges = check(&mut core, "before");
            assert!(edges.iter().all(|&edge| edge == west));
            let original: Vec<_> = houses
                .iter()
                .map(|&b| frontage_products(&core, b))
                .collect();
            let mut states = vec![original.clone()];
            for (step, edit) in [
                ("add", request(west, end, roles, 60.0)),
                ("resize", request(west, end, roles, 100.0)),
                ("removal", request(west, end, (false, false), 100.0)),
            ] {
                assert_eq!(pocket_edit(&mut core, edit), "", "{label} {step}");
                let edges = check(&mut core, step);
                if step != "removal" {
                    let state = core.get_road_pocket_internal(west as i32, end).unwrap();
                    assert!(
                        edges.contains(&state.edge),
                        "{label} {step}: the pocket edge takes houses"
                    );
                }
                states.push(
                    houses
                        .iter()
                        .map(|&b| frontage_products(&core, b))
                        .collect(),
                );
            }
            for step in ["removal", "resize", "add"] {
                assert!(core.undo_action_internal(), "{label} undo {step}");
                check(&mut core, &format!("undo {step}"));
                states.pop();
                let restored: Vec<_> = houses
                    .iter()
                    .map(|&b| frontage_products(&core, b))
                    .collect();
                assert_eq!(
                    &restored,
                    states.last().unwrap(),
                    "{label} undo {step}: bit for bit"
                );
            }
            assert_eq!(states, vec![original]);
            // A house on the curb the pocket would widen: the corridor refuses it.
            assert!(!grow(&mut core, west, widened, [-55.0, -25.0]).is_empty());
            assert_eq!(
                pocket_edit(&mut core, request(west, end, roles, 60.0)),
                "parcel_overlap",
                "{label}: a lot on the widened curb"
            );
        }
    }
}

// The stills' city (`METRUM_R5D3_SAVE`): the 2+2 four-way with a 60 m right pocket on its west
// arm and houses grown on both curbs, the widened (south) and the kept (north) one, along the
// pocket edge and the base road beyond it, built and saved settled. A build before round R5d3
// grows no house on the kept curb of the pocket edge. Ignored: it writes a file.
#[test]
#[ignore = "writes the stills' save: METRUM_R5D3_SAVE"]
fn a_pocketed_arm_with_houses_on_both_curbs_save() {
    let save = std::env::var("METRUM_R5D3_SAVE").expect("METRUM_R5D3_SAVE");
    let (mut core, pocket) = pocketed(2, RIGHT);
    let base = core.region_graph.edge(pocket).start_node;
    let base = *core
        .region_graph
        .node_adjacency(base)
        .iter()
        .find(|&&edge| edge != pocket)
        .expect("the base road");
    for sign in [1.0, -1.0] {
        let houses = [
            grow(&mut core, pocket, sign, [-55.0, -25.0]),
            grow(&mut core, base, sign, [-150.0, -85.0]),
        ];
        eprintln!("R5D3_STILLS side {sign} houses {houses:?}");
    }
    core.allocator.complete_all_construction();
    core.save_game_settled(&save, None).expect("save");
}
