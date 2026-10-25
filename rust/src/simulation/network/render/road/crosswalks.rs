// SPDX-License-Identifier: GPL-2.0-only

//! Finnish zebra paint on the existing asphalt triangles, never an offset paint plane and never a
//! cut: the triangles under a crossing's bars carry its paint frame, and the shader draws the bars.
//!
//! Crossings come from the pieces ([`for_each_marking`]), never from lanes: the preview, which
//! builds no lanes, paints the commit's zebras (`ROAD-68-F`, `ROAD-68` open question 26).

use super::*;
use crate::simulation::network::lanes::CrosswalkMarking;

// Finnish L3_2 at urban speeds: planar metres, preserved from the existing marking contract.
pub(super) const CROSSWALK_STRIPE_WIDTH: f32 = 0.5;
pub(super) const CROSSWALK_STRIPE_LEN: f32 = 2.5;
const CROSSWALK_STRIPE_GAP: f32 = 0.5;

/// Local paint frame shared by the span and junction owning a crossing.
#[derive(Clone, Copy, Debug)]
pub(in crate::simulation::network::render) struct Crossing {
    origin: Vector3,
    across: Vector3,
    along: Vector3,
    width: f32,
    margin: f32,
    age: f32,
}

impl Crossing {
    fn new(marking: CrosswalkMarking, age: f32) -> Option<Self> {
        let delta = marking.end - marking.start;
        let length = Vector2::new(delta.x, delta.z).length();
        let step = CROSSWALK_STRIPE_WIDTH + CROSSWALK_STRIPE_GAP;
        let count = ((length + CROSSWALK_STRIPE_GAP) / step).floor();
        if count < 1.0 {
            return None;
        }
        let width = count * step - CROSSWALK_STRIPE_GAP;
        let margin = (length - width) * 0.5;
        let across = Vector3::new(delta.x / length, 0.0, delta.z / length);
        Some(Self {
            origin: marking.start + across * margin,
            across,
            along: Vector3::new(-across.z, 0.0, across.x),
            width,
            margin,
            age,
        })
    }

    fn uv(self, p: Vector3) -> Vector2 {
        let d = p - self.origin;
        Vector2::new(d.dot(self.across), d.dot(self.along))
    }

    // Whether a triangle meets the bar rectangle `[0, width] x [-half, half]` of the paint frame,
    // by separating axes: the rectangle's two, then the triangle's three edge normals. Touching
    // counts as apart; paint never reaches the rectangle's edge.
    fn meets_bars(self, vertices: [Vector3; 3]) -> bool {
        let uv = vertices.map(|p| self.uv(p));
        let half = CROSSWALK_STRIPE_LEN * 0.5;
        if uv.iter().all(|p| p.x <= 0.0)
            || uv.iter().all(|p| p.x >= self.width)
            || uv.iter().all(|p| p.y <= -half)
            || uv.iter().all(|p| p.y >= half)
        {
            return false;
        }
        let corners = [
            Vector2::new(0.0, -half),
            Vector2::new(self.width, -half),
            Vector2::new(self.width, half),
            Vector2::new(0.0, half),
        ];
        let winding = (uv[1] - uv[0]).cross(uv[2] - uv[0]);
        (0..3).all(|i| {
            let (a, b) = (uv[i], uv[(i + 1) % 3]);
            corners
                .iter()
                .any(|&corner| (b - a).cross(corner - a) * winding > 0.0)
        })
    }

    // CUSTOM0: stripe metres, longitudinal metres, age+1 (zero means no crossing), distance
    // from the kerb for wheel-path phase, including the symmetric margin.
    fn paint(self, p: Vector3) -> Vector4 {
        let uv = self.uv(p);
        Vector4::new(uv.x, uv.y, self.age + 1.0, uv.x + self.margin)
    }
}

/// Visits the zebra markings at `node`: its pieces' crossings
/// ([`RoadbedSystem::for_each_crossing`]) oriented as the pedestrian lanes own them
/// ([`CrosswalkMarking::of_arm`]). The one crossing record zebra paint and crossing signs read,
/// bitwise the kerb segment the node's pedestrian connectors walk, for a preview and a commit
/// alike. `O(degree)`.
pub(super) fn for_each_marking(
    graph: &RegionGraph,
    roadbed: &RoadbedSystem,
    node: u32,
    mut visit: impl FnMut(CrosswalkMarking),
) {
    roadbed.for_each_crossing(graph, node, |arm, kerbs| {
        visit(CrosswalkMarking::of_arm(arm, kerbs));
    });
}

