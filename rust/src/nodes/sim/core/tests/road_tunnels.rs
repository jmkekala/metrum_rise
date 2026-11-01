// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-61` road tunnels: the three probe strokes of `docs/roads.md` (quick validation, the
//! preview's exact plan and the commit agree), the tool's tunnel intent, buried pavement and
//! lanes, and the tunnel refusals.

use super::*;
use crate::nodes::sim::core::road_commit::RoadCommitRequest;
use crate::nodes::sim::core::road_preview::road_tool_snapshot_from_core;
use crate::simulation::network::roadbed::{PieceId, RoadVec3, TUNNEL_LINING_HEIGHT_M};
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
    cone_hill_core(foot_m, 0.5)
}

// `hill_core` with the hill rising at `slope`.
fn cone_hill_core(foot_m: f32, slope: f32) -> SimCore {
    let mut core = flat_core();
    for z in 0..257 {
        for x in 0..257 {
            let r = (x as f32 - 128.0).hypot(z as f32 - 128.0);
            let height = (slope * (foot_m - r)).max(0.0);
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

// The natural height of `cone_hill_core(foot_m, slope)` at an XZ point.
fn cone_hill(foot_m: f32, slope: f32, x: f32, z: f32) -> f32 {
    (slope * (foot_m - x.hypot(z))).max(0.0)
}

// A tunnel through a 1:4 hill 25 m high, its bore from x = -80 to 80 (5 m of cover). The
// approaches' tie-in cones (1:2) cut the hill within 20 m of each mouth; beyond, the graded
// ground is the hill itself.
fn gentle_tunnel_core() -> SimCore {
    let mut core = cone_hill_core(100.0, 0.25);
    let result = run_stroke(&mut core, &THROUGH_HILL, true);
    assert_eq!(result.committed, "committed", "{result:?}");
    assert_eq!(
        stroke_classes(&result),
        [EdgeClass::Standard, EdgeClass::Tunnel, EdgeClass::Standard],
        "{result:?}"
    );
    core.precompute_road_mesh_data();
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    core
}

// `ROAD-61` (found by round R2): over a bore the visible surface is the hill, from portal row to
// portal row (round R8: no cone reaches past a portal row, so no opening cuts the ground over the
// deck). Every visible-surface query (the cursor's and the zoning cells' heights, off-lane
// vehicles, the agent inspector's ground) answered the buried deck over the whole bore.
#[test]
fn the_surface_over_a_bore_is_the_hill() {
    use crate::simulation::terrain::graded::GroundClass;
    let core = gentle_tunnel_core();
    for x in [-78.0, -50.0, -25.0, 0.0, 25.0, 50.0, 78.0] {
        for z in [0.0, 2.0] {
            let hill = cone_hill(100.0, 0.25, x, z);
            let heights = core.with_graded_surface(|surface, scratch| {
                let (x, z) = (f64::from(x), f64::from(z));
                let at = surface.surface_height_at(x, z, scratch);
                let (with_normal, _) = surface.surface_height_and_normal_at(x, z, scratch);
                surface.gather_near(x, z, 3.0, scratch);
                let near = surface.surface_height_near(x, z, scratch);
                let (near_normal, _) = surface.surface_height_and_normal_near(x, z, scratch);
                [at, with_normal, near, near_normal].map(|height| height as f32)
            });
            let world = core.get_world_surface_height_internal(godot::prelude::Vector2::new(x, z));
            for surface in heights.into_iter().chain([world]) {
                assert!(
                    (surface - hill).abs() < 0.01,
                    "surface {surface} over the bore at ({x}, {z}), hill {hill}"
                );
            }
        }
    }
    // 2 m into the bore the ground is the hill, unmasked; 2 m before each mouth the surface is
    // the approach's deck.
    for x in [-78.0, 78.0] {
        core.with_graded_surface(|surface, scratch| {
            let mask = surface.mask_at(f64::from(x), 0.0, scratch);
            assert!(
                matches!(mask, GroundClass::Natural | GroundClass::Graded),
                "{mask:?} at {x}"
            );
        });
    }
    for x in [-82.0, 82.0] {
        let deck = core
            .transit_network
            .roadbed
            .height_at(x, 0.0)
            .expect("the approach deck");
        core.with_graded_surface(|surface, scratch| {
            let (x, z) = (f64::from(x), 0.0);
            assert_eq!(surface.mask_at(x, z, scratch), GroundClass::Road);
            assert_eq!(surface.surface_height_at(x, z, scratch), f64::from(deck));
            assert_eq!(
                surface.surface_height_and_normal_at(x, z, scratch).0,
                f64::from(deck)
            );
            surface.gather_near(x, z, 3.0, scratch);
            assert_eq!(surface.surface_height_near(x, z, scratch), f64::from(deck));
        });
    }
}

// `ROAD-61` (found by round R2b, fixed by round R8): on a hill as steep as the 1:2 tie-in slope
// the approaches' cones cut the hill over the bore by up to the bore clearance (5 m) all the way
// to the 64 m cap, where the ground stepped back up to the hill (the pale rock stripes across the
// bore near the top of R2's test hill). No cone reaches past a portal row now, so the graded
// ground over the bore is the hill.
#[test]
fn a_steep_hill_over_a_bore_keeps_its_height() {
    let mut core = hill_core(80.0);
    assert_eq!(
        run_stroke(&mut core, &THROUGH_HILL, true).committed,
        "committed"
    );
    core.precompute_road_mesh_data();
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    for x in [-30.0, -8.0, 8.0, 30.0] {
        let ground = core.with_graded_surface(|surface, scratch| {
            surface.ground_height_at(f64::from(x), 0.0, scratch) as f32
        });
        let hill = cone_hill(80.0, 0.5, x, 0.0);
        assert!(
            (ground - hill).abs() < 0.01,
            "graded ground {ground} over the bore at x {x}, hill {hill}"
        );
    }
}

// Plants over the bore of `gentle_tunnel_core`, and over its approaches and openings, against
// the same hill without a road: `(x, z)` of every plant of both layers in the strip `|z| < 6`.
fn plants_along_the_stroke(core: &SimCore) -> Vec<(f32, f32)> {
    use crate::nodes::simulation_node::vegetation_api::{
        CANOPY_SALT, UNDERSTORY_SALT, scatter_layer,
    };
    let origin = godot::prelude::Vector2::new(-110.0, -110.0);
    let vegetation = &core.vegetation;
    [
        (vegetation.canopy_cell_m, CANOPY_SALT, true),
        (vegetation.understory_cell_m, UNDERSTORY_SALT, false),
    ]
    .into_iter()
    .flat_map(|(cell_m, salt, canopy)| scatter_layer(core, origin, 220.0, cell_m, salt, canopy))
    .collect::<Vec<_>>()
    .as_chunks::<6>()
    .0
    .iter()
    .map(|record| (record[0], record[2]))
    .filter(|&(_, z)| z.abs() < 6.0)
    .collect()
}

// A dense forest over the whole of `core`'s world.
fn forest(core: &mut SimCore) {
    use crate::simulation::vegetation::{VegetationConfig, VegetationGenerator};
    let config = VegetationConfig {
        coverage: 1.0,
        canopy_stems_per_ha: 400.0,
        ..VegetationConfig::default()
    };
    core.vegetation = VegetationGenerator::resolve(config, &core.config);
}

// `ROAD-61` (found by round R2): plants keep off the approach roads, and the hill over the bore
// keeps every plant it has without the tunnel, up to the portal rows (round R8). Vegetation read the
// roadbed's highest triangle, the buried deck, and cleared the whole bore across the hill.
#[test]
fn the_hill_over_a_bore_keeps_its_plants() {
    let mut core = gentle_tunnel_core();
    forest(&mut core);
    let mut hill = cone_hill_core(100.0, 0.25);
    forest(&mut hill);
    hill.allocator
        .prepare_building_site_query_index(hill.config.zone_cell_m);
    let (tunnel, natural) = (
        plants_along_the_stroke(&core),
        plants_along_the_stroke(&hill),
    );
    // Past the portal rows (at x = -80 and 80) and beyond a canopy footprint and the walkway
    // setback from the approaches the ground is the same hill with and without the tunnel.
    let over_bore = |plants: &[(f32, f32)]| -> Vec<(f32, f32)> {
        plants
            .iter()
            .copied()
            .filter(|&(x, _)| x.abs() < 74.0)
            .collect()
    };
    assert!(
        !over_bore(&natural).is_empty(),
        "the forest covers the hill"
    );
    assert_eq!(over_bore(&tunnel), over_bore(&natural));
    // The approach roads hold no plant.
    assert!(
        tunnel
            .iter()
            .all(|&(x, z)| !(x.abs() > 80.0 && x.abs() < 100.0 && z.abs() < 3.0)),
        "{tunnel:?}"
    );
}

// `ROAD-61` (reported by round R2, not reproduced by R2b): the agent inspector reports a car in
// a tunnel at its lane position, as the renderer draws it, not on the hill above it. Entering a
// lane sets the transit to `TRANSIT_NETWORK` or `TRANSIT_INTERSECTION`, and the inspector reads
// the lane in both, so the surface rule (the hill over the bore) never reaches a car on a lane.
#[test]
fn the_inspector_reports_a_car_in_a_tunnel_on_its_lane() {
    use crate::nodes::sim::render::agent_inspection::agent_position;
    use crate::nodes::sim::render::lane_pose::sample_lane_pose;
    use crate::simulation::network::lanes::LaneType;
    let mut core = gentle_tunnel_core();
    core.transit_network.install_pending_cch();
    let graph = &core.region_graph;
    let tunnel = graph
        .edges()
        .iter()
        .position(|edge| !edge.deleted && edge.class == EdgeClass::Tunnel)
        .unwrap();
    let start = graph.edge(tunnel).start_node;
    let end = graph
        .find_node_within(Vector3::new(100.0, 0.0, 0.0), 1.0)
        .expect("the stroke's east end");
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
    let lanes = &core.transit_network.lane_system;
    let lane_id = lanes.edge_lanes[&tunnel]
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
    let mut checked = 0;
    for _ in 0..3_000 {
        let lane_id = core.agents.agents.current_lane_id[car];
        if let Some(lane) = core.transit_network.lane_system.lanes.get(lane_id)
            && lane.edge_id != usize::MAX
            && core.region_graph.edge(lane.edge_id).class == EdgeClass::Tunnel
        {
            let (pose, _) = sample_lane_pose(lane, core.agents.agents.lane_distance[car]).unwrap();
            let reported = agent_position(&core, car);
            assert_eq!(reported, pose, "the car in the tunnel");
            // Away from the openings the hill stands over the car.
            let surface = core
                .get_world_surface_height_internal(godot::prelude::Vector2::new(pose.x, pose.z));
            assert!(
                pose.x.abs() > 70.0 || pose.y < surface - 4.0,
                "{pose:?} under {surface}"
            );
            checked += 1;
        }
        core.step_frame(1.0 / 30.0);
    }
    assert!(checked > 0, "the car drove through the tunnel");
}

// `ROAD-61` (checked by round R2b): a building fronts only a grounded road. Zoning cells already
// read standard edges only; a parcel the player places beside a tunnel's hill or a bridge
// attached to the structure's span, so its lot, driveway and entrance ran into the bore or off
// the deck.
#[test]
fn no_parcel_fronts_a_tunnel_or_a_bridge() {
    use crate::simulation::zoning::parcels::ParcelPlacementError;
    let place = |core: &mut SimCore, at: (f32, f32)| {
        let core = &mut *core;
        // A 10 m frontage fits the 20 m approaches.
        core.zoning
            .place_or_rezone_parcel_at(at.0, at.1, 0, 10.0, 15.0, &core.region_graph)
            .map(|id| {
                let edge = core.zoning.parcels.get(id).unwrap().edge_idx();
                core.region_graph.edge(edge).class
            })
    };
    let mut tunnel = gentle_tunnel_core();
    // Beside the bore on the hill, and beside the western approach for the control.
    assert_eq!(
        place(&mut tunnel, (0.0, 14.0)),
        Err(ParcelPlacementError::NoRoadAttachment)
    );
    assert_eq!(place(&mut tunnel, (-90.0, 14.0)), Ok(EdgeClass::Standard));
    let mut bridge = flat_core();
    let result = run_stroke(
        &mut bridge,
        &[
            Vector3::new(-100.0, 0.0, 0.0),
            Vector3::new(100.0, 0.0, 0.0),
        ],
        false,
    );
    assert_eq!(result.committed, "committed", "{result:?}");
    let edge = bridge
        .region_graph
        .edges()
        .iter()
        .position(|edge| !edge.deleted)
        .unwrap();
    bridge.set_edge_class_internal(edge as i32, 1);
    assert_eq!(bridge.region_graph.edge(edge).class, EdgeClass::Bridge);
    assert_eq!(
        place(&mut bridge, (0.0, 14.0)),
        Err(ParcelPlacementError::NoRoadAttachment)
    );
}

// The lining and headwall triangles of every published piece of `core` as world points, each
// with whether it is a headwall's, and the edges of every face the pieces publish (the deck's
// top and curb faces, the skirts and caps, the lining's, the headwalls'), each with the number of
// faces meeting it.
#[allow(clippy::type_complexity)]
fn lining_faces(
    core: &SimCore,
) -> (
    Vec<([RoadVec3; 3], bool)>,
    std::collections::HashMap<[[u64; 3]; 2], u32>,
) {
    let roadbed = &core.transit_network.roadbed;
    let ids = (0..core.region_graph.edge_count())
        .map(PieceId::Span)
        .chain((0..core.region_graph.node_count() as u32).map(PieceId::Node));
    let key = |p: RoadVec3| [p.x, p.y, p.z].map(f64::to_bits);
    let (mut faces, mut edges) = (Vec::new(), std::collections::HashMap::new());
    let mut count = |triangle: [RoadVec3; 3]| {
        for (a, b) in [(0, 1), (1, 2), (2, 0)] {
            let mut edge = [key(triangle[a]), key(triangle[b])];
            edge.sort();
            *edges.entry(edge).or_insert(0) += 1;
        }
    };
    for id in ids {
        let Some(mesh) = roadbed.piece_mesh(id) else {
            continue;
        };
        for triangle in mesh.triangles() {
            count(triangle.map(|v| mesh.positions()[v as usize]));
        }
        mesh.for_each_strip_triangle(|points, _| count(points));
        for (list, headwall) in [
            (mesh.lining_triangles(), false),
            (mesh.headwall_triangles(), true),
        ] {
            for triangle in list {
                let points = triangle.map(|v| mesh.lining_point(v));
                count(points);
                faces.push((points, headwall));
            }
        }
    }
    (faces, edges)
}

// `ROAD-61` round R8: the bore is closed around its deck. The tunnel through the 1:2 hill is
// lined from portal row to portal row, and a headwall stands at each row facing its approach.
// Every edge of the lining and the headwalls is shared with another face, the deck's, a curb
// face's, the lining's, a headwall's or the approach's skirt, bitwise, but for the headwalls'
// outer edges, which meet the terrain: no open boundary edge. The lining faces into the bore.
#[test]
fn the_lining_closes_the_bore() {
    let mut core = hill_core(80.0);
    let result = run_stroke(&mut core, &THROUGH_HILL, true);
    assert_eq!(result.committed, "committed", "{result:?}");
    let (faces, edges) = lining_faces(&core);
    let key = |p: RoadVec3| [p.x, p.y, p.z].map(f64::to_bits);
    // A headwall's outer points are the only lining points off the deck's rows and the lining
    // tops: its outer corners and the bottoms below its row's corners.
    let lining_points: std::collections::HashSet<_> = faces
        .iter()
        .filter(|(_, headwall)| !headwall)
        .flat_map(|(points, _)| points.map(key))
        .collect();
    let outer = |p: RoadVec3| !lining_points.contains(&key(p));
    let (mut lining, mut headwalls) = (0, 0);
    for &(points, headwall) in &faces {
        for (a, b) in [(0, 1), (1, 2), (2, 0)] {
            let (p, q) = (points[a], points[b]);
            if headwall && outer(p) && outer(q) {
                continue;
            }
            let mut edge = [key(p), key(q)];
            edge.sort();
            assert!(edges[&edge] >= 2, "an open lining edge {p:?} - {q:?}");
        }
        let [a, b, c] = points;
        let normal = -(b - a).cross(c - a);
        let centroid = (a + b + c) / 3.0;
        if headwall {
            // Out of the tunnel, towards its approach.
            assert!(normal.x * centroid.x > 0.0, "a headwall facing {normal:?}");
            headwalls += 1;
        } else {
            let axis = RoadVec3::new(centroid.x, 0.5 * TUNNEL_LINING_HEIGHT_M, 0.0);
            assert!(
                normal.dot(axis - centroid) > 0.0,
                "a lining face {points:?} facing {normal:?}"
            );
            lining += 1;
        }
    }
    assert!(lining > 50 && headwalls > 10, "{lining} lining, {headwalls} headwall");
}

// `ROAD-61` round R8: a bore that ends underground is closed by an end wall, and where two
// tunnels meet at a bend the node piece is lined, its lining sharing the spans' edges.
#[test]
fn the_lining_closes_buried_ends_and_bends() {
    let mut core = flat_core();
    let first = probe_points(PROBES[1]);
    assert_eq!(run_stroke(&mut core, &first, false).committed, "committed");
    // On from the east end, turning north.
    let outcome = core.commit_road(
        RoadCommitRequest {
            points: vec![Vector3::new(50.0, -6.0, 0.0), Vector3::new(60.0, -6.0, 60.0)],
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            tunnel: false,
            preview: None,
        },
        std::time::Instant::now(),
        Default::default(),
    );
    assert!(outcome.committed, "{}", outcome.rejection_reason);
    core.precompute_road_mesh_data();
    let graph = &core.region_graph;
    let tunnels = graph
        .edges()
        .iter()
        .filter(|edge| !edge.deleted && edge.class == EdgeClass::Tunnel)
        .count();
    assert_eq!(tunnels, 2);
    let roadbed = &core.transit_network.roadbed;
    let bend = graph
        .find_node_within(Vector3::new(50.0, -6.0, 0.0), 1.0)
        .expect("the bend");
    let node = roadbed.piece_mesh(PieceId::Node(bend)).expect("a bend piece");
    assert!(!node.lining_triangles().is_empty(), "the bend is lined");
    let (faces, edges) = lining_faces(&core);
    let key = |p: RoadVec3| [p.x, p.y, p.z].map(f64::to_bits);
    for (points, headwall) in &faces {
        assert!(!headwall, "no portal underground");
        for (a, b) in [(0, 1), (1, 2), (2, 0)] {
            let mut edge = [key(points[a]), key(points[b])];
            edge.sort();
            assert!(
                edges[&edge] >= 2,
                "an open lining edge {:?} - {:?}",
                points[a],
                points[b]
            );
        }
    }
}

// `ROAD-61` round R8: the preview stages of a tunnel stroke through the 1:2 hill (lining,
// headwalls and the portal rows' terrain included), eight end poses along the far side, against
// the stage budgets (`docs/roads.md` "Stage budgets"). Medians and p95 over 50 repetitions after
// 10 warm-up ones.
#[test]
#[ignore = "controlled release timing: cargo test --release --lib benchmark_tunnel_preview_stages -- --ignored --nocapture"]
fn benchmark_tunnel_preview_stages() {
    use crate::nodes::sim::core::road_tool_context::RoadPreviewContext;
    use std::time::Instant;
    let mut core = hill_core(80.0);
    core.precompute_road_mesh_data();
    core.refresh_road_terrain_inputs();
    let mut context =
        RoadPreviewContext::capture(road_tool_snapshot_from_core(&core).expect("current roadbed"));
    let strokes: Vec<Vec<Vector3>> = (0..8)
        .map(|pose| {
            vec![
                THROUGH_HILL[0],
                Vector3::new(100.0 + 2.0 * pose as f32, 0.0, 1.5 * pose as f32),
            ]
        })
        .collect();
    let mut stages = [(); 5].map(|()| Vec::new());
    let mut lining_vertices = 0;
    for repetition in 0..60 {
        for points in &strokes {
            let start = Instant::now();
            let preview = context.preview_road(request(&core, points, true));
            let total_ms = start.elapsed().as_secs_f64() * 1000.0;
            assert!(preview.validation.is_valid, "{}", preview.validation.invalid_reason);
            let pieces = preview.pieces.as_ref().expect("the pose's pieces");
            assert!(pieces.graded.stage.is_ok(), "every pose bakes its graded tiles");
            if repetition == 0 {
                lining_vertices += pieces
                    .chunks
                    .iter()
                    .flat_map(|chunks| chunks.values())
                    .map(|chunk| chunk.overlay.concrete_vertices.len())
                    .sum::<usize>();
            }
            if repetition >= 10 {
                let times = preview.times;
                for (samples, ms) in stages.iter_mut().zip([
                    times.plan_ms,
                    times.roadbed_ms,
                    times.payload_ms,
                    times.graded_ms,
                    total_ms,
                ]) {
                    samples.push(ms);
                }
            }
        }
    }
    let [plan, roadbed, payload, graded, total] = stages.map(|mut samples| {
        samples.sort_by(f64::total_cmp);
        let at = |q: f64| samples[((samples.len() - 1) as f64 * q).round() as usize];
        (at(0.5), at(0.95))
    });
    eprintln!(
        "tunnel_preview_stages workers={} poses=8 samples=400 lining_vertices_per_pose={} \
         plan_ms={:.3}/{:.3} roadbed_ms={:.3}/{:.3} payload_ms={:.3}/{:.3} \
         graded_ms={:.3}/{:.3} total_ms={:.3}/{:.3}",
        crate::thread_pools::install(rayon::current_num_threads),
        lining_vertices / 8,
        plan.0,
        plan.1,
        roadbed.0,
        roadbed.1,
        payload.0,
        payload.1,
        graded.0,
        graded.1,
        total.0,
        total.1,
    );
}
