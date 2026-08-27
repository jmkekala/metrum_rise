//! Deterministic scaling benchmark for local committed-road chunk generation.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use godot::prelude::{Color, Vector2, Vector3};
use metrum_rise::simulation::core::config::WorldConfig;
use metrum_rise::simulation::network::TransitNetwork;
use metrum_rise::simulation::network::graph::{Edge, RegionGraph};
use metrum_rise::simulation::network::render::NetworkMeshData;
use metrum_rise::simulation::network::surface::SurfaceChunkKey;
use metrum_rise::simulation::network::types::{
    EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
};
use metrum_rise::simulation::terrain::TerrainSystem;
use std::collections::BTreeSet;
use std::time::Duration;

const EXPECTED_RENDER_CHUNK_SPAN_M: f32 = 510.0;
const ROAD_LENGTH_M: f32 = 32.0;
const ROAD_INSET_M: f32 = 128.0;
const DISTANT_CHUNK_COUNTS: [usize; 4] = [0, 63, 255, 1023];
const DISTANT_GRID_SIDE: i32 = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MeshSignature {
    chunk_count: usize,
    vertex_count: usize,
    digest: u64,
}

struct RoadChunkBenchFixture {
    graph: RegionGraph,
    terrain: TerrainSystem,
    network: TransitNetwork,
    local_edge: usize,
    target_chunks: BTreeSet<SurfaceChunkKey>,
    occupied_chunks: usize,
    target_edge_count: usize,
    target_node_count: usize,
    expected_signature: MeshSignature,
}

