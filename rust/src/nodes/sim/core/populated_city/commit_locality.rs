// SPDX-License-Identifier: GPL-2.0-only

//! Commit locality on a populated map (`ROAD-68-G` part 8, `docs/roads.md` "Targets",
//! Locality): the edited neighbourhood is held fixed while remote roads, buildings, parcels and
//! agents grow from 0 to 30k, and the commit's local products must stay bitwise identical while
//! its stages stay flat.
//!
//! The neighbourhood is the zoned 800 m street grid with every lot built and its trunk running
//! west to the border, in a world of fixed size; the commits are the breakdown's three 120 m
//! branches off that trunk (`routing_benchmark::profile_commits`, which prints each sample's
//! stages and frames). The background lies east of the grid, beyond every lot region and
//! terrain patch the branches touch: straight two-lane block streets, each fronted by a row of
//! one-cell sites with a parcel and one housed agent each. Each tier builds the neighbourhood
//! and its background afresh, outside timing (`LOCALITY_SETUP`).
//!
//! Ids of everything the commits create follow the background's in every store, so the local
//! products are compared without them: parcels, buildings and their attachments by geometry,
//! each attached road by its end points.
//!
//! After the branches, three strokes take built land of the neighbourhood (`ZONE-09`, round RZ2):
//! each a new diagonal street between two row streets, demolishing the buildings on the lots of
//! its block (the block is zoned whole, so no new cell inherits). Every home is housed first: each
//! background resident gets a household and each of the neighbourhood's homes, which the grid
//! builds empty, a resident with one, so the removal evacuates its homes and its agent and
//! household scans meet a background that grows with the tier (`LOCALITY_DEMOLITION`).

use std::time::Instant;

use godot::prelude::{Vector2, Vector3};

use crate::nodes::sim::core::state::SimCore;
use crate::simulation::zoning::ZoneType;

// The world every tier shares, and the neighbourhood: the zoned grid around the origin.
const WORLD_M: f32 = 12_288.0;
const NEIGHBOURHOOD_SIDE_M: f32 = 800.0;
// Everything west of this x is the neighbourhood (the grid, its trunk and the branches).
const LOCAL_MAX_X: f32 = 700.0;
// Background streets are 480 m blocks running east-west, nine to a row from x = 800 m with 24 m
// between blocks, rows 24 m apart from z = -3,000 m; each block is fronted by 40 sites 12 m apart
// on its north side. Block-length streets, as in a city: the attachment repair's test walks
// each building's street (4.8 km streets cost it 15-17 ms at 100k sites, part 8's first trial).
const STREET_WEST_X: f32 = 800.0;
const STREET_FIRST_Z: f32 = -3_000.0;
const STREET_SPACING_M: f32 = 24.0;
const BLOCK_M: f32 = 480.0;
const BLOCK_GAP_M: f32 = 24.0;
const BLOCKS_PER_ROW: usize = 9;
const SITES_PER_STREET: usize = 40;
const SITE_SPACING_M: f32 = 12.0;
const SITE_SETBACK_M: f32 = 10.0;
// The demolishing strokes: from x on the row street `row` to x + `DEMOLITION_RUN_M` on the next,
// clear of the cross streets (x = -400, 0, 400) and of the parking lots at the row blocks'
// middles. Diagonal, so their cells would lie on a frame of their own.
const DEMOLITIONS: [(f32, usize); 3] = [(-100.0, 1), (-300.0, 3), (100.0, 5)];
const DEMOLITION_RUN_M: f32 = 30.0;

