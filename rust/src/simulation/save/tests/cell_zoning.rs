// SPDX-License-Identifier: GPL-2.0-only

//! Mixed parcel/cell save round trips, grace claims, and malformed cell authority validation.

use super::*;
use crate::simulation::zoning::cells::{
    CellBounds, CellFrontage, CellLot, CellSelection, CellSelectionShape,
};
use glam::DVec2;

mod curves;

fn fixture(angle: f64, erase: bool) -> (RegionGraph, ZoningSystem, BuildingAllocator) {
    let u = DVec2::new(angle.cos(), angle.sin());
    let v = u.perp();
    let config = WorldConfig::new(400.0, 400.0, 10.0, 10.0);
    let mut graph = RegionGraph::new();
    let points: Vec<_> = [-60.0, 60.0]
        .into_iter()
        .map(|s| Vector3::new((u.x * s) as f32, 0.0, (u.y * s) as f32))
        .collect();
    let start = graph.add_node(points[0], NodeType::Junction);
    let end = graph.add_node(points[1], NodeType::Junction);
    let edge = graph.add_edge(Edge {
        start_node: start,
        end_node: end,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: DEFAULT_URBAN_ROAD_SPEED_MS,
        base_cost: 120.0,
        physical_length: points[0].distance_to(points[1]),
        geometry: points.clone(),
        physical_geometry: points,
        ..Default::default()
    });
    let mut zoning = ZoningSystem::new(&config);
    let bounds = CellBounds {
        min: DVec2::splat(-200.0),
        max: DVec2::splat(200.0),
    };
    zoning.generate_cells(&graph, bounds, |_| false);
    let seed = u * -45.0 + v * 10.0;
    let selection = zoning.cells.select(CellSelectionShape::Fill, &[seed]);
    zoning.paint_cells(&selection, 1, |_| false).unwrap();
    let origin = zoning.cells.pick(seed).unwrap();
    let lot = CellLot::new(origin, 3, 3, CellFrontage::MinY).unwrap();
    let id = zoning
        .install_prevalidated_cell_lot(lot, edge, -1, 25.0 / 120.0, 1)
        .unwrap();
    zoning.restore_parcel_build_generation(id.raw(), 17);
    let manual_position = -v * 20.0;
    zoning
        .place_or_rezone_parcel_at(
            manual_position.x as f32,
            manual_position.y as f32,
            0,
            20.0,
            20.0,
            &graph,
        )
        .unwrap();
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(&mut allocator, "cell_test", "home", ZoneClass::Residential);
    let mut building = snapshot_test_building(edge, 1, asset);
    building.parcel_id = id.raw();
    building.side = -1;
    building.frontage_t = 25.0 / 120.0;
    building.build_generation = 17;
    building.pending_redevelopment = erase;
    building.rezone_grace_days_remaining = if erase { 11 } else { 0 };
    allocator.buildings.push(building);
    assert!(zoning.occupy_parcel(id.raw(), 0));
    if erase {
        let selection = zoning.cells.select(CellSelectionShape::Cell, &[seed]);
        zoning.paint_cells(&selection, 0, |_| false).unwrap();
    }
    allocator
        .recompute_derived_transforms(&graph, &zoning)
        .unwrap();
    (graph, zoning, allocator)
}

fn write_snapshot(
    path: &Path,
    graph: &RegionGraph,
    zoning: &ZoningSystem,
    allocator: &BuildingAllocator,
) {
    let config = &zoning.config;
    save_to_sqlite(
        path,
        SaveGameView {
            camera: None,
            config,
            graph,
            zoning,
            allocator,
            vegetation: &VegetationConfig::default(),
            vegetation_edits: &VegetationEdits::default(),
            time: &TimeSystem::new(),
            terrain: &TerrainSystem::from_world_config(config),
            water: &WaterSystem::from_world_config(config),
            resource_deposits: &ResourceDepositSystem::from_world_config(config),
            pollution: &PollutionSystem::new(config),
            noise: &NoiseSystem::new(config),
            demand: &DemandSystem::new(),
            pending_demand_spawns: &VecDeque::new(),
            households: &HouseholdSystem::new(),
            logistics: &ShipmentSystem::new(),
            resource_extraction: &ResourceExtractionSystem::new(),
            agriculture: &AgricultureSystem::new(),
            agents: &AgentSystem::new(),
            network: &TransitNetwork::new_for_world(config),
            treasury: &CityTreasury::new(1_000.0),
            service_policy: &CityServicePolicy::default(),
            fiscal_policy: &CityFiscalPolicy::default(),
            budget_history: &VecDeque::new(),
        },
    )
    .unwrap();
}

