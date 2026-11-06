// SPDX-License-Identifier: GPL-2.0-only

//! Exact topology/profile adoption, cold-commit parity and bounded-index regressions.

use super::*;
use crate::config::HEIGHT_SCALE;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::road_edit::PreparedRoadInput;
use crate::simulation::network::roadbed::{NodeArms, PieceId, RoadbedSystem};
use crate::simulation::network::roundabouts::{
    self, RoundaboutLayout, RoundaboutRequest, RoundaboutSite,
};
use crate::simulation::network::{build_surface_edge, road_types, types::EdgeClass};
use crate::simulation::terrain::TerrainSystem;

struct Fixture {
    graph: RegionGraph,
    network: TransitNetwork,
    terrain: TerrainSystem,
    zoning: ZoningSystem,
    allocator: BuildingAllocator,
}

impl Fixture {
    fn new(sloped: bool) -> Self {
        let mut terrain = TerrainSystem::with_chunking(256, 256, 1.0, 8, 0.0);
        if sloped {
            for z in 0..256 {
                for x in 0..256 {
                    terrain.set_height(x, z, (x as f32 * 0.025 + z as f32 * 0.012) / HEIGHT_SCALE);
                }
            }
        }
        let graph = RegionGraph::new();
        let mut network = TransitNetwork::new_with_surface_chunk_span(256.0);
        network.roadbed.compile_dirty(&graph);
        Self {
            graph,
            network,
            terrain,
            zoning: ZoningSystem::new(&WorldConfig::default()),
            allocator: BuildingAllocator::new(),
        }
    }

    fn point(&self, x: f32, z: f32) -> Vector3 {
        Vector3::new(x, self.terrain.sample_height_world(x, z) * HEIGHT_SCALE, z)
    }

    fn plan(&self, raw: &[Vector3]) -> (PreparedRoadInput, RoadTopologyPlan) {
        let prepared = prepare_road_input_for_tool(
            raw,
            &self.terrain,
            &self.graph,
            &self.network.roadbed,
            true,
            false,
        );
        let (validation, plan) = validate_road_edit(&prepared, 1, 1, &self.terrain, &self.graph);
        assert!(validation.is_valid, "{validation:?}");
        (
            prepared,
            plan.expect("complete local mutation scope must be adoptable"),
        )
    }

    fn cold_add(&mut self, prepared: &PreparedRoadInput) {
        self.network.bulk_load = true;
        self.network.add_road(
            &mut self.graph,
            prepared.points.clone(),
            1,
            1,
            prepared.class,
            &mut self.zoning,
            &mut self.allocator,
        );
        self.network
            .finalize_road_geometry(&mut self.graph, Some(&self.terrain));
        self.network.bulk_load = false;
        self.network.roadbed.compile_dirty(&self.graph);
        assert!(self.network.roadbed.is_current());
    }

    fn adopt(&mut self, plan: &RoadTopologyPlan) {
        let old_node_count = self.graph.node_count();
        let old_edge_count = self.graph.edge_count();
        let result = self
            .network
            .adopt_road_topology_plan(&mut self.graph, plan, &self.zoning, &mut self.allocator)
            .unwrap();
        assert_eq!((result.profile_us, result.clips_us), (0, 0));
        let mut next_node = old_node_count as u32;
        let node_ids: Vec<_> = plan
            .nodes
            .iter()
            .map(|node| {
                node.source.unwrap_or_else(|| {
                    let id = next_node;
                    next_node += 1;
                    id
                })
            })
            .collect();
        let mut next_edge = old_edge_count;
        for edge in &plan.edges {
            let id = edge.source.unwrap_or_else(|| {
                let id = next_edge;
                next_edge += 1;
                id
            });
            if let Some(expected) = &edge.geometry {
                let actual = self.graph.edge(id);
                assert_eq!(actual.start_node, node_ids[expected.start_node as usize]);
                assert_eq!(actual.end_node, node_ids[expected.end_node as usize]);
                assert_eq!(actual.geometry, expected.geometry);
                assert_eq!(actual.physical_geometry, expected.physical_geometry);
                assert_eq!(
                    (actual.start_clip, actual.end_clip),
                    (expected.start_clip, expected.end_clip)
                );
                assert_eq!(actual.deleted, expected.deleted);
            }
        }
        self.network.roadbed.compile_dirty(&self.graph);
        assert!(
            self.network.roadbed.is_current() && self.network.roadbed.failed_nodes().is_empty(),
            "{:?}",
            self.network.roadbed.failed_nodes()
        );
        // A full rebuild of derived indices is an independent oracle, never the adoption path.
        let mut rebuilt = self.graph.clone();
        rebuilt.rebuild_adjacency_list();
        assert_eq!(self.graph.adjacency(), rebuilt.adjacency());
        for (id, edge) in self
            .graph
            .edges()
            .iter()
            .enumerate()
            .filter(|(_, edge)| !edge.deleted)
        {
            let midpoint = edge.physical_geometry[edge.physical_geometry.len() / 2];
            assert!(self.graph.get_edges_near_point(midpoint, 1.0).contains(&id));
        }
    }
}

