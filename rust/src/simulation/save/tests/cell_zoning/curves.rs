// SPDX-License-Identifier: GPL-2.0-only

//! Curved cell guide persistence and validation across mutable road graph subdivision.

use super::*;
use crate::simulation::agriculture::PolygonFootprint;
use crate::simulation::network::topology;
use crate::simulation::zoning::cells::CellLotSize;
use crate::simulation::zoning::cells::road_contact_interior;

fn curved_fixture() -> (RegionGraph, ZoningSystem, CellBounds) {
    let config = WorldConfig::new(600.0, 600.0, 10.0, 10.0);
    let points: Vec<_> = (0..=24)
        .map(|i| {
            let x = i as f32 * 10.0 - 120.0;
            Vector3::new(x, 0.0, x * x * 0.001)
        })
        .collect();
    let mut graph = RegionGraph::new();
    let start_node = graph.add_node(points[0], NodeType::Junction);
    let end_node = graph.add_node(points[24], NodeType::Junction);
    let physical_length = points.windows(2).map(|p| p[0].distance_to(p[1])).sum();
    graph.add_edge(Edge {
        start_node,
        end_node,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: DEFAULT_URBAN_ROAD_SPEED_MS,
        physical_length,
        geometry: points.clone(),
        physical_geometry: points,
        ..Default::default()
    });
    let bounds = CellBounds {
        min: DVec2::splat(-300.0),
        max: DVec2::splat(300.0),
    };
    let mut zoning = ZoningSystem::new(&config);
    zoning.generate_cells(&graph, bounds, |_| false);
    let point = graph.edge(0).physical_geometry[6].lerp(graph.edge(0).physical_geometry[7], 0.3);
    let junction = graph.add_node(point, NodeType::Junction);
    topology::split_edge(
        &mut TransitNetwork::new(),
        &mut graph,
        0,
        6,
        0.3,
        junction,
        &mut zoning,
        &mut BuildingAllocator::new(),
    );
    let mut network = TransitNetwork::new_for_world(&config);
    crate::simulation::save::network::rebuild_loaded_graph_runtime(
        &mut graph,
        &mut network,
        &mut TerrainSystem::from_world_config(&config),
    )
    .unwrap();
    let blocked = |corners: &[DVec2; 4]| {
        PolygonFootprint::from_precise_points(
            road_contact_interior(corners)
                .into_iter()
                .map(|p| [p.x, p.y]),
        )
        .overlaps_roads(&network.road_surface)
    };
    // Retain the unsplit grids only where the final compiled junction permits paint.
    // Runtime generation supplies this same exclusion predicate.
    let mut cells = Vec::new();
    zoning.cells.visit_in_bounds(bounds, |key| {
        if !blocked(&zoning.cells.frame(key.grid).unwrap().corners(key.x, key.y)) {
            cells.push(key);
        }
    });
    cells.sort_unstable();
    let selected = CellSelection {
        revision: zoning.cells.revision(),
        cells,
    };
    zoning.paint_cells(&selected, 1, blocked).unwrap();
    zoning.generate_cells(&graph, bounds, blocked);
    let shape = CellLotSize {
        profile: 1,
        width: 2,
        depth: 2,
    };
    assert!(
        !zoning
            .derive_cell_lots(&graph, bounds, &[shape], |corners, _| blocked(corners))
            .created
            .is_empty()
    );
    (graph, zoning, bounds)
}

#[test]
fn split_curved_grid_and_lots_survive_sqlite_round_trip() {
    let (graph, zoning, bounds) = curved_fixture();
    let path = temp_path("curved_cells_round_trip");
    let allocator = BuildingAllocator::new();
    write_snapshot(&path, &graph, &zoning, &allocator);
    let mut loaded = load_from_sqlite(&path, &allocator.registry).unwrap();
    fs::remove_file(path).unwrap();
    assert_eq!(
        loaded.zoning.cells.saved_cells(),
        zoning.cells.saved_cells()
    );
    assert_eq!(
        loaded.zoning.cells.saved_curve_sources().len(),
        zoning.cells.saved_curve_sources().len()
    );
    assert_eq!(loaded.zoning.parcels().len(), zoning.parcels().len());
    loaded
        .zoning
        .generate_cells(&loaded.graph, bounds, |_| false);
    let shape = CellLotSize {
        profile: 1,
        width: 2,
        depth: 2,
    };
    let report = loaded
        .zoning
        .derive_cell_lots(&loaded.graph, bounds, &[shape], |_, _| false);
    assert!(report.retired.is_empty() && report.created.is_empty());
    for before in zoning.parcels() {
        let after = loaded.zoning.parcel_by_raw_id(before.id().raw()).unwrap();
        assert_eq!(before.corners(), after.corners());
        assert_eq!(before.cell_lot(), after.cell_lot());
    }
}

#[test]
fn malformed_or_duplicate_curve_sources_are_rejected() {
    let (graph, zoning, _) = curved_fixture();
    let allocator = BuildingAllocator::new();
    for mutation in ["duplicate", "point", "width", "rectangle"] {
        let path = temp_path("invalid_curve_source");
        write_snapshot(&path, &graph, &zoning, &allocator);
        let conn = Connection::open(&path).unwrap();
        let (id, json): (i64, String) = conn.query_row(
            "SELECT source_id, source_json FROM zoning_cell_curve_sources ORDER BY source_id LIMIT 1",
            [], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
        if mutation == "duplicate" {
            conn.execute(
                "INSERT INTO zoning_cell_curve_sources VALUES (99999, ?1)",
                [&json],
            )
            .unwrap();
        } else {
            let mut source: serde_json::Value = serde_json::from_str(&json).unwrap();
            match mutation {
                "point" => {
                    source["points"][0][0] = serde_json::json!(900.0_f64.to_bits());
                }
                "width" => {
                    source["half_width"] = serde_json::json!(900.0_f64.to_bits());
                }
                _ => {
                    source["key"]["max"][0] = serde_json::json!(i32::MAX);
                }
            }
            conn.execute(
                "UPDATE zoning_cell_curve_sources SET source_json = ?1 WHERE source_id = ?2",
                params![source.to_string(), id],
            )
            .unwrap();
        }
        drop(conn);
        assert!(
            load_from_sqlite(&path, &allocator.registry).is_err(),
            "accepted {mutation}"
        );
        fs::remove_file(path).unwrap();
    }
}

#[test]
fn version_65_cell_save_loads_without_curve_source_table() {
    let (graph, zoning, allocator) = fixture(0.0, false);
    let path = temp_path("pre_curve_source");
    write_snapshot(&path, &graph, &zoning, &allocator);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("UPDATE save_meta SET version = 65; DROP TABLE zoning_cell_curve_sources;")
        .unwrap();
    drop(conn);
    let loaded = load_from_sqlite(&path, &allocator.registry).unwrap();
    fs::remove_file(path).unwrap();
    assert_eq!(
        loaded.zoning.cells.saved_cells(),
        zoning.cells.saved_cells()
    );
    assert_eq!(loaded.zoning.parcels().len(), zoning.parcels().len());
}
