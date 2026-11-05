// SPDX-License-Identifier: GPL-2.0-only

//! Road-generated zoning cells around turn pockets (`ROAD-95` round R5d1): a pocket's pieces
//! keep their base road's curb frames, its widened curb reads them moved out by the kerb's
//! move, the unwidened curb's cells run on unchanged, no other road re-phases, and resizing,
//! removal, undo and a save give the frames and cells back bit for bit.

use super::*;
use crate::simulation::agriculture::PolygonFootprint;
use crate::simulation::zoning::cells::{
    CellBounds, GridFrame, interiors_overlap, overlaps_road_corridors, road_contact_interior,
};
use glam::DVec2;
use std::collections::{BTreeMap, BTreeSet};

type Cell = (GridFrame, i32, i32);

// The cells and curb frames of a city near its crossing: what a pocket edit must keep.
#[derive(Clone, PartialEq, Debug)]
struct Zoning {
    frames: BTreeMap<usize, [GridFrame; 2]>,
    cells: BTreeSet<Cell>,
    // Each cell's road frontage links: (edge, side).
    fronts: BTreeMap<Cell, Vec<(usize, i8)>>,
}

const RADIUS_M: f64 = 220.0;

// Every cell within `RADIUS_M` of `centre`, generated first (the chunks the square touches, as
// the overlay prepares them), with the curb frames of every road.
fn zoning(core: &mut SimCore, centre: DVec2) -> Zoning {
    let bounds = CellBounds {
        min: centre - DVec2::splat(RADIUS_M),
        max: centre + DVec2::splat(RADIUS_M),
    };
    let size = f64::from(crate::simulation::network::graph::RegionGraph::CHUNK_SIZE);
    for x in (bounds.min.x / size).floor() as i32..=(bounds.max.x / size).floor() as i32 {
        for y in (bounds.min.y / size).floor() as i32..=(bounds.max.y / size).floor() as i32 {
            core.prepare_cell_chunk_internal((x, y));
        }
    }
    let store = &core.zoning.cells;
    let mut cells = BTreeSet::new();
    let mut fronts = BTreeMap::new();
    store.visit_in_bounds(bounds, |key| {
        let cell = (store.frame(key.grid).unwrap(), key.x, key.y);
        cells.insert(cell);
        let links: Vec<_> = store
            .frontages(key)
            .iter()
            .map(|link| (link.edge, link.side))
            .collect();
        if !links.is_empty() {
            fronts.insert(cell, links);
        }
    });
    let frames = store
        .saved_road_alignments()
        .into_iter()
        .map(|(id, alignment)| (id, alignment.frames()))
        .collect();
    Zoning {
        frames,
        cells,
        fronts,
    }
}

// Whether road covers `cell`: a road's corridor or its compiled pavement, a taper included.
fn under_road(core: &SimCore, (frame, x, y): Cell) -> bool {
    let corners = frame.corners(x, y);
    let points = road_contact_interior(&corners).map(|p| [p.x, p.y]);
    overlaps_road_corridors(&core.region_graph, &corners)
        || PolygonFootprint::from_precise_slice(&points)
            .overlaps_exposed_roads(&core.transit_network.roadbed)
}

// The control's cells `after` lacks that no road covers, no cell of `after` overlaps and no
// covered loss cuts off (one of the five cells before it in a lattice row or column, the column's
// frontage cell included: a cell needs an unbroken column to its road): holes an edit left, by
// their centres. `O(lost * after)`.
fn unexplained_losses(
    core: &SimCore,
    control: &BTreeSet<Cell>,
    after: &BTreeSet<Cell>,
) -> Vec<DVec2> {
    let lost: BTreeSet<Cell> = control.difference(after).copied().collect();
    let covered: BTreeSet<Cell> = lost
        .iter()
        .copied()
        .filter(|&cell| under_road(core, cell))
        .collect();
    lost.iter()
        .copied()
        .filter(|&(frame, x, y)| {
            let corners = frame.corners(x, y);
            let cut_off = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .into_iter()
                .any(|(dx, dy)| {
                    (1..6)
                        .map(|k| (frame, x + dx * k, y + dy * k))
                        .take_while(|cell| control.contains(cell))
                        .any(|cell| covered.contains(&cell))
                });
            !covered.contains(&(frame, x, y))
                && !cut_off
                && !after
                    .iter()
                    .any(|&(other, u, v)| interiors_overlap(&corners, &other.corners(u, v)))
        })
        .map(|(frame, x, y)| frame.world(f64::from(x) + 0.5, f64::from(y) + 0.5))
        .collect()
}