fn assert_cold_geometry_matches(actual: &Fixture, cold: &Fixture) {
    assert_eq!(actual.graph.node_count(), cold.graph.node_count());
    assert_eq!(actual.graph.edge_count(), cold.graph.edge_count());
    for (a, b) in actual.graph.edges().iter().zip(cold.graph.edges()) {
        assert_eq!(
            (a.start_node, a.end_node, a.deleted),
            (b.start_node, b.end_node, b.deleted)
        );
        assert_eq!(a.geometry, b.geometry);
        assert_eq!(a.physical_geometry, b.physical_geometry);
        assert_eq!(a.physical_length, b.physical_length);
        assert_eq!(a.base_cost, b.base_cost);
        assert_eq!((a.start_clip, a.end_clip), (b.start_clip, b.end_clip));
    }
    let (actual_bed, cold_bed) = (&actual.network.roadbed, &cold.network.roadbed);
    for id in 0..actual.graph.edge_count() {
        assert_eq!(
            actual_bed.sections(id),
            cold_bed.sections(id),
            "edge {id} sections"
        );
        assert_eq!(
            actual_bed.span_piece(id).map(|span| span.mesh()),
            cold_bed.span_piece(id).map(|span| span.mesh()),
            "edge {id} span mesh"
        );
    }
    for node in 0..actual.graph.node_count() as u32 {
        assert_eq!(
            actual_bed.node_piece(node).map(|piece| piece.mesh()),
            cold_bed.node_piece(node).map(|piece| piece.mesh()),
            "node {node} piece mesh"
        );
        assert_eq!(actual_bed.node_error(node), cold_bed.node_error(node));
    }
    let pieces = |roadbed: &RoadbedSystem| {
        roadbed
            .pieces_in_bounds([f64::MIN, f64::MIN, f64::MAX, f64::MAX])
            .map(|(piece, _)| piece)
            .collect::<Vec<_>>()
    };
    assert_eq!(pieces(actual_bed), pieces(cold_bed));
    assert_eq!(
        actual_bed.occupied_chunks().collect::<Vec<_>>(),
        cold_bed.occupied_chunks().collect::<Vec<_>>()
    );
}

#[test]
fn crossing_and_close_double_t_adopt_exact_final_profiles() {
    for strokes in [
        vec![((-60.0, 0.0), (60.0, 0.0)), ((0.0, -48.0), (0.0, 48.0))],
        vec![
            ((-60.0, 0.0), (60.0, 0.0)),
            ((-16.0, 48.0), (-16.0, 0.0)),
            ((16.0, -48.0), (16.0, 0.0)),
        ],
    ] {
        let mut planned = Fixture::new(true);
        let mut cold = Fixture::new(true);
        for ((sx, sz), (ex, ez)) in strokes {
            let raw = [planned.point(sx, sz), planned.point(ex, ez)];
            let (prepared, plan) = planned.plan(&raw);
            cold.cold_add(&prepared);
            planned.adopt(&plan);
            assert_cold_geometry_matches(&planned, &cold);
        }
    }
}

