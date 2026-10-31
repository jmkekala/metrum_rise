// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-61` road tunnels: the three probe strokes of `docs/roads.md` (quick validation, the
//! preview's exact plan and the commit agree), the tool's tunnel intent, buried pavement and
//! lanes, and the tunnel refusals.

use super::*;
use crate::nodes::sim::core::road_commit::RoadCommitRequest;
use crate::nodes::sim::core::road_preview::road_tool_snapshot_from_core;
use crate::simulation::network::roadbed::PieceId;
use crate::simulation::network::types::EdgeClass;

// The probes of `docs/roads.md` "Road tunnels": `(x, height)` at z = 0 on flat ground at 0.
const PROBES: [&[(f32, f32)]; 3] = [
    &[(-100.0, 0.0), (-50.0, -6.0), (50.0, -6.0), (100.0, 0.0)],
    &[(-50.0, -6.0), (50.0, -6.0)],
    &[(-100.0, 0.0), (-50.0, -6.0), (50.0, -6.0)],
];

fn probe_points(probe: &[(f32, f32)]) -> Vec<Vector3> {
    probe
        .iter()
        .map(|&(x, y)| Vector3::new(x, y, 0.0))
        .collect()
}

// A 256 m world, flat at height 0, with no roads.
fn flat_core() -> SimCore {
    let mut core = test_core();
    core.treasury.balance = 1.0e9;
    core.benchmark_mode = false;
    core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
    core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
    core.precompute_road_mesh_data();
    core
}

fn request(core: &SimCore, points: &[Vector3], tunnel: bool) -> RoadPreviewRequest {
    RoadPreviewRequest {
        surface_generation: core.road_tool_surface_generation,
        points: points.to_vec(),
        fwd_lanes: 1,
        bkw_lanes: 1,
        snap_to_existing_roads: false,
        tunnel,
    }
}

// What each path does with one stroke: the hover check, the preview's exact plan and the commit.
#[derive(Debug)]
struct ProbeResult {
    quick: &'static str,
    quick_heights: Vec<f32>,
    exact: &'static str,
    committed: String,
    // Each committed edge from west to east: class, lowest physical height, span vertices, lanes.
    edges: Vec<(EdgeClass, f32, usize, usize)>,
}

// Runs `points` through the hover check, the preview and the commit of `core`, the commit
// without the preview, so it plans afresh.
fn run_stroke(core: &mut SimCore, points: &[Vector3], tunnel: bool) -> ProbeResult {
    use crate::nodes::sim::core::road_tool_context::RoadPreviewContext;
    core.precompute_road_mesh_data();
    core.refresh_road_terrain_inputs();
    let view = road_tool_snapshot_from_core(core).expect("current roadbed");
    let (quick_points, quick) = view.validate(&request(core, points, tunnel));
    let reason = |valid: bool, reason: &'static str| if valid { "valid" } else { reason };
    let exact = RoadPreviewContext::capture(view)
        .preview_road(request(core, points, tunnel))
        .validation;
    let outcome = core.commit_road(
        RoadCommitRequest {
            points: points.to_vec(),
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: false,
            tunnel,
            preview: None,
        },
        std::time::Instant::now(),
        Default::default(),
    );
    core.precompute_road_mesh_data();
    let network = &core.transit_network;
    let mut edges: Vec<_> = core
        .region_graph
        .edges()
        .iter()
        .enumerate()
        .filter(|(_, edge)| !edge.deleted)
        .map(|(id, edge)| {
            let low = edge
                .physical_geometry
                .iter()
                .chain(&edge.geometry)
                .map(|point| point.y)
                .fold(f32::INFINITY, f32::min);
            let vertices = network
                .roadbed
                .piece_mesh(PieceId::Span(id))
                .map_or(0, |mesh| mesh.positions().len());
            let lanes = network.lane_system.edge_lanes.get(&id).map_or(0, Vec::len);
            let west_x = edge
                .geometry
                .iter()
                .map(|point| point.x)
                .fold(f32::INFINITY, f32::min);
            (west_x, (edge.class, low, vertices, lanes))
        })
        .collect();
    // Stroke order: every probe and hill stroke runs along +x.
    edges.sort_by(|a, b| a.0.total_cmp(&b.0));
    let edges = edges.into_iter().map(|(_, edge)| edge).collect();
    ProbeResult {
        quick: reason(quick.is_valid, quick.invalid_reason),
        quick_heights: quick_points.iter().map(|point| point.y).collect(),
        exact: reason(exact.is_valid, exact.invalid_reason),
        committed: if outcome.committed {
            "committed".to_owned()
        } else {
            outcome.rejection_reason
        },
        edges,
    }
}

