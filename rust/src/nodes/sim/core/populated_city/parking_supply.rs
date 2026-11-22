// SPDX-License-Identifier: GPL-3.0-or-later

//! The populated fixture's parking supply (`PARK-02`, F8): kerbside lanes on the cross streets,
//! none on the rows or the trunk, small lots along the rows, and every door with bays to walk to.

use super::super::state::SimCore;
use super::{CROSS_SPACING_M, ROW_BLOCKS_PER_LOT, zoned_city};
use crate::simulation::network::graph::{RoadParking, SidewalkSides};
use crate::simulation::parking::PlaceSource;

#[test]
fn the_fixture_parks_on_its_cross_streets_and_in_lots_along_its_rows() {
    let side_m = 2.0 * CROSS_SPACING_M;
    let mut core = zoned_city(side_m);
    let graph = &core.region_graph;
    let (mut cross, mut rows) = (0, 0);
    for id in (0..graph.edge_count()).filter(|&id| !graph.edge(id).deleted) {
        let edge = graph.edge(id);
        let (start, end) = (graph.node(edge.start_node).pos, graph.node(edge.end_node).pos);
        if (start.x - end.x).abs() <= 1.0 {
            cross += 1;
            assert!(edge.parking.left && edge.parking.right, "cross street {id}");
            assert_eq!(edge.sidewalk_sides(), SidewalkSides::BOTH);
        } else {
            rows += 1;
            assert_eq!(edge.parking, RoadParking::NONE, "row {id}");
        }
    }
    assert!(cross > 0 && rows > 0);
    let kerb: usize = (0..graph.edge_count() as u32)
        .flat_map(|edge| [(edge, false), (edge, true)])
        .map(|(edge, right)| core.parking.source_bays(PlaceSource::Kerb { edge, right }).len())
        .sum();
    assert!(kerb > 0, "no kerbside bay");
    let is_lot = |index: &usize| core.allocator.buildings[*index].asset_id.ends_with("small_lot");
    let lots: Vec<_> = (0..core.allocator.buildings.len()).filter(is_lot).collect();
    for &lot in &lots {
        let handle = core.allocator.building_handles.handle(lot);
        let issues = core.parking.diagnostics.get(&handle.to_bits());
        assert!(issues.is_none(), "{issues:?}");
    }
    let small_lots = lots.len();
    let row_blocks = 2 * (side_m / 50.0 + 1.0).floor() as usize;
    assert_eq!(small_lots, row_blocks.div_ceil(ROW_BLOCKS_PER_LOT));
    let bays = core.parking.counts().0;
    assert!(bays >= kerb + 12 * small_lots, "{bays} bays, {kerb} at the kerb, {small_lots} lots");
    for (index, building) in core.allocator.buildings.iter().enumerate() {
        let handle = core.allocator.building_handles.handle(index);
        assert!(
            !core.parking.candidate_products(handle.to_bits()).is_empty(),
            "{} at ({}, {}) has no bay within its walk",
            building.asset_id,
            building.center_x,
            building.center_y
        );
    }
    // The supply survives a save: the wider cross streets' curb frames among it. So do the
    // rejected lots' reasons, which the load keeps none of: the queries validate the lots whose
    // saved supply shows a rejection (`PARK-17`).
    let reasons = |core: &SimCore| {
        let lots = 0..core.allocator.buildings.len();
        let lots: Vec<_> = lots.map(|i| core.parking_rejections(i).into_owned()).collect();
        (lots, core.parking_rejection_lines())
    };
    let rejected = reasons(&core);
    let path = std::env::temp_dir().join(format!("metrum_supply_{}.sqlite", std::process::id()));
    let path_text = path.to_str().expect("a temporary path");
    core.save_game_settled_at(path_text, None, 0).expect("the fixture saves");
    core.load_game_internal(path_text).expect("the fixture loads");
    let _ = std::fs::remove_file(&path);
    assert_eq!(core.parking.counts().0, bays);
    assert!(core.parking.diagnostics.is_empty());
    assert!(!rejected.1.is_empty());
    assert_eq!(reasons(&core), rejected);
}