#[test]
fn hillside_crossing_keeps_existing_corridor_authority_after_splits() {
    let mut planned = Fixture::new(false);
    let mut cold = Fixture::new(false);
    for fixture in [&mut planned, &mut cold] {
        for z in 0..256 {
            for x in 0..256 {
                fixture
                    .terrain
                    // 30%: within the 1:3 maximum grade (`ROAD-68` open question 37).
                    .set_height(x, z, x as f32 * 0.3 / HEIGHT_SCALE);
            }
        }
    }
    for ((sx, sz), (ex, ez)) in [((-60.0, 0.0), (60.0, 0.0)), ((0.0, -48.0), (0.0, 48.0))] {
        let raw = [planned.point(sx, sz), planned.point(ex, ez)];
        let (prepared, plan) = planned.plan(&raw);
        cold.cold_add(&prepared);
        planned.adopt(&plan);
        assert_cold_geometry_matches(&planned, &cold);
        for edge in planned.graph.edges().iter().filter(|edge| !edge.deleted) {
            for point in &edge.physical_geometry {
                let source = planned.point(point.x, point.z);
                assert!(
                    (point.y - source.y).abs() < 0.05,
                    "splitting an existing hillside corridor must not flatten it: {point:?} vs {source:?}"
                );
            }
        }
    }
}

// A stroke from a road end joins it like any node (`ROAD-68` open question 30): the adopted plan
// equals the cold add, and the road it extends keeps its own profile.
#[test]
fn a_stroke_from_a_road_end_adopts_like_a_cold_add() {
    let mut planned = Fixture::new(false);
    let mut cold = Fixture::new(false);
    for fixture in [&mut planned, &mut cold] {
        let start = fixture
            .graph
            .add_node(Vector3::new(-48.0, 0.0, 0.0), NodeType::Junction);
        let end = fixture
            .graph
            .add_node(Vector3::new(0.0, 8.0, 0.0), NodeType::Junction);
        fixture.graph.add_edge(build_surface_edge(
            start,
            end,
            vec![
                fixture.graph.node(start).pos,
                Vector3::new(-24.0, 4.0, 0.0),
                fixture.graph.node(end).pos,
            ],
            1,
            1,
            EdgeClass::Standard,
            None,
            road_types::RoadAttributes::DEFAULT,
        ));
        fixture.network.roadbed.mark_edge_dirty(0);
        fixture.network.roadbed.compile_dirty(&fixture.graph);
        assert!(fixture.network.roadbed.is_current());
    }
    let (prepared, plan) =
        planned.plan(&[Vector3::new(0.0, 8.0, 0.0), Vector3::new(48.0, 0.0, 0.0)]);
    let extended = planned.graph.edge(0).geometry.clone();
    cold.cold_add(&prepared);
    planned.adopt(&plan);
    assert_cold_geometry_matches(&planned, &cold);
    assert_eq!(planned.graph.edge(0).geometry, extended);
}

