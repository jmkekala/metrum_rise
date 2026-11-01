// SPDX-License-Identifier: GPL-2.0-only

//! The roadbed's terrain-facing contracts the graded terrain reads (`TERRAIN-07`): outer chains
//! keep the road on their left, and the top triangles' closed coverage predicate. Restored in
//! `TERRAIN-07-G` round 3 from the `ROAD-68-D` patch-loop tests, without the deleted loops.

use super::super::RoadbedSystem;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::LaneSystem;
use crate::simulation::network::roadbed::fixtures::{
    bend, connect, footpath_fixtures, point, t_junction,
};
use crate::simulation::network::roadbed::{PieceId, RoadVec2, RoadVec3};
use crate::simulation::network::types::{EdgeClass, NodeType};
use godot::prelude::Vector3;

fn compiled(graph: &RegionGraph) -> RoadbedSystem {
    let mut roadbed = RoadbedSystem::new(64.0);
    roadbed.compile_dirty(graph);
    assert!(roadbed.failed_nodes().is_empty(), "fixture nodes compile");
    roadbed
}

fn everything(roadbed: &RoadbedSystem) -> [f64; 4] {
    let mut rect = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for (id, _) in roadbed.pieces_in_bounds([-1.0e6, -1.0e6, 1.0e6, 1.0e6]) {
        let bounds = roadbed.pieces[&id].mesh().bounds_xz();
        rect = [
            rect[0].min(bounds[0]),
            rect[1].min(bounds[1]),
            rect[2].max(bounds[2]),
            rect[3].max(bounds[3]),
        ];
    }
    [rect[0] - 5.0, rect[1] - 5.0, rect[2] + 5.0, rect[3] + 5.0]
}

// A straight road through a pass-through node, a lone road between two road ends, and a
// square ring of four bends around a closed block.
fn straight_and_ring() -> Vec<(String, RegionGraph)> {
    let mut straight = RegionGraph::new();
    let a = straight.add_node(point(-80.0, 3.0, true), NodeType::Junction);
    let b = straight.add_node(point(0.0, 0.0, true), NodeType::Junction);
    let c = straight.add_node(point(90.0, -2.0, true), NodeType::Junction);
    connect(&mut straight, a, b, 7.0, true);
    connect(&mut straight, b, c, 7.0, true);
    straight.rebuild_intersection_clips();
    let mut ring = RegionGraph::new();
    let corners = [(-60.0, -60.0), (60.0, -60.0), (60.0, 60.0), (-60.0, 60.0)]
        .map(|(x, z)| ring.add_node(point(x, z, false), NodeType::Junction));
    for index in 0..4 {
        connect(
            &mut ring,
            corners[index],
            corners[(index + 1) % 4],
            7.0,
            false,
        );
    }
    ring.rebuild_intersection_clips();
    vec![("straight".to_owned(), straight), ("ring".to_owned(), ring)]
}

fn fixtures() -> Vec<(String, RegionGraph)> {
    let mut set = vec![
        ("t".to_owned(), t_junction(false)),
        ("t_sloped".to_owned(), t_junction(true)),
        ("bend".to_owned(), bend(60.0, 70.0, true)),
    ];
    set.extend(footpath_fixtures(true));
    set.extend(straight_and_ring());
    set
}

// Every grounded outer-chain segment has the road on its left and none on its right: the side
// the graded terrain's seams and the shader's exact mask take from the segment's cross product.
#[test]
fn every_chain_keeps_the_road_on_its_left() {
    for (name, graph) in fixtures() {
        let roadbed = compiled(&graph);
        let mut segments = 0;
        for (id, _) in roadbed.pieces_in_bounds(everything(&roadbed)) {
            let mesh = roadbed.pieces[&id].mesh();
            for chain in mesh.outer_chains() {
                for pair in chain.vertices().windows(2) {
                    let [a, b] = [pair[0], pair[1]].map(|v| mesh.positions()[v as usize]);
                    let direction = RoadVec2::new(b.x - a.x, b.z - a.z);
                    if direction.length() < 0.05 {
                        continue;
                    }
                    // Left is the counter-clockwise quarter turn from +x towards +z.
                    let left = direction.normalize().perp() * 0.02;
                    let middle = RoadVec2::new(a.x + b.x, a.z + b.z) * 0.5;
                    let probe = |side: RoadVec2| {
                        let at = middle + side;
                        roadbed.band_at(at.x as f32, at.y as f32).is_some()
                    };
                    assert!(probe(left), "{name}: road left of {a:?} -> {b:?}");
                    assert!(!probe(-left), "{name}: no road right of {a:?} -> {b:?}");
                    segments += 1;
                }
            }
        }
        assert!(segments > 0, "{name}: chains were checked");
    }
}