// Missing lattice cells whose four neighbours on their own lattice are all present.
fn holes(cells: &BTreeSet<Cell>) -> BTreeSet<Cell> {
    let around = |(frame, x, y): Cell| {
        [
            (frame, x - 1, y),
            (frame, x + 1, y),
            (frame, x, y - 1),
            (frame, x, y + 1),
        ]
    };
    cells
        .iter()
        .flat_map(|&cell| around(cell))
        .filter(|cell| !cells.contains(cell) && around(*cell).iter().all(|n| cells.contains(n)))
        .collect()
}

// A four-way crossing at the origin of two 400 m roads of `lanes` + `lanes` lanes: one west to
// east, one north to south turned `skew_deg` off square (the user's dump crosses at about
// 90.45 degrees, so its two roads take independent grid bases). Returns the core and the node.
fn four_way(lanes: i32, skew_deg: f64) -> (SimCore, u32) {
    let mut core = test_core();
    core.treasury.balance = 1.0e7;
    let (sin, cos) = skew_deg.to_radians().sin_cos();
    let north = Vector3::new((-200.0 * sin) as f32, 0.0, (-200.0 * cos) as f32);
    for points in [
        vec![
            Vector3::new(-200.0, 0.0, 0.0),
            Vector3::new(200.0, 0.0, 0.0),
        ],
        vec![north, -north],
    ] {
        assert!(core.add_road_internal(points, lanes, lanes).committed);
    }
    core.precompute_road_mesh_data();
    let node = (0..core.region_graph.node_count() as u32)
        .find(|&node| live_arms(&core, node).len() == 4)
        .expect("the crossing");
    (core, node)
}

// The north-to-south road's end towards `sign` (-1 north, +1 south).
fn north_south(skew_deg: f64, sign: f32) -> Vector3 {
    let (sin, cos) = skew_deg.to_radians().sin_cos();
    Vector3::new((200.0 * sin) as f32, 0.0, (200.0 * cos) as f32) * sign
}

// A cell fronting `edge` on curb `side` (-1, +1) lies on that curb: the carriageway's axis plus
// the half-width and sidewalk, D17. True when the front boundary of its lattice passes through
// that line within a micrometre.
fn fronts_the_kerb(edge: &Edge, frame: GridFrame, side: i8) -> bool {
    let axis = edge.carriageway_axis(&edge.physical_geometry);
    let (a, b) = (axis[0], axis[axis.len() - 1]);
    let (a, b) = (
        DVec2::new(f64::from(a.x), f64::from(a.z)),
        DVec2::new(f64::from(b.x), f64::from(b.z)),
    );
    let tangent = (b - a).normalize();
    let normal = DVec2::new(tangent.y, -tangent.x) * f64::from(side);
    let kerb =
        a + normal * (f64::from(edge.width) * 0.5 + f64::from(crate::config::SIDEWALK_WIDTH));
    let (u, v) = frame.axes();
    let lateral = usize::from(normal.dot(v).abs() > normal.dot(u).abs());
    let local = frame.local(kerb)[lateral];
    (local - local.round()).abs() * frame.cell_m() < 1e-6
}

// Whether `frame` keeps `base`'s phase along the road (both on one basis).
fn keeps_the_along_phase(frame: GridFrame, base: GridFrame, tangent: DVec2) -> bool {
    let (u, v) = frame.axes();
    let along = usize::from(tangent.dot(v).abs() > tangent.dot(u).abs());
    let local = frame.local(base.world(0.0, 0.0))[along];
    frame.axes() == base.axes() && (local - local.round()).abs() * frame.cell_m() < 1e-6
}

// The pocket edge and base piece a pocket edit left at the arm `towards`.
fn pieces(core: &SimCore, arm: usize, end: EdgeEnd) -> (usize, usize) {
    let state = core
        .get_road_pocket_internal(arm as i32, end)
        .expect("a pocket");
    let pocket = core.region_graph.edge(state.edge);
    let transition = if pocket.start_node == state.junction {
        pocket.end_node
    } else {
        pocket.start_node
    };
    let base = live_arms(core, transition)
        .into_iter()
        .find(|&edge| edge != state.edge)
        .expect("the base piece");
    (state.edge, base)
}