fn bench_road_chunk_renderer(c: &mut Criterion) {
    let mut group = c.benchmark_group("RoadChunkRenderer");
    group.sample_size(20);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    group.throughput(Throughput::Elements(1));

    let mut baseline_owners = None;
    let mut baseline_signature = None;
    for distant_chunks in DISTANT_CHUNK_COUNTS {
        let mut fixture = build_road_chunk_fixture(distant_chunks);
        let owners = (fixture.target_edge_count, fixture.target_node_count);
        if let Some(expected) = baseline_owners {
            assert_eq!(
                owners, expected,
                "local target owners changed with city size"
            );
        } else {
            baseline_owners = Some(owners);
        }
        if let Some(expected) = baseline_signature {
            assert_eq!(
                fixture.expected_signature, expected,
                "local mesh output changed with city size"
            );
        } else {
            baseline_signature = Some(fixture.expected_signature);
        }

        let occupied_chunks = fixture.occupied_chunks;
        group.bench_with_input(
            BenchmarkId::new("chunk_emit_only", occupied_chunks),
            &occupied_chunks,
            |b, _| {
                b.iter(|| {
                    let chunks = black_box(&mut fixture.network)
                        .try_generate_mesh_chunks(
                            black_box(&fixture.graph),
                            black_box(&fixture.terrain),
                            black_box(&fixture.target_chunks),
                        )
                        .expect("the clean benchmark surface must stay published");
                    black_box(mesh_map_shape(&chunks));
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("full_network_emit", occupied_chunks),
            &occupied_chunks,
            |b, _| {
                b.iter(|| {
                    let mesh = black_box(&mut fixture.network)
                        .try_generate_mesh_data(
                            black_box(&fixture.graph),
                            black_box(&fixture.terrain),
                        )
                        .expect("the clean benchmark surface must stay published");
                    black_box(mesh_vertex_count(&mesh));
                });
            },
        );

        fixture
            .network
            .road_surface
            .mark_edge_dirty(&fixture.graph, fixture.local_edge);
        let dirty_warmup = fixture
            .network
            .try_generate_mesh_chunks(&fixture.graph, &fixture.terrain, &fixture.target_chunks)
            .expect("the diagnostic dirty warmup must publish successfully");
        assert_eq!(
            mesh_map_signature(&dirty_warmup),
            fixture.expected_signature,
            "a no-op local recompile must preserve the exact local mesh"
        );

        group.bench_with_input(
            BenchmarkId::new("dirty_compile_plus_chunk_emit_diagnostic", occupied_chunks),
            &occupied_chunks,
            |b, _| {
                b.iter(|| {
                    fixture
                        .network
                        .road_surface
                        .mark_edge_dirty(black_box(&fixture.graph), fixture.local_edge);
                    let chunks = black_box(&mut fixture.network)
                        .try_generate_mesh_chunks(
                            black_box(&fixture.graph),
                            black_box(&fixture.terrain),
                            black_box(&fixture.target_chunks),
                        )
                        .expect("the local dirty compile must publish successfully");
                    black_box(mesh_map_shape(&chunks));
                });
            },
        );
    }

    group.finish();
}

fn build_road_chunk_fixture(distant_chunks: usize) -> RoadChunkBenchFixture {
    let config = WorldConfig::gameplay_default();
    let render_chunk_span_m = config.terrain_render_chunk_span_m();
    assert_eq!(
        render_chunk_span_m.to_bits(),
        EXPECTED_RENDER_CHUNK_SPAN_M.to_bits(),
        "the benchmark fixture must track the production render span"
    );
    let terrain = TerrainSystem::from_world_config(&config);
    let mut graph = RegionGraph::new();
    let local_edge = add_isolated_road(&mut graph, 0, 0, render_chunk_span_m, &config);
    let first_chunk = -DISTANT_GRID_SIDE / 2;
    let last_chunk = first_chunk + DISTANT_GRID_SIDE;
    let mut added_distant_chunks = 0;
    if distant_chunks > 0 {
        'chunks: for chunk_z in first_chunk..last_chunk {
            for chunk_x in first_chunk..last_chunk {
                if (chunk_x, chunk_z) == (0, 0) {
                    continue;
                }
                add_isolated_road(&mut graph, chunk_x, chunk_z, render_chunk_span_m, &config);
                added_distant_chunks += 1;
                if added_distant_chunks == distant_chunks {
                    break 'chunks;
                }
            }
        }
    }
    assert_eq!(added_distant_chunks, distant_chunks);
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let mut network = TransitNetwork::new_with_surface_chunk_span(render_chunk_span_m);
    network.lane_system.rebuild(&mut graph);
    network.road_surface.compile_dirty(&graph, &terrain);

    let target_chunks = network
        .road_surface
        .surface_chunk_cache()
        .iter()
        .filter_map(|(&chunk, entry)| entry.edge_indices.contains(&local_edge).then_some(chunk))
        .chain(
            network
                .road_surface
                .earthwork_chunk_cache()
                .iter()
                .filter_map(|(&chunk, entry)| {
                    entry.edge_indices.contains(&local_edge).then_some(chunk)
                }),
        )
        .collect::<BTreeSet<_>>();
    assert_eq!(
        target_chunks,
        BTreeSet::from([(0, 0)]),
        "the local fixture must stay wholly inside one render chunk"
    );

    let occupied_chunks = network
        .road_surface
        .surface_chunk_cache()
        .keys()
        .chain(network.road_surface.earthwork_chunk_cache().keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .len();
    assert_eq!(
        occupied_chunks,
        distant_chunks + 1,
        "each isolated fixture road must occupy exactly one distinct chunk"
    );

    let target_edge_count = target_chunks
        .iter()
        .flat_map(|chunk| {
            network
                .road_surface
                .surface_chunk_cache()
                .get(chunk)
                .into_iter()
                .flat_map(|entry| entry.edge_indices.iter().copied())
                .chain(
                    network
                        .road_surface
                        .earthwork_chunk_cache()
                        .get(chunk)
                        .into_iter()
                        .flat_map(|entry| entry.edge_indices.iter().copied()),
                )
        })
        .collect::<BTreeSet<_>>()
        .len();
    let target_node_count = target_chunks
        .iter()
        .flat_map(|chunk| {
            network
                .road_surface
                .surface_chunk_cache()
                .get(chunk)
                .into_iter()
                .flat_map(|entry| entry.node_ids.iter().copied())
                .chain(
                    network
                        .road_surface
                        .earthwork_chunk_cache()
                        .get(chunk)
                        .into_iter()
                        .flat_map(|entry| entry.node_ids.iter().copied()),
                )
        })
        .collect::<BTreeSet<_>>()
        .len();

    let expected_chunks = network
        .try_generate_mesh_chunks(&graph, &terrain, &target_chunks)
        .expect("the initial benchmark surface must publish");
    let expected_signature = mesh_map_signature(&expected_chunks);
    assert!(
        expected_signature.chunk_count > 0,
        "the local road must emit a chunk"
    );
    assert!(
        expected_signature.vertex_count > 0,
        "the local road must emit vertices"
    );

    RoadChunkBenchFixture {
        graph,
        terrain,
        network,
        local_edge,
        target_chunks,
        occupied_chunks,
        target_edge_count,
        target_node_count,
        expected_signature,
    }
}

fn add_isolated_road(
    graph: &mut RegionGraph,
    chunk_x: i32,
    chunk_z: i32,
    render_chunk_span_m: f32,
    config: &WorldConfig,
) -> usize {
    let start_x = chunk_x as f32 * render_chunk_span_m + ROAD_INSET_M;
    let start_z = chunk_z as f32 * render_chunk_span_m + ROAD_INSET_M;
    let end_x = start_x + ROAD_LENGTH_M;
    let half_width_m = config.width_m * 0.5;
    let half_height_m = config.height_m * 0.5;
    assert!(start_x >= -half_width_m && end_x <= half_width_m);
    assert!(start_z >= -half_height_m && start_z <= half_height_m);
    let start = graph.add_node(Vector3::new(start_x, 0.0, start_z), NodeType::Junction);
    let end = graph.add_node(Vector3::new(end_x, 0.0, start_z), NodeType::Junction);
    let midpoint = Vector3::new((start_x + end_x) * 0.5, 0.0, start_z);
    let points = vec![graph.node(start).pos, midpoint, graph.node(end).pos];

    graph.add_edge(Edge {
        start_node: start,
        end_node: end,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 0.0,
        physical_length: ROAD_LENGTH_M,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: points.clone(),
        physical_geometry: points,
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    })
}

fn mesh_map_signature(
    chunks: &std::collections::BTreeMap<SurfaceChunkKey, NetworkMeshData>,
) -> MeshSignature {
    let (chunk_count, vertex_count) = mesh_map_shape(chunks);
    let mut digest = 0xcbf2_9ce4_8422_2325_u64;
    for (&(chunk_x, chunk_z), mesh) in chunks {
        hash_u32(&mut digest, chunk_x as u32);
        hash_u32(&mut digest, chunk_z as u32);
        hash_mesh(&mut digest, mesh);
    }
    MeshSignature {
        chunk_count,
        vertex_count,
        digest,
    }
}

fn mesh_map_shape(
    chunks: &std::collections::BTreeMap<SurfaceChunkKey, NetworkMeshData>,
) -> (usize, usize) {
    (chunks.len(), chunks.values().map(mesh_vertex_count).sum())
}

fn mesh_vertex_count(mesh: &NetworkMeshData) -> usize {
    mesh.earthwork_vertices.len()
        + mesh.curb_vertices.len()
        + mesh.raised_step_vertices.len()
        + mesh.sidewalk_vertices.len()
        + mesh.road_vertices.len()
        + mesh.marking_vertices.len()
        + mesh.concrete_vertices.len()
}

fn hash_mesh(digest: &mut u64, mesh: &NetworkMeshData) {
    macro_rules! hash_layer {
        ($vertices:ident, $normals:ident, $uvs:ident, $colors:ident) => {{
            hash_vector3_slice(digest, &mesh.$vertices);
            hash_vector3_slice(digest, &mesh.$normals);
            hash_vector2_slice(digest, &mesh.$uvs);
            hash_color_slice(digest, &mesh.$colors);
        }};
    }

    hash_layer!(
        earthwork_vertices,
        earthwork_normals,
        earthwork_uvs,
        earthwork_colors
    );
    hash_layer!(curb_vertices, curb_normals, curb_uvs, curb_colors);
    hash_layer!(
        raised_step_vertices,
        raised_step_normals,
        raised_step_uvs,
        raised_step_colors
    );
    hash_layer!(
        sidewalk_vertices,
        sidewalk_normals,
        sidewalk_uvs,
        sidewalk_colors
    );
    hash_layer!(road_vertices, road_normals, road_uvs, road_colors);
    hash_layer!(
        marking_vertices,
        marking_normals,
        marking_uvs,
        marking_colors
    );
    hash_layer!(
        concrete_vertices,
        concrete_normals,
        concrete_uvs,
        concrete_colors
    );
}

fn hash_vector3_slice(digest: &mut u64, values: &[Vector3]) {
    hash_usize(digest, values.len());
    for value in values {
        hash_u32(digest, value.x.to_bits());
        hash_u32(digest, value.y.to_bits());
        hash_u32(digest, value.z.to_bits());
    }
}

fn hash_vector2_slice(digest: &mut u64, values: &[Vector2]) {
    hash_usize(digest, values.len());
    for value in values {
        hash_u32(digest, value.x.to_bits());
        hash_u32(digest, value.y.to_bits());
    }
}

fn hash_color_slice(digest: &mut u64, values: &[Color]) {
    hash_usize(digest, values.len());
    for value in values {
        hash_u32(digest, value.r.to_bits());
        hash_u32(digest, value.g.to_bits());
        hash_u32(digest, value.b.to_bits());
        hash_u32(digest, value.a.to_bits());
    }
}

fn hash_usize(digest: &mut u64, value: usize) {
    for byte in value.to_le_bytes() {
        *digest ^= u64::from(byte);
        *digest = digest.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

fn hash_u32(digest: &mut u64, value: u32) {
    for byte in value.to_le_bytes() {
        *digest ^= u64::from(byte);
        *digest = digest.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

criterion_group!(benches, bench_road_chunk_renderer);
criterion_main!(benches);
