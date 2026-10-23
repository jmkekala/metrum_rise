// SPDX-License-Identifier: GPL-2.0-only

//! The extension's Rayon work never reaches Rayon's global pool (`CODE-17`).
//!
//! The extension disables the global pool when it loads, so work started outside
//! `thread_pools::install`/`spawn` panics where it starts. This test binary keeps its global
//! pool, because most tests call Rayon code from their own thread, and Rayon can disable it
//! only once per process. The check therefore re-runs this binary as a child process that
//! disables the pool first and then drives, from its main thread, the core functions that
//! Godot's main thread calls, the simulation thread, the road-preview worker and spawned jobs.

use super::*;
use crate::assets::scan_pack_dir;
use crate::nodes::sim::core::{RoadCommitAck, RoadCommitRequest};
use crate::nodes::sim::render::car_visual::CarVisualSmoother;
use crate::simulation::economy::fiscal::POLICY_COMMERCIAL_PROPERTY_TAX;
use crate::simulation::zoning::ZoneType;
use crate::simulation::zoning::cells::CellSelectionShape;
use crate::simulation::zoning::profiles::ZoneDensity;
use glam::DVec2;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

const CHILD: &str = "bridge_entry_points_in_a_process_without_a_global_pool";
const CHILD_MARKER: &str = "METRUM_CODE17_GLOBAL_POOL_CHILD";
const FRAME_DT_S: f64 = 1.0 / 60.0;
const WORLD_M: f32 = 2048.0;
// The trunk starts this far inside the terrain edge, so it connects to the border.
const BORDER_INSET_M: f32 = 1.0;
const CROSS_STREETS_X_M: [f32; 3] = [-200.0, 0.0, 200.0];
const COAL_POWER_PLANT: &str = "kenney:building.power.coal_power_plant";
const FARM: &str = "kenney:building.industrial.small_farm";
const WAIT: Duration = Duration::from_secs(120);

#[test]
fn bridge_entry_points_never_use_the_global_pool() {
    let module = module_path!()
        .split_once("::")
        .map_or(module_path!(), |(_, path)| path);
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            format!("{module}::{CHILD}").as_str(),
            "--exact",
            "--include-ignored",
            "--test-threads=1",
            "--nocapture",
        ])
        .env(CHILD_MARKER, "1")
        .output()
        .expect("the test binary must start a copy of itself");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success() && log.contains("1 passed"),
        "the child process without a global pool failed:\n{log}"
    );
}

#[test]
#[ignore = "run in its own process by bridge_entry_points_never_use_the_global_pool"]
fn bridge_entry_points_in_a_process_without_a_global_pool() {
    // Disabling lasts for the whole process, so never in the suite's own.
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }
    assert!(
        crate::thread_pools::disable_global_pool(),
        "the global pool started before the test disabled it"
    );
    // The net is armed: any Rayon work outside the shared pool now panics.
    assert!(catch_unwind(rayon::current_num_threads).is_err());

    let mut entries = Entries::default();
    let mut town = starting_a_town(&mut entries);
    editing_a_town(&mut entries, &mut town);
    loading_a_save(&mut entries, &town);
    running_the_node_threads(&mut entries, town);
    new_worlds(&mut entries);
    a_benchmark_city(&mut entries);
    assert!(
        entries.failed.is_empty(),
        "entry points that panicked (\"The global thread pool has not been initialized\" above \
         names one that bypassed the shared pool): {:?}",
        entries.failed
    );
}

// Runs each entry point on its own, so one run names every entry point that panicked.
#[derive(Default)]
struct Entries {
    failed: Vec<&'static str>,
}

impl Entries {
    fn run<T>(&mut self, name: &'static str, entry: impl FnOnce() -> T) -> Option<T> {
        let result = catch_unwind(AssertUnwindSafe(entry));
        if result.is_err() {
            self.failed.push(name);
        }
        result.ok()
    }
}

