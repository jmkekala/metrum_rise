// SPDX-License-Identifier: GPL-2.0-only

//! Independent cold-commit parity for exact junction meshes and retained source owners.

use super::*;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::TransitNetwork;
use crate::simulation::zoning::ZoningSystem;

struct Fixture {
    terrain: TerrainSystem,
    graph: RegionGraph,
    network: TransitNetwork,
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
            terrain,
            graph,
            network,
            zoning: ZoningSystem::new(&WorldConfig::default()),
            allocator: BuildingAllocator::new(),
        }
    }

    fn point(&self, x: f32, z: f32) -> Vector3 {
        Vector3::new(x, self.terrain.sample_height_world(x, z) * HEIGHT_SCALE, z)
    }

    fn add(&mut self, points: &[Vector3], forward: u8, backward: u8) {
        let prepared = RoadSurfaceSystem::prepare_road_input_for_tool(
            points,
            &self.terrain,
            &self.graph,
            &self.network.road_surface,
            true,
        );
        // Use the authoritative profile/dirty pipeline, without gameplay constructors, zoning
        // generation, buildings, routing or preview artifacts influencing the reference result.
        self.network.bulk_load = true;
        if let Some(extension) = prepared.extension {
            // SimCore applies the terminal extension profile before inserting the new edge.
            // Mirror that geometry/index/dirty step in this isolated cold-commit fixture.
            let edge_idx = extension.existing_edge_idx;
            self.graph
                .set_node_pos(extension.snapped_node_id, extension.snapped_node_pos);
            self.graph.remove_from_spatial_index(edge_idx);
            let edge = self.graph.edge_mut(edge_idx);
            edge.geometry = extension.existing_points.clone();
            edge.physical_geometry = extension.existing_points;
            let (cost, length) =
                crate::simulation::pathing::cost::CostCalculator::calculate_costs(edge);
            edge.base_cost = cost;
            edge.physical_length = length;
            self.graph.add_to_spatial_index(edge_idx);
            let edge = self.graph.edge(edge_idx);
            self.network.mark_surface_dirty_from_sets(
                &self.graph,
                &HashSet::from([edge_idx]),
                &HashSet::from([edge.start_node, edge.end_node]),
            );
            self.network.bulk_dirty_edges.insert(edge_idx);
        }
        self.network.add_road(
            &mut self.graph,
            prepared.points,
            forward,
            backward,
            prepared.class,
            &mut self.zoning,
            &mut self.allocator,
        );
        self.network.bulk_load = false;
        let mut edges = std::mem::take(&mut self.network.bulk_dirty_edges);
        let nodes = self.network.bulk_surface_profile_nodes(&self.graph, &edges);
        edges.extend(self.network.solve_dirty_junction_endpoint_profiles(
            &mut self.graph,
            &nodes,
            &edges,
        ));
        edges.extend(self.network.regrade_dirty_junction_endpoint_profiles(
            &mut self.graph,
            &nodes,
            &edges,
        ));
        self.graph.rebuild_intersection_clips_for_nodes(&nodes);
        self.network
            .mark_surface_dirty_from_sets(&self.graph, &edges, &nodes);
        self.network
            .road_surface
            .compile_dirty(&self.graph, &self.terrain);
        self.network.lane_system.rebuild(&mut self.graph);
        self.network.lane_system.sync_heights_to_visible_surface(
            &self.graph,
            &self.terrain,
            &self.network.road_surface,
        );
    }

    fn mesh(&self) -> NetworkMeshData {
        RoadRenderer.generate_mesh_data_with_surface(
            &self.graph,
            &self.network.lane_system,
            &self.terrain,
            &self.network.road_surface,
        )
    }
}