#[test]
fn mixed_workflows_round_trip_canonical_cells_lots_buildings_and_rezone_grace() {
    for angle in [0.0, 0.35] {
        for erase in [false, true] {
            let (graph, zoning, allocator) = fixture(angle, erase);
            let path = temp_path("cell_zoning_round_trip");
            write_snapshot(&path, &graph, &zoning, &allocator);
            let loaded = load_from_sqlite(&path, &allocator.registry).unwrap();
            fs::remove_file(path).unwrap();
            assert_eq!(
                loaded.zoning.cells.saved_frames().collect::<Vec<_>>(),
                zoning.cells.saved_frames().collect::<Vec<_>>()
            );
            assert_eq!(
                loaded.zoning.cells.saved_cells(),
                zoning.cells.saved_cells()
            );
            assert_eq!(loaded.zoning.parcels().len(), 2);
            for parcel in zoning.parcels() {
                let restored = loaded.zoning.parcel_by_raw_id(parcel.id().raw()).unwrap();
                assert_eq!(restored.cell_lot(), parcel.cell_lot());
                assert_eq!(restored.corners(), parcel.corners());
                assert_eq!(restored.occupied_building(), parcel.occupied_building());
                assert_eq!(
                    restored.zone_profile_runtime_id(),
                    parcel.zone_profile_runtime_id()
                );
                assert_eq!(restored.build_generation(), parcel.build_generation());
                if let Some(lot) = parcel.cell_lot() {
                    assert!(
                        lot.cells()
                            .all(|key| loaded.zoning.cells.lot(key) == Some(parcel.id().raw()))
                    );
                }
            }
            let before = &allocator.buildings[0];
            let after = &loaded.allocator.buildings[0];
            assert_eq!(
                (after.center_x, after.center_y, after.facing_dir),
                (before.center_x, before.center_y, before.facing_dir)
            );
            assert_eq!(after.pending_redevelopment, erase);
            assert_eq!(
                after.rezone_grace_days_remaining,
                before.rezone_grace_days_remaining
            );
            assert_eq!(after.build_generation, 17);
        }
    }
}

#[test]
fn old_parcel_save_loads_without_cell_tables_or_conversion() {
    let (graph, _, allocator) = fixture(0.0, false);
    let mut zoning = ZoningSystem::new(&WorldConfig::new(400.0, 400.0, 10.0, 10.0));
    let id = zoning
        .place_or_rezone_parcel_at(0.0, -20.0, 1, 30.0, 30.0, &graph)
        .unwrap();
    let path = temp_path("pre_cell_zoning");
    write_snapshot(&path, &graph, &zoning, &BuildingAllocator::new());
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "UPDATE save_meta SET version = 64;
        DROP TABLE zoning_cell_frames; DROP TABLE zoning_cells; DROP TABLE zoning_cell_lots;",
    )
    .unwrap();
    drop(conn);
    let loaded = load_from_sqlite(&path, &allocator.registry).unwrap();
    fs::remove_file(path).unwrap();
    assert_eq!(loaded.zoning.parcels().len(), 1);
    assert!(loaded.zoning.parcels()[0].cell_lot().is_none());
    assert_eq!(loaded.zoning.parcels()[0].id(), id);
    assert!(loaded.zoning.cells.saved_frames().next().is_none());
}

