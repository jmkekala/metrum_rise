// SPDX-License-Identifier: GPL-2.0-only

//! Minimal road graphs for roadbed tests: no lanes, zoning or terrain, only edges, nodes and the
//! graph clips that place the span mouths. Also the product probe ([`ProductProbe`]): bitwise
//! hashes of everything the roadbed publishes, which refactors must leave unchanged.

use super::{
    ArmKey, BandKind, EdgeEnd, NodeWalkways, PieceId, PieceMesh, RoadSurfaceTriangleQueryIndex,
    RoadVec2, RoadVec3, RoadbedSystem, triangulate_constrained_rings_xz,
};
use crate::config;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::lanes::LaneSystem;
use crate::simulation::network::types::{
    EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
};
use godot::prelude::Vector3;
use std::collections::HashSet;

/// Grade of the sloped fixtures along +X.
pub(crate) const SLOPE: f32 = 0.025;

/// A two-way road edge (car and foot) through `points`, used for geometry and physical profile.
pub(crate) fn road_edge(start_node: u32, end_node: u32, points: Vec<Vector3>, width: f32) -> Edge {
    let length = points
        .windows(2)
        .map(|segment| segment[0].distance_to(segment[1]))
        .sum();
    Edge {
        start_node,
        end_node,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width,
        fwd_lanes: ((width / config::LANE_WIDTH).round() as u8).max(1),
        bkw_lanes: 0,
        speed_limit: 50.0,
        base_cost: 0.0,
        physical_length: length,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: points.clone(),
        physical_geometry: points,
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    }
}

/// Point on the sloped plane `y = SLOPE * x`, or on the flat plane.
pub(crate) fn point(x: f32, z: f32, sloped: bool) -> Vector3 {
    Vector3::new(x, if sloped { SLOPE * x } else { 0.0 }, z)
}

/// Points from `a` to `b` with a knot every `step_m` metres.
pub(crate) fn polyline(a: (f32, f32), b: (f32, f32), step_m: f32, sloped: bool) -> Vec<Vector3> {
    let length = (b.0 - a.0).hypot(b.1 - a.1);
    let count = (length / step_m).ceil().max(1.0) as usize;
    (0..=count)
        .map(|i| {
            let t = i as f32 / count as f32;
            point(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, sloped)
        })
        .collect()
}

/// Adds an edge between two existing nodes along a knotted polyline.
pub(crate) fn connect(graph: &mut RegionGraph, a: u32, b: u32, width: f32, sloped: bool) -> usize {
    let (pa, pb) = (graph.node(a).pos, graph.node(b).pos);
    let points = polyline((pa.x, pa.z), (pb.x, pb.z), 20.0, sloped);
    graph.add_edge(road_edge(a, b, points, width))
}

/// A T junction at the origin: arms west, east and south of 60 m, then the graph clips.
/// Edges: 0 west to centre, 1 centre to east, 2 south to centre.
pub(crate) fn t_junction(sloped: bool) -> RegionGraph {
    let mut graph = RegionGraph::new();
    let center = graph.add_node(point(0.0, 0.0, sloped), NodeType::Junction);
    let west = graph.add_node(point(-60.0, 0.0, sloped), NodeType::Junction);
    let east = graph.add_node(point(60.0, 0.0, sloped), NodeType::Junction);
    let south = graph.add_node(point(0.0, 60.0, sloped), NodeType::Junction);
    connect(&mut graph, west, center, 7.0, sloped);
    connect(&mut graph, center, east, 7.0, sloped);
    connect(&mut graph, south, center, 7.0, sloped);
    graph.rebuild_intersection_clips();
    graph
}

/// A right-angle bend of two-lane roads with sidewalks at the origin, each arm ending at a road
/// end: edge 0 leaves the bend eastward for `first_m` (its legacy-first arm, D8), edge 1 arrives
/// from `second_m` to the north (+z). Then the graph clips.
pub(crate) fn bend(first_m: f32, second_m: f32, sloped: bool) -> RegionGraph {
    let mut graph = RegionGraph::new();
    let hub = graph.add_node(point(0.0, 0.0, sloped), NodeType::Junction);
    let east = graph.add_node(point(first_m, 0.0, sloped), NodeType::Junction);
    let north = graph.add_node(point(0.0, second_m, sloped), NodeType::Junction);
    for (a, b) in [(hub, east), (north, hub)] {
        let edge = connect(&mut graph, a, b, 7.0, sloped);
        let edge = graph.edge_mut(edge);
        edge.fwd_lanes = 1;
        edge.bkw_lanes = 1;
    }
    graph.rebuild_intersection_clips();
    graph
}

