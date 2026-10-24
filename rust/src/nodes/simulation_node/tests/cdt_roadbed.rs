// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-68-D` acceptance on roadbed fixtures: every tile of a render patch takes its road loops
//! from the roadbed's outer chains, its terrain stays off the footprint, its seam reuses the
//! chain vertices bitwise, closed blocks keep their terrain, and neighbouring tiles agree on
//! their shared sides.

use super::*;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::RoadbedSystem;
use crate::simulation::network::roadbed::fixtures::{connect, point, t_junction};
use crate::simulation::network::types::NodeType;
use std::collections::{BTreeMap, BTreeSet};

// A closed block: four bends on a 120 m square of two-lane roads.
fn ring() -> RegionGraph {
    let mut graph = RegionGraph::new();
    let corners = [(-60.0, -60.0), (60.0, -60.0), (60.0, 60.0), (-60.0, 60.0)]
        .map(|(x, z)| graph.add_node(point(x, z, true), NodeType::Junction));
    for index in 0..4 {
        connect(
            &mut graph,
            corners[index],
            corners[(index + 1) % 4],
            7.0,
            true,
        );
    }
    graph.rebuild_intersection_clips();
    graph
}

fn patch_256() -> TerrainPatchSnapshot {
    TerrainPatchSnapshot {
        patch_x: 0,
        patch_z: 0,
        sample_width: 2,
        sample_height: 2,
        texture_width: 2,
        texture_height: 2,
        inner_offset_x: 0,
        inner_offset_z: 0,
        world_origin_x: -128.0,
        world_origin_z: -128.0,
        world_size_x: 256.0,
        world_size_z: 256.0,
        height_data: vec![0.0; 4],
    }
}

type Bits = (u64, u64, u32);

fn bits(vertex: TerrainCdtVertex) -> Bits {
    (
        vertex.x.to_bits(),
        vertex.z.to_bits(),
        vertex.height_m.to_bits(),
    )
}

#[test]
fn roadbed_tiles_clip_terrain_to_the_footprint_on_shared_chain_vertices() {
    let terrain = TerrainSystem::with_chunking(257, 257, 1.0, 256, 0.0);
    let patch = patch_256();
    for (name, graph) in [("t", t_junction(true)), ("ring", ring())] {
        let mut roadbed = RoadbedSystem::new(64.0);
        roadbed.compile_dirty(&graph);
        assert!(roadbed.failed_nodes().is_empty(), "{name}: nodes compile");
        let plan = SimulationNode::terrain_cdt_window_build_inputs_with_roads(
            &terrain,
            &patch,
            Some((&graph, &roadbed)),
            &[],
            2.0,
            None,
            None,
        );
        assert_eq!(plan.road_error_label, None, "{name}: every chain closes");
        assert!(!plan.windows.is_empty(), "{name}: road tiles are planned");
        let mut sides = BTreeMap::<(i64, bool), Vec<BTreeSet<(i64, i64)>>>::new();
        let mut block_faces = 0;
        for window in plan.windows {
            let core = window.cdt_input.patch;
            // Chain vertices inside the core with each incident chain edge's length. The CDT
            // merges seam edges shorter than its 5 cm source budget and welds vertices inside one
            // 1 mm identity cell (span rows 3 mm apart, a cap apex evaluated twice: HANDOFF-D);
            // every other chain vertex must reach the terrain bitwise.
            let mut shortest = BTreeMap::<Bits, f64>::new();
            for edge in window
                .cdt_input
                .road_loops
                .iter()
                .flat_map(|road_loop| &road_loop.source_edges)
            {
                let length = (edge.end.x - edge.start.x).hypot(edge.end.z - edge.start.z);
                for vertex in [edge.start, edge.end] {
                    let entry = shortest.entry(bits(vertex)).or_insert(f64::MAX);
                    *entry = entry.min(length);
                }
            }
            let interior = shortest
                .iter()
                .filter(|&(&(x, z, _), &length)| {
                    let (x, z) = (f64::from_bits(x), f64::from_bits(z));
                    length >= 0.05
                        && x > core.min_x
                        && x < core.max_x
                        && z > core.min_z
                        && z < core.max_z
                })
                .map(|(vertex, _)| *vertex)
                .collect::<BTreeSet<_>>();
            let mut cells = BTreeMap::<(i64, i64), usize>::new();
            let cell = |&(x, z, _): &Bits| {
                (
                    SimulationNode::quantize_cdt_coord_mm(f64::from_bits(x)),
                    SimulationNode::quantize_cdt_coord_mm(f64::from_bits(z)),
                )
            };
            for vertex in &interior {
                *cells.entry(cell(vertex)).or_default() += 1;
            }
            let seam = interior
                .iter()
                .filter(|vertex| cells[&cell(vertex)] == 1)
                .copied()
                .collect::<BTreeSet<_>>();
            let mesh = build_road_touched_terrain_patch(window.cdt_input)
                .unwrap_or_else(|error| panic!("{name}: tile triangulates: {error:?}"));
            let used = mesh
                .triangles
                .iter()
                .flatten()
                .copied()
                .collect::<BTreeSet<_>>();
            let emitted = used
                .iter()
                .map(|&index| bits(mesh.vertices[index]))
                .collect::<BTreeSet<_>>();
            assert!(
                seam.is_subset(&emitted),
                "{name}: every interior chain vertex is a terrain vertex bitwise"
            );
            for triangle in &mesh.triangles {
                let [a, b, c] = triangle.map(|index| mesh.vertices[index]);
                let (x, z) = ((a.x + b.x + c.x) / 3.0, (a.z + b.z + c.z) / 3.0);
                assert_eq!(
                    roadbed.band_at(x as f32, z as f32),
                    None,
                    "{name}: terrain face centroid ({x:.3}, {z:.3}) lies on the road"
                );
                block_faces += usize::from(x.abs() < 50.0 && z.abs() < 50.0);
            }
            let mut tile_sides = BTreeMap::<(i64, bool), BTreeSet<(i64, i64)>>::new();
            for &index in &used {
                let vertex = mesh.vertices[index];
                let key = |coord: f64| SimulationNode::quantize_cdt_coord_mm(coord);
                for (bound, along, x_side) in [
                    (core.min_x, vertex.z, true),
                    (core.max_x, vertex.z, true),
                    (core.min_z, vertex.x, false),
                    (core.max_z, vertex.x, false),
                ] {
                    let on = if x_side { vertex.x } else { vertex.z };
                    if (on - bound).abs() <= 1.0e-9 {
                        tile_sides
                            .entry((key(bound), x_side))
                            .or_default()
                            .insert((key(along), key(f64::from(vertex.height_m))));
                    }
                }
            }
            for (side, vertices) in tile_sides {
                sides.entry(side).or_default().push(vertices);
            }
        }
        if name == "ring" {
            assert!(block_faces > 0, "the closed block keeps its terrain");
        }
        // Tiles are aligned, so two planned tiles on one side line share its whole extent.
        for ((bound, x_side), tiles) in &sides {
            if let [first, second] = tiles.as_slice() {
                assert_eq!(
                    first,
                    second,
                    "{name}: tiles meeting at {bound} mm ({}) emit the same side vertices",
                    if *x_side { "x" } else { "z" }
                );
            }
        }
    }
}

