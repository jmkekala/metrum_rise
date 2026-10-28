// SPDX-License-Identifier: GPL-2.0-only

//! Zoned yard coverage on sloping roads, using production placement and terrain compilation.

use super::*;
use crate::nodes::simulation_node::SimulationNode;
use crate::simulation::network::roadbed::RoadVec3;
use crate::simulation::terrain::graded::{GradedSurface, QueryScratch, TerrainQueryIndex};
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
        &core.transit_network.roadbed,
        &core.heightmap,
        &catalog,
        core.demand.runtime_tuning(),
    )?;
    assert!(
        !core.allocator.building_site_dirty_bounds.is_empty(),
        "zoned placement must publish its own site change, just like explicit placement"
    );
    core.publish_pending_building_site_changes();
    Ok(building)
}

fn assert_query_index_fresh(core: &SimCore, stage: &str, strict_inputs: bool) {
    let mut fresh = TerrainQueryIndex::default();
    fresh.rebuild_all(&core.transit_network.roadbed, &core.allocator);
    if strict_inputs {
        assert!(
            core.terrain_query_index.same_inputs(&fresh),
            "{stage}: incremental inputs differ from rebuild: {}",
            core.terrain_query_index.difference(&fresh)
        );
    }
    let (tiles, bytes) = core.terrain_query_index.memory();
    assert!(tiles > 0 && bytes > 0, "a road or site fixture has indexed inputs");
    let indexed = core.graded_surface();
    let gathered = GradedSurface {
        terrain: &core.heightmap,
        roadbed: &core.transit_network.roadbed,
        sites: &core.allocator,
        query_index: None,
    };
    let (mut a_scratch, mut b_scratch) = (QueryScratch::default(), QueryScratch::default());
    let cached = indexed.ground_height_at(0.37, 0.19, &mut a_scratch);
    indexed.base_height_at(180.0, 180.0, &mut a_scratch);
    assert_eq!(
        indexed
            .ground_height_at(0.37, 0.19, &mut a_scratch)
            .to_bits(),
        cached.to_bits(),
        "an authored-base lookup must invalidate the prepared query's base window"
    );
    let (x, z) = (-65.690_895_080_566_4, 36.011_367_797_851_56);
    indexed.gather_near(x, z, 2.5, &mut a_scratch);
    assert_eq!(
        indexed.ground_height_near(x, z, &mut a_scratch).to_bits(),
        gathered.ground_height_at(x, z, &mut b_scratch).to_bits(),
        "a diagonal seam outside the cap must not disable a gathered tile's base shortcut"
    );
    for (dx, dz) in [(2.5, -2.5), (-2.4, 1.3), (0.7, 2.5)] {
        let (cx, cz) = (x + dx, z + dz);
        assert_eq!(
            indexed.surface_height_near(cx, cz, &mut a_scratch).to_bits(),
            gathered.surface_height_at(cx, cz, &mut b_scratch).to_bits(),
            "indexed gathered contact at ({cx}, {cz})"
        );
    }
    for z in (-220..=220).step_by(11) {
        for x in (-220..=220).step_by(13) {
            let (x, z) = (f64::from(x) + 0.37, f64::from(z) + 0.19);
            let a = indexed.surface_height_at(x, z, &mut a_scratch);
            let b = gathered.surface_height_at(x, z, &mut b_scratch);
            assert_eq!(a.to_bits(), b.to_bits(), "height at ({x}, {z})");
            let expected_height = a.to_bits();
            assert_eq!(indexed.mask_at(x, z, &mut a_scratch), gathered.mask_at(x, z, &mut b_scratch));
            let a = indexed.normal_at(x, z, &mut a_scratch);
            let b = gathered.normal_at(x, z, &mut b_scratch);
            assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits), "normal at ({x}, {z})");
            let expected_normal = a.map(f64::to_bits);
            let a = indexed.surface_height_and_normal_at(x, z, &mut a_scratch);
            let b = gathered.surface_height_and_normal_at(x, z, &mut b_scratch);
            assert_eq!(
                (a.0.to_bits(), a.1.map(f64::to_bits)),
                (b.0.to_bits(), b.1.map(f64::to_bits)),
                "surface pose at ({x}, {z})"
            );
            assert_eq!(a.0.to_bits(), expected_height);
            if core.transit_network.roadbed.height_at(x as f32, z as f32).is_none() {
                assert_eq!(a.1.map(f64::to_bits), expected_normal);
            }
        }
    }
}

#[test]
fn terrain_query_index_stays_fresh_across_road_load_site_and_base_mutations() {
    let (_, strokes, _) = road_reference_probe::reference_fixtures().swap_remove(0);
    let mut core = road_edit_order::committed(&strokes, &[0], true);
    assert_query_index_fresh(&core, "initial road", true);
    core.benchmark_mode = false;
    graded_edits::commit_stroke(&mut core, strokes[1]);
    let commit_us = core.terrain_query_index_update_us;
    assert_query_index_fresh(&core, "road commit", true);
    assert!(core.undo_action_internal());
    let undo_us = core.terrain_query_index_update_us;
    assert_query_index_fresh(&core, "road undo", true);

    let path = temp_save_path("terrain_query_index_mutations");
    core.save_game_settled(path.to_str().unwrap(), None).unwrap();
    core.load_game_internal(path.to_str().unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    core.precompute_road_mesh_data();
    assert_query_index_fresh(&core, "load", true);

    let asset = register_yard(&mut core);
    let first = place_yard_at(&mut core, 0, 1.0, 0.25, &asset).expect("first site placement");
    place_yard_at(&mut core, 0, -1.0, 0.75, &asset).expect("second site placement");
    let site_us = core.terrain_query_index_update_us;
    assert_query_index_fresh(&core, "sites", true);
    assert!(core.bulldoze_building(first));
    assert_query_index_fresh(&core, "site swap-remove", true);

    core.slope_terrain_internal(
        Vector2::new(48.0, 48.0), 24.0, Vector2::new(24.0, 48.0), 1.0,
        Vector2::new(72.0, 48.0), 8.0, 1.0,
    );
    let base_us = core.terrain_query_index_update_us;
    assert_query_index_fresh(&core, "base", true);
    eprintln!(
        "TERRAIN_QUERY_INDEX_UPDATES commit_us={commit_us} undo_us={undo_us} site_us={site_us} base_us={base_us}"
    );
}

#[test]
#[ignore = "TERRAIN-07-F release query-index Kuopio memory and rebuild benchmark"]
fn benchmark_kuopio_terrain_query_index() {
    let mut core = test_core();
    core.load_game_internal(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../benchmarks/fixtures/kuopio-terrain/kuopio-terrain-map.sqlite"
    ))
    .unwrap();
    let started = std::time::Instant::now();
    let mut rebuilt = TerrainQueryIndex::default();
    rebuilt.rebuild_all(&core.transit_network.roadbed, &core.allocator);
    let rebuild_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert!(core.terrain_query_index.same_inputs(&rebuilt));
    let (tiles, bytes) = core.terrain_query_index.memory();
    eprintln!("TERRAIN_QUERY_INDEX fixture=kuopio tiles={tiles} bytes={bytes} rebuild_ms={rebuild_ms:.3}");
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
                // The graded surface is the pad (`TERRAIN-07-F`): its support height wherever
                // the `GL0` triangle under the point lies inside the footprint. Within one
                // lattice diagonal of a side the triangle reaches the graded ground beyond it.
                if site.contains_point(p) && pad_depth(&site.footprint_world, p) > GL0_DIAGONAL_M {
                    assert!(
                        (queried - site.support_height_m).abs() < 1e-5,
                        "yard query {queried} != pad {} at {p:?}",
                        site.support_height_m
                    );
                }
                let hit = core
                    .intersect_world_surface_internal(
                        Vector3::new(p.x, rendered + 10.0, p.y),
                        Vector3::DOWN,
                    )
                    .unwrap();
                assert!(
                    (hit.y - queried).abs() < 1e-5,
                    "yard picking {hit:?} != surface query {queried}"
                );
                graded_samples += usize::from((rendered - site.support_height_m).abs() > 0.05);
            }
        }
        // The entire frontage follows the road, not just its midpoint: the road 2.5 cm on its
        // side of the seam against the yard 2.5 cm on the other. The roadbed's coverage is
        // exact (`ROAD-68-K19`), so a point on the seam itself may lie outside the road.
        for step in 1..20 {
            let p = a.lerp(b, step as f32 / 20.0);
            let across = (d - a).normalized() * 0.025;
            let (road_side, yard_side) = (p - across, p + across);
            let road = core
                .transit_network
                .roadbed
                .height_at(road_side.x, road_side.y)
                .unwrap();
            let yard = rendered_paving_height(core, yard_side).expect("paved frontage coverage");
            assert!(
                (road - yard).abs() < 0.05,
                "frontage seam {p:?}: road {road}, yard {yard}"
            );
        }
    }
    graded_samples
}