/// The T junction plus one edge of every other profile, apart from it: a road without
/// sidewalks, a footpath, a four-lane road and a bridge.
pub(crate) fn mixed_profiles(sloped: bool) -> RegionGraph {
    let mut graph = t_junction(sloped);
    let mut add = |a: (f32, f32), b: (f32, f32), edge: fn(Edge) -> Edge| {
        let start = graph.add_node(point(a.0, a.1, sloped), NodeType::Junction);
        let end = graph.add_node(point(b.0, b.1, sloped), NodeType::Junction);
        let points = polyline(a, b, 20.0, sloped);
        graph.add_edge(edge(road_edge(start, end, points, 7.0)))
    };
    add((-80.0, -60.0), (80.0, -50.0), |mut edge| {
        edge.allowed_types = TransitFlags::CAR;
        edge
    });
    add((-80.0, -100.0), (40.0, -140.0), |mut edge| {
        edge.primary_type = TransitType::Foot;
        edge.allowed_types = TransitFlags::FOOT;
        edge.width = 2.5;
        edge
    });
    add((100.0, -100.0), (100.0, 100.0), |mut edge| {
        edge.width = 14.0;
        edge.fwd_lanes = 2;
        edge.bkw_lanes = 2;
        edge
    });
    add((-100.0, 100.0), (-160.0, 20.0), |mut edge| {
        edge.class = EdgeClass::Bridge;
        edge
    });
    graph.rebuild_intersection_clips();
    graph
}

/// Footpath width in the footpath fixtures.
pub(crate) const FOOTPATH_M: f32 = 2.5;

// A node at the origin with arms of `(angle in degrees, footpath)`, each ending at a terminal:
// two-lane roads with sidewalks of 120 m and footpaths of 80 m. Even arms leave the hub from
// their edge's start, odd ones from its end.
fn footpath_star(arms: &[(f32, bool)], sloped: bool) -> RegionGraph {
    let mut graph = RegionGraph::new();
    let hub = graph.add_node(point(0.0, 0.0, sloped), NodeType::Junction);
    for (index, &(angle_deg, footpath)) in arms.iter().enumerate() {
        let length = if footpath { 80.0 } else { 120.0 };
        let (sin, cos) = angle_deg.to_radians().sin_cos();
        let far = graph.add_node(
            point(cos * length, sin * length, sloped),
            NodeType::Junction,
        );
        let (a, b) = if index % 2 == 0 {
            (hub, far)
        } else {
            (far, hub)
        };
        let edge = connect(&mut graph, a, b, 7.0, sloped);
        let edge = graph.edge_mut(edge);
        if footpath {
            edge.primary_type = TransitType::Foot;
            edge.allowed_types = TransitFlags::FOOT;
            edge.width = FOOTPATH_M;
            edge.fwd_lanes = 0;
        } else {
            edge.fwd_lanes = 1;
            edge.bkw_lanes = 1;
        }
    }
    finalize(&mut graph);
    graph
}

/// Computes the graph clips and runs the profile finalizer over every node and edge, as an edit
/// does: attached footpaths are pinned to sidewalk height (D5).
pub(crate) fn finalize(graph: &mut RegionGraph) {
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();
    let nodes: HashSet<u32> = (0..graph.node_count() as u32).collect();
    graph.finalize_junction_endpoint_profiles(&nodes);
}