#[test]
fn adoption_preserves_remote_geometry_and_live_metadata_and_rejects_missing_sources() {
    let mut fixture = Fixture::new(false);
    let remote = [
        Vector3::new(1000.0, 0.0, 1000.0),
        Vector3::new(1100.0, 0.0, 1000.0),
    ];
    let a = fixture.graph.add_node(remote[0], NodeType::Junction);
    let b = fixture.graph.add_node(remote[1], NodeType::Junction);
    fixture.graph.add_edge(build_surface_edge(
        a,
        b,
        remote.to_vec(),
        1,
        1,
        EdgeClass::Standard,
        None,
        road_types::RoadAttributes::DEFAULT,
    ));
    fixture.network.roadbed.compile_dirty(&fixture.graph);
    let (_, plan) = fixture.plan(&[Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)]);
    assert!(plan.source_topology_ids().0.is_empty());
    fixture.adopt(&plan);
    let (_, plan) = fixture.plan(&[Vector3::new(0.0, 0.0, -48.0), Vector3::new(0.0, 0.0, 48.0)]);
    assert!(!plan.source_topology_ids().0.contains(&0));
    let mut stale = fixture.graph.clone();
    stale.edges[1].deleted = true;
    let counts = (stale.node_count(), stale.edge_count());
    assert!(
        fixture
            .network
            .adopt_road_topology_plan(&mut stale, &plan, &fixture.zoning, &mut fixture.allocator)
            .is_none()
    );
    assert_eq!(counts, (stale.node_count(), stale.edge_count()));
    fixture.graph.edges[1].no_building_spawn = true;
    fixture.graph.edges[1].speed_limit = 9.0;
    fixture.adopt(&plan);
    assert_eq!(fixture.graph.edge(0).geometry, remote);
    assert!(fixture.graph.edge(1).no_building_spawn);
    let split = plan
        .splits
        .iter()
        .find(|split| plan.edges[split.edge_id].source == Some(1))
        .unwrap();
    let split_local = split.new_edge_id;
    // All source edges precede the candidate; this fixture has one omitted remote edge.
    for id in [1, split_local + 1] {
        let edge = fixture.graph.edge(id);
        assert_eq!(edge.speed_limit, 9.0);
        assert_eq!(
            edge.base_cost,
            crate::simulation::pathing::cost::CostCalculator::calculate_costs(edge).0
        );
    }
}

#[test]
fn node_merge_adoption_clears_survivor_restrictions_and_rejects_open_frontier() {
    let mut fixture = Fixture::new(false);
    let graph = &mut fixture.graph;
    let positions = [
        Vector3::new(-40.0, 0.0, 0.0),
        Vector3::ZERO,
        Vector3::new(0.0, 0.0, 1.0),
        Vector3::new(0.0, 0.0, 40.0),
    ];
    for pos in positions {
        graph.add_node(pos, NodeType::Junction);
    }
    for (a, b) in [(0, 1), (2, 3)] {
        graph.add_edge(build_surface_edge(
            a,
            b,
            vec![positions[a as usize], positions[b as usize]],
            1,
            1,
            EdgeClass::Standard,
            None,
            road_types::RoadAttributes::DEFAULT,
        ));
    }
    graph.add_lane_connection(1, 0, 0, 0, 0);
    for id in [0, 1] {
        fixture.network.roadbed.mark_edge_dirty(id);
    }
    fixture.network.roadbed.compile_dirty(graph);
    assert!(fixture.network.roadbed.is_current());
    assert!(fixture.network.roadbed.node_piece(2).is_some());
    let mut local = graph.clone();
    local.unite_nodes(1, 2);
    local.rebuild_adjacency_list();
    let source_nodes = (0..4).map(|id| (id, id)).collect();
    let source_edges = HashMap::from([(0, 0), (1, 1)]);
    assert!(
        RoadTopologyPlan::capture(
            graph,
            graph,
            &source_nodes,
            &HashMap::from([(0, 0)]),
            FinalizedRoadGeometry {
                affected_nodes: HashSet::from([2]),
                ..Default::default()
            },
            vec![],
            [0, 3],
        )
        .is_none(),
        "an unmoved profile-solve node also requires complete source incidence"
    );
    let finalized = FinalizedRoadGeometry {
        dirty_edges: HashSet::from([0, 1]),
        affected_nodes: HashSet::from([0, 1, 3]),
        ..Default::default()
    };
    let plan = RoadTopologyPlan::capture(
        &local,
        graph,
        &source_nodes,
        &source_edges,
        finalized.clone(),
        vec![],
        [0, 3],
    )
    .unwrap();
    assert!(plan.nodes[1].changed);
    assert!(
        RoadTopologyPlan::capture(
            &local,
            graph,
            &source_nodes,
            &HashMap::from([(0, 0)]),
            finalized,
            vec![],
            [0, 3],
        )
        .is_none(),
        "a merged source node cannot omit live incident edges"
    );
    fixture
        .network
        .adopt_road_topology_plan(graph, &plan, &fixture.zoning, &mut fixture.allocator)
        .unwrap();
    assert_eq!(graph.get_valid_node(2), 1);
    assert!(graph.node(1).lane_connections.is_empty());
    // Pieces change when the adopted generation publishes.
    fixture.network.roadbed.compile_dirty(graph);
    assert!(
        !fixture
            .network
            .roadbed
            .pieces_in_bounds([f64::MIN, f64::MIN, f64::MAX, f64::MAX])
            .any(|(piece, _)| piece == PieceId::Node(2)),
        "merged-node surface ownership must be evicted even with a canonical-only finalization scope"
    );
    assert_eq!(graph.edge(1).start_node, 1);
    assert_eq!(graph.edge(1).physical_geometry[0], Vector3::ZERO);
    assert_eq!(graph.adjacency(), local.adjacency());
}

