// SPDX-License-Identifier: GPL-2.0-only

//! Deterministic scaling benchmark for local committed-road chunk generation.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use godot::prelude::{Color, Vector2, Vector3};
use metrum_rise::simulation::core::config::WorldConfig;
use metrum_rise::simulation::network::TransitNetwork;
use metrum_rise::simulation::network::graph::{
    Edge, RegionGraph, RoadMedian, RoadPocket, SidewalkSides,
};
use metrum_rise::simulation::network::render::NetworkMeshData;
use metrum_rise::simulation::network::roadbed::{EdgeEnd, PieceId, SurfaceChunkKey};
use metrum_rise::simulation::network::types::{
    EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
};
use metrum_rise::simulation::terrain::TerrainSystem;
use metrum_rise::simulation::terrain::graded::NoSites;
use std::collections::{BTreeSet, HashSet};
use std::time::Duration;

const EXPECTED_RENDER_CHUNK_SPAN_M: f32 = 510.0;
const ROAD_LENGTH_M: f32 = 32.0;
const ROAD_INSET_M: f32 = 128.0;
const DISTANT_CHUNK_COUNTS: [usize; 4] = [0, 63, 255, 1023];
const DISTANT_GRID_SIDE: i32 = 32;
// The structure fixtures' city sizes: the smallest and the largest background.
const STRUCTURE_DISTANT_CHUNK_COUNTS: [usize; 2] = [0, 1023];
// A structure fixture's deck height above (a bridge) or below (a tunnel) its approaches' far ends.
const STRUCTURE_RISE_M: f32 = 6.0;

// The local road a fixture edits: one grounded edge, or a bridge or a tunnel edge between two
// grounded approaches (`ROAD-61`: a tunnel's lining and headwalls, a bridge's deck and abutments),
// or the grounded edge with a 3 m kerbed grass median (`ROAD-80`: noses, kerbs, the grass layer),
// or the grounded edge without sidewalks (`ROAD-101`: shoulders, and street lights on the verge,
// each standing on the drawn ground from one graded query), or a T whose west arm ends in a 60 m
// right turn pocket (`ROAD-95`: the transition's taper, the asymmetric span and the pocket
// approach's arrows), the pocket edge the local road.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LocalRoad {
    Plain,
    Bridge,
    Tunnel,
    Divided,
    Shoulders,
    Pocket,
}

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
        let mut fixture = build_road_chunk_fixture(distant_chunks, LocalRoad::Plain);
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
                            &NoSites,
                            black_box(&fixture.target_chunks),
                            None,
                            &Default::default(),
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

        fixture.network.roadbed.mark_edge_dirty(fixture.local_edge);
        let dirty_warmup = fixture
            .network
            .try_generate_mesh_chunks(
                &fixture.graph,
                &fixture.terrain,
                &NoSites,
                &fixture.target_chunks,
                None,
                &Default::default(),
            )
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
                        .roadbed
                        .mark_edge_dirty(black_box(fixture.local_edge));
                    let chunks = black_box(&mut fixture.network)
                        .try_generate_mesh_chunks(
                            black_box(&fixture.graph),
                            black_box(&fixture.terrain),
                            &NoSites,
                            black_box(&fixture.target_chunks),
                            None,
                            &Default::default(),
                        )
                        .expect("the local dirty compile must publish successfully");
                    black_box(mesh_map_shape(&chunks));
                });
            },
        );
    }

    // A bridge's and a tunnel's local chunk: emission only, the city size held as above.
    for (local, name) in [
        (LocalRoad::Bridge, "bridge_chunk_emit_only"),
        (LocalRoad::Tunnel, "tunnel_chunk_emit_only"),
        (LocalRoad::Divided, "divided_chunk_emit_only"),
        (LocalRoad::Shoulders, "shoulder_chunk_emit_only"),
        (LocalRoad::Pocket, "pocket_chunk_emit_only"),
    ] {
        let mut signature = None;
        for distant_chunks in STRUCTURE_DISTANT_CHUNK_COUNTS {
            let mut fixture = build_road_chunk_fixture(distant_chunks, local);
            if let Some(expected) = signature {
                assert_eq!(
                    fixture.expected_signature, expected,
                    "local structure mesh output changed with city size"
                );
            } else {
                signature = Some(fixture.expected_signature);
                println!(
                    "ROAD_CHUNK_STRUCTURE local={local:?} chunks={} vertices={} digest={:016x}",
                    fixture.expected_signature.chunk_count,
                    fixture.expected_signature.vertex_count,
                    fixture.expected_signature.digest,
                );
            }
            let occupied_chunks = fixture.occupied_chunks;
            group.bench_with_input(
                BenchmarkId::new(name, occupied_chunks),
                &occupied_chunks,
                |b, _| {
                    b.iter(|| {
                        let chunks = black_box(&mut fixture.network)
                            .try_generate_mesh_chunks(
                                black_box(&fixture.graph),
                                black_box(&fixture.terrain),
                                &NoSites,
                                black_box(&fixture.target_chunks),
                                None,
                                &Default::default(),
                            )
                            .expect("the clean benchmark surface must stay published");
                        black_box(mesh_map_shape(&chunks));
                    });
                },
            );
        }
    }

    group.finish();
}

