// SPDX-License-Identifier: GPL-2.0-only

//! Exact topology/profile adoption, cold-commit parity and bounded-index regressions.

use super::*;
use crate::config::HEIGHT_SCALE;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::surface::{
    PreparedRoadInput, RoadPreviewTopologyReuse, RoadSurfaceSystem,
};
use crate::simulation::network::{build_surface_edge, types::EdgeClass};
use crate::simulation::terrain::TerrainSystem;

mod earthworks;

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
        network.road_surface.compile_dirty(&graph, &terrain);
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

    fn plan(
        &self,
        raw: &[Vector3],
    ) -> (
        PreparedRoadInput,
        RoadTopologyPlan,
        Option<RoadPreviewTopologyReuse>,
    ) {
        let surface = &self.network.road_surface;
        let prepared = RoadSurfaceSystem::prepare_road_input_for_tool(
            raw,
            &self.terrain,
            &self.graph,
            surface,
            true,
        );
        let (preview, reuse, _, plan) = surface
            .compile_prepared_preview_surface_with_topology_reuse(
                &prepared,
                1,
                1,
                &self.terrain,
                &self.graph,
                surface,
            );
        assert!(preview.is_valid, "{:?}", preview.validation);
        (
            prepared,
            plan.expect("complete local mutation scope must be adoptable"),
            reuse,
        )
    }

    fn cold_add(&mut self, prepared: &PreparedRoadInput) {
        self.network.bulk_load = true;
        if let Some(extension) = &prepared.extension {
            let id = extension.existing_edge_idx;
            self.graph
                .set_node_pos(extension.snapped_node_id, extension.snapped_node_pos);
            self.graph.remove_from_spatial_index(id);
            let edge = self.graph.edge_mut(id);
            edge.geometry = extension.existing_points.clone();
            edge.physical_geometry = extension.existing_points.clone();
            let (cost, length) =
                crate::simulation::pathing::cost::CostCalculator::calculate_costs(edge);
            edge.base_cost = cost;
            edge.physical_length = length;
            let nodes = HashSet::from([edge.start_node, edge.end_node]);
            self.graph.add_to_spatial_index(id);
            self.network
                .mark_surface_dirty_from_sets(&self.graph, &HashSet::from([id]), &nodes);
            self.network.bulk_dirty_edges.insert(id);
            self.network.mark_road_profile_authored(id);
        }
        self.network.add_road(
            &mut self.graph,
            prepared.points.clone(),
            1,
            1,
            prepared.class,
            &mut self.zoning,
            &mut self.allocator,
        );
        self.network.finalize_road_geometry(&mut self.graph);
        self.network.bulk_load = false;
        assert!(
            self.network
                .road_surface
                .compile_dirty(&self.graph, &self.terrain)
        );
    }

    fn adopt(&mut self, plan: &RoadTopologyPlan, reuse: Option<RoadPreviewTopologyReuse>) {
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
        if let Some(reuse) = reuse {
            self.network
                .road_surface
                .enqueue_preview_topology_reuse(reuse);
        }
        assert!(
            self.network
                .road_surface
                .compile_dirty(&self.graph, &self.terrain),
            "{:?}",
            self.network.road_surface.last_compile_failure_label()
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
    assert_eq!(
        actual.network.road_surface.compiled_visual_span_pieces(),
        cold.network.road_surface.compiled_visual_span_pieces()
    );
    let actual_nodes = actual.network.road_surface.compiled_visual_node_pieces();
    let cold_nodes = cold.network.road_surface.compiled_visual_node_pieces();
    assert_eq!(actual_nodes.len(), cold_nodes.len());
    for (id, a) in actual_nodes {
        let b = &cold_nodes[id];
        for (loop_a, loop_b) in a
            .terrain_clip_boundary_loops
            .iter()
            .zip(&b.terrain_clip_boundary_loops)
        {
            assert_eq!(
                loop_a.points_world, loop_b.points_world,
                "node {id} clip points"
            );
            assert_eq!(
                loop_a.source_edges.len(),
                loop_b.source_edges.len(),
                "node {id} clip sources"
            );
            for (source_a, source_b) in loop_a.source_edges.iter().zip(&loop_b.source_edges) {
                assert_eq!(source_a, source_b, "node {id} first differing clip source");
            }
        }
        macro_rules! check_fields {
            ($($field:ident),+ $(,)?) => { $(assert!(a.$field == b.$field, "node {id} field {} differs", stringify!($field));)+ };
        }
        check_fields!(
            node_id,
            kind,
            outer_boundary_loops,
            terrain_clip_boundary_loops,
            road_surface_polygons,
            curb_surface_polygons,
            raised_step_face_polygons,
            raised_step_face_sources,
            sidewalk_surface_polygons,
            explicit_vertical_step_segments,
            node_grade_authorities,
            node_top_surface_sources,
            owned_regions,
            boolean_debug,
            earthwork_owner_sources,
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces
        );
        assert!(a == b, "node {id} complete product differs");
    }
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
            let (prepared, plan, reuse) = planned.plan(&raw);
            cold.cold_add(&prepared);
            planned.adopt(&plan, reuse);
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
                    .set_height(x, z, x as f32 * 0.6 / HEIGHT_SCALE);
            }
        }
    }
    for ((sx, sz), (ex, ez)) in [((-60.0, 0.0), (60.0, 0.0)), ((0.0, -48.0), (0.0, 48.0))] {
        let raw = [planned.point(sx, sz), planned.point(ex, ez)];
        let (prepared, plan, reuse) = planned.plan(&raw);
        cold.cold_add(&prepared);
        planned.adopt(&plan, reuse);
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
        assert!(cold.network.profile_authored_edges.is_empty());
        assert!(planned.network.profile_authored_edges.is_empty());
    }
}