/// Footpath junction fixtures at node 0 (D5), each with its name: a footpath in a T's corner and
/// beside one of its arms (at 30 and 20 degrees), at a road end straight on, to the side and on both sides (around angle
/// 60 degrees, so the cap's sector holds them out of angle order), across a straight road, on
/// both sides of a bend, and footpaths meeting only footpaths at degree one, two and three.
pub(crate) fn footpath_fixtures(sloped: bool) -> Vec<(String, RegionGraph)> {
    let road = |angle: f32| (angle, false);
    let path = |angle: f32| (angle, true);
    [
        (
            "t_corner",
            vec![road(0.0), road(180.0), road(270.0), path(315.0)],
        ),
        (
            "t_beside_arm",
            vec![road(0.0), road(180.0), road(270.0), path(300.0)],
        ),
        // 20 degrees off the arm: its centreline meets the walkway beyond the crossing line.
        (
            "t_beside_arm_shallow",
            vec![road(0.0), road(180.0), road(270.0), path(290.0)],
        ),
        ("road_end", vec![road(0.0), path(180.0)]),
        ("road_end_side", vec![road(0.0), path(90.0)]),
        (
            "road_end_both_sides",
            vec![road(60.0), path(30.0), path(90.0)],
        ),
        ("through_road", vec![road(0.0), road(180.0), path(270.0)]),
        ("bend", vec![road(0.0), road(90.0), path(45.0), path(225.0)]),
        ("path_end", vec![path(0.0)]),
        ("path_bend", vec![path(0.0), path(90.0)]),
        ("path_t", vec![path(0.0), path(180.0), path(270.0)]),
    ]
    .into_iter()
    .map(|(name, arms)| (format!("{name}/{sloped}"), footpath_star(&arms, sloped)))
    .collect()
}

/// FNV-1a hashes of one network's roadbed products, by component: graph clips and physical
/// profiles, node pieces (meshes, query grids, outer chains, mouth sections, crossings,
/// approaches, errors), layouts (their `Debug` form; a refactor that drops a dead field may change
/// it alone), span pieces, node walkways, lanes with their crossings, and surface queries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProductProbe {
    pub(crate) graph: u64,
    pub(crate) nodes: u64,
    pub(crate) layouts: u64,
    pub(crate) spans: u64,
    pub(crate) walkways: u64,
    pub(crate) lanes: u64,
    pub(crate) queries: u64,
}