fn build_road_chunk_fixture(distant_chunks: usize, local: LocalRoad) -> RoadChunkBenchFixture {
    let config = WorldConfig::gameplay_default();
    let render_chunk_span_m = config.terrain_render_chunk_span_m();
    assert_eq!(
        render_chunk_span_m.to_bits(),
        EXPECTED_RENDER_CHUNK_SPAN_M.to_bits(),
        "the benchmark fixture must track the production render span"
    );
    let (chunk_origin_x_m, chunk_origin_z_m) = config.terrain_world_origin_m();
    let terrain = TerrainSystem::from_world_config(&config);
    let mut graph = RegionGraph::new();
    let local_edge = match local {
        LocalRoad::Plain => add_world_road(&mut graph, -ROAD_LENGTH_M * 0.5, 0.0, &config),
        LocalRoad::Bridge => add_structure_chain(&mut graph, EdgeClass::Bridge, STRUCTURE_RISE_M),
        LocalRoad::Tunnel => {
            add_structure_chain(&mut graph, EdgeClass::Tunnel, -STRUCTURE_RISE_M)
        }
        LocalRoad::Divided => {
            let edge = add_world_road(&mut graph, -ROAD_LENGTH_M * 0.5, 0.0, &config);
            let grass = RoadMedian::from_tool(2, 3.0).expect("a valid grass median");
            let road = graph.edge_mut(edge);
            road.median = grass;
            road.width += 3.0;
            edge
        }
        LocalRoad::Shoulders => {
            let edge = add_world_road(&mut graph, -ROAD_LENGTH_M * 0.5, 0.0, &config);
            graph.edge_mut(edge).allowed_types = TransitFlags::CAR;
            graph.edge_mut(edge).sidewalks = SidewalkSides::NONE;
            edge
        }
        LocalRoad::Pocket => add_pocket_junction(&mut graph),
    };
    let local_chunk = (
        ((0.0 - chunk_origin_x_m) / render_chunk_span_m).floor() as i32,
        ((0.0 - chunk_origin_z_m) / render_chunk_span_m).floor() as i32,
    );
    let mut added_distant_chunks = 0;
    if distant_chunks > 0 {
        'chunks: for chunk_z in 0..DISTANT_GRID_SIDE {
            for chunk_x in 0..DISTANT_GRID_SIDE {
                if (chunk_x, chunk_z) == local_chunk {
                    continue;
                }
                add_isolated_chunk_road(
                    &mut graph,
                    chunk_x,
                    chunk_z,
                    render_chunk_span_m,
                    chunk_origin_x_m,
                    chunk_origin_z_m,
                    &config,
                );
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
    if local == LocalRoad::Pocket {
        // The junction and the transition are laid out on solved planes, as an edit lays them
        // out; the other fixtures have no such node, so their surfaces are as before.
        let nodes: HashSet<u32> = (0..graph.node_count() as u32).collect();
        graph.finalize_junction_endpoint_profiles(&nodes, None, &mut HashSet::new());
    }

    let mut network = TransitNetwork::new_for_world(&config);
    network.lane_system.rebuild(&mut graph);
    network.roadbed.compile_dirty(&graph);
    assert!(network.roadbed.is_current());

    let roadbed = &network.roadbed;
    let chunk_pieces =
        |chunk: SurfaceChunkKey| roadbed.pieces_in_bounds(roadbed.chunk_bounds(chunk));
    let target_chunks = roadbed
        .occupied_chunks()
        .filter(|&chunk| chunk_pieces(chunk).any(|(piece, _)| piece == PieceId::Span(local_edge)))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        target_chunks,
        BTreeSet::from([local_chunk]),
        "a typical central edit must stay wholly inside one terrain-aligned render chunk"
    );

    let occupied_chunks = roadbed.occupied_chunks().count();
    assert_eq!(
        occupied_chunks,
        distant_chunks + 1,
        "each isolated fixture road must occupy exactly one distinct chunk"
    );

    let target_pieces = target_chunks
        .iter()
        .flat_map(|&chunk| chunk_pieces(chunk).map(|(piece, _)| piece))
        .collect::<BTreeSet<_>>();
    let target_edge_count = target_pieces
        .iter()
        .filter(|piece| matches!(piece, PieceId::Span(_)))
        .count();
    let target_node_count = target_pieces.len() - target_edge_count;

    let expected_chunks = network
        .try_generate_mesh_chunks(
            &graph,
            &terrain,
            &NoSites,
            &target_chunks,
            None,
            &Default::default(),
        )
        .expect("the initial benchmark surface must publish");
    let expected_signature = mesh_map_signature(&expected_chunks);
    assert_eq!(
        expected_chunks
            .values()
            .any(|mesh| !mesh.median_grass_vertices.is_empty()),
        local == LocalRoad::Divided,
        "only the divided road's chunk has a grass layer"
    );
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

fn add_isolated_chunk_road(
    graph: &mut RegionGraph,
    chunk_x: i32,
    chunk_z: i32,
    render_chunk_span_m: f32,
    chunk_origin_x_m: f32,
    chunk_origin_z_m: f32,
    config: &WorldConfig,
) -> usize {
    let start_x = chunk_origin_x_m + chunk_x as f32 * render_chunk_span_m + ROAD_INSET_M;
    let start_z = chunk_origin_z_m + chunk_z as f32 * render_chunk_span_m + ROAD_INSET_M;
    add_world_road(graph, start_x, start_z, config)
}

fn add_world_road(
    graph: &mut RegionGraph,
    start_x: f32,
    start_z: f32,
    config: &WorldConfig,
) -> usize {
    let end_x = start_x + ROAD_LENGTH_M;
    let half_width_m = config.width_m * 0.5;
    let half_height_m = config.height_m * 0.5;
    assert!(start_x >= -half_width_m && end_x <= half_width_m);
    assert!(start_z >= -half_height_m && start_z <= half_height_m);
    let start = graph.add_node(Vector3::new(start_x, 0.0, start_z), NodeType::Junction);
    let end = graph.add_node(Vector3::new(end_x, 0.0, start_z), NodeType::Junction);
    let (start_pos, end_pos) = (graph.node(start).pos, graph.node(end).pos);
    add_edge(graph, (start, end), (start_pos, end_pos), EdgeClass::Standard)
}

// A two-lane road edge from `start` to `end` through their midpoint.
fn add_edge(
    graph: &mut RegionGraph,
    (start, end): (u32, u32),
    (start_pos, end_pos): (Vector3, Vector3),
    class: EdgeClass,
) -> usize {
    let points = vec![start_pos, (start_pos + end_pos) * 0.5, end_pos];
    graph.add_edge(Edge {
        start_node: start,
        end_node: end,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 0.0,
        physical_length: start_pos.distance_to(end_pos),
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: points.clone(),
        physical_geometry: points,
        deleted: false,
        no_building_spawn: false,
        no_street_lights: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        median: None,
        pocket: None,
        sidewalks: SidewalkSides::BOTH,
        roundabout: None,
    })
}

// A T at the origin whose west arm, from 150 m west, ends in a right turn pocket from its lane
// transition 60 m west of the junction; 80 m east and south arms (the south one the right turn).
// Returns the pocket edge.
fn add_pocket_junction(graph: &mut RegionGraph) -> usize {
    let node = |graph: &mut RegionGraph, x: f32, z: f32, kind: NodeType| {
        graph.add_node(Vector3::new(x, 0.0, z), kind)
    };
    let center = node(graph, 0.0, 0.0, NodeType::Junction);
    let transition = node(graph, -60.0, 0.0, NodeType::LaneTransition);
    let west = node(graph, -150.0, 0.0, NodeType::Junction);
    let east = node(graph, 80.0, 0.0, NodeType::Junction);
    let south = node(graph, 0.0, 80.0, NodeType::Junction);
    let mut connect = |a: u32, b: u32| {
        let (start, end) = (graph.node(a).pos, graph.node(b).pos);
        add_edge(graph, (a, b), (start, end), EdgeClass::Standard)
    };
    connect(west, transition);
    let pocket = connect(transition, center);
    connect(center, east);
    connect(south, center);
    graph.set_edge_pocket(
        pocket,
        Some(RoadPocket {
            end: EdgeEnd::End,
            left: false,
            right: true,
        }),
    );
    pocket
}

// A grounded approach, a `class` edge with its deck `rise_m` above the approaches' far ends, and
// a second approach, west to east along z = 0 from x = -16 m: the three edges inside the local
// chunk. Returns the structure edge.
fn add_structure_chain(graph: &mut RegionGraph, class: EdgeClass, rise_m: f32) -> usize {
    let start_x = -ROAD_LENGTH_M * 0.5;
    let heights = [0.0, rise_m, rise_m, 0.0];
    let nodes: Vec<u32> = heights
        .iter()
        .enumerate()
        .map(|(k, &y)| {
            let x = start_x + k as f32 * ROAD_LENGTH_M;
            graph.add_node(Vector3::new(x, y, 0.0), NodeType::Junction)
        })
        .collect();
    let mut structure = 0;
    for k in 0..3 {
        let (start, end) = (graph.node(nodes[k]).pos, graph.node(nodes[k + 1]).pos);
        let edge_class = if k == 1 { class } else { EdgeClass::Standard };
        let edge = add_edge(graph, (nodes[k], nodes[k + 1]), (start, end), edge_class);
        if k == 1 {
            structure = edge;
        }
    }
    structure
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
    mesh.skirt_vertices.len()
        + mesh.curb_vertices.len()
        + mesh.raised_step_vertices.len()
        + mesh.sidewalk_vertices.len()
        + mesh.road_vertices.len()
        + mesh.marking_vertices.len()
        + mesh.concrete_vertices.len()
        + mesh.lining_vertices.len()
        + mesh.median_grass_vertices.len()
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

    hash_layer!(skirt_vertices, skirt_normals, skirt_uvs, skirt_colors);
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
    // Only a tunnel's chunk has a lining (`ROAD-61`), so every other digest stays comparable
    // with the layer's predecessors'.
    if !mesh.lining_vertices.is_empty() {
        hash_layer!(lining_vertices, lining_normals, lining_uvs, lining_colors);
    }
    // Likewise only a kerbed grass median's chunk has grass (`ROAD-80`).
    if !mesh.median_grass_vertices.is_empty() {
        hash_layer!(
            median_grass_vertices,
            median_grass_normals,
            median_grass_uvs,
            median_grass_colors
        );
    }
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