// A site edit publishes only the doors whose lists change, each door's searches bounded by the
// changed bays (`PERF-25`): a lot bulldozed, then built again, leaves every door's list what a
// rebuild from scratch gives, bit for bit, in a city wider than a door's walk.
#[test]
fn a_removed_and_a_new_lot_leave_every_list_a_rebuild_gives() {
    let mut core = zoned_city(2.0 * CROSS_SPACING_M);
    let lot = (0..core.allocator.buildings.len())
        .filter(|&i| core.allocator.buildings[i].asset_id.ends_with("small_lot"))
        .min_by(|&a, &b| {
            let from_centre = |i: usize| {
                let building = &core.allocator.buildings[i];
                building.center_x.hypot(building.center_y)
            };
            from_centre(a).total_cmp(&from_centre(b))
        })
        .expect("a lot");
    let building = &core.allocator.buildings[lot];
    let (asset, x, z) = (building.asset_id.clone(), building.center_x, building.center_y);
    let bays = core.parking.counts().0;
    assert!(core.transit_network.cch_current(), "a site edit's own publication");
    assert!(core.bulldoze_building(lot));
    let left = core.parking.counts().0;
    assert!(left < bays);
    core.audit_every_parking_catchment();
    core.place_service_building_internal(&asset, x, z)
        .unwrap_or_else(|error| panic!("lot at ({x}, {z}): {error:?}"));
    core.publish_pending_building_site_changes();
    assert!(core.parking.counts().0 > left, "the new lot has bays");
    core.audit_every_parking_catchment();
}

// A construction site's lot is no supply and its door holds no list until completion
// (`PERF-25`): a lot-bearing building bulldozed and spawned again leaves every door's list what a
// rebuild from scratch gives while it is a site, the site's none, and again once completion
// publishes its bays and its list.
#[test]
fn a_construction_site_and_its_completion_leave_every_list_a_rebuild_gives() {
    use crate::simulation::economy::demand::DemandSpawnAction;
    let mut core = zoned_city(2.0 * CROSS_SPACING_M);
    let lot_bearing = |core: &super::SimCore, i: usize| {
        let building = &core.allocator.buildings[i];
        building.zone_type != crate::simulation::zoning::ZoneType::None
            && core
                .parking
                .building_place_count(core.allocator.building_handles.handle(i).to_bits())
                > 0
    };
    let built = (0..core.allocator.buildings.len())
        .filter(|&i| lot_bearing(&core, i))
        .min_by(|&a, &b| {
            let from_centre = |i: usize| {
                let building = &core.allocator.buildings[i];
                building.center_x.hypot(building.center_y)
            };
            from_centre(a).total_cmp(&from_centre(b))
        })
        .expect("a zoned building with a lot");
    let building = &core.allocator.buildings[built];
    let zone = building.zone_type;
    let action = DemandSpawnAction {
        parcel_id: building.parcel_id,
        asset_id: building.asset_id.clone(),
    };
    assert!(core.bulldoze_building(built));
    let sites = core.allocator.buildings.len();
    core.transit_network.roadbed.compile_dirty(&core.region_graph);
    core.allocator.execute_single_demand_spawn_action(
        zone,
        &action,
        None,
        &mut core.zoning,
        &core.region_graph,
        &core.transit_network.lane_system,
        &core.transit_network.roadbed,
        &core.heightmap,
        core.demand.runtime_catalog(),
        core.demand.runtime_tuning(),
    );
    assert_eq!(core.allocator.buildings.len(), sites + 1, "{action:?} spawns again");
    core.publish_pending_building_site_changes();
    let site = sites;
    assert!(core.allocator.buildings[site].is_under_construction());
    let handle = core.allocator.building_handles.handle(site).to_bits();
    // Its driveways and bay entries keep the kerb clear from the start; its bays wait.
    assert_eq!(core.parking.building_place_count(handle), 0, "the site's bays are supply");
    assert!(core.parking.candidate_products(handle).is_empty(), "the site's door has a list");
    core.audit_every_parking_catchment();
    core.complete_construction_site(site);
    assert!(core.parking.building_place_count(handle) > 0, "completion publishes the lot");
    assert!(!core.parking.candidate_products(handle).is_empty());
    core.audit_every_parking_catchment();
}