// `ROAD-103`: a stroke that splits an arm of a junction with turn rules leaves the rules on the
// half that still reaches the junction, so whitelist mode keeps its listed turn. Both the adopted
// plan (the road tool) and the cold add (`split_edge`) carry them.
#[test]
fn a_split_arm_keeps_the_far_junctions_turn_rules() {
    let mut planned = Fixture::new(false);
    let mut cold = Fixture::new(false);
    for ((sx, sz), (ex, ez)) in [((-60.0, 0.0), (60.0, 0.0)), ((0.0, 0.0), (0.0, 48.0))] {
        let (prepared, plan) = planned.plan(&[planned.point(sx, sz), planned.point(ex, ez)]);
        cold.cold_add(&prepared);
        planned.adopt(&plan);
    }
    // The arm of the junction at the origin whose far node lies to the west or east.
    let arm = |graph: &RegionGraph, west: bool| {
        let junction = graph.find_node_within(Vector3::ZERO, 0.1).unwrap();
        let arm = graph
            .node_adjacency(junction)
            .iter()
            .copied()
            .find(|&id| {
                let edge = graph.edge(id);
                let far = if edge.start_node == junction {
                    edge.end_node
                } else {
                    edge.start_node
                };
                !edge.deleted
                    && (graph.node(far).pos.x < -1.0) == west
                    && graph.node(far).pos.x.abs() > 1.0
            })
            .unwrap();
        (junction, arm, graph.edge(arm).end_node == junction)
    };
    for fixture in [&mut planned, &mut cold] {
        let (junction, west, west_arrives_fwd) = arm(&fixture.graph, true);
        let (_, east, east_arrives_fwd) = arm(&fixture.graph, false);
        let (from, to) = (
            if west_arrives_fwd { 0 } else { -1 },
            if east_arrives_fwd { -1 } else { 0 },
        );
        fixture
            .graph
            .add_lane_connection(junction, west, from, east, to);
    }
    let raw = [planned.point(-30.0, -40.0), planned.point(-30.0, 40.0)];
    let (prepared, plan) = planned.plan(&raw);
    cold.cold_add(&prepared);
    planned.adopt(&plan);
    for fixture in [&mut planned, &mut cold] {
        let graph = &mut fixture.graph;
        let (junction, west, _) = arm(graph, true);
        let rules: Vec<_> = graph
            .node(junction)
            .lane_connections
            .keys()
            .copied()
            .collect();
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].0, west,
            "the rule keys the half that reaches the junction"
        );
        let mut lanes = crate::simulation::network::lanes::LaneSystem::new();
        lanes.rebuild(graph);
        let arriving = lanes.edge_lanes[&west]
            .iter()
            .copied()
            .find(|&id| lanes.lanes[id].lane_idx == rules[0].1)
            .unwrap();
        assert!(
            !lanes.lanes[arriving].next_lanes.is_empty(),
            "the listed turn keeps its connector"
        );
    }
}

// `ROAD-99`: the clearances by structure come from one vehicle envelope. The lining is the
// envelope bitwise, so the tunnel cover (5 m) and a road over a tunnel (5.5 m) are as before; a
// bridge's deck stands the envelope and its slab above a road's deck.
#[test]
fn the_structure_clearances_share_the_vehicle_envelope() {
    use crate::simulation::network::roadbed::{
        TUNNEL_COVER_M, TUNNEL_LINING_HEIGHT_M, VEHICLE_CLEARANCE_M,
    };
    assert_eq!(
        TUNNEL_LINING_HEIGHT_M.to_bits(),
        VEHICLE_CLEARANCE_M.to_bits()
    );
    assert_eq!(TUNNEL_COVER_M.to_bits(), 5.0_f64.to_bits());
    assert_eq!(tunnel::ROAD_OVER_TUNNEL_M.to_bits(), 5.5_f32.to_bits());
    assert_eq!(bridge::ROAD_UNDER_BRIDGE_M.to_bits(), 4.85_f32.to_bits());
}