#[test]
#[ignore = "release ROAD-68-G commit locality; builds each tier's neighbourhood and background outside timing"]
fn benchmark_commit_locality() {
    super::contract::timing_mode();
    let tiers: Vec<usize> = std::env::var("METRUM_LOCALITY_TIERS")
        .map(|value| {
            value
                .split(',')
                .map(|t| t.trim().parse().unwrap())
                .collect()
        })
        .unwrap_or_else(|_| vec![0, 1_000, 10_000, 30_000]);
    let mut reference = None;
    for background in tiers {
        let started = Instant::now();
        let mut core = super::zoned_city_in_world(NEIGHBOURHOOD_SIDE_M, WORLD_M);
        let neighbourhood_s = started.elapsed().as_secs_f64();
        let started = Instant::now();
        add_background(&mut core, background);
        let background_s = started.elapsed().as_secs_f64();
        eprintln!(
            "LOCALITY_SETUP {}",
            serde_json::json!({"background": background, "neighbourhood_s": neighbourhood_s,
                "background_s": background_s, "buildings": core.allocator.buildings.len(),
                "parcels": core.zoning.parcels().len(), "agents": core.agents.len(),
                "edges": core.region_graph.edge_count()})
        );
        let samples =
            super::routing_benchmark::profile_commits(background as u32, &mut core, false);
        let demolitions = profile_demolitions(&mut core, background);
        let mut products = local_products(&mut core, &samples);
        products["demolitions"] = demolitions;
        eprintln!(
            "LOCALITY_PRODUCTS {}",
            serde_json::json!({"background": background, "products": products})
        );
        match &reference {
            None => reference = Some(products),
            Some(reference) => assert_eq!(
                &products, reference,
                "a remote background of {background} changed the commits' local products"
            ),
        }
    }
}

// Streets and one-site-per-cell lots east of the neighbourhood: `count` sites, parcels and
// housed agents and one street per `SITES_PER_STREET` of them, with the indices and the road and
// terrain state a loaded city has before its first commit.
fn add_background(core: &mut SimCore, count: usize) {
    let template = core
        .allocator
        .buildings
        .iter()
        .find(|building| building.zone_type == ZoneType::Residential)
        .expect("the neighbourhood houses residents")
        .clone();
    core.transit_network.bulk_load = true;
    let mut streets = Vec::new();
    for street in 0..count.div_ceil(SITES_PER_STREET) {
        let z = STREET_FIRST_Z + (street / BLOCKS_PER_ROW) as f32 * STREET_SPACING_M;
        let west = STREET_WEST_X + (street % BLOCKS_PER_ROW) as f32 * (BLOCK_M + BLOCK_GAP_M);
        let edges = core.region_graph.edge_count();
        let outcome = core.add_road_internal(
            vec![
                Vector3::new(west, 0.0, z),
                Vector3::new(west + BLOCK_M, 0.0, z),
            ],
            1,
            1,
        );
        assert!(
            outcome.committed,
            "background street {street}: {}",
            core.last_road_timing
        );
        assert_eq!(core.region_graph.edge_count(), edges + 1);
        // The next street's plan needs a current roadbed.
        core.transit_network
            .roadbed
            .compile_dirty(&core.region_graph);
        streets.push((edges, west, z));
    }
    {
        let core = &mut *core;
        core.transit_network
            .finalize_bulk_load(&mut core.region_graph, &mut core.allocator);
    }
    let half = core.config.zone_cell_m * 0.5;
    for index in 0..count {
        let (edge, west, z) = streets[index / SITES_PER_STREET];
        let x = west + ((index % SITES_PER_STREET) as f32 + 0.5) * SITE_SPACING_M;
        let center = Vector2::new(x, z + SITE_SETBACK_M);
        let mut building = template.clone();
        building.center_x = center.x;
        building.center_y = center.y;
        building.width_cells = 1;
        building.depth_cells = 1;
        building.facing_dir = Vector2::new(0.0, -1.0);
        building.edge_idx = edge;
        building.frontage_t = (x - west) / BLOCK_M;
        building.occupancy = 1;
        building.parcel_id = core
            .zoning
            .parcels
            .insert_new(
                crate::simulation::zoning::ParcelGeometry {
                    edge_idx: edge,
                    side: 1,
                    frontage_center_t: building.frontage_t,
                    frontage_m: half * 2.0,
                    depth_m: half * 2.0,
                    front_center: center - Vector2::new(0.0, half),
                    center,
                    tangent: Vector2::RIGHT,
                    normal: Vector2::DOWN,
                    corners: [
                        center + Vector2::new(-half, -half),
                        center + Vector2::new(half, -half),
                        center + Vector2::new(half, half),
                        center + Vector2::new(-half, half),
                    ],
                    aabb_min: center - Vector2::new(half, half),
                    aabb_max: center + Vector2::new(half, half),
                },
                building.zone_profile_runtime_id,
            )
            .raw();
        let id = core.allocator.buildings.len();
        assert!(core.zoning.occupy_parcel(building.parcel_id, id));
        core.agents.spawn_housed_agent(id, center.x, center.y);
        core.allocator.buildings.push(building);
    }
    // A loaded city's lot queue is empty: the streets' bulk adds queued their lot regions,
    // which hold no paint.
    core.prepare_cell_lots_internal();
    core.allocator.dirty_index = true;
    core.allocator
        .rebuild_building_site_clients(core.config.zone_cell_m);
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    // As the street grid leaves a bulk build: routing installed, lanes on the surface and the
    // graded terrain reset once, a world-scale road change (`ROAD-68-D` part 6).
    core.transit_network.settle_cch();
    core.fit_lanes_to_road_surface();
    core.subsume_world_scale_road_changes();
    core.bump_global_terrain_payload_generation();
}