fn assets_installed(core: &mut SimCore) {
    let mods = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../godot/bootstrap/mods");
    let scan = scan_pack_dir(&mods);
    assert!(
        !scan.packs.is_empty(),
        "bootstrap packs in {}",
        mods.display()
    );
    core.replace_asset_registry(scan.packs.into_iter().flat_map(|pack| {
        let pack_id = pack.pack.pack_id;
        pack.assets
            .into_iter()
            .map(move |(manifest, dir)| (pack_id.clone(), manifest, dir))
    }));
}

fn committed(core: &mut SimCore, (x0, z0): (f32, f32), (x1, z1): (f32, f32)) {
    let outcome = core.commit_road(
        RoadCommitRequest {
            points: vec![Vector3::new(x0, 0.0, z0), Vector3::new(x1, 0.0, z1)],
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
        },
        Instant::now(),
        Default::default(),
    );
    assert!(outcome.committed, "{}", outcome.rejection);
}

fn painted(core: &mut SimCore, zone_type: ZoneType, path: &[DVec2]) {
    let profile = core
        .zoning
        .profiles
        .runtime_id_for_zone_density(zone_type, ZoneDensity::Low)
        .unwrap();
    let preview =
        core.preview_cell_selection_internal(CellSelectionShape::Brush { radius_m: 40.0 }, path);
    assert!(core.apply_cell_selection_internal(&preview, profile));
}

fn new_core() -> SimCore {
    SimCore::new(WorldConfig::new(WORLD_M, WORLD_M, 40.0, 10.0))
}

// SimulationNode::init's sequence, then the player's first tools and a day of growth: road
// commits, the border connection, cell paint, a funded service building and policy changes.
fn starting_a_town(e: &mut Entries) -> SimCore {
    let mut core = new_core();
    e.run("init", || {
        core.precompute_road_mesh_data();
        core.build_snapshot();
        road_tool_snapshots_from_core(&core).unwrap();
    });
    e.run("asset packs", || assets_installed(&mut core));

    let border_x = BORDER_INSET_M - core.heightmap.half_world_extents().0;
    e.run("road commits", || {
        committed(&mut core, (border_x, 0.0), (300.0, 0.0));
        for x in CROSS_STREETS_X_M {
            committed(&mut core, (x, -250.0), (x, 250.0));
        }
    });
    e.run("border connection", || {
        let border_y = core.heightmap.sample_height_world(border_x, 0.0);
        let border = core.check_border_candidate_internal(Vector3::new(border_x, border_y, 0.0));
        core.set_border_connection_internal(border as i32);
        core.fit_lanes_to_road_surface();
    });
    e.run("cell paint", || {
        for (x, zone_type) in CROSS_STREETS_X_M.into_iter().zip([
            ZoneType::Residential,
            ZoneType::Residential,
            ZoneType::Industrial,
        ]) {
            for (start, end) in [(-210.0, -30.0), (30.0, 210.0)] {
                let x = f64::from(x);
                painted(
                    &mut core,
                    zone_type,
                    &[DVec2::new(x, start), DVec2::new(x, end)],
                );
            }
        }
        painted(
            &mut core,
            ZoneType::Commercial,
            &[DVec2::new(30.0, 0.0), DVec2::new(170.0, 0.0)],
        );
    });
    e.run("service building and funding", || {
        let plant = core
            .place_service_building_internal(COAL_POWER_PLANT, -560.0, 8.0)
            .expect("power plant placement");
        let building = &core.allocator.buildings[plant];
        let (x, z) = (building.center_x, building.center_y);
        assert!(core.set_building_service_funding_override_at(x, z, "electricity", 0.1));
        assert!(core.set_service_funding(SERVICE_POLICY_ELECTRICITY, 0.8));
    });
    e.run("fiscal policy and cheat", || {
        assert!(core.set_fiscal_policy_value(POLICY_COMMERCIAL_PROPERTY_TAX, 12.0));
        core.apply_money_and_max_demand_cheat(1.0e6);
    });
    e.run("explicit industry", || {
        let farm = core
            .place_industry_building_internal(FARM, -900.0, 8.0)
            .expect("farm placement");
        let back = core.allocator.building_sites[farm]
            .lot_footprint_world
            .iter()
            .map(|point| point.y)
            .fold(f32::NEG_INFINITY, f32::max);
        let origin = Vector2::new(-910.0, back + 1.0);
        let field = [(0.0, 0.0), (200.0, 0.0), (200.0, 200.0), (0.0, 200.0)]
            .map(|(x, z)| origin + Vector2::new(x, z));
        core.commit_field_polygon_internal(farm, field.to_vec())
            .expect("grain field");
        let pending = core
            .place_industry_building_internal(FARM, -900.0, -40.0)
            .expect("second farm placement");
        assert!(core.cancel_pending_industry_building_internal(pending));
    });
    e.run("snapshot", || core.build_snapshot());

    e.run("a day of frames", || {
        core.time.speed_multiplier = 32.0;
        let day = core.time.day_index;
        for _ in 0..20_000 {
            if core.time.day_index > day {
                break;
            }
            core.step_frame(FRAME_DT_S);
        }
        assert!(core.time.day_index > day);
        assert!(!core.allocator.buildings.is_empty() && !core.agents.is_empty());
    });
    core
}