// Round R5d1, the user's report "the zoning is quite broken after I make pockets": a right, a
// left and both pockets on the west arm of a 1+1 and a 2+2 crossing (square, and turned like the
// user's): both pieces keep the arm's curb frames (records bit for bit), every other road keeps
// its own, the pocket edge's cells front its true curbs (D17: the carriageway axis plus the
// half-width and sidewalk) and keep the arm's phase along it, the curb no pocket moves keeps its
// lattice's cells bit for bit, and every cell the pocket costs lies under the widened road or
// its taper or under a cell of the moved curb, with no new hole. A resize, a resize back, the
// removal and each undo give the frames and cells back bit for bit. Before round R5d1 the pocket
// edge's frames lay on its centreline's half-width (1.75 m inside the widened curb, so its row 0
// met the corridor and the whole side dropped) and took a new phase from its own ends.
#[test]
fn a_pocket_keeps_its_base_roads_grid_and_moves_only_its_widened_curb() {
    for (lanes, skew) in [(1, 0.0), (2, 0.0), (2, 0.45)] {
        // Directed curb 0 is the right of the eastbound arm (south), 1 its left (north).
        for (roles, widened) in [
            ((false, true), [true, false]),
            ((true, false), [false, true]),
            ((true, true), [true, true]),
        ] {
            let case = format!("{lanes}+{lanes} skew {skew} roles {roles:?}");
            let (mut core, node) = four_way(lanes, skew);
            let (west, end) = arm_end(&core, node, Vector3::new(-200.0, 0.0, 0.0));
            assert_eq!(end, EdgeEnd::End, "{case}");
            let control = zoning(&mut core, DVec2::ZERO);
            let base = control.frames[&west];
            assert_eq!(
                pocket_edit(&mut core, request(west, end, roles, 60.0)),
                "",
                "{case}"
            );
            let pocketed = zoning(&mut core, DVec2::ZERO);
            let (pocket, base_piece) = pieces(&core, west, end);
            assert_eq!(base_piece, west, "{case}");
            for (id, frames) in &pocketed.frames {
                assert_eq!(
                    frames,
                    control.frames.get(id).unwrap_or(&base),
                    "{case}: road {id}'s frames"
                );
            }
            assert!(pocketed.frames.contains_key(&pocket), "{case}");
            let edge = core.region_graph.edge(pocket).clone();
            let mut fronting = [0; 2];
            for (cell, links) in &pocketed.fronts {
                for &(supplier, side) in links {
                    if supplier != pocket {
                        continue;
                    }
                    let index = usize::from(side > 0);
                    fronting[index] += 1;
                    assert!(
                        fronts_the_kerb(&edge, cell.0, side),
                        "{case}: {cell:?} side {side}"
                    );
                    assert!(
                        keeps_the_along_phase(cell.0, base[index], DVec2::X),
                        "{case}: {cell:?} side {side}"
                    );
                    assert_eq!(
                        cell.0 == base[index],
                        !widened[index],
                        "{case}: {cell:?} side {side}"
                    );
                }
            }
            assert!(fronting[0] >= 4 && fronting[1] >= 4, "{case}: {fronting:?}");
            // Beyond the curb no pocket moves (curb 0 south, +z; curb 1 north) every cell stays.
            for (index, &moved) in widened.iter().enumerate() {
                if !moved {
                    let side = |zoning: &Zoning| -> BTreeSet<Cell> {
                        let sign = if index == 0 { 1.0 } else { -1.0 };
                        zoning
                            .cells
                            .iter()
                            .copied()
                            .filter(|&(frame, x, y)| {
                                frame.world(f64::from(x) + 0.5, f64::from(y) + 0.5).y * sign > 0.0
                            })
                            .collect()
                    };
                    let (after, before) = (side(&pocketed), side(&control));
                    let gone: Vec<_> = before.difference(&after).collect();
                    let new: Vec<_> = after.difference(&before).collect();
                    assert!(
                        gone.is_empty() && new.is_empty(),
                        "{case}: curb {index}: gone {gone:?} new {new:?}"
                    );
                }
            }
            assert_eq!(
                unexplained_losses(&core, &control.cells, &pocketed.cells),
                [],
                "{case}"
            );
            let new_holes: Vec<_> = holes(&pocketed.cells)
                .difference(&holes(&control.cells))
                .copied()
                .collect();
            assert_eq!(new_holes, [], "{case}");

            // A resize keeps the frames; resizing back gives the cells back.
            assert_eq!(
                pocket_edit(&mut core, request(west, end, roles, 90.0)),
                "",
                "{case}"
            );
            let longer = zoning(&mut core, DVec2::ZERO);
            assert_eq!(longer.frames, pocketed.frames, "{case}: resized");
            assert_eq!(
                pocket_edit(&mut core, request(west, end, roles, 60.0)),
                "",
                "{case}"
            );
            assert_eq!(
                zoning(&mut core, DVec2::ZERO),
                pocketed,
                "{case}: resized back"
            );
            assert_eq!(
                pocket_edit(&mut core, request(west, end, (false, false), 60.0)),
                "",
                "{case}"
            );
            let mut removed = zoning(&mut core, DVec2::ZERO);
            // The removal deletes the pocket edge, whose record stays to describe the join.
            removed.frames.remove(&pocket);
            assert_eq!(removed, control, "{case}: removed");
            for (step, expected) in [
                ("undo the removal", &pocketed),
                ("undo the resize back", &longer),
                ("undo the resize", &pocketed),
            ] {
                assert!(core.undo_action_internal(), "{case}: {step}");
                assert_eq!(&zoning(&mut core, DVec2::ZERO), expected, "{case}: {step}");
            }
            assert!(core.undo_action_internal(), "{case}: undo the add");
            let mut undone = zoning(&mut core, DVec2::ZERO);
            undone.frames.remove(&pocket);
            assert_eq!(undone, control, "{case}: undo the add");
        }
    }
}