// The demolishing strokes (`DEMOLITIONS`), committed and adopted without a frame between, so the
// background's households never act: per stroke its stages, the removal's scans timed alone on
// the same city and its take. Returns the takes, the products every tier must equal.
fn profile_demolitions(core: &mut SimCore, background: usize) -> serde_json::Value {
    use crate::nodes::sim::benchmark::road_edit::RoadEditMetrics;
    use crate::nodes::sim::core::RoadCommitRequest;
    house_residents(core);
    let half_z = (NEIGHBOURHOOD_SIDE_M / super::ROW_SPACING_M).floor() * super::ROW_SPACING_M * 0.5;
    let median_ms = |run: &mut dyn FnMut()| {
        let mut samples: Vec<f64> = (0..9)
            .map(|_| {
                let start = Instant::now();
                run();
                start.elapsed().as_secs_f64() * 1e3
            })
            .collect();
        samples.sort_by(f64::total_cmp);
        samples[4]
    };
    let mut takes = Vec::new();
    for (stroke, &(x, row)) in DEMOLITIONS.iter().enumerate() {
        let z = row as f32 * super::ROW_SPACING_M - half_z;
        let points = [(x, z), (x + DEMOLITION_RUN_M, z + super::ROW_SPACING_M)]
            .map(|(x, z)| Vector3::new(x, core.heightmap.sample_height_world(x, z), z));
        // The removal's scans alone, on the city as the commit finds it.
        let last = core.allocator.buildings.len() - 1;
        let agent_scan_ms = median_ms(&mut || {
            std::hint::black_box(core.agents.agents_referencing_any_building(&[0, last]));
        });
        let household_scan_ms = median_ms(&mut || {
            std::hint::black_box(core.households.households_referencing_buildings(0, last));
        });
        let mut allocator = core.allocator.clone();
        let zone_index_ms = median_ms(&mut || allocator.rebuild_zone_index());
        drop(allocator);
        let (buildings, households, agents) = (
            core.allocator.buildings.len(),
            core.households.households.len(),
            core.agents.len(),
        );
        let terrain = core.terrain_dirty_patch_states();
        core.acknowledge_terrain_render_patches(&terrain);
        let start = Instant::now();
        let outcome = core.commit_road(
            RoadCommitRequest {
                points: points.to_vec(),
                fwd_lanes: 1,
                bkw_lanes: 1,
                snap_to_existing_roads: true,
                tunnel: false,
                median: None,
                parking: crate::simulation::network::graph::RoadParking::NONE,
                attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
                preview: None,
            },
            start,
            RoadEditMetrics::default(),
        );
        let commit_ms = start.elapsed().as_secs_f64() * 1e3;
        assert!(
            outcome.committed,
            "demolition {stroke} rejected: {}",
            outcome.rejection_reason
        );
        assert!(core.acknowledge_network_render_generation(core.cached_road_mesh_generation));
        let start = Instant::now();
        let lots = core.adopt_pending_cell_lots();
        let adoption_ms = start.elapsed().as_secs_f64() * 1e3;
        let demolished = buildings - core.allocator.buildings.len();
        let (erased, inherited) = core
            .undo_stack
            .back()
            .and_then(|record| record.cells.as_ref())
            .map_or((0, 0), |cells| (cells.erased().len(), cells.inherited().len()));
        let m = &core.last_road_edit_metrics;
        eprintln!(
            "LOCALITY_DEMOLITION {}",
            serde_json::json!({"background": background, "stroke": stroke,
                "buildings": buildings, "households": households, "agents": agents,
                "demolished": demolished, "erased_cells": erased, "inherited_cells": inherited,
                "commit_ms": commit_ms, "core_work_ms": m.core_work_ms,
                "surface_ms": m.surface_ms, "surface_overlap_ms": m.surface_overlap_ms,
                "surface_land_ms": m.surface_land_ms, "buildings_ms": m.buildings_ms,
                "lots_spawn_ms": m.lots_spawn_ms, "adoption_ms": adoption_ms,
                "lots_install_ms": lots.install_ms,
                "agent_scan_ms": agent_scan_ms, "household_scan_ms": household_scan_ms,
                "zone_index_rebuild_ms": zone_index_ms})
        );
        // The block is zoned whole, so its painted cells keep the new street's frame off the
        // land its erased cells leave (rule 5: new cells fill free land only); its plan still
        // hands their paint on, to no cell.
        assert!(demolished >= 2, "demolition {stroke} takes built lots: {demolished}");
        takes.push(serde_json::json!({"demolished": demolished, "erased_cells": erased,
            "inherited_cells": inherited}));
    }
    serde_json::Value::Array(takes)
}

