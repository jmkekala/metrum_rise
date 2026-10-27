// SPDX-License-Identifier: GPL-2.0-only

//! `TERRAIN-07-D` item 4: Godot's surface queries (`get_world_surface_height`,
//! `intersect_world_surface`) answer from the graded surface the tiles render, on the sloped
//! reference T: the road's height on the road, the graded ground beside it, and a vertical ray
//! lands on the same surface.

use super::SimCore;
use super::road_edit_order::committed;
use super::road_reference_probe::reference_fixtures;
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