// Without the tunnel intent the class comes from the stroke's two ends, as before `ROAD-61`'s
// fix (recorded on `dev` c2d49902, `docs/roads.md` "Road tunnels"): grounded ends send the
// first probe to the terrain-following solve, which commits a surface road at height 0 and
// erases the buried interior; the other two commit one tunnel edge with pavement and lanes.
// Quick validation, the preview and the commit agree on all three.
#[test]
fn tunnel_probes_without_tunnel_intent() {
    let expected = [
        (EdgeClass::Standard, 0.0),
        (EdgeClass::Tunnel, -6.0),
        (EdgeClass::Tunnel, -6.0),
    ];
    for (probe, (class, low)) in PROBES.iter().zip(expected) {
        let result = run_stroke(&mut flat_core(), &probe_points(probe), false);
        let case = format!("{result:?}");
        assert_eq!((result.quick, result.exact), ("valid", "valid"), "{case}");
        assert_eq!(result.committed, "committed", "{case}");
        assert_eq!(result.edges.len(), 1, "{case}");
        let (edge_class, edge_low, vertices, lanes) = result.edges[0];
        assert_eq!((edge_class, edge_low), (class, low), "{case}");
        assert!(vertices > 0 && lanes > 0, "{case}");
        if class == EdgeClass::Standard {
            assert!(result.quick_heights.iter().all(|&y| y == 0.0), "{case}");
        }
    }
}

// The edges of the drawn road in stroke order (the stroke runs along +x), with their classes.
fn stroke_classes(result: &ProbeResult) -> Vec<EdgeClass> {
    result.edges.iter().map(|edge| edge.0).collect()
}

// With the tunnel intent every probe keeps its authored profile: the first two lattice knots
// apart at most 4 m on the authored polyline, quick, exact and commit agree, the covered
// stretch commits as a tunnel edge between standard approaches (an end inside the cover is a
// buried continuation), and every edge publishes pavement and lanes.
#[test]
fn tunnel_probes_with_tunnel_intent() {
    let expected = [
        vec![EdgeClass::Standard, EdgeClass::Tunnel, EdgeClass::Standard],
        vec![EdgeClass::Tunnel],
        vec![EdgeClass::Standard, EdgeClass::Tunnel],
    ];
    for (probe, classes) in PROBES.iter().zip(expected) {
        let points = probe_points(probe);
        let result = run_stroke(&mut flat_core(), &points, true);
        let case = format!("{result:?}");
        assert_eq!((result.quick, result.exact), ("valid", "valid"), "{case}");
        assert_eq!(result.committed, "committed", "{case}");
        assert_eq!(stroke_classes(&result), classes, "{case}");
        let low = result
            .edges
            .iter()
            .map(|edge| edge.1)
            .fold(f32::INFINITY, f32::min);
        assert_eq!(low, -6.0, "{case}");
        assert!(
            result.edges.iter().all(|edge| edge.2 > 0 && edge.3 > 0),
            "{case}"
        );
        // The knots lie on the authored polyline: interior heights are never erased.
        assert!(
            result.quick_heights.len() >= points.len() && result.quick_heights.contains(&-6.0),
            "{case}"
        );
    }
}

// A 256 m world with a cone-shaped hill centred on the origin: flat at 0 beyond `foot_m`, rising
// at 1:2 to `0.5 * foot_m` at the top.
fn hill_core(foot_m: f32) -> SimCore {
    let mut core = flat_core();
    for z in 0..257 {
        for x in 0..257 {
            let r = (x as f32 - 128.0).hypot(z as f32 - 128.0);
            let height = (0.5 * (foot_m - r)).max(0.0);
            core.heightmap
                .set_height(x, z, height / crate::config::HEIGHT_SCALE);
        }
    }
    core.precompute_road_mesh_data();
    core
}

// The straight stroke through the hill, both ends on the ground beyond its foot.
const THROUGH_HILL: [Vector3; 2] = [
    Vector3::new(-100.0, 0.0, 0.0),
    Vector3::new(100.0, 0.0, 0.0),
];

