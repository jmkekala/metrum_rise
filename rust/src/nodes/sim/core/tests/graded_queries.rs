// SPDX-License-Identifier: GPL-2.0-only

//! `TERRAIN-07-D` item 4: Godot's surface queries (`get_world_surface_height`,
//! `intersect_world_surface`) answer from the graded surface the tiles render, on the sloped
//! reference T: the road's height on the road, the graded ground beside it, and a vertical ray
//! lands on the same surface.

use super::SimCore;
use super::road_edit_order::committed;
use super::road_reference_probe::reference_fixtures;
use crate::nodes::sim::core::road_commit::RoadCommitRequest;
use crate::simulation::terrain::graded::{GradedSurface, QueryScratch};
use godot::prelude::{Vector2, Vector3};

fn graded_height(core: &SimCore, x: f32, z: f32) -> f32 {
    let surface = GradedSurface {
        terrain: &core.heightmap,
        roadbed: &core.transit_network.roadbed,
        sites: &core.allocator,
        query_index: Some(&core.terrain_query_index),
    };
    surface.surface_height_at(f64::from(x), f64::from(z), &mut QueryScratch::default()) as f32
}

#[test]
fn surface_queries_answer_from_the_graded_surface() {
    let (_, strokes, order) = reference_fixtures().swap_remove(0);
    let mut core = committed(&strokes, &order, true);
    let (mut on_road, mut graded) = (0, 0);
    // Across the east arm and beside the side road: road, curb returns, cones and natural ground.
    for i in 0..60 {
        for j in 0..12 {
            let (x, z) = (-30.0 + 1.07 * i as f32, -14.0 + 2.31 * j as f32);
            let expected = graded_height(&core, x, z);
            let cell_m = core.config.zone_cell_m;
            core.allocator.prepare_building_site_query_index(cell_m);
            let height = core.get_world_surface_height_internal(Vector2::new(x, z));
            assert_eq!(height.to_bits(), expected.to_bits(), "({x}, {z})");
            match core.transit_network.roadbed.height_at(x, z) {
                Some(road) => {
                    assert_eq!(height.to_bits(), road.to_bits(), "road at ({x}, {z})");
                    on_road += 1;
                }
                None => {
                    let base =
                        core.heightmap.sample_height_world(x, z) * crate::config::HEIGHT_SCALE;
                    graded += usize::from((height - base).abs() > 1e-3);
                }
            }
            // A vertical ray meets the same surface at the same point (one plane, two
            // evaluation orders: f32 rounding apart).
            let hit = core
                .intersect_world_surface_internal(
                    Vector3::new(x, 200.0, z),
                    Vector3::new(0.0, -1.0, 0.0),
                )
                .expect("a vertical ray meets the surface");
            assert_eq!((hit.x, hit.z), (x, z));
            assert!(
                (hit.y - height).abs() <= 1e-4,
                "({x}, {z}): ray {} query {height}",
                hit.y
            );
        }
    }
    assert!(
        on_road > 0 && graded > 0,
        "road {on_road}, graded ground {graded}"
    );
}

fn reset_and_commit(core: &mut SimCore, fixture: usize, rise_m: f32) {
    core.create_blank_world_internal(512.0, 512.0, 8.0, 128.0, 0.0)
        .expect("blank world");
    core.treasury.balance = 1.0e9;
    if rise_m > 0.0 {
        core.slope_terrain_internal(
            Vector2::ZERO,
            256.0,
            Vector2::new(-128.0, 0.0),
            0.0,
            Vector2::new(128.0, 0.0),
            rise_m,
            1.0,
        );
    }
    let (_, strokes, order) = reference_fixtures().swap_remove(fixture);
    for index in order {
        let (ends, lanes) = strokes[index];
        let points = ends
            .map(|point| {
                Vector3::new(
                    point.x,
                    core.get_world_surface_height_internal(point),
                    point.y,
                )
            })
            .to_vec();
        let outcome = core.commit_road(
            RoadCommitRequest {
                points,
                fwd_lanes: lanes,
                bkw_lanes: lanes,
                snap_to_existing_roads: true,
                preview: None,
            },
            std::time::Instant::now(),
            Default::default(),
        );
        assert!(outcome.committed, "stroke {index}: {}", outcome.rejection);
    }
}

#[test]
fn steep_t_near_query_matches_the_canonical_grade_at_the_south_reach() {
    let (_, strokes, order) = reference_fixtures().swap_remove(0);
    let mut core = committed(&strokes, &order, false);
    reset_and_commit(&mut core, 0, 20.48);
    let surface = GradedSurface {
        terrain: &core.heightmap,
        roadbed: &core.transit_network.roadbed,
        sites: &core.allocator,
        query_index: Some(&core.terrain_query_index),
    };
    let mut scratch = QueryScratch::default();
    let (x, z) = (29.180_641_174_316_406, -96.002_975_463_867_19);
    // A window gathered around another point may hold no seam: that says nothing about this
    // point's own tile, which the steep T's south reach grades (round 1's stale `quiet()`).
    surface.gather_near(x + 6.0, z + 6.0, 6.01, &mut scratch);
    let near = surface.ground_height_near(x, z, &mut scratch);
    let reference = GradedSurface {
        query_index: None,
        ..surface
    }
    .ground_height_at(x, z, &mut QueryScratch::default());
    assert_eq!(
        near.to_bits(),
        reference.to_bits(),
        "near {near}, at {reference}"
    );
}
