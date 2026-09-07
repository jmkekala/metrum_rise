// SPDX-License-Identifier: GPL-2.0-only

//! Independent buffered-guide reference and local-query scaling checks for ROAD-20.

use super::*;
use crate::simulation::network::graph::Edge;
use crate::simulation::network::types::NodeType;
use rstar::{AABB, PointDistance, RTree, RTreeObject};

#[derive(Clone, Copy)]
struct ReferenceSegment(RoadGhostSnapSegment);

impl RTreeObject for ReferenceSegment {
    type Envelope = AABB<[f32; 2]>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_corners(
            [
                self.0.start.x.min(self.0.end.x),
                self.0.start.y.min(self.0.end.y),
            ],
            [
                self.0.start.x.max(self.0.end.x),
                self.0.start.y.max(self.0.end.y),
            ],
        )
    }
}

impl PointDistance for ReferenceSegment {
    fn distance_2(&self, point: &[f32; 2]) -> f32 {
        let pos = Vector2::new(point[0], point[1]);
        (self.0.closest_point(pos) - pos).length_squared()
    }
}

// Preserve the previous buffered generator as an independent oracle, only in tests.
fn reference_segments(graph: &RegionGraph) -> Vec<ReferenceSegment> {
    let mut result = Vec::new();
    let mut push = |a, b| {
        if let Some(segment) = RoadGhostSnapSegment::new(a, b) {
            result.push(ReferenceSegment(segment));
        }
    };
    for edge in graph.edges().iter().filter(|edge| !edge.deleted) {
        let points = &edge.physical_geometry;
        if points.len() < 2 {
            continue;
        }
        let end = points.len() - 1;
        for (anchor, neighbor) in [(points[0], points[1]), (points[end], points[end - 1])] {
            if let Some(tangent) = endpoint_tangent_xz(anchor, neighbor) {
                let a = Vector2::new(anchor.x, anchor.z);
                push(a, a + tangent * GHOST_OUTWARD_EXTEND_M);
            }
        }
        for index in 1..=GHOST_MAX_OFFSETS {
            for sign in [1.0, -1.0] {
                let offset = index as f32 * GHOST_GRID_SPACING_M * sign;
                let mut buffered = Vec::new();
                for pair in points.windows(2) {
                    let a = Vector2::new(pair[0].x, pair[0].z);
                    let b = Vector2::new(pair[1].x, pair[1].z);
                    let delta = b - a;
                    if delta.length_squared() < 0.01 {
                        continue;
                    }
                    let direction = delta.normalized();
                    let normal = Vector2::new(-direction.y, direction.x);
                    let a = a + normal * offset;
                    let b = b + normal * offset;
                    if (b - a).dot(direction) >= 0.0 {
                        buffered.push((a, b));
                    }
                }
                let mut skip = false;
                for i in 0..buffered.len() {
                    if skip {
                        skip = false;
                        continue;
                    }
                    let (a, b) = buffered[i];
                    if let Some(&(c, d)) = buffered.get(i + 1)
                        && segments_cross_2d(a, b, c, d)
                    {
                        skip = true;
                        continue;
                    }
                    push(a, b);
                }
            }
        }
    }
    result
}

fn reference_nearest(segments: &[ReferenceSegment], pos: Vector2, radius: f32) -> Option<Vector2> {
    segments
        .iter()
        .map(|segment| segment.0.closest_point(pos))
        .filter(|point| (*point - pos).length_squared() < radius * radius)
        .min_by(|a, b| {
            (*a - pos)
                .length_squared()
                .total_cmp(&(*b - pos).length_squared())
                .then(a.x.total_cmp(&b.x))
                .then(a.y.total_cmp(&b.y))
        })
}

fn add_edge(graph: &mut RegionGraph, points: Vec<Vector3>) -> usize {
    let start = graph.add_node(points[0], NodeType::Junction);
    let end = graph.add_node(*points.last().unwrap(), NodeType::Junction);
    graph.add_edge(Edge {
        start_node: start,
        end_node: end,
        geometry: points.clone(),
        physical_geometry: points,
        ..Edge::default()
    })
}

