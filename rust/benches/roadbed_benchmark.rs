// SPDX-License-Identifier: GPL-2.0-only

//! Isolated roadbed span-piece build and publish costs (`ROAD-68-B`).
//!
//! Each case is one edge between two terminal nodes, so no junction plane or graph clip applies:
//! `build` measures sections, mesh, outer chains, triangle grid and chunk footprint for one span;
//! `publish` adds the dirty-closure compile and chunk-index update for that span inside a T
//! junction. The target is a 100 m span in 0.05 ms.

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use godot::prelude::Vector3;
use metrum_rise::simulation::network::graph::{Edge, RegionGraph};
use metrum_rise::simulation::network::roadbed::RoadbedSystem;
use metrum_rise::simulation::network::types::{
    EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
};
use std::time::Duration;

const SLOPE: f32 = 0.025;
// Physical knots every 12 m, the profile solver's sample spacing.
const KNOT_SPACING_M: f32 = 12.0;
const RENDER_CHUNK_SPAN_M: f32 = 512.0;

#[derive(Clone, Copy)]
enum Shape {
    Straight,
    // A circular arc turning 90 degrees over the span.
    Curved,
}

struct Case {
    name: &'static str,
    length_m: f32,
    shape: Shape,
    edge: fn(Edge) -> Edge,
}

fn standard(edge: Edge) -> Edge {
    edge
}

fn four_lane(mut edge: Edge) -> Edge {
    edge.width = 14.0;
    edge.fwd_lanes = 2;
    edge.bkw_lanes = 2;
    edge
}

fn rural(mut edge: Edge) -> Edge {
    edge.allowed_types = TransitFlags::CAR;
    edge
}

fn footpath(mut edge: Edge) -> Edge {
    edge.primary_type = TransitType::Foot;
    edge.allowed_types = TransitFlags::FOOT;
    edge.width = 2.5;
    edge.fwd_lanes = 0;
    edge.bkw_lanes = 0;
    edge
}

fn bridge(mut edge: Edge) -> Edge {
    edge.class = EdgeClass::Bridge;
    edge
}

const CASES: [Case; 8] = [
    Case {
        name: "span_25m",
        length_m: 25.0,
        shape: Shape::Straight,
        edge: standard,
    },
    Case {
        name: "span_100m",
        length_m: 100.0,
        shape: Shape::Straight,
        edge: standard,
    },
    Case {
        name: "span_400m",
        length_m: 400.0,
        shape: Shape::Straight,
        edge: standard,
    },
    Case {
        name: "span_100m_curved",
        length_m: 100.0,
        shape: Shape::Curved,
        edge: standard,
    },
    Case {
        name: "span_100m_four_lane",
        length_m: 100.0,
        shape: Shape::Straight,
        edge: four_lane,
    },
    Case {
        name: "span_100m_rural",
        length_m: 100.0,
        shape: Shape::Straight,
        edge: rural,
    },
    Case {
        name: "span_100m_footpath",
        length_m: 100.0,
        shape: Shape::Straight,
        edge: footpath,
    },
    Case {
        name: "span_100m_bridge",
        length_m: 100.0,
        shape: Shape::Straight,
        edge: bridge,
    },
];

fn centerline(length_m: f32, shape: Shape, origin: (f32, f32)) -> Vec<Vector3> {
    let count = (length_m / KNOT_SPACING_M).ceil().max(1.0) as usize;
    (0..=count)
        .map(|i| {
            let s = length_m * i as f32 / count as f32;
            let (x, z) = match shape {
                Shape::Straight => (s, 0.0),
                Shape::Curved => {
                    let radius = length_m / std::f32::consts::FRAC_PI_2;
                    let angle = s / radius;
                    (radius * angle.sin(), radius * (1.0 - angle.cos()))
                }
            };
            let (x, z) = (origin.0 + x, origin.1 + z);
            Vector3::new(x, SLOPE * x, z)
        })
        .collect()
}

fn edge_through(start_node: u32, end_node: u32, points: Vec<Vector3>) -> Edge {
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
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0 / 3.6,
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

fn add_edge(graph: &mut RegionGraph, points: Vec<Vector3>, edge: fn(Edge) -> Edge) -> usize {
    let start = graph.add_node(points[0], NodeType::Junction);
    let end = graph.add_node(points[points.len() - 1], NodeType::Junction);
    graph.add_edge(edge(edge_through(start, end, points)))
}

fn bench_roadbed_span(c: &mut Criterion) {
    let mut group = c.benchmark_group("roadbed_span");
    group.sample_size(200);
    group.warm_up_time(Duration::from_secs(2));
    group.measurement_time(Duration::from_secs(4));
    let roadbed = RoadbedSystem::new(RENDER_CHUNK_SPAN_M);
    for case in &CASES {
        let mut graph = RegionGraph::new();
        let edge = add_edge(
            &mut graph,
            centerline(case.length_m, case.shape, (10.0, 20.0)),
            case.edge,
        );
        let piece = roadbed.build_span_piece(&graph, edge).expect("span builds");
        assert!(piece.mesh().top_triangle_count() > 0);
        group.bench_function(BenchmarkId::new("build", case.name), |b| {
            b.iter(|| black_box(roadbed.build_span_piece(black_box(&graph), edge)))
        });
    }

    // Publish one 100 m arm of a T junction: closure, build, chunk index and generation.
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::ZERO, NodeType::Junction);
    let mut arm = |points: Vec<Vector3>, at_start: bool| {
        let far = points[if at_start { points.len() - 1 } else { 0 }];
        let far = graph.add_node(far, NodeType::Junction);
        let (start, end) = if at_start {
            (center, far)
        } else {
            (far, center)
        };
        graph.add_edge(edge_through(start, end, points))
    };
    let east = arm(centerline(100.0, Shape::Straight, (0.0, 0.0)), true);
    arm(centerline(100.0, Shape::Straight, (-100.0, 0.0)), false);
    arm(
        (0..=8)
            .map(|i| Vector3::new(0.0, 0.0, 12.5 * i as f32))
            .collect(),
        true,
    );
    graph.rebuild_intersection_clips();
    let mut published = RoadbedSystem::new(RENDER_CHUNK_SPAN_M);
    published.compile_dirty(&graph);
    group.bench_function(BenchmarkId::new("publish", "span_100m_t_arm"), |b| {
        b.iter(|| {
            published.mark_edge_dirty(east);
            black_box(published.compile_dirty(black_box(&graph)))
        })
    });
    group.finish();
}

criterion_group!(benches, bench_roadbed_span);
criterion_main!(benches);