// Triangle order and owner IDs are not rendering identity. Every actual vertex attribute is.
fn signature(mesh: &NetworkMeshData) -> [Vec<[[u32; 12]; 3]>; 7] {
    let mut output: [Vec<[[u32; 12]; 3]>; 7] = Default::default();
    macro_rules! layer {
        ($index:expr, $vertices:ident, $normals:ident, $uvs:ident, $colors:ident) => {
            for start in (0..mesh.$vertices.len()).step_by(3) {
                let mut triangle = std::array::from_fn(|i| {
                    let p = mesh.$vertices[start + i];
                    let n = mesh.$normals[start + i];
                    let uv = mesh.$uvs[start + i];
                    let c = mesh.$colors[start + i];
                    [p.x, p.y, p.z, n.x, n.y, n.z, uv.x, uv.y, c.r, c.g, c.b, c.a].map(f32::to_bits)
                });
                triangle.sort_unstable();
                output[$index].push(triangle);
            }
        };
    }
    layer!(
        0,
        earthwork_vertices,
        earthwork_normals,
        earthwork_uvs,
        earthwork_colors
    );
    layer!(1, curb_vertices, curb_normals, curb_uvs, curb_colors);
    layer!(
        2,
        raised_step_vertices,
        raised_step_normals,
        raised_step_uvs,
        raised_step_colors
    );
    layer!(
        3,
        sidewalk_vertices,
        sidewalk_normals,
        sidewalk_uvs,
        sidewalk_colors
    );
    layer!(4, road_vertices, road_normals, road_uvs, road_colors);
    layer!(
        5,
        marking_vertices,
        marking_normals,
        marking_uvs,
        marking_colors
    );
    layer!(
        6,
        concrete_vertices,
        concrete_normals,
        concrete_uvs,
        concrete_colors
    );
    output
}

fn verify_junction(through: bool, forward: u8, sloped: bool, connected_tail: bool) {
    let mut fixture = Fixture::new(sloped);
    fixture.add(&[fixture.point(-60.0, 0.0), fixture.point(60.0, 0.0)], 1, 1);
    fixture.add(
        &[fixture.point(24.0, 72.0), fixture.point(60.0, 72.0)],
        1,
        1,
    );
    if connected_tail {
        fixture.add(
            &[fixture.point(0.0, -85.0), fixture.point(0.0, -48.0)],
            1,
            1,
        );
    }
    let points = [
        fixture.point(0.0, -48.0),
        fixture.point(0.0, if through { 48.0 } else { 0.0 }),
    ];
    verify_connection(fixture, points, forward);
}