// Per-tile and whole-fixture terrain cost of the sloped T on the shared pool: the patch plan
// (roadbed loops, envelope, guides, samples) and every tile's CDT. Set the workers with
// RAYON_NUM_THREADS; pin to P-cores for acceptance runs (`docs/roads.md` "Method").
#[test]
#[ignore = "controlled release timing: cargo test --release --lib benchmark_roadbed_terrain_tiles -- --ignored --nocapture"]
fn benchmark_roadbed_terrain_tiles() {
    use rayon::prelude::*;
    use std::time::Instant;
    let terrain = TerrainSystem::with_chunking(257, 257, 1.0, 256, 0.0);
    let patch = patch_256();
    let graph = t_junction(true);
    let mut roadbed = RoadbedSystem::new(64.0);
    roadbed.compile_dirty(&graph);
    let run = || {
        crate::thread_pools::install(|| {
            let start = Instant::now();
            let plan = SimulationNode::terrain_cdt_window_build_inputs_with_roads(
                &terrain,
                &patch,
                Some((&graph, &roadbed)),
                &[],
                2.0,
                None,
                None,
            );
            let planned = start.elapsed();
            let tiles = plan
                .windows
                .into_par_iter()
                .map(|window| {
                    let tile_start = Instant::now();
                    let mesh = build_road_touched_terrain_patch(window.cdt_input).unwrap();
                    (tile_start.elapsed(), mesh.triangles.len())
                })
                .collect::<Vec<_>>();
            (start.elapsed(), planned, tiles)
        })
    };
    for _ in 0..5 {
        run();
    }
    let mut totals = Vec::new();
    let mut plans = Vec::new();
    let mut per_tile = Vec::new();
    let mut tile_count = 0;
    for _ in 0..50 {
        let (total, planned, tiles) = run();
        totals.push(total.as_secs_f64() * 1000.0);
        plans.push(planned.as_secs_f64() * 1000.0);
        tile_count = tiles.len();
        per_tile.extend(tiles.iter().map(|(time, _)| time.as_secs_f64() * 1000.0));
    }
    let median = |values: &mut Vec<f64>| {
        values.sort_by(f64::total_cmp);
        values[values.len() / 2]
    };
    eprintln!(
        "roadbed_terrain_tiles fixture=sloped_t tiles={tile_count} workers={} total_ms_p50={:.3} plan_ms_p50={:.3} tile_cdt_ms_p50={:.3} tile_cdt_ms_max={:.3}",
        crate::thread_pools::install(rayon::current_num_threads),
        median(&mut totals),
        median(&mut plans),
        median(&mut per_tile),
        per_tile.iter().copied().fold(0.0, f64::max),
    );
}