#[test]
fn terminal_extension_adopts_existing_road_reprofile() {
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
        ));
        fixture
            .network
            .road_surface
            .mark_edge_dirty(&fixture.graph, 0);
        assert!(
            fixture
                .network
                .road_surface
                .compile_dirty(&fixture.graph, &fixture.terrain)
        );
    }
    let (prepared, plan, reuse) =
        planned.plan(&[Vector3::new(0.0, 8.0, 0.0), Vector3::new(48.0, 0.0, 0.0)]);
    assert!(prepared.extension.is_some());
    cold.cold_add(&prepared);
    planned.adopt(&plan, reuse);
    assert_cold_geometry_matches(&planned, &cold);
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
    ));
    fixture
        .network
        .road_surface
        .compile_dirty(&fixture.graph, &fixture.terrain);
    let (_, plan, reuse) =
        fixture.plan(&[Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)]);
    assert!(plan.source_topology_ids().0.is_empty());
    fixture.adopt(&plan, reuse);
    let (_, plan, reuse) =
        fixture.plan(&[Vector3::new(0.0, 0.0, -48.0), Vector3::new(0.0, 0.0, 48.0)]);
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
    fixture.graph.set_edge_congestion(1, 7.0);
    fixture.graph.edges[1].no_building_spawn = true;
    fixture.graph.edges[1].speed_limit = 9.0;
    fixture.adopt(&plan, reuse);
    assert_eq!(fixture.graph.edge(0).geometry, remote);
    assert_eq!(fixture.graph.edge(1).current_congestion, 7.0);
    assert!(fixture.graph.edge(1).no_building_spawn);
    let split = plan
        .splits
        .iter()
        .find(|split| plan.edges[split.edge_id].source == Some(1))
        .unwrap();
    let split_local = split.new_edge_id;
    // All source edges precede the candidate; this fixture has one omitted remote edge.
    assert_eq!(fixture.graph.edge(split_local + 1).current_congestion, 7.0);
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
        ));
    }
    graph.add_lane_connection(1, 0, 0, 0, 0);
    for id in [0, 1] {
        fixture.network.road_surface.mark_edge_dirty(graph, id);
    }
    assert!(
        fixture
            .network
            .road_surface
            .compile_dirty(graph, &fixture.terrain)
    );
    assert!(
        fixture
            .network
            .road_surface
            .compiled_visual_node_pieces()
            .contains_key(&2)
    );
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
            vec![]
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
    assert!(
        !fixture
            .network
            .road_surface
            .compiled_visual_node_pieces()
            .contains_key(&2),
        "merged-node surface ownership must be evicted even with a canonical-only finalization scope"
    );
    assert_eq!(graph.edge(1).start_node, 1);
    assert_eq!(graph.edge(1).physical_geometry[0], Vector3::ZERO);
    assert_eq!(graph.adjacency(), local.adjacency());
}