// A fixture with a bare ring of `diameter_m` at (128, 128) and its ring nodes.
fn ring_fixture(diameter_m: f32) -> (Fixture, Vec<u32>) {
    ring_fixture_on(diameter_m, false)
}

fn ring_fixture_on(diameter_m: f32, sloped: bool) -> (Fixture, Vec<u32>) {
    let mut fixture = Fixture::new(sloped);
    let site = RoundaboutSite::Ground {
        centre: fixture.point(128.0, 128.0),
        lanes: 1,
    };
    let request = RoundaboutRequest { site, diameter_m };
    let applied = RoundaboutLayout::prepare(&fixture.graph, request)
        .unwrap()
        .apply(&mut fixture.graph);
    crate::simulation::network::roadbed::fixtures::finalize(&mut fixture.graph);
    fixture.network.roadbed.compile_dirty(&fixture.graph);
    (fixture, applied.nodes)
}

impl Fixture {
    fn prepared(&self, raw: &[Vector3]) -> PreparedRoadInput {
        prepare_road_input_for_tool(
            raw,
            &self.terrain,
            &self.graph,
            &self.network.roadbed,
            true,
            false,
        )
    }

    fn verdict(&self, raw: &[Vector3]) -> &'static str {
        validate_road_edit(&self.prepared(raw), 1, 1, &self.terrain, &self.graph)
            .0
            .invalid_reason
    }
}

// R7a's gap: a ring on a real slope stays on its level plane through the road edit's solve, the
// terrain taking the difference. Bare, and with arms joined by strokes: every ring point at the
// centre's height, every arm graded up to it within the road limits, the pieces compiled.
#[test]
fn a_ring_on_a_slope_keeps_its_level_plane_through_the_solve() {
    let (mut fixture, _) = ring_fixture_on(36.0, true);
    let plane = fixture.point(128.0, 128.0).y;
    let level = |fixture: &Fixture| {
        for edge in fixture.graph.edges().iter().filter(|edge| edge.roundabout.is_some()) {
            for point in edge.geometry.iter().chain(&edge.physical_geometry) {
                assert!((point.y - plane).abs() < 1e-3, "{} against {plane}", point.y);
            }
        }
    };
    let finalized = fixture.network.finalize_road_geometry(
        &mut fixture.graph,
        Some(&fixture.terrain),
    );
    assert!(finalized.dirty_edges.is_empty());
    level(&fixture);
    for angle in [0f32, 90.0, 180.0, 270.0] {
        let (sin, cos) = angle.to_radians().sin_cos();
        let at = |fixture: &Fixture, r: f32| fixture.point(128.0 + cos * r, 128.0 + sin * r);
        let join = [at(&fixture, 16.25 + 90.0), at(&fixture, 16.25)];
        let (_, plan) = fixture.plan(&join);
        fixture.adopt(&plan);
        fixture.network.roadbed.compile_dirty(&fixture.graph);
        assert!(fixture.network.roadbed.failed_nodes().is_empty(), "{angle}");
        level(&fixture);
    }
    // A join far from every bare node splits the ring at its own plane, not the ground's.
    let (mut fixture, _) = ring_fixture_on(60.0, true);
    let (sin, cos) = 30f32.to_radians().sin_cos();
    let at = |fixture: &Fixture, r: f32| fixture.point(128.0 + cos * r, 128.0 + sin * r);
    let join = [at(&fixture, 28.25 + 90.0), at(&fixture, 28.25)];
    let (_, plan) = fixture.plan(&join);
    fixture.adopt(&plan);
    fixture.network.roadbed.compile_dirty(&fixture.graph);
    assert!(fixture.network.roadbed.failed_nodes().is_empty());
    let ring_edges = fixture.graph.edges().iter().filter(|edge| edge.roundabout.is_some());
    assert_eq!(ring_edges.count(), 4);
    level(&fixture);
}

