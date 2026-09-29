// SPDX-License-Identifier: GPL-2.0-only

//! Independent cold-commit parity for exact junction meshes and retained source owners.

use super::*;
use crate::config::HEIGHT_SCALE;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::TransitNetwork;
use crate::simulation::zoning::ZoningSystem;
use godot::prelude::Vector3;

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
        self.network.finalize_road_geometry(&mut self.graph);
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

fn verify_connection<const N: usize>(mut fixture: Fixture, points: [Vector3; N], forward: u8) {
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
        .render(&fixture.terrain, &fixture.network.road_surface)
        .expect("valid canonical road scene must render");
    assert!(
        scene.planned.len() >= 2,
        "negative/positive chunk boundaries must be covered"
    );
    for (key, mesh) in &scene.planned {
        // Display partitions are intentionally local; the complete canonical solve above
        // still has independent cold-commit parity coverage.
        assert!(unlifted.contains_key(key));
        assert!(mesh.vertex_count() > 0);
        assert!(mesh.render_payload_validated());
    }
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

// Existing geometry displayed in one chunk: the resident split plus the clipped approaches.
fn existing_signature(
    scene: &RoadJunctionPreview,
    key: &SurfaceChunkKey,
) -> [Vec<[[u32; 12]; 3]>; 7] {
    let empty = NetworkMeshData::new();
    let mut output = signature(scene.retained.get(key).map(Arc::as_ref).unwrap_or(&empty));
    let approach = signature(scene.approach.get(key).map(Arc::as_ref).unwrap_or(&empty));
    for (layer, approach) in output.iter_mut().zip(approach) {
        layer.extend(approach);
        layer.sort_unstable();
    }
    output
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
fn curved_approach_preview_matches_cold_commit() {
    let mut fixture = Fixture::new(true);
    fixture.add(&[fixture.point(-60.0, 0.0), fixture.point(60.0, 0.0)], 1, 1);
    fixture.add(
        &[fixture.point(24.0, 72.0), fixture.point(60.0, 72.0)],
        1,
        1,
    );
    let points = [
        fixture.point(-20.0, -60.0),
        fixture.point(-6.0, -40.0),
        fixture.point(0.0, -20.0),
        fixture.point(0.0, 0.0),
    ];
    verify_connection(fixture, points, 1);
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
fn isolated_strokes_export_canonical_cold_commit_meshes_and_retain_neighbors() {
    for (sloped, neighbor) in [(false, false), (true, false), (true, true)] {
        let mut fixture = Fixture::new(sloped);
        if neighbor {
            fixture.add(
                &[fixture.point(24.0, 72.0), fixture.point(60.0, 72.0)],
                1,
                1,
            );
        }
        let points = [fixture.point(-48.0, 0.0), fixture.point(48.0, 0.0)];
        let (preview, _, input) = fixture
            .network
            .road_surface
            .compile_preview_surface_mesh_only_with_existing_surface_snap_and_topology_reuse(
                &points,
                1,
                1,
                &fixture.terrain,
                &fixture.graph,
                &fixture.network.road_surface,
                true,
            );
        assert!(preview.is_valid);
        let input = input.expect("isolated standard roads need the same canonical render scene");
        assert!(input.removed.is_empty(), "isolated roads replace no owners");
        let mut scene = input
            .render(&fixture.terrain, &fixture.network.road_surface)
            .expect("isolated canonical scene must render");
        assert!(
            scene.replacement_chunks.len() >= 2,
            "cross a chunk boundary"
        );
        let originals = RoadRenderer.generate_mesh_chunks_with_surface(
            &fixture.graph,
            &mut fixture.network.lane_system,
            &fixture.terrain,
            &fixture.network.road_surface,
            &scene.replacement_chunks,
        );
        scene.retain_existing(
            &originals
                .into_iter()
                .map(|(key, mesh)| (key, Arc::new(mesh)))
                .collect(),
        );
        assert_eq!(!scene.retained.is_empty(), neighbor);
        assert!(
            scene
                .retained
                .values()
                .chain(scene.approach.values())
                .all(|mesh| mesh.render_payload_validated())
        );
        fixture.add(&points, 1, 1);
        let committed = RoadRenderer.generate_mesh_chunks_with_surface(
            &fixture.graph,
            &mut fixture.network.lane_system,
            &fixture.terrain,
            &fixture.network.road_surface,
            &scene.replacement_chunks,
        );
        let empty = NetworkMeshData::new();
        for key in &scene.replacement_chunks {
            let mut displayed =
                signature(scene.planned.get(key).map(Arc::as_ref).unwrap_or(&empty));
            for (layer, retained) in displayed.iter_mut().zip(existing_signature(&scene, key)) {
                layer.extend(retained);
                layer.sort_unstable();
            }
            let mut expected = signature(committed.get(key).unwrap_or(&empty));
            for layer in &mut expected {
                layer.sort_unstable();
            }
            assert_eq!(
                displayed, expected,
                "isolated canonical roads plus retained neighbors must match cold commit in {key:?}"
            );
        }
    }
}

#[test]
fn retained_cache_requires_exact_source_ownership_but_not_planned_positions() {
    let mut scene = RoadJunctionPreview {
        source_mesh_generation: 7,
        planned: BTreeMap::new(),
        retained: Arc::new(BTreeMap::from([((0, 0), Arc::new(NetworkMeshData::new()))])),
        retained_revision: 0,
        approach: BTreeMap::new(),
        replacement_chunks: BTreeSet::from([(0, 0)]),
        chunk_span_m: 256.0,
        chunk_origin_x_m: -512.0,
        chunk_origin_z_m: -512.0,
        removed: HashSet::from([NetworkMeshOwner::Edge(4), NetworkMeshOwner::Node(2)]),
        bounded: HashSet::new(),
        bounds: vec![],
        approach_sources: Arc::new(BTreeMap::new()),
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
    // Moving junction/approach bounds changes only the per-request approach clips.
    moved.bounds.push([0.0, 0.0, 16.0, 16.0]);
    assert!(cache.reuse(&mut moved));
    assert!(Arc::ptr_eq(&retained, &moved.retained));
    assert_eq!(revision, moved.retained_revision);
    for changed_key in 0..6 {
        let mut changed = scene.clone();
        match changed_key {
            0 => changed.source_mesh_generation += 1,
            1 => changed.chunk_span_m += 1.0,
            2 => changed.chunk_origin_x_m += 1.0,
            3 => {
                changed.replacement_chunks.insert((1, 0));
            }
            4 => {
                changed.removed.insert(NetworkMeshOwner::Edge(9));
            }
            _ => {
                changed.bounded.insert(NetworkMeshOwner::Edge(9));
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

// A new T on an arm of an existing cross must not rebuild the remote cross.
#[test]
#[ignore = "matched release preview benchmark"]
fn benchmark_neighboring_cross_preview() {
    for sloped in [false, true] {
        let mut fixture = Fixture::new(sloped);
        fixture.add(
            &[fixture.point(-110.0, 0.0), fixture.point(110.0, 0.0)],
            1,
            1,
        );
        fixture.add(
            &[fixture.point(0.0, -110.0), fixture.point(0.0, 110.0)],
            1,
            1,
        );
        let mut times = Vec::new();
        for i in 0..32 {
            let points = [
                fixture.point(60.0 + (i % 8) as f32, -80.0),
                fixture.point(60.0, 0.0),
            ];
            let start = std::time::Instant::now();
            let (preview, _, input) = fixture
                .network
                .road_surface
                .compile_preview_surface_mesh_only_with_existing_surface_snap_and_topology_reuse(
                    &points,
                    1,
                    1,
                    &fixture.terrain,
                    &fixture.graph,
                    &fixture.network.road_surface,
                    true,
                );
            assert!(preview.is_valid, "{:?}", preview.validation);
            let scene = input
                .unwrap()
                .render(&fixture.terrain, &fixture.network.road_surface)
                .unwrap();
            std::hint::black_box(scene);
            if i >= 8 {
                times.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        times.sort_by(f64::total_cmp);
        eprintln!(
            "neighbor_cross sloped={sloped} median_ms={:.3} p95_ms={:.3}",
            times[12], times[22]
        );
    }
}

#[test]
fn neighboring_cross_is_reused_and_existing_geometry_outside_edit_is_unchanged() {
    for sloped in [false, true] {
        let mut fixture = Fixture::new(sloped);
        fixture.add(
            &[fixture.point(-110.0, 0.0), fixture.point(110.0, 0.0)],
            1,
            1,
        );
        fixture.add(
            &[fixture.point(0.0, -110.0), fixture.point(0.0, 110.0)],
            1,
            1,
        );
        let compile =
            |x| {
                let (preview, _, input) = fixture.network.road_surface
                .compile_preview_surface_mesh_only_with_existing_surface_snap_and_topology_reuse(
                    &[fixture.point(x, -80.0), fixture.point(60.0, 0.0)],
                    1, 1, &fixture.terrain, &fixture.graph, &fixture.network.road_surface, true);
                assert!(preview.is_valid);
                input.unwrap()
            };
        let first = compile(60.0);
        let second = compile(61.0);
        let cross = |input: &RoadPreviewRenderInput| {
            input
                .surface
                .compiled_visual_node_pieces
                .iter()
                .find(|(id, _)| {
                    let p = input.graph.node(**id).pos;
                    p.x == 0.0 && p.z == 0.0
                })
                .unwrap()
                .1
                .clone()
        };
        assert!(
            Arc::ptr_eq(&cross(&first), &cross(&second)),
            "remote cross must reuse exact artifact, sloped={sloped}"
        );
        let mut scene = second
            .render(&fixture.terrain, &fixture.network.road_surface)
            .unwrap();
        let originals: BTreeMap<_, _> = RoadRenderer
            .generate_mesh_chunks_with_surface(
                &fixture.graph,
                &mut fixture.network.lane_system,
                &fixture.terrain,
                &fixture.network.road_surface,
                &scene.replacement_chunks,
            )
            .into_iter()
            .map(|(key, mesh)| (key, Arc::new(mesh)))
            .collect();
        scene.retain_existing(&originals);
        let cross_source = fixture
            .graph
            .find_node_within(fixture.point(0.0, 0.0), 0.01)
            .unwrap();
        assert!(
            !scene
                .removed
                .contains(&NetworkMeshOwner::Node(cross_source))
        );
        let cross_owner = HashSet::from([NetworkMeshOwner::Node(cross_source)]);
        for (key, before) in &originals {
            let origin = Vector2::new(
                scene.chunk_origin_x_m + key.0 as f32 * scene.chunk_span_m,
                scene.chunk_origin_z_m + key.1 as f32 * scene.chunk_span_m,
            );
            let empty = NetworkMeshData::new();
            let after = scene.retained.get(key).map(Arc::as_ref).unwrap_or(&empty);
            assert_eq!(
                signature(&before.preview_partition(
                    &cross_owner,
                    &HashSet::new(),
                    &[],
                    origin,
                    true
                )),
                signature(&after.preview_partition(
                    &cross_owner,
                    &HashSet::new(),
                    &[],
                    origin,
                    true
                ))
            );
            // Compare all seven layers outside the exact declared replacement envelope.
            let mut old_outside = signature(&before.preview_partition(
                &scene.removed,
                &scene.bounded,
                &scene.bounds,
                origin,
                false,
            ));
            for layer in &mut old_outside {
                layer.sort_unstable();
            }
            assert_eq!(old_outside, existing_signature(&scene, key));
        }
    }
}

// Sliding a T along an existing road moves only its junction/approach bounds. The resident
// split, including an unrelated road's markings in the same chunks, must be reused exactly.
#[test]
fn moving_junction_bounds_reuse_resident_split_and_clip_only_approaches() {
    for sloped in [false, true] {
        let mut fixture = Fixture::new(sloped);
        fixture.add(
            &[fixture.point(-110.0, 0.0), fixture.point(110.0, 0.0)],
            1,
            1,
        );
        fixture.add(
            &[fixture.point(-110.0, 40.0), fixture.point(110.0, 40.0)],
            1,
            1,
        );
        let scene_at = |fixture: &Fixture, x: f32| {
            let points = [fixture.point(x, -60.0), fixture.point(x, 0.0)];
            let (preview, _, input) = fixture
                .network
                .road_surface
                .compile_preview_surface_mesh_only_with_existing_surface_snap_and_topology_reuse(
                    &points,
                    1,
                    1,
                    &fixture.terrain,
                    &fixture.graph,
                    &fixture.network.road_surface,
                    true,
                );
            assert!(preview.is_valid);
            input
                .expect("a T on an existing road must have a render scene")
                .render(&fixture.terrain, &fixture.network.road_surface)
                .expect("valid canonical road scene must render")
        };
        let mut first = scene_at(&fixture, 10.0);
        let mut second = scene_at(&fixture, 16.0);
        assert_eq!(
            (&first.removed, &first.bounded, &first.replacement_chunks),
            (&second.removed, &second.bounded, &second.replacement_chunks),
            "the fixture must move only the junction bounds, sloped={sloped}"
        );
        assert!(!first.bounded.is_empty() && first.bounds != second.bounds);
        let originals: BTreeMap<_, _> = RoadRenderer
            .generate_mesh_chunks_with_surface(
                &fixture.graph,
                &mut fixture.network.lane_system,
                &fixture.terrain,
                &fixture.network.road_surface,
                &first.replacement_chunks,
            )
            .into_iter()
            .map(|(key, mesh)| (key, Arc::new(mesh)))
            .collect();
        let mut cache = RoadPreviewRetainedCache::default();
        first.retain_existing(&originals);
        cache.store(&mut first);
        assert!(cache.reuse(&mut second));
        second.clip_approaches();
        assert!(Arc::ptr_eq(&first.retained, &second.retained));
        assert_eq!(first.retained_revision, second.retained_revision);
        assert!(
            first
                .retained
                .values()
                .chain(second.approach.values())
                .all(|mesh| mesh.render_payload_validated())
        );
        let vertices = |meshes: &BTreeMap<SurfaceChunkKey, Arc<NetworkMeshData>>| {
            meshes
                .values()
                .map(|mesh| mesh.vertex_count())
                .sum::<usize>()
        };
        let markings: usize = first
            .retained
            .values()
            .map(|mesh| mesh.marking_vertices.len())
            .sum();
        assert!(markings > 0, "unrelated markings must stay resident");
        assert!(
            vertices(&second.approach) < vertices(&second.retained),
            "per-request approach clips must be smaller than the resident split"
        );
        let mut moved = false;
        for (key, mesh) in &originals {
            let origin = second.chunk_origin(*key);
            // The previous single-pass partition at the new bounds is the exact reference.
            let mut expected = signature(&mesh.preview_partition(
                &second.removed,
                &second.bounded,
                &second.bounds,
                origin,
                false,
            ));
            for layer in &mut expected {
                layer.sort_unstable();
            }
            assert_eq!(
                expected,
                existing_signature(&second, key),
                "resident split plus approach must equal a fresh partition in {key:?}, sloped={sloped}"
            );
            moved |= existing_signature(&first, key) != existing_signature(&second, key);
        }
        assert!(moved, "the approach clips must follow the moved bounds");
    }
}