// A household of one in every home, admitted as an immigrant household, outside timing: each
// background resident gets one, and each empty home of the neighbourhood a resident with one.
fn house_residents(core: &mut SimCore) {
    use crate::simulation::economy::agents::household_age_composition;
    let catalog = crate::simulation::economy::definitions::load_runtime_economy_catalog().unwrap();
    let tuning = crate::simulation::economy::definitions::load_runtime_economy_tuning().unwrap();
    for home in 0..core.allocator.buildings.len() {
        let building = &core.allocator.buildings[home];
        if building.center_x >= STREET_WEST_X
            || building.zone_type != ZoneType::Residential
            || building.occupancy != 0
            || core.allocator.household_capacity(home) == 0
        {
            continue;
        }
        let (x, z) = (building.center_x, building.center_y);
        let composition = household_age_composition(home, core.households.households.len(), 1);
        let household =
            core.households
                .admit_immigrant_household(&catalog, &tuning, home, 1, composition);
        core.allocator.claim_vacancy(home);
        let agent = core.agents.spawn_housed_agent(home, x, z);
        core.agents.assign_household_id(agent, household);
    }
    for agent in 0..core.agents.len() {
        let home = core.agents.agents.home_building[agent];
        if core.agents.agents.household_id[agent] != usize::MAX
            || core
                .allocator
                .buildings
                .get(home)
                .is_none_or(|building| building.center_x < STREET_WEST_X)
        {
            continue;
        }
        let composition = household_age_composition(home, core.households.households.len(), 1);
        let household =
            core.households
                .admit_immigrant_household(&catalog, &tuning, home, 1, composition);
        core.agents.assign_household_id(agent, household);
    }
}