// Acceptance 3: a straight stroke keeps the chord between its ends. Every physical point of the
// committed road lies on it, the approaches in a cut up to the bore clearance deep and the bore
// beneath at least that much cover. Undo removes the road.
#[test]
fn a_straight_tunnel_keeps_the_chord_through_a_hill() {
    let mut core = hill_core(80.0);
    let result = run_stroke(&mut core, &THROUGH_HILL, true);
    let case = format!("{result:?}");
    assert_eq!(result.committed, "committed", "{case}");
    assert_eq!(
        stroke_classes(&result),
        [EdgeClass::Standard, EdgeClass::Tunnel, EdgeClass::Standard],
        "{case}"
    );
    for edge in core
        .region_graph
        .edges()
        .iter()
        .filter(|edge| !edge.deleted)
    {
        for point in edge.physical_geometry.iter().chain(&edge.geometry) {
            assert!(point.y.abs() < 1.0e-3, "{case}: {point:?} off the chord");
        }
        let cover = |point: &Vector3| {
            core.heightmap.sample_height_world(point.x, point.z) * crate::config::HEIGHT_SCALE
                - point.y
        };
        if edge.class == EdgeClass::Tunnel {
            assert!(
                edge.geometry.iter().all(|point| cover(point) >= 5.0),
                "{case}"
            );
        }
    }
    // Without the intent the same stroke climbs the hill.
    let mut surface = hill_core(80.0);
    let climbed = run_stroke(&mut surface, &THROUGH_HILL, false);
    assert!(
        climbed.quick_heights.iter().any(|&y| y > 5.0),
        "{climbed:?}"
    );
    assert!(core.undo_action_internal());
    assert!(core.region_graph.edges().iter().all(|edge| edge.deleted));
}

// Acceptance 3: a curved stroke keeps its authored profile: every knot lies on the authored
// polyline, here a dip under the hill.
#[test]
fn a_curved_tunnel_keeps_its_authored_profile() {
    let mut core = hill_core(80.0);
    let points: Vec<Vector3> = (0..=50)
        .map(|i| {
            let x = -100.0 + 4.0 * i as f32;
            Vector3::new(
                x,
                -0.0004 * (100.0 * 100.0 - x * x),
                0.2 * x - 0.002 * x * x,
            )
        })
        .collect();
    let result = run_stroke(&mut core, &points, true);
    let case = format!("{result:?}");
    assert_eq!(result.committed, "committed", "{case}");
    // Every authored point is a knot, in order; the knots between interpolate them.
    let mut knots = result.quick_heights.iter();
    assert!(
        points.iter().all(|point| knots.any(|&y| y == point.y)),
        "{case}"
    );
    assert!(
        stroke_classes(&result).contains(&EdgeClass::Tunnel),
        "{case}"
    );
}

// Acceptance 4: a car drives the committed tunnel from one approach through the bore to the
// other approach's end.
#[test]
fn a_car_drives_through_the_tunnel() {
    use crate::simulation::network::lanes::LaneType;
    let mut core = hill_core(80.0);
    let result = run_stroke(&mut core, &THROUGH_HILL, true);
    assert_eq!(result.committed, "committed", "{result:?}");
    core.transit_network.install_pending_cch();
    let graph = &core.region_graph;
    let node_at = |x: f32| {
        graph
            .find_node_within(Vector3::new(x, 0.0, 0.0), 1.0)
            .expect("stroke end node")
    };
    let (start, end) = (node_at(-100.0), node_at(100.0));
    let (_, _, path) = core
        .transit_network
        .cch_graph
        .find_path(
            start,
            end,
            usize::MAX,
            usize::MAX,
            graph,
            crate::simulation::network::types::TransitFlags::CAR,
        )
        .expect("a car route through the tunnel");
    let first_edge = graph.get_edge_between_nodes(start, path[1]).unwrap();
    let lanes = &core.transit_network.lane_system;
    let lane_id = lanes.edge_lanes[&first_edge]
        .iter()
        .copied()
        .find(|&id| lanes.lanes[id].is_fwd && lanes.lanes[id].lane_type == LaneType::Vehicle)
        .expect("a forward vehicle lane");
    let at = lanes.lanes[lane_id].geometry[0];
    let core_ref = &mut core;
    let car = core_ref.agents.spawn_debug_car(
        path,
        &core_ref.transit_network.lane_system,
        lane_id,
        godot::prelude::Vector2::new(at.x, at.z),
    );
    core.time.speed_multiplier = 1.0;
    let (mut in_bore, mut max_x) = (false, f32::NEG_INFINITY);
    for _ in 0..3_000 {
        core.step_frame(1.0 / 30.0);
        let lane = core.agents.agents.current_lane_id[car];
        if lane != usize::MAX {
            let edge = core.transit_network.lane_system.lanes[lane].edge_id;
            in_bore |= core.region_graph.edge(edge).class == EdgeClass::Tunnel;
        }
        max_x = max_x.max(core.agents.agents.pos_x[car]);
    }
    assert!(in_bore, "the car entered the bore");
    assert!(
        max_x > 95.0,
        "the car reached the far approach's end: {max_x}"
    );
}