fn verify_connection(mut fixture: Fixture, points: [Vector3; 2], forward: u8) {
    let before = fixture.mesh();
    let (_, _, input) = fixture
        .network
        .road_surface
        .compile_preview_surface_mesh_only_with_existing_surface_snap_and_topology_reuse(
            &points,
            forward,
            1,
            &fixture.terrain,
            &fixture.graph,
            &fixture.network.road_surface,
            true,
        );
    let mut input = input.expect("affected connection must have a render scene");
    assert!(
        !input.removed.contains(&NetworkMeshOwner::Edge(1)),
        "unrelated road must not be replaced"
    );
    let kept = before.without_owners(&input.removed);
    assert!(
        kept.vertex_count() > 0,
        "unrelated source geometry must survive filtering"
    );
    let mut lanes = LaneSystem::new();
    lanes.rebuild(&mut input.graph);
    lanes.sync_heights_to_visible_surface(&input.graph, &fixture.terrain, &input.surface);
    let exact = RoadRenderer.generate_mesh_data_with_surface(
        &input.graph,
        &lanes,
        &fixture.terrain,
        &input.surface,
    );
    let mut combined = signature(&kept);
    for (layer, planned) in combined.iter_mut().zip(signature(&exact)) {
        layer.extend(planned);
    }
    let keys = input
        .surface
        .surface_chunk_cache()
        .keys()
        .copied()
        .chain(input.surface.earthwork_chunk_cache().keys().copied())
        .collect();
    let unlifted = RoadRenderer.generate_mesh_chunks_with_surface(
        &input.graph,
        &mut lanes,
        &fixture.terrain,
        &input.surface,
        &keys,
    );
    let scene = input
        .render(
            &fixture.terrain,
            &fixture.graph,
            &fixture.network.road_surface,
        )
        .expect("valid local cutout infill must render");
    assert!(
        scene.planned.len() >= 2,
        "negative/positive chunk boundaries must be covered"
    );
    let mut anchored_vertices = 0;
    let mut cleared_vertices = 0;
    let flat = exact.road_vertices.iter().all(|point| point.y == 0.0);
    let empty = NetworkMeshData::new();
    for (key, mesh) in &scene.planned {
        let original = unlifted.get(key).unwrap_or(&empty);
        if flat {
            // Include vertices introduced by contact splitting, not only compiler vertices.
            for point in &mesh.road_vertices {
                let x = point.x + scene.chunk_origin_x_m + key.0 as f32 * scene.chunk_span_m;
                let z = point.z + scene.chunk_origin_z_m + key.1 as f32 * scene.chunk_span_m;
                if fixture
                    .network
                    .road_surface
                    .sample_visible_surface_height(&fixture.graph, &fixture.terrain, x, z)
                    .is_some()
                {
                    assert_eq!(
                        point.y, 0.0,
                        "inserted interior contact must not lift at {x}, {z}"
                    );
                }
            }
        }
        for (displayed, raw) in [
            (&mesh.road_vertices, &original.road_vertices),
            (&mesh.sidewalk_vertices, &original.sidewalk_vertices),
            (&mesh.curb_vertices, &original.curb_vertices),
            (&mesh.raised_step_vertices, &original.raised_step_vertices),
            (&mesh.marking_vertices, &original.marking_vertices),
            (&mesh.earthwork_vertices, &original.earthwork_vertices),
            (&mesh.concrete_vertices, &original.concrete_vertices),
        ] {
            // Contact splits retain source vertices and add interpolated seam vertices.
            assert!(displayed.len() >= raw.len());
            let displayed_positions: HashSet<_> = displayed
                .iter()
                .map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()])
                .collect();
            for source in raw {
                let x = source.x + scene.chunk_origin_x_m + key.0 as f32 * scene.chunk_span_m;
                let z = source.z + scene.chunk_origin_z_m + key.1 as f32 * scene.chunk_span_m;
                if fixture
                    .network
                    .road_surface
                    .sample_visible_surface_height(&fixture.graph, &fixture.terrain, x, z)
                    .is_some()
                {
                    assert!(
                        displayed_positions.contains(&[
                            source.x.to_bits(),
                            source.y.to_bits(),
                            source.z.to_bits()
                        ]),
                        "preview must not detach replacement roads from committed terrain cutouts at {x}, {z}"
                    );
                    anchored_vertices += 1;
                }
            }
        }
        for point in &original.road_vertices {
            let x = point.x + scene.chunk_origin_x_m + key.0 as f32 * scene.chunk_span_m;
            let z = point.z + scene.chunk_origin_z_m + key.1 as f32 * scene.chunk_span_m;
            if fixture
                .network
                .road_surface
                .sample_visible_surface_height(&fixture.graph, &fixture.terrain, x, z)
                .is_some()
            {
                continue;
            }
            let height = fixture.terrain.sample_visual_height_world(x, z) * HEIGHT_SCALE;
            assert!(
                mesh.road_vertices
                    .iter()
                    .any(|displayed| displayed.x == point.x
                        && displayed.z == point.z
                        && displayed.y >= height + 0.149),
                "display mesh must stay above sampled terrain"
            );
            cleared_vertices += 1;
        }
    }
    assert!(anchored_vertices > 0 && cleared_vertices > 0);
    fixture.add(&points, forward, 1);
    for (index, (mut displayed, mut committed)) in combined
        .into_iter()
        .zip(signature(&fixture.mesh()))
        .enumerate()
    {
        displayed.sort_unstable();
        committed.sort_unstable();
        assert!(
            displayed == committed,
            "layer {index}: preview triangles {} != cold commit {}; points={points:?} forward={forward}; first mismatch={:?}",
            displayed.len(),
            committed.len(),
            displayed
                .iter()
                .zip(&committed)
                .find(|(a, b)| a != b)
                .map(|(a, b)| (
                    a.map(|vertex| vertex.map(f32::from_bits)),
                    b.map(|vertex| vertex.map(f32::from_bits)),
                ))
        );
    }
}

#[test]
fn t_preview_matches_cold_commit_and_preserves_unrelated_owners() {
    verify_junction(false, 1, false, false);
}