// The commits' products in the neighbourhood, without ids: each sample's published chunks and
// graded tiles (`profile_commits`), then the neighbourhood's cell claims and generated cells
// with grids, lots and roads named by geometry, and its parcels and buildings by geometry,
// sorted.
fn local_products(core: &mut SimCore, samples: &serde_json::Value) -> serde_json::Value {
    let hash =
        |value: &dyn std::fmt::Debug| format!("{:016x}", crate::debug::work::debug_hash(value));
    let graph = &core.region_graph;
    // An attached road by the bits of its two end points.
    let road = |edge: usize| {
        let geometry = &graph.edge(edge).geometry;
        [geometry[0], geometry[geometry.len() - 1]].map(|p| [p.x, p.y, p.z].map(f32::to_bits))
    };
    let bits = |v: Vector2| [v.x.to_bits(), v.y.to_bits()];
    let mut parcels: Vec<_> = core
        .zoning
        .parcels()
        .iter()
        .filter(|parcel| parcel.center().x < LOCAL_MAX_X)
        .map(|parcel| {
            (
                [
                    bits(parcel.center()),
                    bits(parcel.front_center()),
                    bits(parcel.tangent()),
                    bits(parcel.normal()),
                ],
                parcel.frontage_m().to_bits(),
                parcel.depth_m().to_bits(),
                parcel.frontage_center_t().to_bits(),
                parcel.side(),
                parcel.zone_profile_runtime_id(),
                parcel.occupied_building().is_some(),
                parcel.build_generation(),
                format!("{:?}", parcel.cell_lot()),
                road(parcel.edge_idx()),
            )
        })
        .collect();
    parcels.sort();
    let mut buildings: Vec<_> = core
        .allocator
        .buildings
        .iter()
        .filter(|building| building.center_x < LOCAL_MAX_X)
        .map(|building| {
            (
                [building.center_x, building.center_y].map(f32::to_bits),
                bits(building.facing_dir),
                building.frontage_t.to_bits(),
                building.side,
                building.width_cells,
                building.depth_cells,
                building.zone_profile_runtime_id,
                building.asset_id.clone(),
                road(building.edge_idx),
            )
        })
        .collect();
    buildings.sort();
    // Lots by their parcel's centre, frontage roads by their end points.
    let (claims, generated) = core.zoning.cells.digests_by(
        |center| center.x < f64::from(LOCAL_MAX_X),
        |lot| {
            core.zoning
                .parcel_by_raw_id(lot)
                .map(|parcel| bits(parcel.center()))
        },
        road,
    );
    let per_sample: Vec<_> = samples["samples"]
        .as_array()
        .map(|samples| {
            samples
                .iter()
                .map(|sample| {
                    serde_json::json!({
                        "published_chunks": sample["published_chunks"],
                        "graded_digest": sample["graded_digest"],
                        "graded_tile_keys": sample["graded_tile_keys"],
                        "graded_changed_pieces": sample["graded_changed_pieces"],
                        "graded_changed_segments": sample["graded_changed_segments"],
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    serde_json::json!({
        "samples": per_sample,
        "cell_claims": format!("{claims:016x}"), "generated_cells": format!("{generated:016x}"),
        "parcels": parcels.len(), "parcels_digest": hash(&parcels),
        "buildings": buildings.len(), "buildings_digest": hash(&buildings),
    })
}

#[test]
#[ignore = "PARK-01 matched release locality; setup excluded, run alone with eight workers"]
fn benchmark_parking_locality() {
    use crate::simulation::parking::{PlaceSource, lots::lot_bays};
    use crate::nodes::sim::benchmark::road_edit::RoadEditMetrics;
    use crate::nodes::sim::core::RoadCommitRequest;
    use std::collections::BTreeSet;
    super::contract::timing_mode();
    let tiers: Vec<usize> = std::env::var("METRUM_LOCALITY_TIERS")
        .unwrap_or_else(|_| "0,1000,10000".to_owned())
        .split(',')
        .map(|s| s.parse().unwrap())
        .collect();
    let mut reference = None;
    let mut candidate_reference = None;
    for background in tiers {
        let start = Instant::now();
        let mut core = super::zoned_city_in_world(NEIGHBOURHOOD_SIDE_M, WORLD_M);
        add_background(&mut core, background);
        let id = super::register_small_lot(&mut core);
        let edge = core.region_graph.edge_count();
        assert!(
            core.commit_road(
                RoadCommitRequest {
                    points: vec![
                        Vector3::new(-300.0, 0.0, -700.0),
                        Vector3::new(300.0, 0.0, -700.0),
                    ],
                    fwd_lanes: 1,
                    bkw_lanes: 1,
                    snap_to_existing_roads: true,
                    preview: None,
                    tunnel: false,
                    median: None,
                    parking: crate::simulation::network::graph::RoadParking::NONE,
                    attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
                },
                Instant::now(),
                RoadEditMetrics::default(),
            ).committed
        );
        // Explicit placement validates the newly committed frontage's compiled surface.
        core.precompute_road_mesh_data();
        core.publish_road_tool_view();
        let mut owners = Vec::new();
        for n in 0..12 {
            let index = core.allocator.buildings.len();
            core.place_service_building_internal(&id, -240.0 + n as f32 * 40.0, -730.0)
                .unwrap();
            owners.push(core.allocator.building_handles.handle(index));
        }
        core.precompute_road_mesh_data();
        core.publish_road_tool_view();
        core.publish_pending_building_site_changes();
        for owner in &owners {
            assert!(
                !core.parking.diagnostics.contains_key(&owner.to_bits()),
                "lot diagnostics: {:?}",
                core.parking.diagnostics.get(&owner.to_bits())
            );
        }
        core.transit_network.settle_cch();
        // The fixture publishes compiled road geometry without running a simulation frame.
        // Publish its final entrance/catchment inputs before taking the timed local control.
        core.rebuild_building_entrances_internal();
        let indices: Vec<_> = owners.iter().map(|&owner| core.allocator.building_handles.resolve(owner).unwrap()).collect();
        core.publish_parking_catchments(&indices);
        core.complete_parking_economy_publication();
        let setup_s = start.elapsed().as_secs_f64();
        let products = |core: &SimCore| {
            owners
                .iter()
                .map(|owner| {
                    (2..8)
                        .flat_map(|anchor| {
                            core.parking.source_bays(PlaceSource::Building {
                                building: owner.to_bits(),
                                anchor,
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        let expected = products(&core);
        let geometry: Vec<_> = expected
            .iter()
            .map(|bays| bays.iter().map(|(_, bay)| *bay).collect::<Vec<_>>())
            .collect();
        assert!(geometry.iter().all(|bays| bays.len() == 6));
        if let Some(reference) = &reference {
            assert_eq!(reference, &geometry);
        } else {
            reference = Some(geometry);
        }
        let candidate_products = |core: &SimCore| owners.iter().map(|owner|
            core.parking.candidate_products(owner.to_bits())
        ).collect::<Vec<_>>();
        let expected_candidates = candidate_products(&core);
        assert!(expected_candidates.iter().all(|c| !c.is_empty()),
            "local destination entrances: {:?}", indices.iter().map(|&i| &core.allocator.entrances[i]).collect::<Vec<_>>());
        if let Some(reference) = &candidate_reference {
            assert_eq!(reference, &expected_candidates);
        } else {
            candidate_reference = Some(expected_candidates.clone());
        }
        let mut candidate_samples = Vec::new();
        let mut road_samples = Vec::new();
        let mut site_samples = Vec::new();
        let mut lot_samples = Vec::new();
        for _ in 0..9 {
            let start = Instant::now();
            core.publish_parking_catchments(&indices);
            candidate_samples.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(candidate_products(&core), expected_candidates);
            let start = Instant::now();
            core.reconcile_parking_frontages(&BTreeSet::from([edge]));
            road_samples.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(products(&core), expected);
            core.allocator
                .parking_site_changes
                .extend_from_slice(&owners);
            let start = Instant::now();
            core.publish_parking_sites();
            site_samples.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(products(&core), expected);
            for &owner in &owners {
                let index = core.allocator.building_handles.resolve(owner).unwrap();
                let start = Instant::now();
                let result = lot_bays(
                    &core.allocator,
                    index,
                    &core.region_graph,
                    &core.transit_network.roadbed,
                );
                lot_samples.push(start.elapsed().as_secs_f64() * 1000.0);
                assert_eq!(result.0.len(), 12);
                assert!(result.1.is_empty());
            }
        }
        candidate_samples.sort_by(f64::total_cmp);
        road_samples.sort_by(f64::total_cmp);
        site_samples.sort_by(f64::total_cmp);
        lot_samples.sort_by(f64::total_cmp);
        eprintln!(
            "PARKING_LOCALITY {}",
            serde_json::json!({
                "background":background, "buildings":core.allocator.buildings.len(),
                "parcels":core.zoning.parcels().len(), "agents":core.agents.len(),
                "roads":core.region_graph.edge_count(), "lots":owners.len(), "places":72,
                "candidate_rebuild_ms":candidate_samples[4], "candidate_entries":expected_candidates.iter().map(Vec::len).sum::<usize>(),
                "setup_s":setup_s, "road_rebuild_ms":road_samples[4], "site_publication_ms":site_samples[4],
                "per_lot_ms":lot_samples[lot_samples.len()/2], "max_lot_ms":lot_samples.last(), "products":"identical"
            })
        );
    }
}
