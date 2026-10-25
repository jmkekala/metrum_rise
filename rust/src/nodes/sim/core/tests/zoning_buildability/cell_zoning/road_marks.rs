// SPDX-License-Identifier: GPL-2.0-only

//! A road commit marks the cell lots of the roads whose geometry or curb frames changed
//! (`ROAD-68-G` part 3); edits that leave their regions to the `cell lots` queue mark every road
//! the curb-frame refresh touched (part 4). Every road edit invalidates generated cells over its
//! roads' candidate rows, without the lot regions' parcel frontage reach (part 4). Every kind of
//! edit must leave the zoning products bit for bit as marking that whole ring with the wider
//! invalidation did.

use super::*;
use crate::nodes::sim::core::populated_city::zoned_city;
use crate::nodes::sim::core::road_commit::RoadCommitRequest;
use crate::simulation::network::graph::RegionGraph;
use std::time::Instant;

// The zoned street grid: rows every 50 m at z = -H..H between cross streets at x = -H, 0 and
// H, the middle row continuing west to the border as the trunk, every lot built.
const CITY_SIDE_M: f32 = 800.0;
const H: f32 = CITY_SIDE_M * 0.5;

// Digests of what the marks decide, with every chunk's cells generated first so that a cached
// chunk a narrower mark left valid is compared with the one a wider mark regenerated: paint and
// lot claims, the generated cells, parcels in store order, the queued lot regions, the curb
// frames and every building's attachment.
fn products(core: &mut SimCore) -> [u64; 6] {
    let (half_x, half_z) = core.heightmap.half_world_extents();
    let chunk = |value: f32| (value / RegionGraph::CHUNK_SIZE).floor() as i32;
    for x in chunk(-half_x)..=chunk(half_x) {
        for z in chunk(-half_z)..=chunk(half_z) {
            core.prepare_cell_chunk_internal((x, z));
        }
    }
    let hash = |value: &dyn std::fmt::Debug| crate::debug::work::debug_hash(value);
    let (claims, generated) = core.zoning.cells.digests();
    let attachments: Vec<_> = core
        .allocator
        .buildings
        .iter()
        .map(|b| (b.parcel_id, b.edge_idx, b.frontage_t.to_bits()))
        .collect();
    [
        claims,
        generated,
        hash(&core.zoning.parcels()),
        hash(&core.zoning.saved_cell_lot_queue()),
        hash(&core.zoning.cells.saved_road_alignments()),
        hash(&attachments),
    ]
}

fn live(graph: &RegionGraph, node: u32) -> bool {
    (node as usize) < graph.node_adjacency_count()
        && graph
            .node_adjacency(node)
            .iter()
            .any(|&edge| !graph.edge(edge).deleted)
}

fn node_at(core: &SimCore, x: f32, z: f32) -> u32 {
    let graph = &core.region_graph;
    let node = (0..graph.node_count() as u32)
        .filter(|&node| live(graph, node))
        .min_by(|&a, &b| {
            let distance = |node: u32| {
                let pos = graph.node(node).pos;
                (pos.x - x).hypot(pos.z - z)
            };
            distance(a).total_cmp(&distance(b))
        })
        .unwrap();
    let pos = graph.node(node).pos;
    assert!((pos.x - x).hypot(pos.z - z) < 1.0, "no node at ({x}, {z})");
    node
}

fn edge_between(core: &SimCore, a: (f32, f32), b: (f32, f32)) -> usize {
    let graph = &core.region_graph;
    let (a, b) = (node_at(core, a.0, a.1), node_at(core, b.0, b.1));
    graph
        .node_adjacency(a)
        .iter()
        .copied()
        .find(|&id| {
            let edge = graph.edge(id);
            !edge.deleted && {
                let ends = [edge.start_node, edge.end_node].map(|n| graph.get_valid_node(n));
                ends == [a, b] || ends == [b, a]
            }
        })
        .unwrap()
}

fn point(core: &SimCore, x: f32, z: f32) -> Vector3 {
    Vector3::new(x, core.heightmap.sample_height_world(x, z), z)
}

fn commit(core: &mut SimCore, from: (f32, f32), to: (f32, f32)) -> bool {
    let points = vec![point(core, from.0, from.1), point(core, to.0, to.1)];
    let outcome = core.commit_road(
        RoadCommitRequest {
            points,
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            preview: None,
        },
        Instant::now(),
        Default::default(),
    );
    assert!(outcome.committed, "{}", outcome.rejection);
    true
}