#[test]
fn corrupted_cell_authority_is_rejected_before_publishing_the_loaded_world() {
    let (graph, zoning, allocator) = fixture(0.35, true);
    let path = temp_path("invalid_cell_zoning");
    write_snapshot(&path, &graph, &zoning, &allocator);
    let valid = fs::read(&path).unwrap();
    for sql in [
        "UPDATE zoning_cell_frames SET frame_json = '{}'",
        "UPDATE zoning_cells SET profile_runtime_id = 65535",
        "UPDATE zoning_cells SET profile_runtime_id = 0 WHERE x = (SELECT MAX(x) FROM zoning_cells)",
        "DELETE FROM zoning_cells",
        "UPDATE zoning_cell_lots SET parcel_id = 999999",
        "UPDATE zoning_cell_lots SET coverage_json = '{}'",
        "UPDATE zoning_parcels SET frontage_m = 20 WHERE parcel_id = 1",
    ] {
        fs::write(&path, &valid).unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(sql).unwrap();
        drop(conn);
        assert!(
            load_from_sqlite(&path, &allocator.registry).is_err(),
            "accepted {sql}"
        );
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn saved_road_frames_validate_sources_and_legacy_saves_initialize_them() {
    let (graph, mut zoning, allocator) = fixture(0.35, false);
    zoning
        .cells
        .refresh_road_alignments(&graph, &zoning.config, 0..graph.edge_count());
    let path = temp_path("road_grid_authority");
    write_snapshot(&path, &graph, &zoning, &allocator);
    let valid = fs::read(&path).unwrap();
    let restored = load_from_sqlite(&path, &allocator.registry).unwrap();
    assert_eq!(
        restored.zoning.cells.saved_road_alignments(),
        zoning.cells.saved_road_alignments()
    );
    for sql in [
        "UPDATE zoning_cell_road_alignments SET edge_id = 999999",
        "UPDATE zoning_cell_road_alignments SET alignment_json = '{}'",
        "UPDATE zoning_cell_road_alignments SET alignment_json = json_set(alignment_json, '$.source[0]', 0)",
        "UPDATE zoning_cell_road_alignments SET alignment_json = json_set(alignment_json, '$.frames[0].key.phase[1]', 0)",
    ] {
        fs::write(&path, &valid).unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(sql).unwrap();
        drop(conn);
        assert!(
            load_from_sqlite(&path, &allocator.registry).is_err(),
            "accepted {sql}"
        );
    }
    fs::write(&path, &valid).unwrap();
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "UPDATE save_meta SET version = 66; DROP TABLE zoning_cell_road_alignments;",
    )
    .unwrap();
    drop(conn);
    let restored = load_from_sqlite(&path, &allocator.registry).unwrap();
    assert_eq!(
        restored.zoning.cells.saved_cells(),
        zoning.cells.saved_cells()
    );
    assert_eq!(restored.zoning.cells.saved_road_alignments().len(), 1);
    fs::remove_file(path).unwrap();
}

fn insert_saved_field(conn: &Connection, corners: &[Vector2; 4]) {
    let area = crate::simulation::agriculture::validate_field_polygon_world(corners).unwrap();
    conn.execute(
        "INSERT INTO agriculture_field_sites VALUES (0, 0, 'grain', ?1)",
        [area],
    )
    .unwrap();
    for (index, point) in corners.iter().enumerate() {
        conn.execute(
            "INSERT INTO agriculture_field_site_points VALUES (0, ?1, ?2, ?3)",
            params![index, point.x, point.y],
        )
        .unwrap();
    }
}

#[test]
fn loaded_field_cannot_overlap_painted_or_erased_occupied_cells() {
    for angle in [0.0, 0.35] {
        for erase_all in [false, true] {
            let (graph, mut zoning, allocator) = fixture(angle, false);
            let parcel = zoning
                .parcel_by_raw_id(allocator.buildings[0].parcel_id)
                .unwrap();
            let corners = parcel.corners();
            if erase_all {
                let selection = CellSelection {
                    revision: zoning.cells.revision(),
                    cells: zoning
                        .cells
                        .saved_cells()
                        .into_iter()
                        .map(|(key, _)| key)
                        .collect(),
                };
                zoning.paint_cells(&selection, 0, |_| false).unwrap();
                assert!(
                    zoning
                        .cells
                        .saved_cells()
                        .iter()
                        .all(|(_, profile)| *profile == 0)
                );
            }
            let path = temp_path("cell_field_conflict");
            write_snapshot(&path, &graph, &zoning, &allocator);
            // Erased occupied lots themselves remain valid saved authority.
            assert!(load_from_sqlite(&path, &allocator.registry).is_ok());
            let conn = Connection::open(&path).unwrap();
            insert_saved_field(&conn, &corners);
            drop(conn);
            let error = load_from_sqlite(&path, &allocator.registry).err().unwrap();
            assert!(
                error
                    .to_string()
                    .contains("saved field overlaps reserved zoning cells"),
                "{error}"
            );
            fs::remove_file(path).unwrap();
        }
    }
}

#[test]
fn loaded_field_may_touch_cells_but_cannot_intrude_into_their_interiors() {
    let (graph, zoning, allocator) = fixture(0.0, false);
    let outer_z = zoning
        .cells
        .saved_cells()
        .iter()
        .flat_map(|(key, _)| zoning.cells.frame(key.grid).unwrap().corners(key.x, key.y))
        .map(|point| point.y)
        .fold(f64::NEG_INFINITY, f64::max) as f32;
    for offset in [0.0, 1.0, -0.001] {
        let path = temp_path("cell_field_shared_edge");
        write_snapshot(&path, &graph, &zoning, &allocator);
        let z = outer_z + offset;
        let corners = [
            Vector2::new(0.0, z),
            Vector2::new(20.0, z),
            Vector2::new(20.0, z + 20.0),
            Vector2::new(0.0, z + 20.0),
        ];
        let conn = Connection::open(&path).unwrap();
        insert_saved_field(&conn, &corners);
        drop(conn);
        assert_eq!(
            load_from_sqlite(&path, &allocator.registry).is_ok(),
            offset >= 0.0
        );
        fs::remove_file(path).unwrap();
    }
}

#[test]
fn load_checks_explicit_sites_and_compiled_roads_against_cell_reservations() {
    let (graph, zoning, allocator) = fixture(0.0, false);
    let path = temp_path("cell_external_reservations");
    write_snapshot(&path, &graph, &zoning, &allocator);
    let valid = fs::read(&path).unwrap();
    for (sql, expected) in [
        (
            "UPDATE buildings SET parcel_id = 0, profile_runtime_id = 0, frontage_t = 0.8, width = 1, depth = 1",
            "saved explicit site overlaps reserved zoning cells",
        ),
        (
            "DELETE FROM building_inventories; DELETE FROM buildings;
             DELETE FROM zoning_cell_lots; DELETE FROM zoning_parcels;
             UPDATE network_edges SET width = width + 10",
            "saved road overlaps reserved zoning cells",
        ),
    ] {
        fs::write(&path, &valid).unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(sql).unwrap();
        drop(conn);
        let error = load_from_sqlite(&path, &allocator.registry).err().unwrap();
        assert!(error.to_string().contains(expected), "{error}");
    }
    // Disabling growth removes frontage eligibility, not the player's saved paint.
    fs::write(&path, &valid).unwrap();
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("UPDATE network_edges SET no_building_spawn = 1")
        .unwrap();
    drop(conn);
    let loaded = load_from_sqlite(&path, &allocator.registry).unwrap();
    assert_eq!(
        loaded.zoning.cells.saved_cells(),
        zoning.cells.saved_cells()
    );
    assert_eq!(loaded.zoning.parcels().len(), zoning.parcels().len());
    assert_eq!(loaded.allocator.buildings.len(), allocator.buildings.len());
    fs::remove_file(path).unwrap();
}

#[test]
#[ignore = "isolated release timing of load reservation checks, excluding hydration and index setup"]
fn benchmark_cell_load_reservations() {
    use crate::simulation::zoning::cells::{CellKey, GridFrame};
    use std::hint::black_box;
    use std::time::Instant;

    let (graph, zoning, allocator) = fixture(0.0, false);
    let path = temp_path("cell_load_reservations_benchmark");
    write_snapshot(&path, &graph, &zoning, &allocator);
    let mut loaded = load_from_sqlite(&path, &allocator.registry).unwrap();
    fs::remove_file(path).unwrap();
    let grid = loaded
        .zoning
        .cells
        .register_frame(GridFrame::new(DVec2::ZERO, DVec2::X, 10.0).unwrap());
    let mut previous_cells = 0;
    for remote_cells in [0, 16_384, 131_072] {
        for index in previous_cells..remote_cells {
            assert!(loaded.zoning.cells.restore_saved_cell(
                CellKey {
                    grid,
                    x: 500 + index % 512,
                    y: 500 + index / 512,
                },
                1
            ));
        }
        previous_cells = remote_cells;
        for owners in [0, 1_000, 10_000] {
            // Build only the arrays read by this validator. Hydration, site derivation and
            // road compilation are separate load costs, excluded from repeated measurements.
            let mut allocator = loaded.allocator.clone();
            let mut fields = Vec::with_capacity(owners);
            for index in 0..owners {
                let mut building = allocator.buildings[0].clone();
                building.parcel_id = 0;
                building.zone_profile_runtime_id = 0;
                building.zone_type = ZoneType::None;
                let mut site = allocator.building_sites[0].clone();
                let offset = Vector2::new(
                    -10_000.0 + (index % 100) as f32 * 40.0,
                    -((index / 100) as f32) * 40.0,
                );
                for point in &mut site.lot_footprint_world {
                    *point += offset;
                }
                fields.push(FieldSite {
                    building_idx: allocator.buildings.len(),
                    resource_id: "grain".to_owned(),
                    polygon_world: site.lot_footprint_world.to_vec(),
                    area_m2: 900.0,
                });
                allocator.buildings.push(building);
                allocator.building_sites.push(site);
            }
            let agriculture = AgricultureSystem::from_sites(fields);
            let run = || {
                super::super::cell_zoning::validate_reservations(
                    black_box(&loaded.zoning),
                    black_box(&allocator),
                    black_box(&agriculture),
                    black_box(&loaded.transit_network.road_surface),
                )
                .unwrap()
            };
            for _ in 0..3 {
                run();
            }
            let mut samples = [0.0_f64; 21];
            for sample in &mut samples {
                let start = Instant::now();
                for _ in 0..8 {
                    run();
                }
                *sample = start.elapsed().as_secs_f64() * 1e6 / 8.0;
            }
            samples.sort_unstable_by(f64::total_cmp);
            println!(
                "remote_cells={remote_cells} fields={owners} explicit_sites={owners} validation_us={:.3} p95_us={:.3}",
                samples[10], samples[19]
            );
        }
    }
}
