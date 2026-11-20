// SPDX-License-Identifier: GPL-3.0-or-later

//! An empty joint between houses dissolves (`ROAD-118` round JS2): a street drawn as two roads
//! meeting at a joint, houses grown on its north curb on both sides of the joint, and a side
//! street from the south into the east road 12 m from the joint. The joint makes way for the T:
//! the piece between them joins the west road, and the house fronting it goes with it. Every house keeps its lot and its place, now fronting
//! the road under it at the station its centre projects to, on its own side; the paint is as it
//! was. Undo restores the city bit for bit and the stroke committed again builds it again. Both
//! roads are drawn either way, so the piece joins before or after the west road's own stations,
//! running with or against it, cut from the east road's start (which keeps its id on the rest) or
//! end.

use super::batch;
use super::*;

// The street in two roads meeting at the origin, each drawn eastward or westward, with houses on
// their north curbs over x -60 to 60. Returns the core, the west and east roads and
// the houses.
fn street_in_two(west_east: bool, east_east: bool) -> (SimCore, [usize; 2], Vec<usize>) {
    let mut core = test_core();
    core.benchmark_mode = false;
    core.treasury.balance = 1.0e9;
    register_houses(&mut core);
    let mut roads = [0; 2];
    for (road, (from, to, eastward)) in [(-150.0, 0.0, west_east), (0.0, 150.0, east_east)]
        .into_iter()
        .enumerate()
    {
        let (from, to) = if eastward { (from, to) } else { (to, from) };
        commit_ready(
            &mut core,
            vec![Vector3::new(from, 0.0, 0.0), Vector3::new(to, 0.0, 0.0)],
        );
        roads[road] = (0..core.region_graph.edge_count())
            .rev()
            .find(|&edge| !core.region_graph.edge(edge).deleted)
            .expect("the road just drawn");
    }
    let mut houses = grow(&mut core, roads[0], 1.0, [-60.0, -0.5]);
    houses.extend(grow(&mut core, roads[1], 1.0, [0.5, 60.0]));
    assert!(houses.len() >= 4, "houses along the street: {houses:?}");
    (core, roads, houses)
}

// `batch::products` without the cell store's state digest, which an undo of any road does not
// restore bit for bit (its queued work differs; a plain T 30 m on shows the same): the cells, their
// paint, claims and lots are in `land_state`'s cell digest.
fn products(core: &SimCore) -> Vec<u64> {
    let mut products = batch::products(core);
    products.remove(9);
    products
}

// Whether a house's centre lies at its frontage: on its side of its road (`side` 1 left of the
// road's forward direction, -1 right) and within a metre of the station its frontage parameter
// names along the road's length.
fn fronts_its_road(core: &SimCore, house: usize) -> bool {
    let b = &core.allocator.buildings[house];
    let road = core.region_graph.edge(b.edge_idx);
    let centre = Vector2::new(b.center_x, b.center_y);
    let (a, z) = (road.physical_geometry[0], *road.physical_geometry.last().unwrap());
    let (a, z) = (Vector2::new(a.x, a.z), Vector2::new(z.x, z.z));
    let along = (z - a).normalized();
    let station_m = (centre - a).dot(along);
    let right = (centre - a).dot(Vector2::new(-along.y, along.x)) > 0.0;
    !road.deleted
        && right == (b.side == -1)
        && (station_m - b.frontage_t * road.physical_length).abs() < 1.0
}

// Both roads drawn either way: the joint dissolves, the roads keep their ids and meet the T, every
// house fronts its road where it stands and keeps its lot, the paint stays; undo restores every
// product and the stroke committed again gives the same products.
#[test]
fn an_empty_joint_between_houses_dissolves_and_its_houses_follow() {
    let side_street = [Vector3::new(12.0, 0.0, -60.0), Vector3::new(12.0, 0.0, 0.0)];
    for (west_east, east_east) in [(true, true), (true, false), (false, true), (false, false)] {
        let label = format!("west eastward {west_east}, east eastward {east_east}");
        let (mut core, [west, east], houses) = street_in_two(west_east, east_east);
        core.adopt_pending_cell_lots();
        for &house in &houses {
            assert!(fronts_its_road(&core, house), "{label}: house {house} before");
        }
        let painted_before = painted(&core);
        let before = products(&core);
        let buildings = core.allocator.buildings.len();
        let joint = core
            .region_graph
            .find_node_within(Vector3::ZERO, 0.5)
            .expect("the joint");

        assert_eq!(commit(&mut core, &side_street), "", "{label}");
        core.adopt_pending_cell_lots();
        let graph = &core.region_graph;
        assert!(graph.get_valid_node(joint) != joint, "{label}: the joint dissolved");
        assert!(!graph.edge(west).deleted && !graph.edge(east).deleted, "{label}: ids kept");
        let t = graph
            .find_node_within(Vector3::new(12.0, 0.0, 0.0), 0.5)
            .expect("the T");
        for road in [west, east] {
            let edge = graph.edge(road);
            assert!([edge.start_node, edge.end_node].contains(&t), "{label}: road {road}");
        }
        assert_eq!(core.allocator.buildings.len(), buildings, "{label}: no house goes");
        for &house in &houses {
            assert!(fronts_its_road(&core, house), "{label}: house {house} after");
            let parcel = core
                .zoning
                .parcel_by_raw_id(core.allocator.buildings[house].parcel_id)
                .expect("its lot");
            assert_eq!(parcel.edge_idx(), core.allocator.buildings[house].edge_idx, "{label}");
        }
        assert_eq!(painted(&core), painted_before, "{label}: the paint stays");
        // The house whose frontage centre lies on the piece (x 10) moved to the west road with
        // it; the others kept their roads.
        for &house in &houses {
            let b = &core.allocator.buildings[house];
            let road = if b.center_x < 12.0 { west } else { east };
            assert_eq!(b.edge_idx, road, "{label}: house at x {}", b.center_x);
        }
        assert_references_consistent(&core);
        let after = products(&core);

        assert!(core.undo_action_internal(), "{label}");
        core.precompute_road_mesh_data();
        core.prepare_cell_lots_internal();
        assert_eq!(products(&core), before, "{label}: undo");
        assert_eq!(commit(&mut core, &side_street), "", "{label}: again");
        core.adopt_pending_cell_lots();
        assert_eq!(products(&core), after, "{label}: committed again");
    }
}