// A `GL0` lattice cell's diagonal: the graded surface's triangle under a point lies within it.
const GL0_DIAGONAL_M: f32 = 0.5 * std::f32::consts::SQRT_2 + 0.01;

// Distance from a point inside a footprint to its nearest side.
fn pad_depth(footprint: &[Vector2], point: Vector2) -> f32 {
    (0..footprint.len())
        .map(|i| {
            let (a, b) = (footprint[i], footprint[(i + 1) % footprint.len()]);
            let t = ((point - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0);
            point.distance_to(a + (b - a) * t)
        })
        .fold(f32::INFINITY, f32::min)
}

fn rendered_paving_height(core: &SimCore, point: Vector2) -> Option<f32> {
    rendered_paving_surface(core, point).map(|sample| sample.0)
}

// The paved terrain face (site pads included, `ROAD-68-D`) over a point.
fn rendered_paving_surface(core: &SimCore, point: Vector2) -> Option<(f32, Vector3)> {
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

// Height of a face over a point. Every face that is not exactly degenerate covers its closed plan
// area, however thin (`ROAD-68-D`): the orientation signs of f32 points are exact in f64, so a
// point on a shared edge lies in both faces.
fn triangle_height(triangle: [Vector3; 3], point: Vector2) -> Option<f32> {
    let [a, b, c] =
        triangle.map(|p| RoadVec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z)));
    let q = RoadVec3::new(f64::from(point.x), 0.0, f64::from(point.y));
    let orient = |u: RoadVec3, v: RoadVec3, w: RoadVec3| {
        (v.x - u.x) * (w.z - u.z) - (v.z - u.z) * (w.x - u.x)
    };
    let area = orient(a, b, c);
    let sides = [orient(a, b, q), orient(b, c, q), orient(c, a, q)];
    if area == 0.0 || sides.iter().any(|side| side * area < 0.0) {
        return None;
    }
    let [wc, wa, wb] = sides.map(|side| side / area);
    Some((a.y * wa + b.y * wb + c.y * wc) as f32)
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
                            id: 0,
                            due_minute: 1,
                            zone_type: ZoneType::Residential,
                            action: DemandSpawnAction {
                                parcel_id: parcel.raw(),
                                asset_id: asset.clone(),
                            },
                            planned_day_index: 1,
                            planned_minute_of_day: 0,
                            permit: Default::default(),
                            counted: false,
                        });
                }
            }
            for minute in 1..=6 {
                let cells_before = core.zoning.cell_external_revision();
                if explicit {
                    let pending = core.pending_demand_spawns.pop_front().unwrap();
                    let parcel = core
                        .zoning
                        .parcel_by_raw_id(pending.action.parcel_id)
                        .unwrap();
                    let center = parcel.front_center() + parcel.normal() * 10.0;
                    // These empty parcels only supply matched fixture positions. Explicit
                    // sites must release the marker's land reservation before claiming it.
                    core.zoning
                        .remove_parcels_by_raw_ids(&HashSet::from([pending.action.parcel_id]));
                    core.place_service_building_internal(&asset, center.x, center.y)
                        .unwrap();
                } else {
                    assert_eq!(core.execute_pending_demand_spawns_for_minute(1, minute), 1);
                    assert_eq!(
                        core.zoning.cell_external_revision(),
                        cells_before,
                        "placing a building inside its reserved parcel cannot change cell exclusions"
                    );
                }
                assert_eq!(core.allocator.buildings.len(), usize::from(minute));
                assert!(
                    core.allocator.take_pending_site_dirty_bounds().is_empty(),
                    "all modes must consume the shared site-change outbox"
                );
                assert!(
                    core.allocator
                        .pop_pending_cell_site_dirty_bounds()
                        .is_none(),
                    "explicit cell-exclusion events must publish with their site changes"
                );
                let keys: Vec<_> = core
                    .heightmap
                    .dirty_render_patches()
                    .iter()
                    .copied()
                    .collect();
                assert!(!keys.is_empty());
                let patches =
                    SimulationNode::build_road_terrain_patches(&mut core, &keys).unwrap();
                assert!(patches.iter().any(|patch| patch.site_clip_loop_count > 0));
                core.insert_refined_terrain_patch_cache_entries(patches);
                for site in &core.allocator.building_sites {
                    for point in &site.footprint_world {
                        let center = site.footprint_world.iter().copied().sum::<Vector2>()
                            / site.footprint_world.len() as f32;
                        // Deep enough that the graded surface's `GL0` triangle lies in the pad.
                        let inside = *point + (center - *point).normalized() * 1.5;
                        assert!(pad_depth(&site.footprint_world, inside) > GL0_DIAGONAL_M);
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

// The pad is composed into the terrain buffers (`ROAD-68-D`): every face over a pad point is a
// pad face, flat at the support height, never ground graded or left over the pad.
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
                        triangle_height(triangle, point - patch_center).is_none()
                            || triangle.iter().all(|p| p.y == site.support_height_m),
                        "terrain overlaps the flat building pad at {point:?}: {triangle:?}"
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
        assert!(!core.allocator.building_site_dirty_bounds.is_empty());
        core.publish_pending_building_site_changes();
        assert!(settle_and_check_yards(&mut core) > 0);
    }
}

#[test]
fn kuopio_reference_supports_zoned_graded_yards_without_changing_fixture() {
    let _ = kuopio_yard_fixture();
}

// One render patch's ownership: road margin, site ownership, engineered margin and the two
// sorted key lists' membership.
fn patch_ownership(
    core: &SimCore,
    key: (usize, usize),
) -> (Option<f32>, bool, Option<f32>, bool, bool) {
    (
        core.road_locked_terrain_patch_margins.get(&key).copied(),
        core.building_site_owned_terrain_patch_keys.contains(&key),
        core.engineered_terrain_patch_margins.get(&key).copied(),
        core.road_locked_terrain_patch_keys
            .binary_search(&key)
            .is_ok(),
        core.engineered_terrain_patch_keys
            .binary_search(&key)
            .is_ok(),
    )
}

// `SIM-02-K`: site changes far apart in one publication refresh the union of the patches
// each touches. Their ownership equals what refreshing the box merged over both gives, and
// every patch between them keeps its ownership, payload generation and render state.
#[test]
fn site_changes_far_apart_refresh_only_the_patches_they_touch() {
    use crate::simulation::terrain::ROAD_LOCKED_TERRAIN_RENDER_STEP_M;
    let mut core = kuopio_yard_fixture();
    let margin_m = crate::simulation::terrain::terrain_cdt_local_sample_margin_m(
        &core.heightmap,
        ROAD_LOCKED_TERRAIN_RENDER_STEP_M,
    );
    let patches = |core: &SimCore, (x0, z0, x1, z1): (f32, f32, f32, f32)| {
        core.heightmap.render_patch_keys_for_world_bounds(
            x0 - margin_m,
            z0 - margin_m,
            x1 + margin_m,
            z1 + margin_m,
        )
    };
    let sites: Vec<_> = (0..core.allocator.buildings.len())
        .map(|index| core.allocator.site_world_bounds(index).unwrap())
        .collect();
    let center = |b: (f32, f32, f32, f32)| Vector2::new((b.0 + b.2) * 0.5, (b.1 + b.3) * 0.5);
    let distance = |(a, b): (usize, usize)| center(sites[a]).distance_to(center(sites[b]));
    let (a, b) = (0..sites.len())
        .flat_map(|a| (a + 1..sites.len()).map(move |b| (a, b)))
        .max_by(|&x, &y| distance(x).total_cmp(&distance(y)))
        .unwrap();
    // Each farthest site goes with every site sharing one of its patches (the yard across its
    // road, since the 64 m tie-in reach of `docs/terrain.md` T12 (a) accepts both), so the
    // patches it touches lose their site ownership.
    let shares = |i: usize, j: usize| {
        let theirs = patches(&core, sites[j]);
        patches(&core, sites[i]).iter().any(|key| theirs.contains(key))
    };
    let mut removed: Vec<usize> = (0..sites.len())
        .filter(|&j| shares(a, j) || shares(b, j))
        .collect();
    removed.sort_unstable_by(|x, y| y.cmp(x));
    let mut touched: Vec<_> = removed.iter().flat_map(|&j| patches(&core, sites[j])).collect();
    touched.sort_unstable();
    touched.dedup();
    let merged = patches(
        &core,
        removed.iter().fold(
            (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY),
            |m, &j| {
                let site = sites[j];
                (m.0.min(site.0), m.1.min(site.1), m.2.max(site.2), m.3.max(site.3))
            },
        ),
    );
    assert!(
        merged.len() > touched.len(),
        "the fixture's farthest sites must leave patches between them: {} of {}",
        touched.len(),
        merged.len()
    );
    let before: Vec<_> = merged
        .iter()
        .map(|&key| patch_ownership(&core, key))
        .collect();
    let generations = core.terrain_payload_patch_generations.clone();
    let dirty = core.heightmap.dirty_render_patches().clone();
    for &index in &removed {
        assert!(core.allocator.remove_building_for_bulldoze(
            index,
            &mut core.zoning,
            &mut core.agents,
            &mut core.households,
            &mut core.logistics,
            &mut core.treasury.balance,
        ));
    }
    assert_eq!(
        core.allocator.building_site_dirty_bounds.len(),
        removed.len(),
        "each site change keeps its own box"
    );
    core.publish_pending_building_site_changes();
    assert!(core.allocator.building_site_dirty_bounds.is_empty());

    let after: Vec<_> = merged
        .iter()
        .map(|&key| patch_ownership(&core, key))
        .collect();
    assert_ne!(
        after, before,
        "removing the sites must change their patches"
    );
    for (index, key) in merged.iter().enumerate() {
        let generation = core.terrain_payload_patch_generations.get(key);
        if touched.binary_search(key).is_ok() {
            assert_ne!(generation, generations.get(key), "touched patch {key:?}");
            continue;
        }
        assert_eq!(after[index], before[index], "untouched patch {key:?}");
        assert_eq!(generation, generations.get(key), "untouched patch {key:?}");
        assert!(
            dirty.contains(key) || !core.heightmap.dirty_render_patches().contains(key),
            "untouched patch {key:?} must not be re-uploaded"
        );
    }
    // The merged box's result: refreshing every patch in it changes no ownership and bumps
    // no payload generation.
    let generations = core.terrain_payload_patch_generations.clone();
    core.refresh_engineered_terrain_patch_ownership_for_keys(
        ROAD_LOCKED_TERRAIN_RENDER_STEP_M,
        &merged,
    );
    let merged_result: Vec<_> = merged
        .iter()
        .map(|&key| patch_ownership(&core, key))
        .collect();
    assert_eq!(merged_result, after);
    assert_eq!(core.terrain_payload_patch_generations, generations);
}

#[test]
fn large_paved_yards_remain_flat_over_hills_and_depressions() {
    // A relief the pad cannot tie into within the tie-in reach (the graded terrain's 64 m cap at
    // 1:2, `docs/terrain.md` T12 (a)) refuses the yard.
    for (grade, max_relief, accepted) in [
        (-0.35_f32, 3.0, true),
        (0.35, 3.0, true),
        (-0.35, 40.0, false),
        (0.35, 40.0, false),
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

// The roads the Kuopio yard tests build on, by place rather than edge id, so a re-recorded or
// replaced reference save keeps them (`ROAD-68` open question 38): each road's midpoint and
// direction on the network recorded 2026-09-08, whose edges 3, 9, 15 and 18 they were.
const KUOPIO_YARD_ROADS: [([f32; 2], [f32; 2]); 4] = [
    ([68.694, -106.08], [-0.8815, 0.4722]),
    ([558.77, 196.18], [-0.4818, 0.8763]),
    ([390.827, -398.329], [-0.042, -0.9991]),
    ([67.467, -979.44], [0.0579, -0.9983]),
];

// The live road whose midpoint lies nearest yard road `road`, and the side of it that `side` of
// the recorded road is: flipped where the edge runs the other way. Test-only linear scan.
fn kuopio_yard_edge(core: &SimCore, road: usize, side: f32) -> (usize, f32) {
    let ([x, z], [dx, dz]) = KUOPIO_YARD_ROADS[road];
    let graph = &core.region_graph;
    let midpoint = |id: usize| BuildingAllocator::sample_pos_on_edge(graph, id, 0.5);
    let edge = (0..graph.edge_count())
        .filter(|&id| {
            let edge = graph.edge(id);
            !edge.deleted && edge.primary_type == TransitType::Road
        })
        .min_by(|&a, &b| {
            let target = Vector2::new(x, z);
            midpoint(a)
                .distance_to(target)
                .total_cmp(&midpoint(b).distance_to(target))
        })
        .unwrap();
    let tangent = BuildingAllocator::sample_tangent_on_edge(graph, edge, 0.5);
    let along = tangent.dot(Vector2::new(dx, dz));
    (edge, if along < 0.0 { -side } else { side })
}

// Places a yard beside yard road `road` (`KUOPIO_YARD_ROADS`), on its recorded `side`.
fn place_kuopio_yard(
    core: &mut SimCore,
    road: usize,
    side: f32,
    asset: &str,
) -> Result<usize, crate::simulation::buildings::allocator::DemandSpawnPlacementRejection> {
    let (edge, side) = kuopio_yard_edge(core, road, side);
    place_yard(core, edge, side, asset)
}

fn kuopio_yard_fixture() -> SimCore {
    let mut core = kuopio_yard_core();
    assert!(settle_and_check_yards(&mut core) > 100);
    core
}

// The Kuopio yards placed on the reference save, before any terrain is built.
fn kuopio_yard_core() -> SimCore {
    let mut core = test_core();
    core.load_game_internal(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../benchmarks/fixtures/kuopio-terrain/kuopio-terrain-map.sqlite"
    ))
    .unwrap();
    core.precompute_road_mesh_data();
    let asset = register_yard(&mut core);
    let mut rejected = Vec::new();
    for road in 0..KUOPIO_YARD_ROADS.len() {
        for side in [-1.0, 1.0] {
            if let Err(reason) = place_kuopio_yard(&mut core, road, side, &asset) {
                rejected.push((road, side, reason));
            }
        }
    }
    // The third road's yards stood beside a four-way 29 m above the ground in the network
    // recorded 2026-09-08; on the re-recorded network it lies on the ground and takes both. The
    // first and fourth roads' yards on their +1 sides, refused while the tie-in rays reached the
    // exact cut's 20 m margin, grade into the hillside within the graded terrain's 64 m cap
    // (`docs/terrain.md` T12 (a), `TERRAIN-07-F`).
    assert_eq!(rejected, vec![]);
    assert_eq!(core.allocator.buildings.len(), 8);
    assert_eq!(core.zoning.parcels().len(), 8);
    core
}

// `TERRAIN-07-C`: the Kuopio yards as graded terrain clients. Their tiles bake without errors at
// every level, each pad's samples hold its support height bitwise, and every lattice point
// near the yards takes the material class the legacy paving partition gives it.
#[test]
fn kuopio_yards_bake_as_pads_with_the_partition_materials() {
    let mut core = kuopio_yard_core();
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    use crate::simulation::terrain::graded::site_tests::assert_allocator_sites_bake;
    let (baked, pads, paved) = assert_allocator_sites_bake(
        &core.heightmap,
        &core.transit_network.roadbed,
        &core.allocator,
    );
    assert!(
        baked > 0 && pads > 1_000 && paved > 1_000,
        "{baked} tiles, {pads} pad and {paved} paved samples"
    );
    // Pruned equals unpruned bitwise around the first yard, on the hills.
    use crate::simulation::terrain::graded::TileKey;
    let (x, z) = {
        let b = core.allocator.building_sites[0].bounds();
        (f64::from(b.0 + b.2) * 0.5, f64::from(b.1 + b.3) * 0.5)
    };
    let centre = TileKey::at(x, z);
    let tiles: Vec<TileKey> = (-2..2)
        .flat_map(|dx| (-2..2).map(move |dz| TileKey { x: centre.x + dx, z: centre.z + dz }))
        .collect();
    crate::simulation::terrain::graded::tests::assert_exact_pruning(
        &core.heightmap,
        &core.transit_network.roadbed,
        &core.allocator,
        &tiles,
        &[centre],
        "kuopio yards",
    );
}

// `TERRAIN-07-C`: the graded terrain stage of each Kuopio yard placement on the reference save
// (dirty tile selection from the placement's site change, then the `GL0` bake), on the hills.
#[test]
#[ignore = "controlled release timing: cargo test --release --lib benchmark_kuopio_yard_placements \
            -- --ignored --nocapture"]
fn benchmark_kuopio_yard_placements() {
    let mut core = test_core();
    core.load_game_internal(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../benchmarks/fixtures/kuopio-terrain/kuopio-terrain-map.sqlite"
    ))
    .unwrap();
    core.precompute_road_mesh_data();
    let asset = register_yard(&mut core);
    for road in 0..KUOPIO_YARD_ROADS.len() {
        for side in [-1.0, 1.0] {
            if place_kuopio_yard(&mut core, road, side, &asset).is_err() {
                continue;
            }
            core.allocator
                .prepare_building_site_query_index(core.config.zone_cell_m);
            let mut record = crate::simulation::terrain::graded::site_tests::site_stage(
                &core.heightmap,
                &core.transit_network.roadbed,
                &core.allocator,
                &core.last_site_terrain_changes,
            );
            record["road"] = road.into();
            record["side"] = side.into();
            record["sites"] = core.allocator.buildings.len().into();
            eprintln!("GRADED_KUOPIO_YARD {record}");
        }
    }
}

#[test]
fn new_world_clears_populated_city_and_engineered_terrain() {
    let mut core = kuopio_yard_fixture();
    let registry_len = core.allocator.registry.len();
    assert!(!core.refined_terrain_patch_cache.is_empty());
    core.create_blank_world_internal(256.0, 256.0, 8.0, 128.0, 50.0)
        .unwrap();
    assert!(core.allocator.buildings.is_empty());
    assert!(core.allocator.building_sites.is_empty());
    assert!(core.region_graph.edges().is_empty());
    assert!(core.zoning.parcels().is_empty());
    assert!(core.refined_terrain_patch_cache.is_empty());
    assert!(core.engineered_terrain_patch_keys.is_empty());
    assert!(core.building_site_owned_terrain_patch_keys.is_empty());
    assert!(core.cached_road_mesh_chunks.is_empty());
    assert!(
        core.collect_refined_terrain_patch_build_inputs(2.0)
            .is_empty()
    );
    assert_eq!(core.allocator.registry.len(), registry_len);
    assert_eq!(core.heightmap.sample_height_world(0.0, 0.0), 50.0);
}

#[test]
#[ignore = "unprofiled local terrain rebuild measurement; run alone with --release --ignored --nocapture"]
fn building_site_terrain_rebuild_benchmark() {
    let mut core = kuopio_yard_fixture();
    // Fixture placement and road compilation are outside this terrain-only measurement.
    core.refined_terrain_patch_cache.clear();
    let mut samples = Vec::new();
    for round in 0..22 {
        let start = std::time::Instant::now();
        let inputs = core.collect_refined_terrain_patch_build_inputs(2.0);
        let tiles = inputs.iter().map(|p| p.windows.len()).sum::<usize>();
        let assembled_ms = start.elapsed().as_secs_f64() * 1000.0;
        let start = std::time::Instant::now();
        let patches = SimCore::build_refined_terrain_patch_cache_entries(inputs);
        let built_ms = start.elapsed().as_secs_f64() * 1000.0;
        assert!(
            patches
                .iter()
                .all(|p| SimulationNode::cached_refined_cdt_failure_label(p).is_none())
        );
        if round >= 2 {
            samples.push((assembled_ms, built_ms, patches.len(), tiles));
        }
    }
    let mut assembly = samples.iter().map(|p| p.0).collect::<Vec<_>>();
    let mut build = samples.iter().map(|p| p.1).collect::<Vec<_>>();
    assembly.sort_by(f64::total_cmp);
    build.sort_by(f64::total_cmp);
    eprintln!(
        "building_site_terrain_rebuild: samples=20 patches={} tiles={} assembly_p50_ms={:.3} build_p50_ms={:.3} assembly_p90_ms={:.3} build_p90_ms={:.3}",
        samples[0].2, samples[0].3, assembly[10], build[10], assembly[18], build[18]
    );
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
                assert!(
                    rendered_paving_height(&core, point).is_some(),
                    "building {building} yard ({u}, {v}) at {point:?} is not paved"
                );
                // Agents stand on the visible graded surface (`TERRAIN-07-F`).
                let height = core.get_world_surface_height_internal(point);
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
                .roadbed
                .height_at(point.x, point.y)
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
        let mut visual = core.heightmap.clone_visual_dense();
        for (x, z) in writes {
            visual[z * core.heightmap.width + x] = (support + delta) / crate::config::HEIGHT_SCALE;
        }
        core.heightmap.replace_visual_from_dense(&visual).unwrap();
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
                    core.transit_network.roadbed.height_at(point.x, point.y),
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
        // The visible surface the contacts stand on: the graded surface (`TERRAIN-07-F`).
        let ground = core.get_world_surface_height_internal(point);
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
    assert!(core.ground_vehicle_transforms(0, &mut transforms, &[1, 0], None));
    check_vehicle_contacts(&core, &transforms[..12]);
    assert_eq!(&transforms[12..], &original);
    assert_eq!(transforms[3], original[3]);
    assert_eq!(transforms[11], original[11]);
    // Input failures are atomic, including malformed model IDs and nonfinite lane entries.
    for (vehicle, flags) in [(5, vec![1, 0]), (0, vec![1])] {
        let before = transforms.clone();
        assert!(!core.ground_vehicle_transforms(vehicle, &mut transforms, &flags, None));
        assert_eq!(transforms, before);
    }
    transforms[19] = f32::INFINITY;
    let before = transforms.clone();
    assert!(!core.ground_vehicle_transforms(0, &mut transforms, &[1, 0], None));
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
fn vehicle_ground_lod_keeps_boundary_exact_and_shares_the_far_pose() {
    use crate::nodes::sim::render::vehicle_ground::ground_vehicle_basis;

    let core = kuopio_yard_fixture();
    let site = &core.allocator.building_sites[0];
    let p = &site.surfaces[0].vertices_world;
    let point = p[0].lerp(p[1], 0.43).lerp(p[3].lerp(p[2], 0.43), 0.2);
    let heading = Vector3::BACK;
    let bits = |(height, basis): (f32, [Vector3; 3])| {
        let mut answer = vec![height.to_bits()];
        for axis in basis {
            answer.extend(axis.to_array().map(f32::to_bits));
        }
        answer
    };
    let exact = core.vehicle_ground_pose_exact(0, point, heading);
    let boundary = point + Vector2::new(core.vehicle_ground_full_detail_m, 0.0);
    assert_eq!(
        bits(core.vehicle_ground_pose(0, point, heading, Some(boundary))),
        bits(exact),
        "the full-detail boundary is inclusive"
    );

    let camera = boundary + Vector2::new(0.25, 0.0);
    let (centre_height, expected_basis) = core.with_graded_surface(|surface, scratch| {
        let (height, normal) =
            surface.surface_height_and_normal_at(f64::from(point.x), f64::from(point.y), scratch);
        (
            height as f32,
            ground_vehicle_basis(heading, Vector3::from_array(normal.map(|v| v as f32))),
        )
    });
    let mut support_y = centre_height;
    for contact in core.vehicle_ground_support[0].contacts() {
        let offset = expected_basis[0] * contact.x
            + expected_basis[1] * contact.y
            + expected_basis[2] * contact.z;
        support_y = support_y.max(
            core.get_world_surface_height_internal(point + Vector2::new(offset.x, offset.z))
                - offset.y,
        );
    }
    let expected = (support_y + 0.02, expected_basis);
    let far = core.vehicle_ground_pose(0, point, heading, Some(camera));
    assert_eq!(bits(far), bits(expected));
    for contact in core.vehicle_ground_support[0].contacts() {
        let offset = far.1[0] * contact.x + far.1[1] * contact.y + far.1[2] * contact.z;
        let ground =
            core.get_world_surface_height_internal(point + Vector2::new(offset.x, offset.z));
        assert!(far.0 + offset.y - ground >= 0.019_999);
    }

    let mut transform = vec![
        1.0, 0.0, 0.0, point.x, 0.0, 1.0, 0.0, -100.0, 0.0, 0.0, 1.0, point.y,
    ];
    assert!(core.ground_vehicle_transforms(0, &mut transform, &[1], Some(camera)));
    let (height, [x, y, z]) = far;
    assert_eq!(
        transform
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        [
            x.x, y.x, z.x, point.x, x.y, y.y, z.y, height, x.z, y.z, z.z, point.y,
        ]
        .map(f32::to_bits)
    );
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
            assert!(core.ground_vehicle_transforms(0, &mut transforms, &flags, None));
        }
        let mut samples = Vec::with_capacity(100);
        for _ in 0..100 {
            let start = std::time::Instant::now();
            assert!(std::hint::black_box(core.ground_vehicle_transforms(
                0,
                &mut transforms,
                &flags,
                None,
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

// Hedge modules standing in the vegetation edit store.
fn yard_hedge_count(core: &SimCore) -> usize {
    core.vegetation_edits
        .sorted_cells()
        .into_iter()
        .flat_map(|cell| core.vegetation_edits.cell(cell).1.to_vec())
        .filter(|plant| plant.species == 2 && plant.variant > 12)
        .count()
}

#[test]
fn a_spawned_house_lays_its_yard_hedge_which_bulldozing_takes_and_undo_returns() {
    let mut core = test_core();
    core.load_game_internal(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../benchmarks/fixtures/kuopio-terrain/kuopio-terrain-map.sqlite"
    ))
    .unwrap();
    core.precompute_road_mesh_data();
    let asset = register_yard(&mut core);
    let mut manifest = core
        .allocator
        .registry
        .get(&asset)
        .unwrap()
        .manifest
        .clone();
    manifest.building.as_mut().unwrap().yard_hedge = Some(crate::assets::asset::YardHedge {
        hedge: crate::assets::asset::YardHedgeKind::Medium,
        edges: crate::assets::asset::LotEdge::ALL.to_vec(),
    });
    core.allocator
        .registry
        .register("test", manifest, String::new());
    assert_eq!(yard_hedge_count(&core), 0);
    let building = place_kuopio_yard(&mut core, 0, -1.0, &asset).unwrap();
    let laid = yard_hedge_count(&core);
    // Most of a 20 m x 20 m lot's 80 m perimeter, less its gaps and anything the site rejects.
    assert!(laid > 40, "laid {laid}");
    // The record survives a save and load.
    let path = temp_save_path("yard_hedge");
    core.save_game_internal(path.to_str().unwrap(), None)
        .unwrap();
    core.load_game_internal(path.to_str().unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    core.precompute_road_mesh_data();
    assert_eq!(yard_hedge_count(&core), laid);
    assert!(core.bulldoze_building(building));
    assert_eq!(
        yard_hedge_count(&core),
        0,
        "an untouched yard hedge goes with its house"
    );
    assert!(core.undo_action_internal());
    assert_eq!(
        yard_hedge_count(&core),
        laid,
        "undo puts the house back with its hedge"
    );
}

#[test]
fn a_house_reaching_its_back_line_lays_every_planned_module() {
    let mut core = test_core();
    core.load_game_internal(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../benchmarks/fixtures/kuopio-terrain/kuopio-terrain-map.sqlite"
    ))
    .unwrap();
    core.precompute_road_mesh_data();
    let asset = register_yard(&mut core);
    let mut manifest = core
        .allocator
        .registry
        .get(&asset)
        .unwrap()
        .manifest
        .clone();
    // The fixture's 10.5 m house box moved back until its eaves reach the back lot line (+z).
    manifest.mesh_parts[0].position = [0.0, 0.0, 4.75];
    manifest.building.as_mut().unwrap().yard_hedge = Some(crate::assets::asset::YardHedge {
        hedge: crate::assets::asset::YardHedgeKind::Medium,
        edges: vec![
            crate::assets::asset::LotEdge::Back,
            crate::assets::asset::LotEdge::Left,
            crate::assets::asset::LotEdge::Right,
        ],
    });
    let structures: Vec<_> = manifest
        .mesh_parts
        .iter()
        .filter_map(crate::assets::asset::structure_footprint)
        .collect();
    let surfaces: Vec<_> = manifest
        .site_surfaces
        .iter()
        .map(|s| s.vertices.clone())
        .collect();
    let lot = crate::assets::asset::YardLot {
        half_width_m: 10.0,
        half_depth_m: 10.0,
        frontage: [0.0, -1.0],
        surfaces: &surfaces,
        entrance: None,
        structures: &structures,
    };
    let edges = manifest
        .building
        .as_ref()
        .unwrap()
        .yard_hedge
        .as_ref()
        .unwrap()
        .edges
        .clone();
    let rows = crate::assets::asset::plan_yard_hedge(&lot, &edges);
    // The back row opens behind the house instead of running under its eaves.
    assert_eq!(
        rows.iter()
            .filter(|row| row.from[1] == 10.0 && row.to[1] == 10.0)
            .count(),
        2
    );
    core.allocator
        .registry
        .register("test", manifest, String::new());
    place_kuopio_yard(&mut core, 0, -1.0, &asset).unwrap();
    // Every planned module is laid: none is refused on the house's own lawn. A joined corner
    // runs a row on by half the hedge's width, which can add one module per end.
    let lengths: Vec<f32> = rows
        .iter()
        .map(|row| ((row.to[0] - row.from[0]).powi(2) + (row.to[1] - row.from[1]).powi(2)).sqrt())
        .collect();
    let least: usize = lengths.iter().map(|l| l.ceil() as usize).sum();
    let most: usize = lengths.iter().map(|l| (l + 0.9).ceil() as usize).sum();
    let laid = yard_hedge_count(&core);
    assert!(
        (least..=most).contains(&laid),
        "laid {laid}, planned {least}..={most}: {rows:?}"
    );
}

// Every recorded yard planting plant, in record order.
fn yard_planting(core: &SimCore) -> Vec<crate::simulation::vegetation::edits::AuthoredPlant> {
    core.vegetation_edits
        .yard_planting()
        .flat_map(|(_, plants)| plants.iter().map(|(_, plant)| *plant))
        .collect()
}

#[test]
fn spawned_houses_plant_their_own_yards_which_go_with_them_and_return_on_undo() {
    use crate::assets::asset::{YardPlantKind, YardPlanting};
    let mut core = test_core();
    core.load_game_internal(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../benchmarks/fixtures/kuopio-terrain/kuopio-terrain-map.sqlite"
    ))
    .unwrap();
    core.precompute_road_mesh_data();
    let asset = register_yard(&mut core);
    let mut manifest = core
        .allocator
        .registry
        .get(&asset)
        .unwrap()
        .manifest
        .clone();
    // The strips beside the fixture's 10.5 m house: trees on one side, within a wild tree's 6 m
    // of the wall, and shrubs on the other.
    manifest.building.as_mut().unwrap().yard_planting = vec![
        YardPlanting {
            plants: YardPlantKind::Trees,
            name: String::new(),
            vertices: vec![[6.0, -3.0], [9.5, -3.0], [9.5, 9.5], [6.0, 9.5]],
        },
        YardPlanting {
            plants: YardPlantKind::Bushes,
            name: String::new(),
            vertices: vec![[-9.5, -3.0], [-6.0, -3.0], [-6.0, 9.5], [-9.5, 9.5]],
        },
    ];
    core.allocator
        .registry
        .register("test", manifest, String::new());
    let first = place_kuopio_yard(&mut core, 0, -1.0, &asset).unwrap();
    let first_plants = yard_planting(&core);
    let trees = first_plants
        .iter()
        .filter(|plant| plant.species < 2)
        .count();
    assert!(trees > 0 && first_plants.len() > trees, "{first_plants:?}");
    let second = place_kuopio_yard(&mut core, 1, -1.0, &asset).unwrap();
    let both = yard_planting(&core);
    // The same asset grows a different yard: compare each yard's plants in its own lot frame.
    let local = |building: usize,
                 plants: &[crate::simulation::vegetation::edits::AuthoredPlant]| {
        let b = &core.allocator.buildings[building];
        let front = core
            .allocator
            .registry
            .get(&b.asset_id)
            .unwrap()
            .manifest
            .building_frontage_forward();
        let (bx, bz) =
            crate::simulation::buildings::allocator::building_local_xz_basis(b.facing_dir, front);
        let centre = Vector2::new(b.center_x, b.center_y);
        let mut out: Vec<_> = plants
            .iter()
            .map(|p| {
                let d = Vector2::new(p.x, p.z) - centre;
                (
                    (d.dot(bx) * 10.0).round() as i32,
                    (d.dot(bz) * 10.0).round() as i32,
                )
            })
            .collect();
        out.sort_unstable();
        out
    };
    let second_plants = both[first_plants.len()..].to_vec();
    assert!(!second_plants.is_empty());
    assert_ne!(local(first, &first_plants), local(second, &second_plants));
    // A save and load keeps the yards as they grew.
    let path = temp_save_path("yard_planting");
    core.save_game_internal(path.to_str().unwrap(), None)
        .unwrap();
    core.load_game_internal(path.to_str().unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    core.precompute_road_mesh_data();
    assert_eq!(yard_planting(&core), both);
    // Bulldozing takes the first yard's plants only; undo plants the same ones again.
    assert!(core.bulldoze_building(first));
    let standing = |core: &SimCore, plant: &crate::simulation::vegetation::edits::AuthoredPlant| {
        core.vegetation_edits
            .sorted_cells()
            .into_iter()
            .any(|cell| core.vegetation_edits.cell(cell).1.contains(plant))
    };
    assert!(!first_plants.iter().any(|plant| standing(&core, plant)));
    assert!(second_plants.iter().all(|plant| standing(&core, plant)));
    assert!(core.undo_action_internal());
    assert!(first_plants.iter().all(|plant| standing(&core, plant)));
}

#[test]
fn a_flower_bed_grows_drifts_of_flowers_that_keep_their_room() {
    use crate::assets::asset::{YardPlantKind, YardPlanting};
    let mut core = test_core();
    core.load_game_internal(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../benchmarks/fixtures/kuopio-terrain/kuopio-terrain-map.sqlite"
    ))
    .unwrap();
    core.precompute_road_mesh_data();
    let asset = register_yard(&mut core);
    let mut manifest = core
        .allocator
        .registry
        .get(&asset)
        .unwrap()
        .manifest
        .clone();
    manifest.building.as_mut().unwrap().yard_planting = vec![YardPlanting {
        plants: YardPlantKind::Flowers,
        name: String::new(),
        vertices: vec![[6.0, -3.0], [9.5, -3.0], [9.5, 9.5], [6.0, 9.5]],
    }];
    core.allocator
        .registry
        .register("test", manifest, String::new());
    place_kuopio_yard(&mut core, 0, -1.0, &asset).unwrap();
    let plants = yard_planting(&core);
    // Flower variants are bush pins 16 to 21: the six flowers past the hedges.
    assert!(plants.len() >= 10, "{plants:?}");
    assert!(
        plants
            .iter()
            .all(|plant| plant.species == 2 && (16..=21).contains(&plant.variant))
    );
    let kinds: std::collections::HashSet<_> = plants.iter().map(|plant| plant.variant).collect();
    assert!(kinds.len() >= 2, "{kinds:?}");
    // Perennials keep 0.7 m between them and a flowering shrub 1.05 m to anything else.
    for (i, a) in plants.iter().enumerate() {
        for b in &plants[i + 1..] {
            let room = |plant: &crate::simulation::vegetation::edits::AuthoredPlant| {
                if plant.variant >= 19 && plant.variant <= 20 {
                    0.7
                } else {
                    0.35
                }
            };
            let gap = Vector2::new(a.x - b.x, a.z - b.z).length();
            assert!(gap + 1e-3 >= room(a) + room(b), "{a:?} {b:?}");
        }
    }
}

#[test]
fn yard_hedges_stand_on_the_graded_lawn_of_a_sloping_yard() {
    use crate::nodes::simulation_node::vegetation_api::{CANOPY_SALT, hedge_runs, scatter_layer};
    let mut core = test_core();
    core.load_game_internal(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../benchmarks/fixtures/kuopio-terrain/kuopio-terrain-map.sqlite"
    ))
    .unwrap();
    core.precompute_road_mesh_data();
    let asset = register_yard(&mut core);
    let mut manifest = core
        .allocator
        .registry
        .get(&asset)
        .unwrap()
        .manifest
        .clone();
    manifest.building.as_mut().unwrap().yard_hedge = Some(crate::assets::asset::YardHedge {
        hedge: crate::assets::asset::YardHedgeKind::Medium,
        edges: crate::assets::asset::LotEdge::ALL.to_vec(),
    });
    core.allocator
        .registry
        .register("test", manifest, String::new());
    // The first yard road's yards are graded up to 2.7 m below the heightmap under their hedges.
    for side in [-1.0, 1.0] {
        place_kuopio_yard(&mut core, 0, side, &asset).unwrap();
    }
    settle_and_check_yards(&mut core);
    let (mut min, mut max) = (Vector2::splat(f32::MAX), Vector2::splat(f32::MIN));
    for cell in core.vegetation_edits.sorted_cells() {
        for plant in core.vegetation_edits.cell(cell).1 {
            min = Vector2::new(min.x.min(plant.x), min.y.min(plant.z));
            max = Vector2::new(max.x.max(plant.x), max.y.max(plant.z));
        }
    }
    let origin = min.floor();
    let span = (max - origin).x.max((max - origin).y).ceil() + 1.0;
    let packed = scatter_layer(
        &core,
        origin,
        span,
        core.vegetation.canopy_cell_m,
        CANOPY_SALT,
        true,
    );
    let mut modules = 0;
    for plant in packed.as_chunks::<6>().0 {
        let (species, variant) = (plant[5] as u32 & 3, plant[5] as u32 >> 2);
        if !(species == 2 && variant > 12) {
            continue;
        }
        let pos = Vector2::new(plant[0], plant[2]);
        let lawn = core.get_world_surface_height_internal(pos);
        // A module settles 0.05 m into the lawn and no further.
        assert!(
            (plant[1] + 0.05 - lawn).abs() < 0.001,
            "module at {pos:?} y={} lawn={lawn}",
            plant[1]
        );
        modules += 1;
    }
    assert!(modules > 40, "modules {modules}");
    let runs = hedge_runs(&core, origin, span);
    assert!(!runs.is_empty());
    for run in runs.as_chunks::<7>().0 {
        let pos = Vector2::new(run[0], run[2]);
        let lawn = core.get_world_surface_height_internal(pos);
        assert!(
            (run[1] - lawn).abs() < 0.001,
            "run at {pos:?} y={} lawn={lawn}",
            run[1]
        );
    }
    // A distant run is one sheared box, so it is cut wherever the graded lawn bends under it.
    for run in runs.as_chunks::<7>().0 {
        let along = Vector2::new(run[3].cos(), -run[3].sin());
        for k in 0..=16 {
            let t = k as f32 / 16.0 - 0.5;
            let p = Vector2::new(run[0], run[2]) + along * run[4] * t;
            let lawn = core.get_world_surface_height_internal(p);
            assert!(
                (run[1] + run[5] * t - lawn).abs() < 0.2,
                "run {run:?} leaves the lawn at {p:?}"
            );
        }
    }
}


// `TERRAIN-07-F` vegetation placement digest on hills, compared across builds: the Kuopio
// reference save with its yards (each placement's verdict as one `VEG_YARD road side verdict`
// line), then the scatter of the 64 m patches over 512 m around the yards' centre, both layers,
// one `VEG_PLANT` line per placed plant (its packed record's bits), and per plant one
// `VEG_CLASS x z class base ground` line (the bits of its position, its ground class, 0 natural,
// 1 road, 2 pad, 3 paved, 4 graded, and the base and graded ground heights there in metres).
// With `METRUM_VEG_WHY` naming a file of `x z canopy` lines (position bits, 1 for the canopy
// layer), one `VEG_WHY x z canopy graded legacy` line each: the clearance tests that refuse a
// wild plant there under the graded rule (`clear_site`, T31) and under the legacy one it replaced
// ([`legacy_and_graded_clearance`]).
#[test]
#[ignore = "diagnostic: the hillside vegetation placement digest compared across builds; --ignored --nocapture"]
fn kuopio_vegetation_placement_digest_dump() {
    use crate::nodes::simulation_node::vegetation_api::{CANOPY_SALT, UNDERSTORY_SALT, scatter_layer};
    use crate::simulation::terrain::graded::{GradedSurface, GroundClass, QueryScratch};
    let mut core = test_core();
    core.load_game_internal(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../benchmarks/fixtures/kuopio-terrain/kuopio-terrain-map.sqlite"
    ))
    .unwrap();
    core.precompute_road_mesh_data();
    let asset = register_yard(&mut core);
    for road in 0..KUOPIO_YARD_ROADS.len() {
        for side in [-1.0, 1.0] {
            let verdict = place_kuopio_yard(&mut core, road, side, &asset).map(|_| ());
            println!("VEG_YARD {road} {side} {verdict:?}");
        }
    }
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    let core = &core;
    // The yard roads' midpoints' centre, fixed whichever yards a build accepts.
    let centre = KUOPIO_YARD_ROADS
        .iter()
        .map(|&([x, z], _)| Vector2::new(x, z))
        .sum::<Vector2>()
        / KUOPIO_YARD_ROADS.len() as f32;
    let corner = (centre / 64.0).floor() * 64.0 - Vector2::splat(256.0);
    let buildings = core.allocator.buildings.len();
    println!("VEG_CITY buildings={buildings} centre={centre:?}");
    let surface = GradedSurface {
        terrain: &core.heightmap,
        roadbed: &core.transit_network.roadbed,
        sites: &core.allocator,
        query_index: Some(&core.terrain_query_index),
    };
    let mut scratch = QueryScratch::default();
    let generator = core.vegetation;
    for pz in 0..8 {
        for px in 0..8 {
            let origin = corner + Vector2::new(px as f32, pz as f32) * 64.0;
            for (cell, salt, canopy) in [
                (generator.canopy_cell_m, CANOPY_SALT, true),
                (generator.understory_cell_m, UNDERSTORY_SALT, false),
            ] {
                for record in scatter_layer(core, origin, 64.0, cell, salt, canopy).as_chunks::<6>().0 {
                    let bits: Vec<String> = record.iter().map(|v| v.to_bits().to_string()).collect();
                    println!("VEG_PLANT {}", bits.join(" "));
                    let (x, z) = (f64::from(record[0]), f64::from(record[2]));
                    let class = match surface.mask_at(x, z, &mut scratch) {
                        GroundClass::Natural => 0,
                        GroundClass::Road => 1,
                        GroundClass::Pad => 2,
                        GroundClass::Paved(_) => 3,
                        GroundClass::Graded => 4,
                    };
                    println!(
                        "VEG_CLASS {} {} {class} {} {}",
                        record[0].to_bits(),
                        record[2].to_bits(),
                        surface.base_height_at(x, z, &mut scratch),
                        surface.ground_height_at(x, z, &mut scratch)
                    );
                }
            }
        }
    }
    let Ok(path) = std::env::var("METRUM_VEG_WHY") else {
        return;
    };
    for line in std::fs::read_to_string(path).unwrap().lines() {
        let fields: Vec<u32> = line.split(' ').map(|v| v.parse().unwrap()).collect();
        let (x, z) = (f32::from_bits(fields[0]), f32::from_bits(fields[1]));
        let (graded, legacy) =
            legacy_and_graded_clearance(core, &surface, &mut scratch, x, z, fields[2] == 1);
        println!("VEG_WHY {} {} {} {graded} {legacy}", fields[0], fields[1], fields[2]);
    }
}

// The tests that refuse a wild plant at `(x, z)` under the graded clearance (`clear_site`,
// `TERRAIN-07-F` round 1, T31) and under the legacy one (the visual base, the visible roads and
// the site supports), each as a mask over the 3x3 footprint: 1 road class, 2 pad class, 4 paved
// class, 8 visible road (a deck or a grounded road), 16 water, 32 field, 64 relief past the
// layer's limit from the centre (the graded ground, or the legacy visual base), 128 a site's
// support (legacy). The canopy's tree clearance is the same in both and not tested.
fn legacy_and_graded_clearance(
    core: &SimCore,
    surface: &crate::simulation::terrain::graded::GradedSurface<'_>,
    scratch: &mut crate::simulation::terrain::graded::QueryScratch,
    x: f32,
    z: f32,
    canopy: bool,
) -> (u32, u32) {
    use crate::simulation::terrain::graded::GroundClass;
    let (radius, relief) = if canopy { (6.0f32, 3.0f32) } else { (2.5, 1.6) };
    let visual = |p: Vector2| {
        core.heightmap.sample_visual_height_world(p.x, p.y) * crate::config::HEIGHT_SCALE
    };
    let (cx, cz) = (f64::from(x), f64::from(z));
    surface.gather_near(cx, cz, f64::from(radius) + 0.01, scratch);
    let graded_y = surface.ground_height_near(cx, cz, scratch) as f32;
    let legacy_y = visual(Vector2::new(x, z));
    let (mut graded, mut legacy) = (0, 0);
    for dz in [-radius, 0.0, radius] {
        for dx in [-radius, 0.0, radius] {
            let p = Vector2::new(x + dx, z + dz);
            let (px, pz) = (f64::from(p.x), f64::from(p.y));
            let mut shared = 0;
            if core.watermap.visible_depth_world(p.x, p.y) > 0.001 {
                shared |= 16;
            }
            if core.transit_network.roadbed.height_at(p.x, p.y).is_some() {
                shared |= 8;
            }
            if core.allocator.field_clearance.covers_point(p) {
                shared |= 32;
            }
            graded |= shared
                | match surface.mask_near(px, pz, scratch) {
                    GroundClass::Road => 1,
                    GroundClass::Pad => 2,
                    GroundClass::Paved(_) => 4,
                    GroundClass::Natural | GroundClass::Graded => 0,
                };
            if (surface.ground_height_near(px, pz, scratch) as f32 - graded_y).abs() > relief {
                graded |= 64;
            }
            legacy |= shared;
            if core.allocator.sample_building_site_height(p).is_some() {
                legacy |= 128;
            }
            if (visual(p) - legacy_y).abs() > relief {
                legacy |= 64;
            }
        }
    }
    (graded, legacy)
}