// Edits Godot's main thread makes under the core lock, each followed by the snapshot refresh
// the node does after it, and the per-frame render calls.
fn editing_a_town(e: &mut Entries, core: &mut SimCore) {
    e.run("water reflection bridge", || {
        let directions = [
            Vector3::new(-0.25, -1.0, 0.0).normalized(),
            Vector3::new(0.25, -1.0, 0.0).normalized(),
        ];
        let levels = super::super::water_api::first_water_surfaces_for_rays(
            core,
            Vector3::new(0.0, 500.0, 0.0),
            &directions,
            1_000.0,
        );
        assert_eq!(levels.len(), directions.len());
    });
    e.run("cell chunks", || {
        core.prepare_stale_cell_chunks_internal();
        core.prepare_requested_cell_chunk((0, 0), core.terrain_payload_global_generation);
    });
    e.run("surface ray", || {
        core.intersect_world_surface_internal(
            Vector3::new(0.0, 500.0, 50.0),
            Vector3::new(0.0, -1.0, 0.0),
        )
    });
    e.run("edge flags", || {
        let edge = (0..core.region_graph.edge_count())
            .find(|&edge| {
                core.region_graph
                    .get_edge(edge)
                    .is_some_and(|edge| !edge.deleted)
            })
            .unwrap() as i32;
        core.set_no_building_spawn_internal(edge, true);
        core.set_no_building_spawn_internal(edge, false);
        core.set_vehicle_frontage_access_internal(edge, 1);
        core.set_edge_class_internal(edge, 1);
        core.set_edge_class_internal(edge, 0);
        core.build_snapshot();
    });
    e.run("node move", || {
        let node = core.region_graph.get_edge(0).unwrap().end_node;
        let at = core.region_graph.node(node).pos;
        core.move_network_node_internal(node as i32, at + Vector3::new(4.0, 0.0, 4.0));
        core.build_snapshot();
    });
    e.run("terrain tools", || {
        core.start_terrain_stroke_internal();
        core.sculpt_terrain_internal(Vector2::new(700.0, 700.0), 30.0, 2.0);
        core.end_terrain_stroke_internal();
        core.build_snapshot();
        let at = Vector2::new(-100.0, 120.0);
        core.level_terrain_internal(at, 20.0, 2.0, 1.0);
        core.smooth_terrain_internal(at, 20.0, 1.0);
        let (from, to) = (at - Vector2::new(10.0, 0.0), at + Vector2::new(10.0, 0.0));
        core.slope_terrain_internal(at, 20.0, from, 0.0, to, 2.0, 1.0);
        core.build_snapshot();
    });
    e.run("world tools", || {
        let at = Vector2::new(600.0, -600.0);
        core.paint_world_coal_deposit_internal(at, 40.0, 80.0);
        let _ = core.begin_world_lake_fill_preview_internal(at, 3.0);
        let _ = core.update_world_lake_fill_preview_internal(4.0);
        let _ = core.commit_world_lake_fill_preview_internal();
        core.remove_world_lake_fill_near_internal(at, 50.0);
        let _ = core.begin_world_open_water_fill_preview_internal(at, 3.0);
        core.cancel_world_water_fill_preview_internal();
        core.build_snapshot();
    });
    e.run("vegetation", || {
        let pos = Vector2::new(-600.0, 400.0);
        super::super::vegetation_api::plant_at(core, pos);
        super::super::vegetation_api::scatter_layer(core, pos, 64.0, 8.0, 1, true);
        super::super::vegetation_api::remove_at(core, pos, 20.0, 0);
    });
    e.run("car grounding and smoothing", || {
        let mut cars = core.build_snapshot();
        for (&key, transforms) in cars.car_transforms.iter_mut() {
            let flags = &cars.car_ground_flags[&key];
            core.ground_vehicle_transforms(0, transforms, flags);
            let ids = &cars.car_render_ids[&key];
            CarVisualSmoother::default().interpolate(transforms, ids, flags, 0.5);
        }
    });
}