#[test]
fn narrow_road_marks_match_marking_the_whole_refreshed_ring() {
    let mut cores = [false, true].map(|full| {
        let mut core = zoned_city(CITY_SIDE_M);
        // The player's edits record undo.
        core.benchmark_mode = false;
        core.zoning.full_road_marks = full;
        core
    });
    assert!(cores[0].zoning.cells.has_reservations());
    let initial = cores.each_mut().map(products);
    assert_eq!(initial[0], initial[1]);

    let mut step = |name: &str, edit: &dyn Fn(&mut SimCore) -> bool| {
        let outcomes = cores.each_mut().map(edit);
        assert_eq!(outcomes[0], outcomes[1], "{name}: outcome");
        assert!(outcomes[0], "{name}: the edit must apply");
        let [narrow, full] = cores.each_mut().map(products);
        assert_eq!(narrow, full, "{name}: narrow marks against the whole ring");
    };

    // A T split of the trunk 100 m west of the grid, its undo, and the split again.
    let (x, end) = (-H - 100.0, -H - 94.0);
    step("T split", &|core| commit(core, (x, 0.0), (x, 120.0)));
    step("undo of the T split", &|core| core.undo_action_internal());
    step("T split again", &|core| commit(core, (x, 0.0), (x, 120.0)));
    // The branch's free end moves, then the branch extends straight on from it.
    step("node move", &|core| {
        let node = node_at(core, x, 120.0);
        core.move_network_node_internal(node as i32, point(core, end, 128.0))
    });
    step("straight extension", &|core| {
        let direction = DVec2::new(6.0, 128.0).normalize() * 80.0;
        commit(
            core,
            (end, 128.0),
            (end + direction.x as f32, 128.0 + direction.y as f32),
        )
    });
    // A painted block's cross street becomes a bridge and a street again.
    for (name, class) in [("class change to a bridge", 1), ("class change back", 0)] {
        step(name, &|core| {
            let edge = edge_between(core, (H, 50.0), (H, 100.0));
            core.set_edge_class_internal(edge as i32, class);
            core.region_graph.edge(edge).class
                == if class == 1 {
                    EdgeClass::Bridge
                } else {
                    EdgeClass::Standard
                }
        });
    }
    // A painted row's crossing at its west junction, off and on again.
    for (name, enabled) in [("crosswalk off", false), ("crosswalk on", true)] {
        step(name, &|core| {
            let node = node_at(core, -H, -100.0);
            let edge = edge_between(core, (-H, -100.0), (0.0, -100.0));
            core.set_crosswalk_override_internal(node, edge as i32, enabled);
            core.region_graph.node(node).crosswalk_overrides.get(&edge) == Some(&enabled)
        });
    }
    step("bulldoze of a painted block's cross street", &|core| {
        let edge = edge_between(core, (H, -150.0), (H, -100.0));
        core.bulldoze_road_edge_for_test(edge)
    });
    // A road beside the painted east edge of the grid, and a T off the trunk just west of the
    // painted rows.
    step("road beside painted cells", &|core| {
        commit(core, (H + 55.0, -120.0), (H + 55.0, 120.0))
    });
    // A T off its middle: every changed road is short, so each must mark its own corridor.
    step("T off the road beside painted cells", &|core| {
        commit(core, (H + 55.0, 0.0), (H + 135.0, 0.0))
    });
    step("T beside painted cells", &|core| {
        commit(core, (-H - 36.0, 0.0), (-H - 36.0, -70.0))
    });
    // Painted grid junctions move 1 m, an edge T and an interior crossing, and the moves are
    // undone.
    step("grid node move", &|core| {
        let node = node_at(core, H, 150.0);
        core.move_network_node_internal(node as i32, point(core, H + 1.0, 150.0))
    });
    step("interior node move", &|core| {
        let node = node_at(core, 0.0, -250.0);
        core.move_network_node_internal(node as i32, point(core, 1.0, -250.0))
    });
    step("undo of the interior move", &|core| {
        core.undo_action_internal()
    });
    step("undo of the grid node move", &|core| {
        core.undo_action_internal()
    });
}