// The coverage predicate `TERRAIN-07` takes from the roadbed (`top_triangle_height_at`, which
// `height_at` uses) covers no point outside its triangle (`roads.md` "Footprint and terrain",
// found by `TERRAIN-07-D` round 2, fixed by `ROAD-68-K19`). It accepted points up to 0.1% of the
// triangle's height over an edge past that edge: 1.09 mm past the sloped T's outer chain at
// (64.011, 2.977), where the exact mask keeps the terrain. The probe stands a quarter of that
// band outside each edge's midpoint, strictly outside by exact orientation.
#[test]
fn the_coverage_predicate_covers_no_point_outside_its_triangle() {
    let xz = |v: RoadVec3| robust::Coord { x: v.x, y: v.z };
    for (name, graph) in fixtures() {
        let roadbed = compiled(&graph);
        let (mut probed, mut covered) = (0, 0);
        let mut first = None;
        for (id, _) in roadbed.pieces_in_bounds(everything(&roadbed)) {
            let mesh = roadbed.pieces[&id].mesh();
            for index in 0..mesh.top_triangle_count() {
                let triangle = mesh.triangle_points(index);
                let area = robust::orient2d(xz(triangle[0]), xz(triangle[1]), xz(triangle[2]));
                for edge in 0..3 {
                    let (p, q) = (triangle[edge], triangle[(edge + 1) % 3]);
                    let along = RoadVec2::new(q.x - p.x, q.z - p.z);
                    let height = area.abs() / along.length();
                    // Outward: away from the opposite vertex.
                    let outward = along.normalize().perp() * -area.signum();
                    let middle = RoadVec2::new(p.x + q.x, p.z + q.z) * 0.5;
                    let probe = middle + outward * (0.25e-3 * height);
                    let at = robust::Coord {
                        x: probe.x,
                        y: probe.y,
                    };
                    if robust::orient2d(xz(p), xz(q), at) * area >= 0.0 {
                        continue;
                    }
                    probed += 1;
                    if RoadbedSystem::top_triangle_height_at(&triangle, probe.x, probe.y).is_some()
                    {
                        covered += 1;
                        first.get_or_insert((id, index, probe));
                    }
                }
            }
        }
        assert!(probed > 0, "{name}: edges probed");
        assert_eq!(
            covered, 0,
            "{name}: {covered} of {probed} probes outside covered; first {first:?}"
        );
    }
}

// Whether the closed top triangle holds a point, from exact orientations alone: no orientation
// of the sign opposite the triangle's winding; a triangle without area holds nothing.
fn closed_holds(triangle: &[RoadVec3; 3], x: f64, z: f64) -> bool {
    let [a, b, c] = triangle.map(|v| robust::Coord { x: v.x, y: v.z });
    let at = robust::Coord { x, y: z };
    let area = robust::orient2d(a, b, c);
    area != 0.0
        && [(a, b), (b, c), (c, a)]
            .iter()
            .all(|&(p, q)| robust::orient2d(p, q, at) * area.signum() >= 0.0)
}

