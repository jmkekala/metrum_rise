// SPDX-License-Identifier: GPL-2.0-only

//! Zoned yard coverage on sloping roads, using production placement and terrain compilation.

use super::*;
use crate::simulation::terrain::graded::{GradedSurface, QueryScratch, TerrainQueryIndex};
use godot::prelude::Vector2;

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
#[ignore = "manual release query-index Kuopio memory and rebuild benchmark"]
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
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    core
}

fn graded_surface_pose(core: &SimCore, point: Vector2) -> (f32, Vector3) {
    core.with_graded_surface(|surface, scratch| {
        let (height, normal) = surface.surface_height_and_normal_at(
            f64::from(point.x),
            f64::from(point.y),
            scratch,
        );
        (
            height as f32,
            Vector3::from_array(normal.map(|value| value as f32)),
        )
    })
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
            .all(|p| core.get_world_surface_height_internal(*p).is_finite())
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
                    site.contains_point(point),
                    "building {building} yard ({u}, {v}) escaped its pad at {point:?}"
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
                let surface = graded_surface_pose(&core, point);
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
                    "mode={mode} phase={phase} point={point:?}: rendered agent y={} != ground {height} + clearance; road={:?} site={:?}",
                    transform[7],
                    core.transit_network.roadbed.height_at(point.x, point.y),
                    core.allocator.sample_building_site_height(point),
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
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
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
