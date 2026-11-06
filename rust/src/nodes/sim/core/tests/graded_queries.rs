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
                tunnel: false,
                median: None,
                parking: crate::simulation::network::graph::RoadParking::NONE,
                attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
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

// `TERRAIN-19`: a tile's interpolation choice in the shared query index is the gather's own
// (`quiet()`): no road or pad seam whose box comes within the cap and window pad of the tile's
// apron rectangle, by Euclidean box distance. A road ending diagonally off a tile's corner, its
// seams inside the axis-aligned cap window but all farther than that, leaves the tile on its
// bilinear base, so `ground_height_near` answers what `ground_height_at` and the gather reference
// do. Before round R5d3 the index counted every seam the window met, and on a curved base the
// two disagreed by micrometres: the debug-only check panicked loading the user's save
// ((34.68, -416.08), 145.0654401 against 145.0654313).
#[test]
fn a_road_beyond_the_cap_off_a_tiles_corner_leaves_it_on_its_base() {
    let (_, strokes, order) = reference_fixtures().swap_remove(0);
    let mut core = committed(&strokes, &order, false);
    core.create_blank_world_internal(512.0, 512.0, 8.0, 128.0, 0.0)
        .expect("blank world");
    core.treasury.balance = 1.0e9;
    // A curved base over the tile [0, 64]^2, where the GL0 triangles leave the bilinear base.
    core.sculpt_terrain_internal(Vector2::new(32.0, 32.0), 40.0, 4.0);
    let points = [Vector2::new(126.0, 126.0), Vector2::new(220.0, 220.0)]
        .map(|p| Vector3::new(p.x, core.get_world_surface_height_internal(p), p.y))
        .to_vec();
    let outcome = core.commit_road(
        RoadCommitRequest {
            points,
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            tunnel: false,
            median: None,
            parking: Default::default(),
            attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
            preview: None,
        },
        std::time::Instant::now(),
        Default::default(),
    );
    assert!(outcome.committed, "{}", outcome.rejection);
    // The precondition: the cap window of the tile's apron meets the road's seams, every one
    // more than the cap and window pad from the apron.
    let apron = [-0.5, -0.5, 64.5, 64.5];
    let reach = crate::simulation::terrain::graded::CAP_M + 1.0;
    let window = [-0.5 - reach, -0.5 - reach, 64.5 + reach, 64.5 + reach];
    let roadbed = &core.transit_network.roadbed;
    let (mut pieces, mut seams) = (Vec::new(), Vec::new());
    roadbed.grounded_pieces_in_bounds(window, &mut pieces);
    roadbed.grounded_seams_from_pieces(window, &pieces, &mut seams);
    assert!(!seams.is_empty(), "the window meets the road's seams");
    for ([a, b], _) in &seams {
        let dx = (a.x.min(b.x) - apron[2])
            .max(apron[0] - a.x.max(b.x))
            .max(0.0);
        let dz = (a.z.min(b.z) - apron[3])
            .max(apron[1] - a.z.max(b.z))
            .max(0.0);
        assert!(dx.hypot(dz) > reach, "a seam {} m off", dx.hypot(dz));
    }
    let cell_m = core.config.zone_cell_m;
    core.allocator.prepare_building_site_query_index(cell_m);
    let surface = GradedSurface {
        terrain: &core.heightmap,
        roadbed: &core.transit_network.roadbed,
        sites: &core.allocator,
        query_index: Some(&core.terrain_query_index),
    };
    let reference = GradedSurface {
        query_index: None,
        ..surface
    };
    for i in 0..16 {
        for j in 0..16 {
            let (x, z) = (2.13 + 3.71 * f64::from(i), 1.37 + 3.83 * f64::from(j));
            let base = reference.base_height_at(x, z, &mut QueryScratch::default());
            let expected = reference.ground_height_at(x, z, &mut QueryScratch::default());
            assert_eq!(
                expected.to_bits(),
                base.to_bits(),
                "({x}, {z}): the bilinear base"
            );
            let mut scratch = QueryScratch::default();
            surface.gather_near(x, z, 1.0, &mut scratch);
            let near = surface.ground_height_near(x, z, &mut scratch);
            let at = surface.ground_height_at(x, z, &mut QueryScratch::default());
            let (rendered, _) =
                surface.surface_height_and_normal_near(x, z, &mut QueryScratch::default());
            for (name, value) in [("near", near), ("at", at), ("surface near", rendered)] {
                assert_eq!(
                    value.to_bits(),
                    expected.to_bits(),
                    "({x}, {z}): {name} {value}, base {expected}"
                );
            }
        }
    }
}
