// SPDX-License-Identifier: GPL-2.0-only

//! Footpath junctions (D5, `ROAD-68` open question 14): footpaths attached to road nodes end on
//! the sidewalk ring's outer boundary on shared vertices, at sidewalk height, and footpaths
//! meeting only footpaths lay out as ordinary arms. Flat and on a 2.5% slope.

use super::*;
use crate::simulation::network::roadbed::fixtures::footpath_fixtures;
use crate::simulation::network::types::TransitType;

fn fixtures() -> Vec<(String, RegionGraph)> {
    [false, true]
        .into_iter()
        .flat_map(footpath_fixtures)
        .collect()
}

fn footpaths_at(graph: &RegionGraph, node: u32) -> usize {
    graph
        .node_adjacency(node)
        .iter()
        .filter(|&&edge| graph.edge(edge).primary_type == TransitType::Foot)
        .count()
}

#[test]
fn footpath_junctions_build_watertight_pieces() {
    for (name, graph) in fixtures() {
        let roadbed = compiled(&graph);
        let failed: Vec<_> = roadbed
            .failed_nodes()
            .iter()
            .map(|&node| (node, roadbed.node_error(node)))
            .collect();
        assert!(failed.is_empty(), "{name}: failed {failed:?}");
        let hub = roadbed.node_piece(0);
        let hub = hub.unwrap_or_else(|| {
            panic!(
                "{name}: no hub piece, {:?}",
                NodeArms::collect(&graph, 0).map(|arms| arms.layout())
            )
        });
        let degree = graph.node_adjacency(0).len();
        let paths = footpaths_at(&graph, 0);
        let attached = if paths < degree { paths } else { 0 };
        assert_eq!(hub.layout().attachments.len(), attached, "{name}");
        assert_eq!(hub.layout().arms.len(), degree - attached, "{name}");
        // One outer chain per sector, plus one per attached footpath.
        let chains = hub.mesh().outer_chains().len();
        assert_eq!(chains, hub.layout().sectors.len() + attached, "{name}");
        for piece in node_pieces(&roadbed, &graph) {
            check_watertight(&name, &roadbed, piece);
        }
        for edge in 0..graph.edge_count() {
            assert!(roadbed.span_piece(edge).is_some(), "{name}: span {edge}");
        }
    }
}

#[test]
fn footpath_junction_heights_are_the_plane_plus_level_and_footpaths_end_at_sidewalk_height() {
    for (name, graph) in fixtures() {
        let roadbed = compiled(&graph);
        let hub = roadbed.node_piece(0).unwrap();
        for &point in hub.mesh().positions() {
            let xz = RoadVec2::new(point.x, point.z);
            let levels = [Level::Ground, Level::Raised]
                .map(|level| f64::from(hub.surface_height_m(xz, level)).to_bits());
            assert!(levels.contains(&point.y.to_bits()), "{name}: {point:?}");
        }
        for attachment in &hub.layout().attachments {
            let mouth = hub.mouth(attachment.key).unwrap();
            // The mouth and the ribbon stand at sidewalk height, P + raised.
            for vertex in mouth.vertices.iter() {
                let raised = hub.surface_height_m(RoadVec2::new(vertex.x, vertex.z), Level::Raised);
                assert_eq!(vertex.y.to_bits(), f64::from(raised).to_bits(), "{name}");
            }
            // The finalized footpath centreline reaches it there (the 1 mm mouth check).
            let centre = mouth.section.center_xz;
            let raised = hub.surface_height_m(centre, Level::Raised);
            assert_eq!(mouth.section.center_height_m, raised, "{name}");
            // Its two edge points are vertices of the sidewalk's outer boundary, at its level.
            for edge in attachment.edges {
                let y = f64::from(hub.surface_height_m(edge.point, Level::Raised));
                let vertex = RoadVec3::new(edge.point.x, y, edge.point.y);
                assert!(hub.mesh().positions().contains(&vertex), "{name}");
            }
        }
    }
}

#[test]
fn attached_footpaths_end_on_the_outer_boundary_beyond_the_sidewalk() {
    for (name, graph) in fixtures() {
        let roadbed = compiled(&graph);
        let hub = roadbed.node_piece(0).unwrap();
        for attachment in &hub.layout().attachments {
            // The ribbon between boundary and mouth is footpath; the sidewalk lies inside it.
            let input_mouth = attachment.mouth_station_m;
            let along = attachment.direction;
            let probe = |station_m: f64| {
                let xz = hub.layout().node + along * station_m;
                roadbed.band_at(xz.x as f32, xz.y as f32)
            };
            let reach = attachment.edges[0]
                .station_m
                .max(attachment.edges[1].station_m);
            assert_eq!(
                probe(0.5 * (reach + input_mouth)),
                Some(BandKind::Footpath),
                "{name}"
            );
            let near = attachment.edges[0]
                .station_m
                .min(attachment.edges[1].station_m);
            assert_ne!(probe(near - 1.0), Some(BandKind::Footpath), "{name}");
        }
    }
}

#[test]
fn footpath_junctions_do_not_depend_on_edge_insertion_order() {
    for (name, forward) in fixtures() {
        let mut reordered = RegionGraph::new();
        for node in 0..forward.node_count() as u32 {
            reordered.add_node(forward.node(node).pos, NodeType::Junction);
        }
        // The same finalized edges in the opposite order.
        for edge in (0..forward.edge_count()).rev() {
            reordered.add_edge(forward.edge(edge).clone());
        }
        reordered.rebuild_adjacency_list();
        reordered.rebuild_intersection_clips();
        let a = compiled(&forward);
        let b = compiled(&reordered);
        assert_eq!(
            canonical_triangles(a.node_piece(0).unwrap()),
            canonical_triangles(b.node_piece(0).unwrap()),
            "{name}"
        );
    }
}

#[test]
fn a_footpath_edit_rebuilds_only_its_closure() {
    for (name, graph) in fixtures() {
        let Some(footpath) = (0..graph.edge_count())
            .find(|&edge| graph.edge(edge).primary_type == TransitType::Foot)
        else {
            continue;
        };
        let mut roadbed = compiled(&graph);
        let world = [f64::MIN, f64::MIN, f64::MAX, f64::MAX];
        let before: BTreeMap<_, _> = roadbed.pieces_in_bounds(world).collect();
        roadbed.mark_edge_dirty(footpath);
        roadbed.compile_dirty(&graph);
        let after: BTreeMap<_, _> = roadbed.pieces_in_bounds(world).collect();
        let edge = graph.edge(footpath);
        let nodes = [edge.start_node, edge.end_node];
        let closure: BTreeSet<usize> = nodes
            .iter()
            .flat_map(|&node| graph.node_adjacency(node).iter().copied())
            .collect();
        for (id, generation) in &after {
            let rebuilt = match id {
                PieceId::Node(node) => nodes.contains(node),
                PieceId::Span(edge) => closure.contains(edge),
            };
            assert_eq!(generation != &before[id], rebuilt, "{name}: {id:?}");
        }
    }
}