// Probes of a compiled fixture: every top triangle's vertices and edge midpoints, and points
// 1e-7 m and a quarter of the old 0.1% band inside and outside each edge's midpoint.
fn edge_probes(roadbed: &RoadbedSystem) -> Vec<RoadVec2> {
    let xz = |v: RoadVec3| robust::Coord { x: v.x, y: v.z };
    let mut probes = Vec::new();
    for (id, _) in roadbed.pieces_in_bounds(everything(roadbed)) {
        let mesh = roadbed.pieces[&id].mesh();
        for index in 0..mesh.top_triangle_count() {
            let triangle = mesh.triangle_points(index);
            let area = robust::orient2d(xz(triangle[0]), xz(triangle[1]), xz(triangle[2]));
            for edge in 0..3 {
                let (p, q) = (triangle[edge], triangle[(edge + 1) % 3]);
                probes.push(RoadVec2::new(p.x, p.z));
                let along = RoadVec2::new(q.x - p.x, q.z - p.z);
                let middle = RoadVec2::new(p.x + q.x, p.z + q.z) * 0.5;
                probes.push(middle);
                if area == 0.0 {
                    continue;
                }
                let outward = along.normalize().perp() * -area.signum();
                let band = 0.25e-3 * area.abs() / along.length();
                for offset in [1.0e-7, band] {
                    probes.extend([middle + outward * offset, middle - outward * offset]);
                }
            }
        }
    }
    probes
}

// The top triangles whose piece grid cells hold `probe`, unique by bits: every triangle that can
// hold it.
fn candidates_at(roadbed: &RoadbedSystem, probe: RoadVec2, out: &mut Vec<[RoadVec3; 3]>) {
    let bits = |t: &[RoadVec3; 3]| t.map(|v| [v.x, v.y, v.z].map(f64::to_bits));
    out.clear();
    roadbed.any_top_triangle_points_in_bounds([probe.x, probe.y, probe.x, probe.y], |t| {
        out.push(t);
        false
    });
    out.sort_by_key(bits);
    out.dedup_by_key(|t| bits(t));
}

// Coverage (the top triangles under the roadbed's predicate, as `TERRAIN-07` reads them) equals
// "inside some closed top triangle" exactly: at every probe, on an edge or vertex, a hair inside
// or outside one, each triangle holds the probe exactly when its closed region does by exact
// orientation, and a probe on an edge shared by two triangles is held by both.
#[test]
fn coverage_equals_inside_some_top_triangle_exactly() {
    let mut candidates = Vec::new();
    for (name, graph) in fixtures() {
        let roadbed = compiled(&graph);
        let (mut held, mut shared) = (0, 0);
        for probe in edge_probes(&roadbed) {
            candidates_at(&roadbed, probe, &mut candidates);
            let mut holders = 0;
            for triangle in &candidates {
                let covers =
                    RoadbedSystem::top_triangle_height_at(triangle, probe.x, probe.y).is_some();
                assert_eq!(
                    covers,
                    closed_holds(triangle, probe.x, probe.y),
                    "{name}: probe {probe:?} in {triangle:?}"
                );
                holders += usize::from(covers);
            }
            held += usize::from(holders > 0);
            shared += usize::from(holders > 1);
        }
        assert!(
            held > 0 && shared > 0,
            "{name}: {held} held, {shared} shared"
        );
    }
}

