// SPDX-License-Identifier: GPL-2.0-only

use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use godot::prelude::{Vector2, Vector3};
use metrum_rise::config;
use metrum_rise::simulation::network::graph::{Edge, RegionGraph};
use metrum_rise::simulation::network::surface::RoadSurfaceSystem;
use metrum_rise::simulation::network::types::{
    EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
};
use metrum_rise::simulation::terrain::TerrainSystem;
use std::time::Duration;

const GRID_NODES_PER_AXIS: usize = 18;
const GRID_SPACING_M: f32 = 36.0;
const GRID_ORIGIN_M: f32 = 96.0;
const TERRAIN_CELLS: usize = 513;
const TERRAIN_CELL_SIZE_M: f32 = 2.0;
const SURFACE_CHUNK_SPAN_M: f32 = 128.0;

struct SurfaceBenchSetup {
    graph: RegionGraph,
    terrain: TerrainSystem,
    dirty_edge: usize,
    terrain_edit_center: Vector2,
}

fn bench_surface_system(c: &mut Criterion) {
    let setup = build_surface_bench_setup();
    let mut group = c.benchmark_group("RoadSurfaceSystem");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(2));

    group.bench_function("compile_all_grid", |b| {
        b.iter_batched(
            || RoadSurfaceSystem::new(SURFACE_CHUNK_SPAN_M),
            |mut surface| {
                surface.compile_dirty(&setup.graph, &setup.terrain);
                black_box(surface.compiled_sections().len());
                black_box(surface.compiled_visual_span_pieces().len());
                black_box(surface.compiled_visual_node_pieces().len());
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("compile_dirty_single_edge", |b| {
        b.iter_batched(
            || {
                let mut surface = RoadSurfaceSystem::new(SURFACE_CHUNK_SPAN_M);
                surface.compile_dirty(&setup.graph, &setup.terrain);
                surface
            },
            |mut surface| {
                surface.mark_edge_dirty(&setup.graph, setup.dirty_edge);
                surface.compile_dirty(&setup.graph, &setup.terrain);
                black_box(surface.compiled_sections().len());
                black_box(surface.compiled_visual_span_pieces().len());
                black_box(surface.compiled_visual_node_pieces().len());
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("compile_dirty_terrain_edit", |b| {
        b.iter_batched(
            || {
                let mut surface = RoadSurfaceSystem::new(SURFACE_CHUNK_SPAN_M);
                surface.compile_dirty(&setup.graph, &setup.terrain);
                surface
            },
            |mut surface| {
                surface.mark_terrain_edit_dirty(
                    &setup.graph,
                    setup.terrain_edit_center,
                    GRID_SPACING_M * 1.75,
                );
                surface.compile_dirty(&setup.graph, &setup.terrain);
                black_box(surface.compiled_sections().len());
                black_box(surface.compiled_visual_span_pieces().len());
                black_box(surface.compiled_visual_node_pieces().len());
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("rebuild_all_earthworks_grid", |b| {
        b.iter_batched(
            || {
                let mut surface = RoadSurfaceSystem::new(SURFACE_CHUNK_SPAN_M);
                let terrain = build_bench_terrain();
                surface.compile_dirty(&setup.graph, &terrain);
                (surface, terrain)
            },
            |(mut surface, mut terrain)| {
                let chunks = surface.rebuild_all_earthworks(&setup.graph, &mut terrain);
                black_box(chunks.len());
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn build_surface_bench_setup() -> SurfaceBenchSetup {
    let terrain = build_bench_terrain();
    let mut graph = RegionGraph::new();
    let mut node_ids = vec![vec![0_u32; GRID_NODES_PER_AXIS]; GRID_NODES_PER_AXIS];

    for (z, row) in node_ids.iter_mut().enumerate() {
        for (x, node_id) in row.iter_mut().enumerate() {
            let world_x = GRID_ORIGIN_M + x as f32 * GRID_SPACING_M;
            let world_z = GRID_ORIGIN_M + z as f32 * GRID_SPACING_M;
            *node_id = graph.add_node(bench_point(world_x, world_z), NodeType::Junction);
        }
    }

    let mut dirty_edge = 0;
    for z in 0..GRID_NODES_PER_AXIS {
        for x in 0..GRID_NODES_PER_AXIS - 1 {
            let start = node_ids[z][x];
            let end = node_ids[z][x + 1];
            let edge_idx = graph.add_edge(bench_edge(start, end, x, z, true));
            if x == GRID_NODES_PER_AXIS / 2 && z == GRID_NODES_PER_AXIS / 2 {
                dirty_edge = edge_idx;
            }
        }
    }
    for z in 0..GRID_NODES_PER_AXIS - 1 {
        for x in 0..GRID_NODES_PER_AXIS {
            let start = node_ids[z][x];
            let end = node_ids[z + 1][x];
            graph.add_edge(bench_edge(start, end, x, z, false));
        }
    }

    let terrain_edit_center = Vector2::new(
        GRID_ORIGIN_M + GRID_SPACING_M * GRID_NODES_PER_AXIS as f32 * 0.5,
        GRID_ORIGIN_M + GRID_SPACING_M * GRID_NODES_PER_AXIS as f32 * 0.5,
    );

    SurfaceBenchSetup {
        graph,
        terrain,
        dirty_edge,
        terrain_edit_center,
    }
}

fn build_bench_terrain() -> TerrainSystem {
    let mut terrain =
        TerrainSystem::with_chunking(TERRAIN_CELLS, TERRAIN_CELLS, TERRAIN_CELL_SIZE_M, 64, 0.0);
    for z in 0..TERRAIN_CELLS {
        for x in 0..TERRAIN_CELLS {
            let world_x = x as f32 * TERRAIN_CELL_SIZE_M;
            let world_z = z as f32 * TERRAIN_CELL_SIZE_M;
            terrain.set_height(x, z, bench_raw_height(world_x, world_z));
        }
    }
    terrain
}

fn bench_edge(start_node: u32, end_node: u32, x: usize, z: usize, horizontal: bool) -> Edge {
    let start_x = GRID_ORIGIN_M + x as f32 * GRID_SPACING_M;
    let start_z = GRID_ORIGIN_M + z as f32 * GRID_SPACING_M;
    let end_x = start_x + if horizontal { GRID_SPACING_M } else { 0.0 };
    let end_z = start_z + if horizontal { 0.0 } else { GRID_SPACING_M };
    let mid_x = (start_x + end_x) * 0.5;
    let mid_z = (start_z + end_z) * 0.5;
    let points = vec![
        bench_point(start_x, start_z),
        bench_point(mid_x, mid_z),
        bench_point(end_x, end_z),
    ];
    let physical_length = points
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
        speed_limit: 50.0,
        base_cost: 0.0,
        physical_length,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: points.clone(),
        physical_geometry: points,
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    }
}

fn bench_point(world_x: f32, world_z: f32) -> Vector3 {
    Vector3::new(
        world_x,
        bench_raw_height(world_x, world_z) * config::HEIGHT_SCALE,
        world_z,
    )
}

fn bench_raw_height(world_x: f32, world_z: f32) -> f32 {
    (world_x * 0.011).sin() * 0.04 + (world_z * 0.017).cos() * 0.035
}

criterion_group!(benches, bench_surface_system);
criterion_main!(benches);