// `load_game`: the save replaces a fresh node's world, which then runs.
fn loading_a_save(e: &mut Entries, core: &SimCore) {
    let path = std::env::temp_dir().join(format!("metrum-code17-{}.sav", std::process::id()));
    let path = path.to_str().unwrap();
    core.save_game_internal(path, None).unwrap();
    let mut loaded = new_core();
    assets_installed(&mut loaded);
    e.run("load", || {
        loaded.load_game_internal(path).unwrap();
        loaded.precompute_road_mesh_data();
        loaded.build_snapshot();
        loaded.step_frame(FRAME_DT_S);
    });
    std::fs::remove_file(path).unwrap();
}

// The node's threads and jobs while Godot's main thread sends commands, stopped and joined as
// freeing the node does.
fn running_the_node_threads(e: &mut Entries, core: SimCore) {
    let (context, query) = road_tool_snapshots_from_core(&core).unwrap();
    let generation = core.terrain_payload_global_generation;
    let surface_generation = core.road_tool_surface_generation;
    let snapshot = Arc::new(RwLock::new(RenderSnapshot::default()));
    let context = Arc::new(RwLock::new(context));
    let result = Arc::new(RwLock::new(None));
    let core = Arc::new(Mutex::new(core));
    let (commands, command_rx) = std::sync::mpsc::channel();
    let sim_thread = {
        let (core, snapshot, context, result) = (
            Arc::clone(&core),
            Arc::clone(&snapshot),
            Arc::clone(&context),
            Arc::clone(&result),
        );
        let query = Arc::new(RwLock::new(query));
        std::thread::spawn(move || {
            run_sim_thread(
                core,
                snapshot,
                context,
                query,
                result,
                Arc::new(PublishedRoadPreviewChecks::default()),
                command_rx,
            );
        })
    };
    let (mut road_preview, road_preview_rx) = road_preview_channel();
    let road_preview_worker = {
        let (core, context, result) =
            (Arc::clone(&core), Arc::clone(&context), Arc::clone(&result));
        std::thread::spawn(move || {
            run_road_preview_worker(core, context, result, road_preview_rx);
        })
    };
    // A job that panics aborts this process, which fails the parent test.
    let jobs = Arc::new(async_jobs::AsyncJobs::default());
    jobs.spawn(|| {
        assert_eq!(
            (0..4_096_u64).into_par_iter().sum::<u64>(),
            4_096 * 4_095 / 2
        );
    });

    let points = vec![
        Vector3::new(-300.0, 0.0, 150.0),
        Vector3::new(-300.0, 0.0, 400.0),
    ];
    assert!(road_preview.submit(RoadPreviewRequest {
        include_terrain: true,
        request_id: 1,
        surface_generation,
        points: points.clone(),
        fwd_lanes: 1,
        bkw_lanes: 1,
        snap_to_existing_roads: true,
    }));
    let (cell_done, cell_done_rx) = std::sync::mpsc::sync_channel(1);
    let (road_done, road_done_rx) = std::sync::mpsc::sync_channel::<RoadCommitAck>(1);
    for command in [
        SimCommand::SetSpeed(32.0),
        SimCommand::SetCameraAabb(-1_000.0, 1_000.0, -1_000.0, 1_000.0),
        SimCommand::SetCellOverlayVisible(true),
        SimCommand::PrepareCellChunk {
            chunk: (0, 0),
            world_generation: generation,
            completion: cell_done,
        },
        SimCommand::AddRoad {
            points,
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            enqueued_at: Instant::now(),
            completion: road_done,
        },
        SimCommand::Undo,
    ] {
        commands.send(command).unwrap();
    }
    e.run("simulation thread commands", || {
        cell_done_rx.recv_timeout(WAIT).unwrap();
        let road = road_done_rx.recv_timeout(WAIT).unwrap();
        assert!(road.committed, "{}: {}", road.reason, road.detail);
    });
    // Both threads publish: a preview result and frames whose clock moves.
    e.run("published preview and frames", || {
        let started = Instant::now();
        let minute = || snapshot.read().unwrap().current_minute_of_day;
        let first = minute();
        while result.read().unwrap().is_none() || minute() == first {
            assert!(!sim_thread.is_finished() && !road_preview_worker.is_finished());
            assert!(started.elapsed() < WAIT);
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    e.run("main thread under the lock", || {
        let mut core = core.lock().unwrap();
        core.build_snapshot();
    });

    commands.send(SimCommand::Stop).unwrap();
    road_preview.disconnect();
    e.run("simulation thread", || sim_thread.join().unwrap());
    e.run("road-preview worker", || {
        road_preview_worker.join().unwrap()
    });
    jobs.wait();
}

// New game and world-editor resets: a blank world, then a saved world definition loaded back.
fn new_worlds(e: &mut Entries) {
    let mut core = new_core();
    let path = std::env::temp_dir().join(format!("metrum-code17-{}.world", std::process::id()));
    let path = path.to_str().unwrap();
    e.run("blank world", || {
        core.create_blank_world_internal(WORLD_M, WORLD_M, 10.0, 640.0, 0.0)
            .unwrap();
        core.build_snapshot();
    });
    core.save_world_definition_internal(path, "code17").unwrap();
    e.run("world definition", || {
        core.load_world_definition_internal(path, VegetationConfig::default())
            .unwrap();
        core.build_snapshot();
    });
    std::fs::remove_file(path).unwrap();
}

// What `setup_benchmark_city` and `--generate-benchmark` run, without their Godot logging: a
// bulk-loaded street grid, then the routing hierarchy.
fn a_benchmark_city(e: &mut Entries) {
    let mut core = new_core();
    e.run("benchmark city", || {
        core.transit_network.bulk_load = true;
        for i in 0..=6 {
            let offset = -600.0 + 200.0 * i as f32;
            core.add_road_internal(
                vec![
                    Vector3::new(-600.0, 0.0, offset),
                    Vector3::new(600.0, 0.0, offset),
                ],
                2,
                2,
            );
            core.add_road_internal(
                vec![
                    Vector3::new(offset, 0.0, -600.0),
                    Vector3::new(offset, 0.0, 600.0),
                ],
                2,
                2,
            );
        }
        core.transit_network
            .finalize_bulk_load(&mut core.region_graph, &mut core.allocator);
        core.transit_network.rebuild_pathing(&mut core.region_graph);
        core.precompute_road_mesh_data();
        core.build_snapshot();
    });
}