#[test]
fn four_way_preview_matches_cold_commit() {
    verify_junction(true, 1, false, false);
}

#[test]
fn unequal_width_preview_matches_cold_commit() {
    verify_junction(false, 2, false, false);
}

#[test]
fn sloped_junction_preview_matches_cold_commit() {
    verify_junction(false, 1, true, false);
}

#[test]
fn junction_preview_removes_a_terminal_that_becomes_pass_through() {
    verify_junction(false, 1, false, true);
}

#[test]
fn bends_match_cold_commit_including_unequal_width_and_slope() {
    for (end_x, forward, sloped) in [
        (0.0, 1, false),
        (24.0, 1, false),
        (0.0, 2, false),
        (0.0, 1, true),
    ] {
        let mut fixture = Fixture::new(sloped);
        fixture.add(&[fixture.point(-60.0, 0.0), fixture.point(0.0, 0.0)], 1, 1);
        fixture.add(
            &[fixture.point(24.0, 72.0), fixture.point(60.0, 72.0)],
            1,
            1,
        );
        let points = [fixture.point(0.0, 0.0), fixture.point(end_x, -48.0)];
        verify_connection(fixture, points, forward);
    }
}

#[test]
fn straight_continuation_removes_terminal_without_a_junction_n() {
    let mut fixture = Fixture::new(false);
    fixture.add(&[fixture.point(-60.0, 0.0), fixture.point(0.0, 0.0)], 1, 1);
    fixture.add(
        &[fixture.point(24.0, 72.0), fixture.point(60.0, 72.0)],
        1,
        1,
    );
    let points = [fixture.point(0.0, 0.0), fixture.point(48.0, 0.0)];
    verify_connection(fixture, points, 1);
}

#[test]
fn isolated_stroke_does_not_export_a_replacement_scene() {
    let fixture = Fixture::new(false);
    let (preview, _, input) = fixture
        .network
        .road_surface
        .compile_preview_surface_mesh_only_with_existing_surface_snap_and_topology_reuse(
            &[fixture.point(-48.0, 0.0), fixture.point(48.0, 0.0)],
            1,
            1,
            &fixture.terrain,
            &fixture.graph,
            &fixture.network.road_surface,
            true,
        );
    assert!(preview.is_valid);
    assert!(input.is_none());
}

#[test]
fn retained_cache_requires_exact_source_ownership_but_not_planned_positions() {
    let mut scene = RoadJunctionPreview {
        source_mesh_generation: 7,
        planned: BTreeMap::new(),
        retained: Arc::new(BTreeMap::from([((0, 0), Arc::new(NetworkMeshData::new()))])),
        retained_revision: 0,
        replacement_chunks: BTreeSet::from([(0, 0)]),
        chunk_span_m: 256.0,
        chunk_origin_x_m: -512.0,
        chunk_origin_z_m: -512.0,
        removed: HashSet::from([NetworkMeshOwner::Edge(4), NetworkMeshOwner::Node(2)]),
    };
    let mut cache = RoadPreviewRetainedCache::default();
    assert!(!cache.reuse(&mut scene));
    cache.store(&mut scene);
    let retained = Arc::clone(&scene.retained);
    let revision = scene.retained_revision;
    let mut moved = scene.clone();
    moved
        .planned
        .insert((0, 0), Arc::new(NetworkMeshData::new()));
    assert!(cache.reuse(&mut moved));
    assert!(Arc::ptr_eq(&retained, &moved.retained));
    assert_eq!(revision, moved.retained_revision);
    for changed_key in 0..5 {
        let mut changed = scene.clone();
        match changed_key {
            0 => changed.source_mesh_generation += 1,
            1 => changed.chunk_span_m += 1.0,
            2 => changed.chunk_origin_x_m += 1.0,
            3 => {
                changed.replacement_chunks.insert((1, 0));
            }
            _ => {
                changed.removed.insert(NetworkMeshOwner::Edge(9));
            }
        }
        assert!(
            !cache.reuse(&mut changed),
            "retained dependency {changed_key} changed"
        );
    }
    cache.store(&mut moved);
    assert!(moved.retained_revision > revision);
}
