// SPDX-License-Identifier: GPL-2.0-only

//! Mixed node earthwork visibility tests.

use super::*;

#[test]
fn mixed_standard_bridge_node_earthwork_visibility_is_owner_scoped() {
    let terrain = flat_terrain(97, 97);
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let standard_end = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    let bridge_end = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
    graph.add_edge(test_edge(
        center,
        standard_end,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        bridge_end,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 24.0)],
        10.0,
        EdgeClass::Bridge,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let piece = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .unwrap_or_else(|| {
            panic!(
                "mixed standard/bridge bend should compile a node piece: {}",
                canonical_node_pipeline_report(
                    &surface,
                    &graph,
                    center,
                    RoadSurfaceVisualNodePieceKind::Bend
                )
            )
        });

    let mut saw_standard_face = false;
    for face in &piece.render_earthwork_faces {
        let Some(edge_class) = node_earthwork_face_edge_class(piece, face.source) else {
            continue;
        };
        let visible = surface
            .node_earthwork_face_uses_visible_earthwork(&graph, &terrain, center, piece, face);
        match edge_class {
            EdgeClass::Standard => {
                saw_standard_face = true;
                assert!(
                    !visible,
                    "standard-owned node earthwork face must remain terrain/CDT-only"
                );
            }
            EdgeClass::Bridge => {
                assert!(
                    !visible,
                    "bridge-owned node earthwork face must not raise terrain or render fill"
                );
            }
            EdgeClass::Tunnel => {}
        }
    }

    assert!(
        saw_standard_face,
        "test setup should expose a standard-owned node boundary face"
    );
}

#[test]
fn mixed_standard_visible_tunnel_node_earthwork_visibility_is_owner_scoped() {
    let terrain = flat_terrain(97, 97);
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let standard_end = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    let tunnel_end = graph.add_node(Vector3::new(0.0, -6.0, 24.0), NodeType::Junction);
    graph.add_edge(test_edge(
        center,
        standard_end,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        tunnel_end,
        vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, 16.0),
            Vector3::new(0.0, -6.0, 24.0),
        ],
        10.0,
        EdgeClass::Tunnel,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let piece = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .unwrap_or_else(|| {
            panic!(
                "mixed standard/visible-tunnel bend should compile a node piece: {}",
                canonical_node_pipeline_report(
                    &surface,
                    &graph,
                    center,
                    RoadSurfaceVisualNodePieceKind::Bend
                )
            )
        });

    let mut saw_standard_face = false;
    let mut saw_tunnel_face = false;
    for face in &piece.render_earthwork_faces {
        let Some(edge_class) = node_earthwork_face_edge_class(piece, face.source) else {
            continue;
        };
        let visible = surface
            .node_earthwork_face_uses_visible_earthwork(&graph, &terrain, center, piece, face);
        match edge_class {
            EdgeClass::Standard => {
                saw_standard_face = true;
                assert!(
                    !visible,
                    "standard-owned node earthwork face must remain terrain/CDT-only"
                );
            }
            EdgeClass::Tunnel => {
                saw_tunnel_face = true;
                assert!(
                    visible,
                    "visible tunnel-owned node earthwork face should remain structural"
                );
            }
            EdgeClass::Bridge => {}
        }
    }

    assert!(
        saw_standard_face,
        "test setup should expose a standard-owned node boundary face"
    );
    assert!(
        saw_tunnel_face,
        "test setup should expose a visible tunnel-owned node boundary face"
    );
}