/// Registers local crossing frames before emitting asphalt triangles.
pub(super) fn prepare_crosswalks(
    mesh: &mut NetworkMeshData,
    graph: &RegionGraph,
    roadbed: &RoadbedSystem,
    coverage: &super::standard_surface::CompiledSurfaceCoverage,
) {
    // A crossing may sit in a span chunk whose junction mesh is in the neighboring chunk.
    let mut nodes: BTreeSet<_> = coverage.node_ids.iter().copied().collect();
    for &edge_id in &coverage.edge_indices {
        let edge = graph.edge(edge_id);
        nodes.extend([edge.start_node, edge.end_node].map(|node| graph.get_valid_node(node)));
    }
    for node_id in nodes {
        for_each_marking(graph, roadbed, node_id, |marking| {
            let Some(crossing) =
                Crossing::new(marking, cosmetic_road_age(graph.edge(marking.edge_id)))
            else {
                return;
            };
            for owner in [
                NetworkMeshOwner::Node(node_id),
                NetworkMeshOwner::Edge(marking.edge_id),
            ] {
                mesh.crosswalks.entry(owner).or_default().push(crossing);
            }
        });
    }
}

/// Emits an asphalt triangle whole. A triangle that meets one of its owner's crossings' bar
/// rectangle carries that crossing's paint frame on its three vertices; the frame is affine in
/// XZ, so it interpolates exactly, and the shader draws only the bars (their count leaves less
/// than a bar's pitch between the outer bars and the kerbs). No triangle is cut, so both sides
/// of every asphalt edge keep the piece's vertices (`ROAD-68-D`, open question 24). The layout's
/// strips are disjoint and bounded by mesh rows, so at most one crossing meets a triangle.
/// `O(K)` for the owner's `K` local crossings, without allocation.
pub(super) fn emit_triangle(
    mesh: &mut NetworkMeshData,
    world: [RoadVec3; 3],
    uvs: [Vector2; 3],
    color: Color,
    normal: Vector3,
) {
    let vertices = world.map(render_point);
    let crossings = mesh
        .current_owner
        .and_then(|owner| mesh.crosswalks.get(&owner))
        .map_or(&[][..], Vec::as_slice);
    debug_assert!(
        crossings
            .iter()
            .filter(|crossing| crossing.meets_bars(vertices))
            .count()
            <= 1,
        "an asphalt triangle meets two crossings"
    );
    let paint = crossings
        .iter()
        .find(|crossing| crossing.meets_bars(vertices))
        .map_or([Vector4::ZERO; 3], |crossing| {
            vertices.map(|p| crossing.paint(p))
        });
    push_triangle_to_layer_with_crossing(mesh, MeshLayer::Road, world, uvs, color, normal, paint);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::lanes::LaneSystem;
    use crate::simulation::network::roadbed::fixtures::{self, connect, finalize, point};
    use crate::simulation::network::types::NodeType;

    // A straight two-way road through a pass-through node at the origin: one span-owned crossing.
    fn pass_through(sloped: bool) -> RegionGraph {
        let mut graph = RegionGraph::new();
        let hub = graph.add_node(point(0.0, 0.0, sloped), NodeType::Junction);
        let west = graph.add_node(point(-80.0, 0.0, sloped), NodeType::Junction);
        let east = graph.add_node(point(80.0, 0.0, sloped), NodeType::Junction);
        for (a, b) in [(west, hub), (hub, east)] {
            let edge = connect(&mut graph, a, b, 7.0, sloped);
            let edge = graph.edge_mut(edge);
            edge.fwd_lanes = 1;
            edge.bkw_lanes = 1;
        }
        finalize(&mut graph);
        graph
    }

    #[test]
    fn zebra_markings_and_their_connectors_share_one_kerb_segment() {
        // The crossing contract (`docs/roads.md` "Pedestrian Junction Crossings", `ROAD-68-F`):
        // the marking the renderer paints and signs, read from the pieces, is the lanes' own
        // crossing bitwise, kerbs in order, and both directions of its connector walk exactly
        // that segment.
        let bits = |p: Vector3| p.to_array().map(f32::to_bits);
        let mut cases = Vec::new();
        for sloped in [false, true] {
            cases.push((format!("t {sloped}"), fixtures::t_junction(sloped)));
            cases.push((format!("bend {sloped}"), fixtures::bend(60.0, 60.0, sloped)));
            cases.push((format!("pass-through {sloped}"), pass_through(sloped)));
            cases.extend(fixtures::footpath_fixtures(sloped));
        }
        let mut pass_through_crossings = 0;
        for (name, graph) in cases {
            let mut roadbed = RoadbedSystem::new(64.0);
            roadbed.compile_dirty(&graph);
            let mut lanes = LaneSystem::new();
            lanes.rebuild(&mut graph.clone());
            for node in 0..graph.node_count() as u32 {
                let mut drawn = Vec::new();
                for_each_marking(&graph, &roadbed, node, |marking| drawn.push(marking));
                pass_through_crossings +=
                    usize::from(!drawn.is_empty() && roadbed.node_piece(node).is_none());
                let connectors = lanes.node_lanes.get(&(node as usize));
                let connectors = connectors.map_or(&[][..], Vec::as_slice);
                let key = |marking: &CrosswalkMarking| {
                    (marking.edge_id, bits(marking.start), bits(marking.end))
                };
                let mut walked: Vec<_> = connectors
                    .iter()
                    .filter_map(|&lane| lanes.crosswalk_marking(lane))
                    .map(|marking| key(&marking))
                    .collect();
                let mut painted: Vec<_> = drawn.iter().map(key).collect();
                walked.sort_unstable();
                painted.sort_unstable();
                assert_eq!(
                    painted, walked,
                    "{name}: node {node} paints the lanes' crossings"
                );
                for marking in &drawn {
                    let walks: Vec<_> = connectors
                        .iter()
                        .filter(|&&lane| {
                            lanes.crosswalk(lane).map(|c| c.edge_id) == Some(marking.edge_id)
                        })
                        .map(|&lane| &lanes.lanes[lane].geometry)
                        .collect();
                    assert_eq!(walks.len(), 2, "{name}: node {node}, both directions");
                    for geometry in walks {
                        let segment = [geometry[1], geometry[2]].map(bits);
                        let forward = [marking.start, marking.end].map(bits);
                        let backward = [marking.end, marking.start].map(bits);
                        assert!(
                            segment == forward || segment == backward,
                            "{name}: node {node}, a connector leaves the zebra's kerb segment"
                        );
                    }
                }
            }
        }
        assert!(
            pass_through_crossings >= 2,
            "the pass-through fixtures cross on a span"
        );
    }

    #[test]
    fn zebra_paint_frames_whole_asphalt_triangles_under_its_bars() {
        // A diagonal, sloping road: crossing geometry must not inherit the crossfall tilt.
        let across = Vector3::new(0.6, 0.0, 0.8);
        let along = Vector3::new(-0.8, 0.0, 0.6);
        let point = |s: f32, l: f32| {
            let p = along * s + across * l;
            Vector3::new(p.x, 2.0 + s * 0.1 + l * 0.05, p.z)
        };
        let crossing = Crossing::new(
            CrosswalkMarking {
                edge_id: 0,
                start: point(0.0, -3.5),
                end: point(0.0, 3.5),
            },
            0.7,
        )
        .unwrap();
        assert!((crossing.width - 6.5).abs() < 1e-5);
        assert!((crossing.margin - 0.25).abs() < 1e-5);
        let mut mesh = NetworkMeshData::new();
        let owner = NetworkMeshOwner::Edge(0);
        mesh.set_owner(owner);
        mesh.crosswalks.insert(owner, vec![crossing]);
        // Under the bars, then beyond them.
        for (s0, s1) in [(-4.0, 4.0), (4.0, 8.0)] {
            push_quad(
                &mut mesh,
                MeshLayer::Road,
                [point(s0, -3.5), point(s0, 3.5), point(s1, 3.5), point(s1, -3.5)],
                [
                    Vector2::new(s0, -3.5),
                    Vector2::new(s0, 3.5),
                    Vector2::new(s1, 3.5),
                    Vector2::new(s1, -3.5),
                ],
                Color::from_rgba(0.0, 0.0, 0.0, 0.0), // junction asphalt: no road space
            );
        }
        assert!(mesh.marking_vertices.is_empty());
        // No cut: the four triangles come out whole.
        assert_eq!(mesh.road_vertices.len(), 12);
        assert_eq!(mesh.road_crossings.len(), mesh.road_vertices.len());
        for (index, (paint, uv)) in mesh.road_crossings.iter().zip(&mesh.road_uvs).enumerate() {
            if index >= 6 {
                assert_eq!(*paint, Vector4::ZERO);
                continue;
            }
            // The frame: metres across from the margin, metres along from the crossing line.
            assert!((paint.z - 1.7).abs() < 1e-5);
            assert!((paint.x - (uv.y + 3.25)).abs() < 1e-5);
            assert!((paint.y - uv.x).abs() < 1e-5);
            assert!((paint.w - paint.x - 0.25).abs() < 1e-5);
        }
    }
}
