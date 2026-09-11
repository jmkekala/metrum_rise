// SPDX-License-Identifier: GPL-2.0-only

//! Zoned yard coverage on sloping roads, using production placement and terrain compilation.

use super::*;
use crate::nodes::simulation_node::SimulationNode;
use crate::simulation::network::surface::{RoadVec3, road_ray_triangle_intersection_t};
use godot::prelude::Vector2;
use std::sync::Arc;

fn register_yard(core: &mut SimCore) -> String {
    let mut manifest: AssetManifest = toml::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../benchmarks/fixtures/kuopio-terrain/building-site.toml"
    )))
    .unwrap();
    // Portable box geometry; production obtains these bounds from the model importer.
    manifest.mesh_parts[0].imported_bounds = Some([[-0.75, 0.0, -0.75], [0.75, 1.0, 0.75]]);
    let id = format!("test:{}", manifest.asset_id);
    core.allocator
        .registry
        .register("test", manifest, String::new());
    id
}

fn place_yard(
    core: &mut SimCore,
    edge: usize,
    side: f32,
    asset: &str,
) -> Result<usize, crate::simulation::buildings::allocator::DemandSpawnPlacementRejection> {
    place_yard_at(core, edge, side, 0.5, asset)
}

fn place_yard_at(
    core: &mut SimCore,
    edge: usize,
    side: f32,
    t: f32,
    asset: &str,
) -> Result<usize, crate::simulation::buildings::allocator::DemandSpawnPlacementRejection> {
    let building = core
        .allocator
        .registry
        .get(asset)
        .unwrap()
        .manifest
        .building
        .as_ref()
        .unwrap();
    let width = building.lot_width_cells as f32 * core.config.zone_cell_m;
    let depth = building.lot_depth_cells as f32 * core.config.zone_cell_m;
    let zone = crate::simulation::buildings::allocator::zone_class_to_zone_type(
        building.zone_type.unwrap(),
    );
    let midpoint = BuildingAllocator::sample_pos_on_edge(&core.region_graph, edge, t);
    let tangent = BuildingAllocator::sample_tangent_on_edge(&core.region_graph, edge, t);
    let point = midpoint + Vector2::new(tangent.y, -tangent.x) * side * (10.0 + depth * 0.5);
    let profile = core
        .zoning
        .profiles
        .default_runtime_id_for_zone_type(zone)
        .unwrap();
    let parcel = core
        .zoning
        .place_or_rezone_parcel_at(point.x, point.y, profile, width, depth, &core.region_graph)
        .unwrap();
    assert_eq!(
        core.zoning
            .parcel_by_raw_id(parcel.raw())
            .unwrap()
            .edge_idx(),
        edge
    );
    let catalog = load_runtime_economy_catalog().unwrap();
    let building = core.allocator.execute_demand_spawn_action(
        &DemandSpawnAction {
            parcel_id: parcel.raw(),
            asset_id: asset.to_owned(),
        },
        &mut core.zoning,
        &core.region_graph,
        &core.transit_network.road_surface,
        &core.heightmap,
        &catalog,
        core.demand.runtime_tuning(),
    )?;
    assert!(
        core.allocator.building_site_dirty_bounds.is_some(),
        "zoned placement must publish its own site change, just like explicit placement"
    );
    core.publish_pending_building_site_changes();
    Ok(building)
}

fn settle_and_check_yards(core: &mut SimCore) -> usize {
    let inputs = core.collect_refined_terrain_patch_build_inputs(2.0);
    let patches = SimCore::build_refined_terrain_patch_cache_entries(inputs);
    for patch in &patches {
        if let Some(failure) = SimulationNode::cached_refined_cdt_failure_label(patch) {
            let mut worst = Vec::new();
            for window in &patch.windows {
                if let Ok(mesh) = &window.mesh_result {
                    for indices in &mesh.triangles {
                        let p = indices.map(|i| {
                            let p = mesh.vertices[i];
                            RoadVec3::new(p.x, f64::from(p.height_m), p.z)
                        });
                        let normal = (p[1] - p[0]).cross(p[2] - p[0]);
                        let slope = normal.x.hypot(normal.z) / normal.y.abs();
                        if slope > 2.0 {
                            worst.push((slope, p));
                        }
                    }
                }
            }
            worst.sort_by(|a, b| b.0.total_cmp(&a.0));
            panic!(
                "patch {:?}: {failure}; worst {:?}",
                patch.key,
                &worst[..worst.len().min(8)]
            );
        }
    }
    core.insert_refined_terrain_patch_cache_entries(patches);
    let mut graded_samples = 0;
    for index in 0..core.allocator.building_sites.len() {
        let site = core.allocator.building_sites[index].clone();
        for (_, triangle) in site.foundation_mesh() {
            assert!(
                triangle
                    .iter()
                    .all(|p| (p.y - site.support_height_m).abs() < 0.0001)
            );
        }
        // Interior quadrature of every material region: no site/road/terrain hole may
        // hide behind a successful CDT status or a nonempty patch buffer.
        let surface = &site.surfaces[0];
        let [a, b, c, d] = <[Vector2; 4]>::try_from(surface.vertices_world.as_slice()).unwrap();
        for u in [0.2, 0.4, 0.6, 0.8] {
            for v in [0.5, 0.75] {
                let p = a.lerp(b, u).lerp(d.lerp(c, u), v);
                assert!(
                    site.contains_point(p),
                    "usable yard is not on the flat pad: {p:?}"
                );
                let y = rendered_paving_height(core, p).expect("flat yard paving coverage");
                assert!(
                    (y - site.support_height_m).abs() < 0.003,
                    "yard {p:?} follows terrain: {y}"
                );
            }
        }
        for x in 0..20 {
            for z in 0..8 {
                let u = (x as f32 + 0.5) / 20.0;
                let v = (z as f32 + 0.5) / 8.0;
                let p = a.lerp(b, u).lerp(d.lerp(c, u), v);
                let rendered = rendered_paving_height(core, p)
                    .unwrap_or_else(|| panic!("yard {index} missing paving at {p:?}"));
                let queried = core.get_world_surface_height_internal(p);
                assert!(
                    (rendered - queried).abs() < 0.003,
                    "yard query {queried} != render {rendered} at {p:?}"
                );
                let hit = core
                    .intersect_world_surface_internal(
                        Vector3::new(p.x, rendered + 10.0, p.y),
                        Vector3::DOWN,
                    )
                    .unwrap();
                assert!(
                    (hit.y - rendered).abs() < 0.003,
                    "yard picking {hit:?} != render {rendered}"
                );
                graded_samples += usize::from((rendered - site.support_height_m).abs() > 0.05);
            }
        }
        // The entire frontage follows the road, not just its midpoint.
        for step in 1..20 {
            let p = a.lerp(b, step as f32 / 20.0);
            let inside = p + (d - a).normalized() * 0.025;
            let road = core
                .transit_network
                .road_surface
                .sample_visible_surface_height(&core.region_graph, &core.heightmap, p.x, p.y)
                .unwrap();
            let yard = rendered_paving_height(core, inside).expect("paved frontage coverage");
            assert!(
                (road - yard).abs() < 0.05,
                "frontage seam {p:?}: road {road}, yard {yard}"
            );
        }
    }
    graded_samples
}