// The user's layout (`ROAD-95` round R5d1): a 2+2 crossing turned like their dump, a left
// pocket on the west arm, a right one on the east, both on the north and a left one on the south.
// Every original arm's frames are held by both its pieces and no cell is lost but under road or
// a moved curb's cell; a save and load keeps every frame (the load accepts the pieces' records)
// and gives the live cells; removing the four pockets gives the crossing's frames and cells back.
#[test]
fn the_users_pocketed_crossing_keeps_its_grid_through_a_save_and_removal() {
    let skew = 0.45;
    let (mut core, node) = four_way(2, skew);
    let control = zoning(&mut core, DVec2::ZERO);
    let arms = [
        (Vector3::new(-200.0, 0.0, 0.0), (true, false)),
        (Vector3::new(200.0, 0.0, 0.0), (false, true)),
        (north_south(skew, -1.0), (true, true)),
        (north_south(skew, 1.0), (true, false)),
    ]
    .map(|(towards, roles)| {
        let (arm, end) = arm_end(&core, node, towards);
        (arm, end, roles)
    });
    for &(arm, end, roles) in &arms {
        assert_eq!(pocket_edit(&mut core, request(arm, end, roles, 60.0)), "");
    }
    let pocketed = zoning(&mut core, DVec2::ZERO);
    for &(arm, end, _) in &arms {
        let (pocket, base) = pieces(&core, arm, end);
        assert_eq!(pocketed.frames[&pocket], control.frames[&arm]);
        assert_eq!(pocketed.frames[&base], control.frames[&arm]);
    }
    assert_eq!(
        unexplained_losses(&core, &control.cells, &pocketed.cells),
        []
    );
    assert!(holes(&pocketed.cells).is_subset(&holes(&control.cells)));

    let path = temp_save_path("pocket_zoning");
    core.save_game_settled(path.to_str().unwrap(), None)
        .unwrap();
    let mut loaded = test_core();
    loaded.load_game_internal(path.to_str().unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(zoning(&mut loaded, DVec2::ZERO), pocketed, "loaded");

    for &(arm, end, _) in &arms {
        assert_eq!(
            pocket_edit(&mut core, request(arm, end, (false, false), 60.0)),
            ""
        );
    }
    let mut removed = zoning(&mut core, DVec2::ZERO);
    removed
        .frames
        .retain(|id, _| control.frames.contains_key(id));
    assert_eq!(removed, control, "removed");
}

// The user's crossing (`METRUM_R5D1_DUMP`): their dump, a copy with its crossing's alignment
// records dropped (written by the pre-R5d1 rules, which the load now refuses, so the frames start
// from the network as an older save's do), or a control save this test wrote from it. A dump's
// pockets are removed first; the city without them is the control (`METRUM_R5D1_CONTROL_SAVE`
// saves it and stops). Then the user's four pockets go in (west left, east right, north both,
// south left, 60 m each, as in the dump) and the test prints each road's frames, a map, and the
// cell table per quadrant, near the crossing (within the pockets' reach) or far; with
// `METRUM_R5D1_SAVE` it saves the pocketed city (the stills). Last it removes the pockets and
// says whether the control came back bit for bit. The same control save run on two binaries
// compares their pocket edits on one city. Ignored: the dump is not in the repository.
#[test]
#[ignore = "needs the user's dump: METRUM_R5D1_DUMP"]
fn the_users_pocketed_crossing_dump() {
    let path = std::env::var("METRUM_R5D1_DUMP").expect("METRUM_R5D1_DUMP");
    let mut core = test_core();
    core.benchmark_mode = false;
    core.treasury.balance = 1.0e7;
    core.load_game_internal(&path).expect("load dump");
    core.zoning.cells.refresh_road_alignments(
        &core.region_graph,
        &core.config,
        0..core.region_graph.edge_count(),
    );
    core.precompute_road_mesh_data();
    core.refresh_road_terrain_inputs();
    let at = |node: u32| {
        let pos = core.region_graph.node(node).pos;
        DVec2::new(f64::from(pos.x), f64::from(pos.z))
    };
    let junction = at(4);
    let east = (at(1) - at(0)).normalize();
    let south = (at(3) - at(2)).normalize();
    // Each arm by its far node (west, east, north, south) and the user's pocket on it.
    let arms = [
        (0, (true, false)),
        (1, (false, true)),
        (2, (true, true)),
        (3, (true, false)),
    ];
    let far = |core: &SimCore, node: u32| {
        let pos = core.region_graph.node(node).pos;
        arm_end(core, 4, pos)
    };
    let report = |core: &mut SimCore, label: &str| -> Zoning {
        let zoning = zoning(core, junction);
        for (id, edge) in core.region_graph.edges().iter().enumerate() {
            if !edge.deleted {
                eprintln!(
                    "{label} edge {id} {}->{} width {} base {} shift {} pocket {:?} frames {:?}",
                    edge.start_node,
                    edge.end_node,
                    edge.width,
                    edge.base_width_m(),
                    edge.kerb_shift_m(),
                    edge.pocket,
                    zoning.frames.get(&id)
                );
            }
        }
        eprintln!(
            "{label} cells {} holes {}",
            zoning.cells.len(),
            holes(&zoning.cells).len()
        );
        eprintln!("{}", map(core, &zoning.cells, junction, 140.0));
        zoning
    };
    let present: Vec<_> = arms
        .iter()
        .filter_map(|&(node, roles)| {
            let (arm, end) = far(&core, node);
            let state = core.get_road_pocket_internal(arm as i32, end)?;
            assert_eq!((state.pocket.left, state.pocket.right), roles);
            // The edit takes 10 m steps; the measured body rounds a little off one.
            assert_eq!((state.body_m / 10.0).round(), 6.0);
            Some(node)
        })
        .collect();
    if !present.is_empty() {
        report(&mut core, "dump");
    }
    for node in present {
        let (arm, end) = far(&core, node);
        assert_eq!(
            pocket_edit(&mut core, request(arm, end, (false, false), 60.0)),
            ""
        );
    }
    let control = report(&mut core, "control");
    if let Ok(save) = std::env::var("METRUM_R5D1_CONTROL_SAVE") {
        core.save_game_settled(&save, None).expect("save");
        return;
    }
    for &(node, roles) in &arms {
        let (arm, end) = far(&core, node);
        assert_eq!(pocket_edit(&mut core, request(arm, end, roles, 60.0)), "");
    }
    let pocketed = report(&mut core, "pocketed");
    // The table: per quadrant, near the crossing (both stations within 85 m: the pockets and
    // their tapers) or far, the control's and the pocketed counts, the cells lost and gained,
    // and the losses under road, cut off or under a moved cell, and unexplained.
    let class = |&(frame, x, y): &Cell| {
        let centre = frame.world(f64::from(x) + 0.5, f64::from(y) + 0.5) - junction;
        let (a, b) = (centre.dot(east), centre.dot(south));
        let quadrant = match (b < 0.0, a < 0.0) {
            (true, true) => "NW",
            (true, false) => "NE",
            (false, true) => "SW",
            (false, false) => "SE",
        };
        (quadrant, a.abs() <= 85.0 && b.abs() <= 85.0)
    };
    let lost: BTreeSet<Cell> = control.cells.difference(&pocketed.cells).copied().collect();
    let gained: BTreeSet<Cell> = pocketed.cells.difference(&control.cells).copied().collect();
    let unexplained = unexplained_losses(&core, &control.cells, &pocketed.cells);
    let road: BTreeSet<Cell> = lost
        .iter()
        .copied()
        .filter(|&cell| under_road(&core, cell))
        .collect();
    let centre = |&(frame, x, y): &Cell| frame.world(f64::from(x) + 0.5, f64::from(y) + 0.5);
    eprintln!("unexplained losses at {unexplained:?}");
    eprintln!(
        "| quadrant | reach | control | pocketed | lost | gained | lost under road | lost cut off or under a moved cell | unexplained |"
    );
    for quadrant in ["NW", "NE", "SW", "SE"] {
        for near in [true, false] {
            let count = |cells: &BTreeSet<Cell>| {
                cells
                    .iter()
                    .filter(|c| class(c) == (quadrant, near))
                    .count()
            };
            let unexplained_here = unexplained
                .iter()
                .filter(|point| {
                    lost.iter()
                        .find(|cell| centre(cell) == **point)
                        .is_some_and(|cell| class(cell) == (quadrant, near))
                })
                .count();
            eprintln!(
                "| {quadrant} | {} | {} | {} | {} | {} | {} | {} | {unexplained_here} |",
                if near { "near" } else { "far" },
                count(&control.cells),
                count(&pocketed.cells),
                count(&lost),
                count(&gained),
                count(&road),
                count(&lost) - count(&road) - unexplained_here,
            );
        }
    }
    if let Ok(save) = std::env::var("METRUM_R5D1_SAVE") {
        core.save_game_settled(&save, None).expect("save");
    }
    for &(node, _) in &arms {
        let (arm, end) = far(&core, node);
        assert_eq!(
            pocket_edit(&mut core, request(arm, end, (false, false), 60.0)),
            ""
        );
    }
    let mut removed = zoning(&mut core, junction);
    removed
        .frames
        .retain(|id, _| control.frames.contains_key(id));
    let control_frames: BTreeMap<_, _> = control
        .frames
        .iter()
        .filter(|(id, _)| removed.frames.contains_key(id))
        .map(|(&id, &frames)| (id, frames))
        .collect();
    eprintln!(
        "removed again: frames equal the control's {}, cells {}",
        removed.frames == control_frames,
        removed.cells == control.cells
    );
}

// A text map of `cells` around `centre`, one character per 2.5 m: a letter per frame, `#` on a
// road's carriageway, `.` for nothing. For the reproduction's log.
fn map(core: &SimCore, cells: &BTreeSet<Cell>, centre: DVec2, radius: f64) -> String {
    let step = 2.5;
    let frames: Vec<GridFrame> = cells
        .iter()
        .map(|&(frame, _, _)| frame)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let axes: Vec<(Vec<DVec2>, f64)> = core
        .region_graph
        .edges()
        .iter()
        .filter(|edge| !edge.deleted)
        .map(|edge| {
            let axis = edge.carriageway_axis(&edge.physical_geometry);
            (
                axis.iter()
                    .map(|p| DVec2::new(f64::from(p.x), f64::from(p.z)))
                    .collect(),
                f64::from(edge.width) * 0.5,
            )
        })
        .collect();
    let mut out = String::new();
    let count = (2.0 * radius / step) as i32;
    for row in 0..count {
        for column in 0..count {
            let point = centre - DVec2::splat(radius)
                + DVec2::new(f64::from(column) + 0.5, f64::from(row) + 0.5) * step;
            let on_road = axes.iter().any(|(axis, half)| {
                axis.windows(2).any(|pair| {
                    let (a, b) = (pair[0], pair[1]);
                    let t = ((point - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0);
                    point.distance(a + (b - a) * t) <= *half
                })
            });
            let cell = frames.iter().enumerate().find(|(_, frame)| {
                let local = frame.local(point).floor();
                cells.contains(&(**frame, local.x as i32, local.y as i32))
            });
            out.push(match (cell, on_road) {
                (Some((index, _)), _) => (b'a' + (index % 26) as u8) as char,
                (None, true) => '#',
                (None, false) => '.',
            });
        }
        out.push('\n');
    }
    out
}