#[test]
fn local_ghost_query_matches_buffered_reference_and_snapshot_edits() {
    let mut graph = RegionGraph::new();
    for shift in [Vector3::ZERO, Vector3::new(10000.0, 0.0, -10000.0)] {
        for points in [
            vec![Vector3::ZERO, Vector3::new(180.0, 4.0, 0.0)],
            vec![
                Vector3::new(-70.0, 0.0, 10.0),
                Vector3::ZERO,
                Vector3::new(2.0, 6.0, 3.0),
                Vector3::new(-50.0, 0.0, 40.0),
            ],
            vec![
                Vector3::ZERO,
                Vector3::ZERO,
                Vector3::new(0.02, 1.0, 0.01),
                Vector3::new(90.0, 0.0, -85.0),
            ],
        ] {
            add_edge(
                &mut graph,
                points.into_iter().map(|point| point + shift).collect(),
            );
        }
    }
    let old = graph.clone();
    graph.remove_from_spatial_index(0);
    graph.edge_mut(0).deleted = true;
    add_edge(
        &mut graph,
        vec![
            Vector3::new(10.0, 0.0, -100.0),
            Vector3::new(10.0, 0.0, 100.0),
        ],
    );
    for graph in [&old, &graph] {
        let reference = reference_segments(graph);
        let mut streamed = Vec::new();
        for edge in graph.edges().iter().filter(|edge| !edge.deleted) {
            visit_ghost_snap_segments(&edge.physical_geometry, &mut |segment| {
                streamed.push(segment)
            });
        }
        assert_eq!(reference.len(), streamed.len());
        for (a, b) in reference.iter().zip(&streamed) {
            assert_eq!((a.0.start, a.0.end), (b.start, b.end));
        }
        for shift in [Vector2::ZERO, Vector2::new(10000.0, -10000.0)] {
            for z in -14..=14 {
                for x in -14..=14 {
                    let pos = shift + Vector2::new(x as f32 * 23.17, z as f32 * 19.41);
                    assert_eq!(
                        nearest_road_ghost_point(graph, pos, 10.0),
                        reference_nearest(&reference, pos, 10.0)
                    );
                }
            }
        }
        // Query just inside the outermost offset and outward-end reach.
        for pos in [
            Vector2::new(90.0, 250.0_f32.next_down()),
            Vector2::new(-210.0_f32.next_up(), 0.0),
        ] {
            assert_eq!(
                nearest_road_ghost_point(graph, pos, 10.0),
                reference_nearest(&reference, pos, 10.0)
            );
        }
    }
}

#[test]
fn ghost_query_ties_do_not_depend_on_edge_index_order() {
    let mut graph = RegionGraph::new();
    for z in [-5.0, 5.0] {
        add_edge(
            &mut graph,
            vec![Vector3::new(0.0, 0.0, z), Vector3::new(90.0, 0.0, z)],
        );
    }
    let pos = Vector2::new(-50.0, 0.0);
    let expected = Some(Vector2::new(-50.0, -5.0));
    assert_eq!(nearest_road_ghost_point(&graph, pos, 10.0), expected);
    for edge in 0..2 {
        graph.remove_from_spatial_index(edge);
    }
    for edge in (0..2).rev() {
        graph.add_to_spatial_index(edge);
    }
    assert_eq!(nearest_road_ghost_point(&graph, pos, 10.0), expected);
    assert_eq!(nearest_road_ghost_point(&graph, pos, 0.0), None);
}

#[test]
#[ignore = "release-only diagnostic; run separately from other benchmarks"]
fn ghost_snap_query_scaling_diagnostic() {
    use std::hint::black_box;
    use std::time::Instant;
    for side in [8, 32] {
        let mut graph = RegionGraph::new();
        for z in 0..side {
            for x in 0..side {
                for (dx, dz) in [(1, 0), (0, 1)] {
                    if x + dx >= side || z + dz >= side {
                        continue;
                    }
                    let points = (0..=30)
                        .map(|i| {
                            Vector3::new(
                                (x as f32 + dx as f32 * i as f32 / 30.0) * 90.0,
                                0.0,
                                (z as f32 + dz as f32 * i as f32 / 30.0) * 90.0,
                            )
                        })
                        .collect();
                    add_edge(&mut graph, points);
                }
            }
        }
        let build = Instant::now();
        let reference = RTree::bulk_load(reference_segments(&graph));
        let build_ms = build.elapsed().as_secs_f64() * 1000.0;
        for (case, shift) in [
            ("dense", Vector2::new(270.0, 270.0)),
            ("remote", Vector2::new(-2000.0, -2000.0)),
        ] {
            let points: Vec<_> = (0..1000)
                .map(|i| shift + Vector2::new((i % 31) as f32 * 2.13, (i % 17) as f32 * 3.91))
                .collect();
            let measure = |local: bool| {
                let start = Instant::now();
                let mut sum = 0.0;
                for &pos in &points {
                    let result = if local {
                        nearest_road_ghost_point(black_box(&graph), black_box(pos), 10.0)
                    } else {
                        reference
                            .locate_within_distance([pos.x, pos.y], 100.0)
                            .map(|segment| segment.0.closest_point(pos))
                            .filter(|point| (*point - pos).length_squared() < 100.0)
                            .min_by(|a, b| {
                                (*a - pos)
                                    .length_squared()
                                    .total_cmp(&(*b - pos).length_squared())
                                    .then(a.x.total_cmp(&b.x))
                                    .then(a.y.total_cmp(&b.y))
                            })
                    };
                    sum += result.map_or(0.0, |point| point.x + point.y);
                }
                (
                    start.elapsed().as_secs_f64() * 1e6 / points.len() as f64,
                    black_box(sum),
                )
            };
            measure(false);
            measure(true);
            let before = measure(false);
            let after = measure(true);
            assert_eq!(before.1, after.1);
            println!(
                "ghost_query side={side} case={case} old_index_build_ms={build_ms:.3} old_query_us={:.3} local_query_us={:.3} checksum={}",
                before.0, after.0, after.1
            );
        }
    }
}