fn rendered_paving_height(core: &SimCore, point: Vector2) -> Option<f32> {
    rendered_paving_surface(core, point).map(|sample| sample.0)
}

fn rendered_paving_surface(core: &SimCore, point: Vector2) -> Option<(f32, Vector3)> {
    for site in &core.allocator.building_sites {
        for (material, triangle) in site.foundation_mesh() {
            if material.is_some()
                && let Some(y) = triangle_height(*triangle, point)
            {
                return Some((y, upward_triangle_normal(*triangle)));
            }
        }
    }
    for patch in core.refined_terrain_patch_cache.values() {
        let Some(buffers) = &patch.mesh_buffers else {
            continue;
        };
        let center = Vector2::new(
            patch.patch.world_origin_x + patch.patch.world_size_x * 0.5,
            patch.patch.world_origin_z + patch.patch.world_size_z * 0.5,
        );
        for face in buffers.terrain_indices.as_chunks::<3>().0 {
            if buffers
                .terrain_colors
                .get(face[0] as usize)
                .is_none_or(|color| color.b > 0.5)
            {
                continue;
            }
            let triangle =
                [face[0], face[1], face[2]].map(|i| buffers.terrain_vertices[i as usize]);
            if let Some(y) = triangle_height(triangle, point - center) {
                return Some((y, upward_triangle_normal(triangle)));
            }
        }
    }
    None
}

fn upward_triangle_normal(triangle: [Vector3; 3]) -> Vector3 {
    let normal = (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]);
    normal.normalized() * normal.y.signum()
}

fn triangle_height(triangle: [Vector3; 3], point: Vector2) -> Option<f32> {
    let triangle = triangle.map(|p| RoadVec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z)));
    let top = triangle
        .iter()
        .map(|p| p.y)
        .fold(f64::NEG_INFINITY, f64::max)
        + 1.0;
    road_ray_triangle_intersection_t(
        triangle,
        RoadVec3::new(f64::from(point.x), top, f64::from(point.y)),
        -RoadVec3::Y,
    )
    .map(|t| (top - t) as f32)
}

