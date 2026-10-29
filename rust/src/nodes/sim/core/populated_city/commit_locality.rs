// SPDX-License-Identifier: GPL-2.0-only

//! Commit locality on a populated map (`ROAD-68-G` part 8, `docs/roads.md` "Targets",
//! Locality): the edited neighbourhood is held fixed while remote roads, buildings, parcels and
//! agents grow from 0 to 100k, and the commit's local products must stay bitwise identical while
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
        .unwrap_or_else(|_| vec![0, 1_000, 10_000, 100_000]);
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
        let products = local_products(&mut core, &samples);
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
