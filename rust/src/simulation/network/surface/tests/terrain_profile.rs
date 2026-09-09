// SPDX-License-Identifier: GPL-2.0-only

//! Source-terrain and staged profile regressions from the immutable Kuopio capture.

use super::*;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::zoning::ZoningSystem;
use std::sync::OnceLock;

fn kuopio_terrain() -> &'static TerrainSystem {
    static TERRAIN: OnceLock<TerrainSystem> = OnceLock::new();
    TERRAIN.get_or_init(|| {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../benchmarks/fixtures/kuopio-terrain/kuopio-terrain-map.sqlite"
        );
        let db =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        let source: Vec<u8> = db
            .query_row("SELECT height_blob_f32_le FROM terrain_state", [], |row| {
                row.get(0)
            })
            .unwrap();
        let heights: Vec<f32> = source
            .chunks_exact(4)
            .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
            .collect();
        let config = WorldConfig::new(18_000.0, 18_000.0, 40.0, 10.0);
        let mut terrain = TerrainSystem::from_world_config(&config);
        terrain.replace_source_from_dense(&heights).unwrap();
        terrain
    })
}

fn case_strokes(case_id: &str, terrain: &TerrainSystem) -> Vec<Vec<Vector3>> {
    let manifest: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../benchmarks/fixtures/kuopio-terrain/placements.json",
    )))
    .unwrap();
    let case = manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["case_id"] == case_id)
        .unwrap();
    case["segments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|stroke| {
            ["start_xz", "end_xz"]
                .map(|key| {
                    let x = stroke[key][0].as_f64().unwrap() as f32;
                    let z = stroke[key][1].as_f64().unwrap() as f32;
                    Vector3::new(
                        x,
                        terrain.sample_height_world(x, z) * crate::config::HEIGHT_SCALE,
                        z,
                    )
                })
                .to_vec()
        })
        .collect()
}

fn assert_profile(points: &[Vector3], terrain: &TerrainSystem, stage: &str) {
    for point in points {
        let source = terrain.sample_height_world(point.x, point.z) * crate::config::HEIGHT_SCALE;
        assert!(
            (point.y - source).abs() < 5.0,
            "{stage}: source offset {} m at {point:?}",
            point.y - source
        );
    }
    for span in points.windows(2) {
        let run = (span[1].x - span[0].x).hypot(span[1].z - span[0].z);
        let grade = (span[1].y - span[0].y).abs() / run;
        assert!(
            grade <= 1.0,
            "{stage}: grade {grade} over {run} m: {span:?}"
        );
    }
}

#[test]
fn kuopio_04_preparation_preserves_source_relief() {
    let terrain = kuopio_terrain();
    let raw = &case_strokes("kuopio_04", terrain)[0];
    let (points, class) = RoadSurfaceSystem::prepare_road_input_points(raw, terrain);
    assert_eq!(class, EdgeClass::Standard);
    assert_profile(&points, terrain, "prepared");
}

#[test]
fn kuopio_isolated_profiles_preserve_interior_relief() {
    let terrain = kuopio_terrain();
    for case in [
        "kuopio_03",
        "kuopio_05",
        "kuopio_06",
        "kuopio_08",
        "kuopio_09",
    ] {
        let raw = &case_strokes(case, terrain)[0];
        let (points, class) = RoadSurfaceSystem::prepare_road_input_points(raw, terrain);
        assert_eq!(class, EdgeClass::Standard);
        assert_profile(&points, terrain, case);
    }
}

#[test]
fn steep_planar_source_does_not_accumulate_grade_error_at_a_pin() {
    let terrain = planar_world_terrain(129, 33, 1.0, 0.0, 0.24, 0.0);
    for (start, end) in [(-48.0, 48.0), (48.0, -48.0)] {
        let raw = [start, end].map(|x| {
            Vector3::new(
                x,
                terrain.sample_height_world(x, 0.0) * crate::config::HEIGHT_SCALE,
                0.0,
            )
        });
        let (points, class) = RoadSurfaceSystem::prepare_road_input_points(&raw, &terrain);
        assert_eq!(class, EdgeClass::Standard);
        for point in points {
            let source =
                terrain.sample_height_world(point.x, point.z) * crate::config::HEIGHT_SCALE;
            assert!(
                (point.y - source).abs() < 0.05,
                "planar source distorted: {point:?} vs {source}"
            );
        }
    }
}

#[test]
fn kuopio_07_junction_stages_do_not_introduce_vertical_spans() {
    let terrain = kuopio_terrain();
    let mut graph = RegionGraph::new();
    let mut network = TransitNetwork::new_with_surface_chunk_span(510.0);
    let mut zoning = ZoningSystem::new(&WorldConfig::new(18_000.0, 18_000.0, 40.0, 10.0));
    let mut allocator = BuildingAllocator::new();
    network.road_surface.compile_dirty(&graph, terrain);
    for raw in case_strokes("kuopio_07", terrain) {
        let prepared = RoadSurfaceSystem::prepare_road_input_for_tool(
            &raw,
            terrain,
            &graph,
            &network.road_surface,
            true,
        );
        assert!(prepared.extension.is_none());
        assert_profile(&prepared.points, terrain, "prepared");
        network.bulk_load = true;
        network.add_road(
            &mut graph,
            prepared.points,
            1,
            1,
            prepared.class,
            &mut zoning,
            &mut allocator,
        );
        network.bulk_load = false;
        for edge in graph.edges().iter().filter(|edge| !edge.deleted) {
            assert_profile(&edge.physical_geometry, terrain, "topology");
        }
        let finalized = network.finalize_road_geometry(&mut graph);
        let nodes = finalized.affected_nodes;
        let edges = finalized.dirty_edges;
        for edge in graph.edges().iter().filter(|edge| !edge.deleted) {
            assert_profile(&edge.physical_geometry, terrain, "final plan profile");
        }
        let physical_before_clips: Vec<_> = graph
            .edges()
            .iter()
            .map(|edge| edge.physical_geometry.clone())
            .collect();
        graph.rebuild_intersection_clips_for_nodes(&nodes);
        graph.rebuild_intersection_clips();
        for (edge, before) in graph.edges().iter().zip(&physical_before_clips) {
            assert_eq!(
                &edge.physical_geometry, before,
                "clip-only rebuild changed physical heights"
            );
        }
        for edge in graph.edges().iter().filter(|edge| !edge.deleted) {
            assert_profile(&edge.physical_geometry, terrain, "clip rebuild");
        }
        network.mark_surface_dirty_from_sets(&graph, &edges, &nodes);
        assert!(
            network.road_surface.compile_dirty(&graph, terrain),
            "surface compilation failed: {:?}",
            network.road_surface.last_compile_failure_label()
        );
    }
}