#[test]
fn queued_asymmetric_houses_publish_graded_terrain() {
    use crate::nodes::sim::core::state::PendingDemandSpawnAction;

    for cross_grade in [-0.05, 0.05] {
        let mut zoned_sites: Vec<crate::simulation::buildings::allocator::BuildingSiteClient> =
            Vec::new();
        let mut zoned_patches = std::collections::HashMap::new();
        for explicit in [false, true] {
            let mut core = test_core();
            core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 64, 0.0);
            core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
            for z in 0..257 {
                for x in 0..257 {
                    core.heightmap.set_height(
                        x,
                        z,
                        (20.0 + (x as f32 - 128.0) * 0.15 + (z as f32 - 128.0) * cross_grade)
                            / crate::config::HEIGHT_SCALE,
                    );
                }
            }
            road_terrain_plan::commit_ready(
                &mut core,
                vec![Vector3::new(-96.0, 5.6, 0.0), Vector3::new(96.0, 34.4, 0.0)],
            );
            let asset = register_yard(&mut core);
            let mut manifest = core
                .allocator
                .registry
                .get(&asset)
                .unwrap()
                .manifest
                .clone();
            // The installed family-house layout has an off-center structure and asymmetric paving.
            manifest.mesh_parts[0].position = [2.07, 0.0, 0.14];
            manifest.anchors[0].position = [4.03, 0.0, -2.71];
            manifest.anchors[1].position = [0.68, 0.0, -10.0];
            manifest.site_surfaces[0].vertices =
                vec![[-10.0, -10.0], [5.39, -10.0], [5.53, -2.69], [-10.0, -2.77]];
            if explicit {
                let building = manifest.building.as_mut().unwrap();
                building.placement_mode = PlacementMode::Explicit;
                building.zone_type = None;
                building.density = None;
                building.service_class = Some("power".into());
                building.economy_profile = Some("power_plant_basic".into());
                building.worker_capacity = Some(20);
            }
            core.allocator
                .registry
                .register("test", manifest, String::new());
            let profile = core
                .zoning
                .profiles
                .default_runtime_id_for_zone_type(ZoneType::Residential)
                .unwrap();
            for side in [-1.0, 1.0] {
                for x in [-20.0, 0.0, 20.0] {
                    let parcel = core
                        .zoning
                        .place_or_rezone_parcel_at(
                            x,
                            side * 20.0,
                            profile,
                            20.0,
                            20.0,
                            &core.region_graph,
                        )
                        .unwrap();
                    core.pending_demand_spawns
                        .push_back(PendingDemandSpawnAction {
                            due_minute: 1,
                            zone_type: ZoneType::Residential,
                            action: DemandSpawnAction {
                                parcel_id: parcel.raw(),
                                asset_id: asset.clone(),
                            },
                            planned_day_index: 1,
                            planned_minute_of_day: 0,
                        });
                }
            }
            for minute in 1..=6 {
                if explicit {
                    let pending = core.pending_demand_spawns.pop_front().unwrap();
                    let parcel = core
                        .zoning
                        .parcel_by_raw_id(pending.action.parcel_id)
                        .unwrap();
                    let center = parcel.front_center() + parcel.normal() * 10.0;
                    core.place_service_building_internal(&asset, center.x, center.y)
                        .unwrap();
                } else {
                    assert_eq!(core.execute_pending_demand_spawns_for_minute(1, minute), 1);
                }
                assert_eq!(core.allocator.buildings.len(), usize::from(minute));
                assert!(
                    core.allocator.take_pending_site_dirty_bounds().is_none(),
                    "all modes must consume the shared site-change outbox"
                );
                let keys: Vec<_> = core
                    .heightmap
                    .dirty_render_patches()
                    .iter()
                    .copied()
                    .collect();
                assert!(!keys.is_empty());
                let patches =
                    SimulationNode::validate_staged_road_terrain(&mut core, &keys, None).unwrap();
                assert!(patches.iter().any(|patch| patch.site_clip_loop_count > 0));
                core.insert_refined_terrain_patch_cache_entries(patches);
                for site in &core.allocator.building_sites {
                    for point in &site.footprint_world {
                        let center = site.footprint_world.iter().copied().sum::<Vector2>()
                            / site.footprint_world.len() as f32;
                        let inside = point.lerp(center, 0.01);
                        assert!(
                            (core.get_world_surface_height_internal(inside)
                                - site.support_height_m)
                                .abs()
                                < 0.003
                        );
                    }
                }
                // Exercise the renderer acknowledgement, so the next spawn cannot rely on old dirtiness.
                let states = core.terrain_dirty_patch_states();
                assert!(core.acknowledge_terrain_render_patches(&states));
            }
            assert_no_terrain_over_site_pads(&core);
            if explicit {
                assert_eq!(core.allocator.building_sites.len(), zoned_sites.len());
                for (site, expected) in core.allocator.building_sites.iter().zip(&zoned_sites) {
                    assert_eq!(site.footprint_world, expected.footprint_world);
                    assert_eq!(site.lot_footprint_world, expected.lot_footprint_world);
                    assert_eq!(site.support_height_m, expected.support_height_m);
                    assert_eq!(site.surfaces, expected.surfaces);
                    assert_eq!(site.foundation_mesh(), expected.foundation_mesh());
                }
                for (key, expected) in &zoned_patches {
                    assert_eq!(
                        &core.refined_terrain_patch_cache[key].mesh_buffers, expected,
                        "placement mode must not change final terrain triangles or material regions"
                    );
                }
            } else {
                zoned_sites = core.allocator.building_sites.clone();
                zoned_patches = core
                    .refined_terrain_patch_cache
                    .iter()
                    .map(|(key, patch)| (*key, patch.mesh_buffers.clone()))
                    .collect();
            }
        }
    }
}