#[derive(Clone, Copy)]
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn word(&mut self, value: u64) {
        for byte in value.to_le_bytes() {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn f32(&mut self, value: f32) {
        self.word(u64::from(value.to_bits()));
    }

    fn f64(&mut self, value: f64) {
        self.word(value.to_bits());
    }

    fn vec3(&mut self, point: Vector3) {
        point
            .to_array()
            .into_iter()
            .for_each(|value| self.f32(value));
    }

    fn road3(&mut self, point: RoadVec3) {
        point
            .to_array()
            .into_iter()
            .for_each(|value| self.f64(value));
    }

    fn debug(&mut self, value: &impl std::fmt::Debug) {
        let text = format!("{value:?}");
        self.word(text.len() as u64);
        text.bytes().for_each(|byte| self.word(u64::from(byte)));
    }

    fn mesh(&mut self, mesh: &PieceMesh) {
        self.word(mesh.positions().len() as u64);
        mesh.positions().iter().for_each(|&point| self.road3(point));
        for [station, lateral] in mesh.road_space() {
            self.f32(*station);
            self.f32(*lateral);
        }
        self.word(mesh.triangles().len() as u64);
        mesh.triangles()
            .iter()
            .flatten()
            .for_each(|&v| self.word(u64::from(v)));
        self.word(mesh.top_triangle_count() as u64);
        self.word(mesh.curb_face_triangles().len() as u64);
        for chain in mesh.outer_chains() {
            self.word(chain.vertices().len() as u64);
            chain
                .vertices()
                .iter()
                .for_each(|&v| self.word(u64::from(v)));
        }
        mesh.bounds_xz()
            .into_iter()
            .for_each(|value| self.f64(value));
        self.debug(mesh.grid());
    }
}

impl ProductProbe {
    /// Hashes `roadbed`'s published pieces over `graph`, the walkways of every node and `lanes`.
    pub(crate) fn of(graph: &RegionGraph, roadbed: &RoadbedSystem, lanes: &LaneSystem) -> Self {
        let (mut edges, mut spans) = (Fnv::new(), Fnv::new());
        for index in 0..graph.edge_count() {
            let Some(edge) = graph.get_edge(index).filter(|edge| !edge.deleted) else {
                continue;
            };
            edges.word(index as u64);
            edges.word(u64::from(edge.start_node) << 32 | u64::from(edge.end_node));
            edges.f32(edge.start_clip);
            edges.f32(edge.end_clip);
            edge.physical_geometry.iter().for_each(|&p| edges.vec3(p));
            if let Some(span) = roadbed.span_piece(index) {
                spans.word(index as u64);
                spans.mesh(span.mesh());
                spans.debug(&span.sections());
                spans.debug(span.start_mouth());
                spans.debug(span.end_mouth());
                spans.debug(&[EdgeEnd::Start, EdgeEnd::End].map(|end| span.crossing_strip_m(end)));
            }
        }
        let (mut nodes, mut layouts, mut walks) = (Fnv::new(), Fnv::new(), Fnv::new());
        for node in 0..graph.node_count() as u32 {
            nodes.word(u64::from(node));
            nodes.debug(&roadbed.node_error(node));
            if let Some(piece) = roadbed.node_piece(node) {
                nodes.debug(&piece.kind());
                nodes.mesh(piece.mesh());
                nodes.debug(&piece.mouths());
                for arm in &piece.layout().arms {
                    nodes.debug(&piece.crosswalk_kerbs(arm.key));
                    nodes.debug(&piece.crosswalk_approach(arm.key));
                }
                layouts.word(u64::from(node));
                layouts.debug(piece.layout());
            }
            walks.word(u64::from(node));
            let Some(walkways) = NodeWalkways::collect(graph, node) else {
                continue;
            };
            walks.word(u64::from(walkways.has_layout()));
            walkways
                .crossings()
                .for_each(|crossing| walks.debug(&crossing));
            for corner in walkways.corners() {
                walks.debug(&(corner.from, corner.to));
                corner.points.iter().for_each(|&p| walks.road3(p));
            }
            let incident = if (node as usize) < graph.node_adjacency_count() {
                graph.node_adjacency(node)
            } else {
                &[]
            };
            for &edge in incident {
                for end in [EdgeEnd::Start, EdgeEnd::End] {
                    let key = ArmKey { edge, end };
                    walks.debug(&[true, false].map(|side| walkways.sidewalk_end(key, side)));
                    walks.debug(&walkways.footpath_end(key));
                    let Some(join) = walkways.footpath_join(key) else {
                        continue;
                    };
                    walks.debug(&(join.corner.from, join.corner.to, &join.route, join.segment));
                    join.corner.points.iter().for_each(|&p| walks.road3(p));
                    join.walk.iter().for_each(|&p| walks.road3(p));
                    walks.road3(join.point);
                }
            }
        }
        Self {
            graph: edges.0,
            nodes: nodes.0,
            layouts: layouts.0,
            spans: spans.0,
            walkways: walks.0,
            lanes: lane_hash(lanes),
            queries: query_hash(roadbed),
        }
    }

    /// Lanes rebuilt on a copy of `graph`, then the roadbed compiled on `graph`: neither reads
    /// the other's products.
    pub(crate) fn of_graph(graph: &RegionGraph) -> Self {
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph.clone());
        let mut roadbed = RoadbedSystem::new(64.0);
        roadbed.compile_dirty(graph);
        Self::of(graph, &roadbed, &lanes)
    }
}