// Acceptance 5: a tunnel whose cover or approach lies under water is refused before and at the
// click, as are a tunnel crossing another one at its height and one passing under it, a stroke
// that never gets the bore clearance of cover, and one whose approach is shorter than two
// lattice steps. Nothing is repaired: the refused stroke leaves the graph as it was.
#[test]
fn invalid_tunnels_are_refused_with_a_reason() {
    let refused = |core: &mut SimCore, points: &[Vector3], reason: &str| {
        let edges = core.region_graph.edge_count();
        let result = run_stroke(core, points, true);
        assert_eq!(result.exact, reason, "{result:?}");
        assert_eq!(result.committed, reason, "{result:?}");
        assert_eq!(core.region_graph.edge_count(), edges);
        result
    };
    // A pond on the western approach.
    let mut core = hill_core(80.0);
    let mut water = crate::simulation::water::WaterSystem::with_chunking(257, 257, 1.0, 129);
    let mut depth = vec![0.0; 257 * 257];
    for z in 124..=132 {
        for x in 36..=46 {
            depth[x + z * 257] = 2.0;
        }
    }
    water.replace_baseline_depth_from_dense(&depth).unwrap();
    core.watermap = water;
    let result = refused(&mut core, &THROUGH_HILL, "tunnel_under_water");
    assert_eq!(result.quick, "tunnel_under_water");

    // Two tunnels crossing, at one height and 12 m apart.
    let mut core = hill_core(80.0);
    assert_eq!(
        run_stroke(&mut core, &THROUGH_HILL, true).committed,
        "committed"
    );
    let across = [
        Vector3::new(0.0, 0.0, -100.0),
        Vector3::new(0.0, 0.0, 100.0),
    ];
    refused(&mut core, &across, "tunnel_crossing");
    let below = [
        Vector3::new(0.0, -12.0, -50.0),
        Vector3::new(0.0, -12.0, 50.0),
    ];
    refused(&mut core, &below, "tunnel_crossing");

    // A hill 4 m high never gives 5 m of cover: the quick check refuses it too.
    let mut core = hill_core(8.0);
    let result = refused(&mut core, &THROUGH_HILL, "tunnel_cover");
    assert_eq!(result.quick, "tunnel_cover");

    // The ground reaches 5 m of cover 4 m from the stroke's start: no room for an approach.
    let mut core = hill_core(80.0);
    let short = [Vector3::new(-74.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)];
    let result = refused(&mut core, &short, "tunnel_approach_too_short");
    assert_eq!(result.quick, "tunnel_approach_too_short");
}

// Open (`ROAD-61`, found by round R2, pre-existing on `dev` for every tunnel): the visible-surface
// queries (`GradedSurface::surface_height_at` and its siblings: the cursor, zoning cell heights,
// vegetation, off-lane vehicles) take the highest road triangle at a point, so over a bore they
// answer the buried deck instead of the hill. The graded bake leaves the hill closed; the queries
// need the same rule. Ignored until that fix.
#[test]
#[ignore = "ROAD-61 open: surface queries read the buried deck over a bore"]
fn the_surface_over_a_bore_is_the_hill() {
    let mut core = hill_core(80.0);
    assert_eq!(
        run_stroke(&mut core, &THROUGH_HILL, true).committed,
        "committed"
    );
    core.precompute_road_mesh_data();
    for x in [-60.0, -30.0, 0.0, 30.0, 60.0] {
        let surface = core.get_world_surface_height_internal(godot::prelude::Vector2::new(x, 0.0));
        let hill = 0.5 * (80.0 - f32::abs(x));
        assert!(
            (surface - hill).abs() < 0.5,
            "surface {surface} over the bore at x {x}, hill {hill}"
        );
    }
}