fn assert_no_terrain_over_site_pads(core: &SimCore) {
    for site in &core.allocator.building_sites {
        let center = site.footprint_world.iter().copied().sum::<Vector2>()
            / site.footprint_world.len() as f32;
        for point in site
            .footprint_world
            .iter()
            .map(|p| p.lerp(center, 0.1))
            .chain([center])
        {
            for patch in core.refined_terrain_patch_cache.values() {
                let Some(buffers) = &patch.mesh_buffers else {
                    continue;
                };
                let patch_center = Vector2::new(
                    patch.patch.world_origin_x + patch.patch.world_size_x * 0.5,
                    patch.patch.world_origin_z + patch.patch.world_size_z * 0.5,
                );
                for face in buffers.terrain_indices.as_chunks::<3>().0 {
                    let triangle =
                        [face[0], face[1], face[2]].map(|i| buffers.terrain_vertices[i as usize]);
                    assert!(
                        triangle_height(triangle, point - patch_center).is_none(),
                        "terrain overlaps the flat building pad at {point:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn zoned_yards_cover_uphill_and_downhill_frontages() {
    for grade in [-0.15, 0.15] {
        let mut core = test_core();
        core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 64, 0.0);
        core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
        for z in 0..257 {
            for x in 0..257 {
                core.heightmap.set_height(
                    x,
                    z,
                    (20.0 + (x as f32 - 128.0) * grade) / crate::config::HEIGHT_SCALE,
                );
            }
        }
        road_terrain_plan::commit_ready(
            &mut core,
            vec![
                Vector3::new(-96.0, 20.0 - 96.0 * grade, 0.0),
                Vector3::new(96.0, 20.0 + 96.0 * grade, 0.0),
            ],
        );
        let asset = register_yard(&mut core);
        for side in [-1.0, 1.0] {
            for x in [-20.0, 0.0, 20.0] {
                place_yard_at(&mut core, 0, side, (96.0 + x) / 192.0, &asset).unwrap();
            }
        }
        assert!(settle_and_check_yards(&mut core) > 100);
        // A subsequent road edit must carry the paved terrain through ready preview and adoption.
        let y = 20.0 + 64.0 * grade;
        road_terrain_plan::commit_ready(
            &mut core,
            vec![Vector3::new(64.0, y, -80.0), Vector3::new(64.0, y, 0.0)],
        );
        assert!(settle_and_check_yards(&mut core) > 100);
    }
}

#[test]
fn small_commercial_pads_leave_a_watertight_sloping_frontage() {
    for grade in [-0.04, 0.04] {
        let mut core = test_core();
        core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 64, 0.0);
        core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
        for z in 0..257 {
            for x in 0..257 {
                core.heightmap.set_height(
                    x,
                    z,
                    (20.0 + (x as f32 - 128.0) * grade) / crate::config::HEIGHT_SCALE,
                );
            }
        }
        road_terrain_plan::commit_ready(
            &mut core,
            vec![
                Vector3::new(-96.0, 20.0 - 96.0 * grade, 0.0),
                Vector3::new(96.0, 20.0 + 96.0 * grade, 0.0),
            ],
        );
        let asset = register_yard(&mut core);
        let mut manifest = core
            .allocator
            .registry
            .get(&asset)
            .unwrap()
            .manifest
            .clone();
        let building = manifest.building.as_mut().unwrap();
        building.zone_type = Some(ZoneClass::Commercial);
        building.economy_profile = Some("personal_service_small".into());
        building.lot_width_cells = 1;
        building.lot_depth_cells = 1;
        manifest.mesh_parts[0].position = [-0.6, 0.0, -0.86];
        manifest.mesh_parts[0].imported_bounds =
            Some([[-0.441794, 0.0, -0.47], [0.441794, 1.293, 0.47]]);
        manifest.anchors[0].position = [1.1, 0.0, -3.5];
        manifest.anchors[1].position = [-3.5, 0.0, -5.0];
        manifest.site_surfaces[0].vertices =
            vec![[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0]];
        core.allocator
            .registry
            .register("test", manifest, String::new());
        for side in [-1.0, 1.0] {
            place_yard_at(&mut core, 0, side, 0.5, &asset).unwrap();
        }
        assert!(settle_and_check_yards(&mut core) > 0);
        for site in &core.allocator.building_sites {
            assert!(
                site.footprint_world.iter().all(|p| p.y.abs() > 5.0),
                "flat support must not touch the sidewalk"
            );
        }
        let old_footprint = core.allocator.building_sites[0].footprint_world.clone();
        let old_revision = core.get_building_site_revision_internal();
        let mut revised = core
            .allocator
            .registry
            .get(&asset)
            .unwrap()
            .manifest
            .clone();
        revised.mesh_parts[0].position[0] += 0.05;
        core.allocator
            .registry
            .register("test", revised, String::new());
        core.allocator
            .refresh_building_sites_after_asset_reload(core.config.zone_cell_m);
        assert_ne!(core.get_building_site_revision_internal(), old_revision);
        assert_ne!(
            core.allocator.building_sites[0].footprint_world,
            old_footprint
        );
        assert!(core.allocator.building_site_dirty_bounds.is_some());
        core.publish_pending_building_site_changes();
        assert!(settle_and_check_yards(&mut core) > 0);
    }
}

#[test]
fn kuopio_reference_supports_zoned_graded_yards_without_changing_fixture() {
    let _ = kuopio_yard_fixture();
}

#[test]
fn large_paved_yards_remain_flat_over_hills_and_depressions() {
    for (grade, max_relief, accepted) in [
        (-0.35_f32, 3.0, true),
        (0.35, 3.0, true),
        (-0.35, 14.0, false),
        (0.35, 14.0, false),
    ] {
        let mut core = test_core();
        core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 64, 0.0);
        core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
        for z in 0..257 {
            for x in 0..257 {
                let hill = (((z as f32 - 128.0).abs() - 10.0).max(0.0) * grade.abs())
                    .min(max_relief)
                    * grade.signum();
                core.heightmap
                    .set_height(x, z, (20.0 + hill) / crate::config::HEIGHT_SCALE);
            }
        }
        road_terrain_plan::commit_ready(
            &mut core,
            vec![
                Vector3::new(-96.0, 20.0, 0.0),
                Vector3::new(96.0, 20.0, 0.0),
            ],
        );
        let asset = register_yard(&mut core);
        let mut manifest = core
            .allocator
            .registry
            .get(&asset)
            .unwrap()
            .manifest
            .clone();
        let building = manifest.building.as_mut().unwrap();
        building.lot_width_cells = 4;
        building.lot_depth_cells = 4;
        manifest.mesh_parts[0].position = [-10.0, 0.0, -10.0];
        manifest.anchors[0].position = [-10.0, 0.0, -15.0];
        manifest.anchors[1].position[2] = -20.0;
        manifest.site_surfaces[0].vertices =
            vec![[-20.0, -20.0], [20.0, -20.0], [20.0, 20.0], [-20.0, 20.0]];
        core.allocator
            .registry
            .register("test", manifest, String::new());
        for side in [-1.0, 1.0] {
            let result = place_yard(&mut core, 0, side, &asset);
            if accepted {
                result.unwrap();
            } else {
                assert_eq!(result, Err(crate::simulation::buildings::allocator::DemandSpawnPlacementRejection::SiteSupportTieInInvalid));
            }
        }
        if !accepted {
            assert!(core.allocator.buildings.is_empty());
            continue;
        }
        settle_and_check_yards(&mut core);
        for site in &core.allocator.building_sites {
            let [a, b, c, d] =
                <[Vector2; 4]>::try_from(site.surfaces[0].vertices_world.as_slice()).unwrap();
            for x in 0..11 {
                for z in 0..11 {
                    let u = (3.0 + x as f32 * 3.4) / 40.0;
                    let v = (3.0 + z as f32 * 3.4) / 40.0;
                    let p = a.lerp(b, u).lerp(d.lerp(c, u), v);
                    let y = rendered_paving_height(&core, p).expect("large yard interior coverage");
                    assert!(
                        (y - site.support_height_m).abs() < 0.003,
                        "large yard must be flat at {p:?}: {y}"
                    );
                }
            }
        }
    }
}

fn kuopio_yard_fixture() -> SimCore {
    let mut core = test_core();
    core.load_game_internal(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../benchmarks/fixtures/kuopio-terrain/kuopio-terrain-map.sqlite"
    ))
    .unwrap();
    core.precompute_road_mesh_data();
    let asset = register_yard(&mut core);
    let mut rejected = Vec::new();
    for edge in [3, 9, 15, 18] {
        for side in [-1.0, 1.0] {
            if let Err(reason) = place_yard(&mut core, edge, side, &asset) {
                rejected.push((edge, side, reason));
            }
        }
    }
    use crate::simulation::buildings::allocator::DemandSpawnPlacementRejection::SiteSupportTieInInvalid;
    assert_eq!(
        rejected,
        vec![
            (9, 1.0, SiteSupportTieInInvalid),
            (15, -1.0, SiteSupportTieInInvalid),
            (15, 1.0, SiteSupportTieInInvalid),
            (18, 1.0, SiteSupportTieInInvalid)
        ]
    );
    assert_eq!(core.allocator.buildings.len(), 4);
    assert_eq!(core.zoning.parcels().len(), 8);
    assert!(settle_and_check_yards(&mut core) > 100);
    core
}

#[test]
fn yard_material_changes_reuse_cdt_but_not_final_paving_buffers() {
    let mut core = kuopio_yard_fixture();
    let previous = core.refined_terrain_patch_cache.clone();
    let asset = core.allocator.buildings[0].asset_id.clone();
    let mut manifest = core
        .allocator
        .registry
        .get(&asset)
        .unwrap()
        .manifest
        .clone();
    for surface in &mut manifest.site_surfaces {
        surface.material = crate::assets::SiteSurfaceMaterial::Concrete;
    }
    core.allocator
        .registry
        .register("test", manifest, String::new());
    core.allocator
        .rebuild_building_site_clients(core.config.zone_cell_m);
    let bounds: Vec<_> = core
        .allocator
        .building_sites
        .iter()
        .map(|site| site.bounds())
        .collect();
    for bounds in bounds {
        core.mark_building_site_terrain_dirty_bounds(bounds);
    }
    let inputs = core.collect_refined_terrain_patch_build_inputs(2.0);
    let patches = SimCore::build_refined_terrain_patch_cache_entries(inputs);
    let mut reused = 0;
    let mut repaved = 0;
    for patch in patches {
        let Some(old) = previous.get(&patch.key) else {
            continue;
        };
        for window in &patch.windows {
            reused += usize::from(old.windows.iter().any(|w| Arc::ptr_eq(w, window)));
        }
        let buffers = patch.mesh_buffers.as_ref().unwrap();
        if buffers
            .terrain_colors
            .iter()
            .any(|c| c.b == 0.0 && c.g == 1.0)
        {
            assert!(!Arc::ptr_eq(old.mesh_buffers.as_ref().unwrap(), buffers));
            assert_ne!(old.site_surfaces, patch.site_surfaces);
            repaved += 1;
        }
    }
    assert!(reused > 0 && repaved > 0);
}

#[test]
fn engineered_queries_reject_render_rejected_ground() {
    let mut core = kuopio_yard_fixture();
    let point = core
        .allocator
        .building_sites
        .iter()
        .find_map(|site| {
            let p = &site.surfaces[0].vertices_world;
            let point = p[0].lerp(p[1], 0.5).lerp(p[3].lerp(p[2], 0.5), 0.25);
            core.sample_engineered_ground_height(point).map(|_| point)
        })
        .unwrap();
    for patch in core.refined_terrain_patch_cache.values_mut() {
        let patch = Arc::make_mut(patch);
        if let Some(buffers) = &mut patch.mesh_buffers {
            // Finite, index-valid output still fails publication if faces were omitted.
            let buffers = Arc::make_mut(buffers);
            assert!(buffers.variant_payload_valid);
            buffers.omitted_pathological_terrain_faces = 1;
            assert_eq!(
                SimulationNode::cached_refined_cdt_failure_label(patch),
                Some("terrain_cdt_pathological_output")
            );
        }
    }
    assert_eq!(core.sample_engineered_ground_height(point), None);
    let rejected = std::mem::take(&mut core.refined_terrain_patch_cache);
    let origin = Vector3::new(point.x, 1000.0, point.y);
    let fallback = core.intersect_world_surface_internal(origin, Vector3::DOWN);
    core.refined_terrain_patch_cache = rejected;
    assert_eq!(
        core.intersect_world_surface_internal(origin, Vector3::DOWN),
        fallback
    );
}

#[test]
#[ignore = "unprofiled query measurement; run alone with --release --ignored --nocapture"]
fn graded_yard_height_query_benchmark() {
    let core = kuopio_yard_fixture(); // Placement, compilation and coverage checks are not timed.
    let points: Vec<_> = core
        .allocator
        .building_sites
        .iter()
        .flat_map(|site| {
            let p = &site.surfaces[0].vertices_world;
            (1..20).map(move |i| {
                p[0].lerp(p[1], i as f32 / 20.0)
                    .lerp(p[3].lerp(p[2], i as f32 / 20.0), 0.25)
            })
        })
        .collect();
    assert!(
        points
            .iter()
            .all(|p| core.sample_engineered_ground_height(*p).is_some())
    );
    let count = 100_000;
    let mut samples = Vec::new();
    for _ in 0..10 {
        let start = std::time::Instant::now();
        for i in 0..count {
            std::hint::black_box(
                core.get_world_surface_height_internal(std::hint::black_box(
                    points[i % points.len()],
                )),
            );
        }
        samples.push(start.elapsed().as_secs_f64() * 1e9 / count as f64);
    }
    samples.sort_by(f64::total_cmp);
    eprintln!(
        "graded_yard_height_query: samples=10 queries_per_sample={count} points={} p50_ns={:.1} p90_ns={:.1}",
        points.len(),
        samples[5],
        samples[9]
    );
}

fn spawn_yard_access_agent(
    core: &mut SimCore,
    building: usize,
    point: Vector2,
    mode: u8,
    phase: u8,
) {
    let id = core.agents.spawn_housed_agent(building, point.x, point.y);
    core.agents.transit_mode[id] = mode;
    core.agents.transit[id] = phase;
    core.agents.target_building[id] = building;
}

#[test]
fn cars_and_walkers_enter_and_exit_on_compiled_yards() {
    let mut core = kuopio_yard_fixture();
    let mut points = Vec::new();
    let mut cut = 0;
    let mut fill = 0;
    let mut graded = 0;
    for (building, site) in core.allocator.building_sites.iter().enumerate() {
        let p = &site.surfaces[0].vertices_world;
        for u in [0.1, 0.3, 0.5, 0.7, 0.9] {
            for v in [0.05, 0.25, 0.5, 0.9] {
                let point = p[0].lerp(p[1], u).lerp(p[3].lerp(p[2], u), v);
                let height = rendered_paving_height(&core, point).unwrap();
                let source = core.get_height_at_internal(point);
                cut += usize::from(source - height > 0.1);
                fill += usize::from(height - source > 0.1);
                graded += usize::from((height - site.support_height_m).abs() > 0.05);
                points.push((building, point, height));
            }
        }
        // Include the sidewalk and carriageway portions of the same off-network access.
        let frontage = p[0].lerp(p[1], 0.5);
        let toward_road = (p[0] - p[3]).normalized();
        for distance in [0.5, 3.5] {
            let point = frontage + toward_road * distance;
            let height = core
                .transit_network
                .road_surface
                .sample_visible_surface_height(
                    &core.region_graph,
                    &core.heightmap,
                    point.x,
                    point.y,
                )
                .unwrap();
            points.push((building, point, height));
        }
    }
    assert!(
        cut > 0 && fill > 0 && graded > 0,
        "fixture must cover cut, fill and aprons"
    );
    check_access_snapshot_heights(&mut core, &points);
}

#[test]
fn access_snapshot_pad_ownership_overrides_pending_visual_terrain() {
    let mut core = kuopio_yard_fixture();
    let site = &core.allocator.building_sites[0];
    let p = &site.surfaces[0].vertices_world;
    let point = p[0].lerp(p[1], 0.5).lerp(p[3].lerp(p[2], 0.5), 0.75);
    assert!(site.contains_point(point));
    let support = site.support_height_m;
    let (gx, gz) = core.heightmap.world_to_grid_coords(point.x, point.y);
    core.refined_terrain_patch_cache.clear();
    // Pending terrain must not raise a cut pad or lower a filled pad. The underlying
    // visual heightfield is deliberately different from the authoritative flat site.
    for delta in [-5.0, 5.0] {
        let writes = [
            (gx.floor() as usize, gz.floor() as usize),
            (gx.floor() as usize + 1, gz.floor() as usize),
            (gx.floor() as usize, gz.floor() as usize + 1),
            (gx.floor() as usize + 1, gz.floor() as usize + 1),
        ];
        core.heightmap
            .set_visual_heights_at_grid_unmarked(&writes, |&(x, z)| {
                (x, z, (support + delta) / crate::config::HEIGHT_SCALE)
            });
        assert!(
            (core.heightmap.sample_visual_height_world(point.x, point.y)
                * crate::config::HEIGHT_SCALE
                - support
                - delta)
                .abs()
                < 0.001
        );
        check_access_snapshot_heights(&mut core, &[(0, point, support)]);
        assert_eq!(core.get_world_surface_height_internal(point), support);
    }
}

#[test]
fn cars_pitch_and_roll_on_compiled_yards_in_both_access_directions() {
    use crate::simulation::economy::agents::MODE_CAR;

    let mut core = kuopio_yard_fixture();
    core.allocator
        .rebuild_entrance_cache(&core.region_graph, &core.transit_network.lane_system);
    let mut points = Vec::new();
    for (building, site) in core.allocator.building_sites.iter().enumerate() {
        let p = &site.surfaces[0].vertices_world;
        for u in [0.13, 0.37, 0.61, 0.83] {
            for v in [0.11, 0.29, 0.53, 0.87] {
                let point = p[0].lerp(p[1], u).lerp(p[3].lerp(p[2], u), v);
                let surface = rendered_paving_surface(&core, point).unwrap();
                points.push((building, point, surface));
            }
        }
    }
    assert!(points.iter().any(|(_, _, (_, n))| n.y < 0.99));
    assert!(points.iter().any(|(_, _, (_, n))| *n == Vector3::UP));
    for phase in [TRANSIT_ACCESS_INGRESS, TRANSIT_ACCESS_EGRESS] {
        core.agents = AgentSystem::new();
        for &(building, point, _) in &points {
            spawn_yard_access_agent(&mut core, building, point, MODE_CAR, phase);
            let id = core.agents.len() - 1;
            let entrance = &core.allocator.entrances[building];
            let lane_id = if entrance.car_lane_fwd != usize::MAX {
                entrance.car_lane_fwd
            } else {
                entrance.car_lane_bkw
            };
            assert!(lane_id < core.transit_network.lane_system.lanes.len());
            core.agents.planned_attach_lane_id[id] = lane_id as u32;
            core.agents.planned_attach_lane_d[id] = 10.0;
        }
        let snapshot = core.build_snapshot();
        let mut count = 0;
        for transform in snapshot
            .car_transforms
            .values()
            .flat_map(|buffer| buffer.as_chunks::<12>().0)
        {
            let point = Vector2::new(transform[3], transform[11]);
            let &(building, _, _) = points.iter().find(|(_, p, _)| *p == point).unwrap();
            let up = Vector3::new(transform[1], transform[5], transform[9]);
            let back = Vector3::new(transform[2], transform[6], transform[10]);
            check_vehicle_contacts(&core, transform);
            assert!(up.cross(back).length() > 0.999);
            let entrance = &core.allocator.entrances[building];
            let target = if phase == TRANSIT_ACCESS_INGRESS {
                entrance.door_pos
            } else {
                let lane_id = if entrance.car_lane_fwd != usize::MAX {
                    entrance.car_lane_fwd
                } else {
                    entrance.car_lane_bkw
                };
                BuildingAllocator::sample_pos_on_lane(
                    &core.transit_network.lane_system.lanes[lane_id],
                    10.0,
                )
            };
            assert!(
                Vector2::new(-back.x, -back.z)
                    .normalized()
                    .distance_to((target - point).normalized())
                    < 1e-5
            );
            count += 1;
        }
        assert_eq!(count, points.len());
    }
}

fn check_access_snapshot_heights(core: &mut SimCore, points: &[(usize, Vector2, f32)]) {
    use crate::simulation::economy::agents::{MODE_CAR, MODE_WALK};

    for mode in [MODE_CAR, MODE_WALK] {
        for phase in [TRANSIT_ACCESS_INGRESS, TRANSIT_ACCESS_EGRESS] {
            core.agents = AgentSystem::new();
            for &(building, point, _) in points {
                spawn_yard_access_agent(core, building, point, mode, phase);
            }
            let snapshot = core.build_snapshot();
            let (buffers, stride) = if mode == MODE_CAR {
                (&snapshot.car_transforms, 12)
            } else {
                (&snapshot.pedestrian_transforms, 16)
            };
            let mut count = 0;
            for transform in buffers
                .values()
                .flat_map(|buffer| buffer.chunks_exact(stride))
            {
                let point = Vector2::new(transform[3], transform[11]);
                let (_, _, height) = points.iter().find(|(_, p, _)| *p == point).unwrap();
                if mode == MODE_CAR {
                    check_vehicle_contacts(core, transform);
                    count += 1;
                    continue;
                }
                assert!(
                    (transform[7] - height - 0.02).abs() < 0.003,
                    "mode={mode} phase={phase} point={point:?}: rendered agent y={} != ground {height} + clearance; road={:?} site={:?} cdt={:?}",
                    transform[7],
                    core.transit_network
                        .road_surface
                        .sample_visible_surface_height(
                            &core.region_graph,
                            &core.heightmap,
                            point.x,
                            point.y
                        ),
                    core.allocator.sample_building_site_height(point),
                    core.sample_engineered_ground_height(point),
                );
                count += 1;
            }
            assert_eq!(count, points.len());
        }
    }
}

fn check_vehicle_contacts(core: &SimCore, transform: &[f32]) {
    let origin = Vector3::new(transform[3], transform[7], transform[11]);
    let x = Vector3::new(transform[0], transform[4], transform[8]);
    let y = Vector3::new(transform[1], transform[5], transform[9]);
    let z = Vector3::new(transform[2], transform[6], transform[10]);
    let mut closest = f32::INFINITY;
    for contact in core.vehicle_ground_support[0].contacts() {
        let p = origin + x * contact.x + y * contact.y + z * contact.z;
        let point = Vector2::new(p.x, p.z);
        let ground = rendered_paving_height(core, point)
            .unwrap_or_else(|| core.get_world_surface_height_internal(point));
        assert!(p.y - ground >= 0.017, "contact={p:?}, ground={ground}");
        closest = closest.min(p.y - ground);
    }
    assert!(
        (closest - 0.02).abs() < 0.003,
        "support must be tight, clearance={closest}"
    );
}

#[test]
#[ignore = "unprofiled snapshot measurement; run alone with --release --ignored --nocapture"]
fn graded_yard_access_snapshot_benchmark() {
    use crate::simulation::economy::agents::{MODE_CAR, MODE_WALK};

    let mut core = kuopio_yard_fixture(); // Setup/compilation are outside the measurement.
    let points: Vec<_> = core
        .allocator
        .building_sites
        .iter()
        .enumerate()
        .flat_map(|(id, site)| {
            let p = &site.surfaces[0].vertices_world;
            (1..20).map(move |i| {
                (
                    id,
                    p[0].lerp(p[1], i as f32 / 20.0)
                        .lerp(p[3].lerp(p[2], i as f32 / 20.0), 0.25),
                )
            })
        })
        .collect();
    for count in [100, 1_000, 10_000] {
        core.agents = AgentSystem::new();
        for i in 0..count {
            let (building, point) = points[i % points.len()];
            let mode = if i % 2 == 0 { MODE_CAR } else { MODE_WALK };
            let phase = if i % 4 < 2 {
                TRANSIT_ACCESS_INGRESS
            } else {
                TRANSIT_ACCESS_EGRESS
            };
            spawn_yard_access_agent(&mut core, building, point, mode, phase);
        }
        let mut snapshot = core.build_snapshot();
        for _ in 0..10 {
            snapshot = core.build_snapshot_reusing(snapshot);
        }
        let mut samples = Vec::with_capacity(100);
        for _ in 0..100 {
            let start = std::time::Instant::now();
            snapshot = std::hint::black_box(core.build_snapshot_reusing(snapshot));
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "graded_yard_access_snapshot: agents={count} samples=100 p50_ms={:.4} p90_ms={:.4}",
            samples[50], samples[90]
        );
    }
}

#[test]
fn interpolated_vehicle_batch_keeps_lane_poses_and_supports_changed_xz() {
    use crate::simulation::economy::agents::MODE_CAR;

    let mut core = kuopio_yard_fixture();
    let site = &core.allocator.building_sites[0];
    let p = &site.surfaces[0].vertices_world;
    let point = p[0].lerp(p[1], 0.43).lerp(p[3].lerp(p[2], 0.43), 0.2);
    let original = [
        1.0, 0.0, 0.0, point.x, 0.0, 1.0, 0.0, -100.0, 0.0, 0.0, 1.0, point.y,
    ];
    let mut transforms = original.repeat(2);
    assert!(core.ground_vehicle_transforms(0, &mut transforms, &[1, 0]));
    check_vehicle_contacts(&core, &transforms[..12]);
    assert_eq!(&transforms[12..], &original);
    assert_eq!(transforms[3], original[3]);
    assert_eq!(transforms[11], original[11]);
    // Input failures are atomic, including malformed model IDs and nonfinite lane entries.
    for (vehicle, flags) in [(5, vec![1, 0]), (0, vec![1])] {
        let before = transforms.clone();
        assert!(!core.ground_vehicle_transforms(vehicle, &mut transforms, &flags));
        assert_eq!(transforms, before);
    }
    transforms[19] = f32::INFINITY;
    let before = transforms.clone();
    assert!(!core.ground_vehicle_transforms(0, &mut transforms, &[1, 0]));
    assert_eq!(transforms, before);

    spawn_yard_access_agent(&mut core, 0, point, MODE_CAR, TRANSIT_ACCESS_INGRESS);
    let snapshot = core.build_snapshot();
    for (&key, buffer) in &snapshot.car_transforms {
        assert_eq!(buffer.len(), snapshot.car_render_ids[&key].len() * 12);
        assert_eq!(snapshot.car_ground_flags[&key], vec![1; buffer.len() / 12]);
    }
    core.agents = AgentSystem::new();
    let recycled = core.build_snapshot_reusing(snapshot);
    assert!(recycled.car_transforms.values().all(Vec::is_empty));
    assert!(recycled.car_render_ids.values().all(Vec::is_empty));
    assert!(recycled.car_ground_flags.values().all(Vec::is_empty));
}

#[test]
#[ignore = "unprofiled support-batch measurement; run alone with --release --ignored --nocapture"]
fn graded_yard_vehicle_support_batch_benchmark() {
    let core = kuopio_yard_fixture();
    for count in [100, 1_000, 10_000] {
        let mut transforms = Vec::with_capacity(count * 12);
        let flags = vec![1; count];
        for i in 0..count {
            let site = &core.allocator.building_sites[i % core.allocator.building_sites.len()];
            let p = &site.surfaces[0].vertices_world;
            let t = (1 + i % 19) as f32 / 20.0;
            let point = p[0].lerp(p[1], t).lerp(p[3].lerp(p[2], t), 0.25);
            transforms.extend_from_slice(&[
                1.0, 0.0, 0.0, point.x, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, point.y,
            ]);
        }
        for _ in 0..10 {
            assert!(core.ground_vehicle_transforms(0, &mut transforms, &flags));
        }
        let mut samples = Vec::with_capacity(100);
        for _ in 0..100 {
            let start = std::time::Instant::now();
            assert!(std::hint::black_box(core.ground_vehicle_transforms(
                0,
                &mut transforms,
                &flags
            )));
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "graded_yard_vehicle_support_batch: cars={count} samples=100 p50_ms={:.4} p90_ms={:.4}",
            samples[50], samples[90]
        );
    }
}