// Queries at every fifth top triangle's centroid of every piece: heights, bands, rays, discs and
// the lane surface; the generic grid and triangulation helpers over the same triangles and the
// node cores; and the bounds walks.
fn query_hash(roadbed: &RoadbedSystem) -> u64 {
    let mut hash = Fnv::new();
    let world = [-1.0e6, -1.0e6, 1.0e6, 1.0e6];
    let pieces: Vec<_> = roadbed.pieces_in_bounds(world).collect();
    for (id, generation) in pieces {
        hash.debug(&(id, generation));
        let (mesh, lane) = match id {
            PieceId::Span(edge) => (roadbed.span_piece(edge).unwrap().mesh(), (edge, usize::MAX)),
            PieceId::Node(node) => (
                roadbed.node_piece(node).unwrap().mesh(),
                (usize::MAX, node as usize),
            ),
        };
        let tops: Vec<[RoadVec3; 3]> = (0..mesh.top_triangle_count())
            .map(|triangle| mesh.triangle_points(triangle))
            .collect();
        let ground = RoadSurfaceTriangleQueryIndex::from_ground_triangles(tops.iter().copied());
        for triangle in tops.iter().step_by(5) {
            let centre = (triangle[0] + triangle[1] + triangle[2]) / 3.0;
            let (x, z) = (centre.x as f32, centre.z as f32);
            let xz = RoadVec2::new(centre.x, centre.z);
            hash.debug(&(roadbed.height_at(x, z), roadbed.carriageway_height_at(x, z)));
            hash.debug(&roadbed.band_at(x, z));
            hash.debug(&roadbed.raycast(Vector3::new(x, 100.0, z), Vector3::new(0.0, -1.0, 0.0)));
            let from = Vector3::new(x - 30.0, 50.0, z - 20.0);
            hash.debug(&roadbed.raycast(from, Vector3::new(30.0, -50.0, 20.0)));
            let carriageway = |kind| kind == BandKind::Carriageway;
            hash.debug(&roadbed.any_band_near_disc(xz, 0.75, carriageway));
            for carriageway_only in [false, true] {
                let query = roadbed.lane_query(lane.0, lane.1, carriageway_only);
                hash.debug(&query.height_at(x, z));
            }
            hash.debug(&ground.sample_ground_height(xz));
        }
        let mut visits = 0_u64;
        roadbed.any_top_triangle_in_bounds(mesh.bounds_xz(), |points, kind| {
            visits += 1;
            hash.road3(points[0]);
            hash.debug(&kind);
            false
        });
        hash.word(visits);
        if let PieceId::Node(node) = id {
            let core = &roadbed.node_piece(node).unwrap().layout().core;
            let ring: Vec<_> = core
                .iter()
                .map(|vertex| RoadVec3::new(vertex.position.x, 0.0, vertex.position.y))
                .collect();
            hash.debug(&triangulate_constrained_rings_xz(&[ring]));
        }
    }
    hash.0
}

fn lane_hash(lanes: &LaneSystem) -> u64 {
    let mut hash = Fnv::new();
    for (id, lane) in lanes.lanes.iter().enumerate() {
        hash.word(lane.edge_id as u64);
        hash.word(u64::from(lane.is_fwd));
        hash.word(lane.lane_idx as u64);
        lane.geometry.iter().for_each(|&p| hash.vec3(p));
        hash.f32(lane.length);
        lane.cum_dist.iter().for_each(|&d| hash.f32(d));
        hash.debug(&lane.lane_type);
        lane.next_lanes
            .iter()
            .for_each(|&next| hash.word(next as u64));
        hash.word(lane.node_id as u64);
        hash.debug(&lanes.crosswalk(id));
    }
    for map in [&lanes.edge_lanes, &lanes.node_lanes] {
        let mut keys: Vec<_> = map.keys().copied().collect();
        keys.sort_unstable();
        for key in keys {
            hash.word(key as u64);
            map[&key].iter().for_each(|&lane| hash.word(lane as u64));
        }
    }
    hash.0
}

/// Asserts each named probe against its recorded value; on any difference, prints the whole
/// table in this function's input format, so an intended change can be re-recorded.
pub(crate) fn assert_probes(actual: &[(String, ProductProbe)], expected: &[(&str, [u64; 7])]) {
    let table: Vec<String> = actual
        .iter()
        .map(|(name, p)| {
            format!(
                "(\"{name}\", [{:#018x}, {:#018x}, {:#018x}, {:#018x}, {:#018x}, {:#018x}, {:#018x}]),",
                p.graph, p.nodes, p.layouts, p.spans, p.walkways, p.lanes, p.queries
            )
        })
        .collect();
    let matches = actual.len() == expected.len()
        && actual
            .iter()
            .zip(expected)
            .all(|((name, p), (want, hashes))| {
                name == want
                    && [
                        p.graph, p.nodes, p.layouts, p.spans, p.walkways, p.lanes, p.queries,
                    ] == *hashes
            });
    assert!(
        matches,
        "product probe changed; actual:\n{}",
        table.join("\n")
    );
}