// The closed predicate depends on no direction in the world: the compiled triangles and the
// probes (every lane point of the fixture, and the edge probes above) turned by a quarter, a half
// and three quarters, and mirrored, all exactly in `f64`, give every probe the same holding
// triangles and the same highest height, bit for bit, so lane heights and coverage verdicts are
// the same in every orientation. The junction is turned after the compile, which is not itself
// bitwise rotation-equivariant (arm angles). A tie rule by world direction fails this on curb
// lines, where a curb's top and foot share an XZ edge.
#[test]
fn coverage_and_heights_are_the_same_in_every_quarter_turn_and_mirror() {
    let turns: [fn(RoadVec3) -> RoadVec3; 4] = [
        |p| RoadVec3::new(-p.z, p.y, p.x),
        |p| RoadVec3::new(-p.x, p.y, -p.z),
        |p| RoadVec3::new(p.z, p.y, -p.x),
        |p| RoadVec3::new(-p.x, p.y, p.z),
    ];
    let answer = |triangles: &[[RoadVec3; 3]], p: RoadVec3| {
        let mut held = Vec::new();
        let mut top: Option<f32> = None;
        for (index, t) in triangles.iter().enumerate() {
            if let Some(height) = RoadbedSystem::top_triangle_height_at(t, p.x, p.z) {
                held.push(index);
                top = Some(top.map_or(height, |best| best.max(height)));
            }
        }
        (held, top.map(f32::to_bits))
    };
    let mut candidates = Vec::new();
    for (name, graph) in fixtures() {
        let roadbed = compiled(&graph);
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph.clone());
        let lane_points = lanes
            .lanes
            .iter()
            .flat_map(|lane| lane.geometry.iter())
            .map(|p| RoadVec2::new(f64::from(p.x), f64::from(p.z)));
        let probes: Vec<_> = lane_points.chain(edge_probes(&roadbed)).collect();
        let mut on_curbs = 0;
        for probe in probes {
            candidates_at(&roadbed, probe, &mut candidates);
            let at = RoadVec3::new(probe.x, 0.0, probe.y);
            let expected = answer(&candidates, at);
            on_curbs += usize::from(expected.0.len() > 1);
            for turn in turns {
                let turned: Vec<_> = candidates.iter().map(|t| t.map(turn)).collect();
                assert_eq!(
                    answer(&turned, turn(at)),
                    expected,
                    "{name}: probe {probe:?}"
                );
            }
        }
        assert!(on_curbs > 0, "{name}: no probe on a shared edge");
    }
}

// `ROAD-61` round R8e: a tunnel attached to the far end of an existing road turns that road's
// span into an approach, whose seams the graded terrain retains. The span's chain vertices away
// from the mouth keep their bits, so without the flag in the key the far half of its segments
// cancelled out of the changes and the tiles there kept their cut. Every segment of the span is
// reported, and recompiling the same tunnel reports none.
#[test]
fn a_tunnel_at_a_roads_end_changes_every_approach_segment() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(point(-120.0, 0.0, false), NodeType::Junction);
    let mouth = graph.add_node(point(0.0, 0.0, false), NodeType::Junction);
    let road = connect(&mut graph, west, mouth, 7.0, false);
    graph.rebuild_intersection_clips();
    let mut roadbed = compiled(&graph);
    roadbed.take_terrain_changes();
    let segments = |roadbed: &RoadbedSystem| {
        let mesh = roadbed.piece_mesh(PieceId::Span(road)).expect("the road's span");
        let mut all = Vec::new();
        for chain in mesh.outer_chains() {
            for pair in chain.vertices().windows(2) {
                all.push([pair[0], pair[1]].map(|v| mesh.positions()[v as usize]));
            }
        }
        (mesh.retained(), all)
    };
    let (retained, before) = segments(&roadbed);
    assert!(!retained, "a road without a tunnel is no approach");

    let east = graph.add_node(Vector3::new(80.0, -8.0, 0.0), NodeType::Junction);
    let tunnel = connect(&mut graph, mouth, east, 7.0, false);
    let edge = graph.edge_mut(tunnel);
    edge.class = EdgeClass::Tunnel;
    for point in edge.geometry.iter_mut() {
        point.y = -8.0 * point.x / 80.0;
    }
    edge.physical_geometry = edge.geometry.clone();
    graph.rebuild_intersection_clips();
    roadbed.mark_edge_dirty(tunnel);
    roadbed.mark_node_dirty(mouth);
    roadbed.compile_dirty(&graph);
    assert!(roadbed.failed_nodes().is_empty());
    let (retained, after) = segments(&roadbed);
    assert!(retained, "the road is the tunnel's approach");
    let changes = roadbed.take_terrain_changes();
    let reported = |[a, b]: [RoadVec3; 2]| changes.segments.contains(&[a, b]);
    // The far end's segments keep their bits, and are reported all the same.
    let kept = after.iter().filter(|segment| before.contains(segment)).count();
    assert!(kept >= 4, "{kept} segments kept their bits");
    let missing: Vec<_> = after.iter().copied().filter(|&s| !reported(s)).collect();
    assert!(missing.is_empty(), "approach segments not reported: {missing:?}");

    roadbed.mark_edge_dirty(tunnel);
    roadbed.compile_dirty(&graph);
    assert!(roadbed.take_terrain_changes().segments.is_empty());
}