// D9 (`ROAD-62`). Ring nodes stand at 90, 210 and 330 degrees.
#[test]
fn a_stroke_joins_a_ring_from_outside_and_goes_no_further() {
    let (fixture, nodes) = ring_fixture(36.0);
    let at = |x: f32, z: f32| fixture.point(128.0 + x, 128.0 + z);
    let through = [at(-90.0, 0.0), at(90.0, 0.0)];
    assert_eq!(fixture.verdict(&through), "roundabout_crossed");
    let onto_island = [at(90.0, 0.0), at(4.0, 0.0)];
    assert_eq!(fixture.verdict(&onto_island), "roundabout_island");
    let on_island = [at(4.0, 3.0), at(-4.0, 3.0)];
    assert_eq!(fixture.verdict(&on_island), "roundabout_island");
    let [first, second] = [nodes[0], nodes[1]].map(|node| fixture.graph.node(node).pos);
    assert_eq!(
        fixture.verdict(&[first, second]),
        "roundabout_island",
        "a chord"
    );
    // A bridge passes over; its end still may not stand on the island.
    let over = |raw: &[Vector3]| {
        let prepared = fixture.prepared(raw);
        roundabouts::stroke_ring_refusal(&fixture.graph, &prepared.points, false)
    };
    assert_eq!(over(&through), None);
    assert_eq!(over(&onto_island), Some("roundabout_island"));
}

#[test]
fn a_stroke_joined_to_a_ring_keeps_the_ring_and_takes_the_point() {
    // At 30 m the join is near a bare node and takes its place; further off it splits the ring.
    for (diameter_m, radius_m, edges) in [(30.0, 13.25, 3), (36.0, 16.25, 4), (60.0, 28.25, 4)] {
        let (mut fixture, nodes) = ring_fixture(diameter_m);
        // Halfway between two ring nodes, at 30 degrees.
        let (sin, cos) = 30f32.to_radians().sin_cos();
        let (radial, far) = ((cos, sin), 90.0);
        let at = |fixture: &Fixture, along: f32| {
            fixture.point(128.0 + radial.0 * along, 128.0 + radial.1 * along)
        };
        let join = [at(&fixture, far), at(&fixture, radius_m)];
        let (_, plan) = fixture.plan(&join);
        fixture.adopt(&plan);
        fixture.network.roadbed.compile_dirty(&fixture.graph);
        assert!(
            fixture.network.roadbed.failed_nodes().is_empty(),
            "D{diameter_m}"
        );
        let ring_edges = fixture
            .graph
            .edges()
            .iter()
            .filter(|edge| !edge.deleted && edge.roundabout.is_some())
            .count();
        assert_eq!(ring_edges, edges, "D{diameter_m}: the ring stays whole");
        let joined = fixture
            .graph
            .find_node_within(join[1], 0.1)
            .expect("the join");
        let mut arms = NodeArms::collect(&fixture.graph, joined).expect("a junction now");
        arms.layout().unwrap();
        // The join's node has its arm; a ring node without one takes the next.
        let (sin, cos) = 60f32.to_radians().sin_cos();
        let beside = fixture.point(128.0 + 90.0 * cos, 128.0 + 90.0 * sin);
        let again = [beside, fixture.graph.node(joined).pos];
        assert_eq!(
            fixture.verdict(&again),
            "roundabout_arm_node_taken",
            "D{diameter_m}"
        );
        // The spread may have moved it: the stroke comes in along its radius.
        let free_at = fixture.graph.node(nodes[0]).pos;
        let (out_x, out_z) = (free_at.x - 128.0, free_at.z - 128.0);
        let reach = 100.0 / out_x.hypot(out_z);
        let outside = fixture.point(128.0 + out_x * reach, 128.0 + out_z * reach);
        assert_eq!(fixture.verdict(&[outside, free_at]), "", "D{diameter_m}");
    }
}