// The car products a lot-proof change must keep (`PARK-19`): every place row of the 800 m
// fixture, every rejected lot's reasons as the inspector and the panel show them, and the asset
// editor's rejections of every fixture-pack asset. Recorded at `dev` 2c8b47bdd before the vehicle
// class table; a change that moves any bay or reason moves this digest.
#[test]
fn the_fixtures_car_bays_and_rejections_keep_their_recorded_digest() {
    use crate::simulation::core::stable_hash64;
    let core = zoned_city(2.0 * CROSS_SPACING_M);
    let text_hash = |hash: u64, text: &str| {
        text.bytes().fold(stable_hash64(hash ^ text.len() as u64), |h, b| stable_hash64(h ^ u64::from(b)))
    };
    let lots = core.parking.place_product_hash();
    let reasons = (0..core.allocator.buildings.len())
        .flat_map(|i| core.parking_rejections(i).into_owned())
        .chain(core.parking_rejection_lines())
        .fold(0, |hash, line| text_hash(hash, &line));
    let mut assets: Vec<_> = core.allocator.registry.qualified_ids().collect();
    assets.sort_unstable();
    let editor = assets.iter().fold(0, |hash, id| {
        let manifest = &core.allocator.registry.get(id).expect("a listed asset").manifest;
        let Some(params) = crate::simulation::parking::validation::editor_params(manifest) else {
            return hash;
        };
        let payload = crate::simulation::parking::validation::editor_payload(&params, &[]);
        text_hash(text_hash(hash, id), &payload.to_string())
    });
    assert!(core.parking.counts().0 > 0 && !core.parking_rejection_lines().is_empty());
    // No fixture asset has a freight Driveway (`PARK-19`): no dock, no dock rejection.
    assert!(core.parking.dock_rows().is_empty() && core.parking.dock_diagnostics.is_empty());
    assert!(core.allocator.freight_gates.is_empty());
    let digest = [lots, reasons, editor];
    let recorded = [0x8576_f0fd_bbcc_ed0d, 0x2ba3_ad70_d4ef_a643, 0xbf68_7441_d5fc_8bfa];
    assert_eq!(digest, recorded, "{digest:016x?}");
}

/// Diagnostic, not acceptance timing (`PARK-30`): the bay paint of a prepared city
/// (`METRUM_PERF_RESIDENTS`, `METRUM_PERF_CACHE`), as a load builds it (every chunk), then one
/// lot's republication alone. Prints `LOT_PAINT` with the chunks (one draw call each in range),
/// merged lines and their instances (two triangles each), places and docks painted, and the
/// milliseconds of both builds.
#[test]
#[ignore = "PARK-30 bay paint cost on a prepared city; run alone, release"]
fn lot_paint_cost_on_a_prepared_city() {
    let residents = super::contract::residents();
    let mut city = super::snapshot::load_snapshot(residents, &super::contract::cache_dir());
    let core = &mut city.core;
    core.prepare_site_queries();
    // The merged lines before they are laid on the ground.
    let segments: usize = (0..core.allocator.buildings.len())
        .map(|b| core.allocator.building_handles.handle(b).to_bits())
        .map(|handle| core.parking.paint_lines(handle).0.len())
        .sum();
    // The load marked every lot: what a loaded game builds at its first snapshot.
    let started = std::time::Instant::now();
    core.build_lot_paint();
    let load_ms = started.elapsed().as_secs_f64() * 1000.0;
    let paint = core.parking.paint_publication();
    let chunks = paint.chunks.values().filter(|c| !c.rows.is_empty()).count();
    let lines: usize = paint.chunks.values().map(|c| c.rows.len()).sum::<usize>()
        / crate::simulation::parking::paint::STRIDE;
    let biggest = paint.chunks.values().map(|c| c.rows.len()).max().unwrap_or(0)
        / crate::simulation::parking::paint::STRIDE;
    let (places, docks) = (0..core.allocator.buildings.len())
        .map(|b| core.allocator.building_handles.handle(b).to_bits())
        .fold((0, 0), |(p, d), h| {
            let parking = &core.parking;
            (p + parking.building_place_count(h), d + parking.building_docks(h).count())
        });
    // One lot repainted, as an edit of its site does: its lines, then its chunk's rows copied.
    let lot = (0..core.allocator.buildings.len())
        .map(|b| core.allocator.building_handles.handle(b).to_bits())
        .max_by_key(|&handle| core.parking.building_place_count(handle))
        .expect("a lot");
    let lot_places = core.parking.building_place_count(lot);
    core.parking.repaint_building(lot);
    let started = std::time::Instant::now();
    core.build_lot_paint();
    let edit_ms = started.elapsed().as_secs_f64() * 1000.0;
    println!(
        "LOT_PAINT residents={residents} chunks={chunks} merged_lines={segments} \
         instances={lines} triangles={} \
         biggest_chunk_lines={biggest} places={places} docks={docks} load_ms={load_ms:.2} \
         one_lot_places={lot_places} one_lot_ms={edit_ms:.3}",
        lines * 2
    );
}
