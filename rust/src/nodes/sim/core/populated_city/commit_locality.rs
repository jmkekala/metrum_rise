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
//!
//! Then the four in-place edits take built land of the neighbourhood (`ZONE-10`, round RZ10a):
//! a row street widened from 1+1 to 2+2 (its front rows), a 40 m right pocket at a cross-street
//! junction, a one-lane ring replacing a crossing (its island's lots too) and a T's junction
//! moved 6 m into a block, each with its publication, as the Road Properties window, the Junction
//! window, the roundabout tool and the move tool's release commit them (`LOCALITY_IN_PLACE`). The
//! narrowing back and the pocket's removal follow, the same transactions taking no land: the
//! reference for what the take adds at each tier.
//!
//! Last, the plotted placements take built land of the grid's south edge (`ZONE-11`, round
//! RZ11a): a parking lot over the houses of the south row's south curb, a farm and a mine over
//! others, the farm's 4 ha field behind the rows (taking nothing) then resized over their rear
//! row, and the mine's area over the rear row, after a parking lot on unzoned land beside the
//! trunk, the same placement taking nothing (`LOCALITY_PLACEMENTS`): each one's wall time, the
//! take of its footprint timed alone on the city as the placement finds it, the explicit sites
//! that take's query visits, and what it demolished and erased.

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
        let in_place = profile_in_place_edits(&mut core, background);
        let placements = profile_placements(&mut core, background);
        let mut products = local_products(&mut core, &samples);
        products["demolitions"] = demolitions;
        products["in_place_edits"] = in_place;
        products["placements"] = placements;
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

// The in-place edits of `ZONE-10` on the neighbourhood, after the demolitions: each committed
// through its own transaction and published, with the edit's and the publication's wall time,
// the buildings it demolished and the paint its record erased and its new cells inherited.
// Clear of the demolitions' rows, the trunk and the row blocks' parking lots. Returns the takes,
// the products every tier must equal.
fn profile_in_place_edits(core: &mut SimCore, background: usize) -> serde_json::Value {
    use crate::simulation::network::graph::RoadParking;
    use crate::simulation::network::pockets::PocketRequest;
    use crate::simulation::network::road_types::RoadAttributes;
    use crate::simulation::network::roadbed::EdgeEnd;
    use crate::simulation::network::roundabouts::{
        RoundaboutRequest, RoundaboutSite, default_diameter_m,
    };
    let node_at = |core: &SimCore, x: f32, z: f32| {
        let y = core.heightmap.sample_height_world(x, z);
        core.region_graph
            .find_node_within(Vector3::new(x, y, z), 1.0)
            .unwrap_or_else(|| panic!("a junction at ({x}, {z})"))
    };
    // The live edge between two nodes and its end at `to`.
    let edge_between = |core: &SimCore, from: u32, to: u32| {
        let graph = &core.region_graph;
        graph
            .node_adjacency(to)
            .iter()
            .copied()
            .filter(|&edge| !graph.edge(edge).deleted)
            .find_map(|edge| {
                let record = graph.edge(edge);
                let (start, end) = (
                    graph.get_valid_node(record.start_node),
                    graph.get_valid_node(record.end_node),
                );
                (start == from && end == to)
                    .then_some((edge, EdgeEnd::End))
                    .or((start == to && end == from).then_some((edge, EdgeEnd::Start)))
            })
            .expect("a street between the nodes")
    };
    // Row 11's west block; row 13's west block into the crossing at x = 0; row 15's west T, whose
    // move bends no street onto a parking lot.
    let (row, _) = edge_between(core, node_at(core, -400.0, 150.0), node_at(core, 0.0, 150.0));
    let (arm, end) = edge_between(core, node_at(core, -400.0, 250.0), node_at(core, 0.0, 250.0));
    let tee = node_at(core, -400.0, 350.0);
    let mut takes = Vec::new();
    // Each taking edit, and the narrowing and the pocket's removal, the same transactions taking
    // no land: the reference for what the take adds. (A node moved back takes the cells beside
    // its street: the round trip leaves the street millimetres off its line.)
    for name in [
        "widening", "narrowing", "pocket", "pocket_off", "ring", "move",
    ] {
        let (buildings, households, agents) = (
            core.allocator.buildings.len(),
            core.households.households.len(),
            core.agents.len(),
        );
        let depth = core.undo_stack.len();
        let terrain = core.terrain_dirty_patch_states();
        core.acknowledge_terrain_render_patches(&terrain);
        let lanes = |core: &SimCore, lanes| {
            let attributes = RoadAttributes::of_edge(core.region_graph.edge(row));
            (row as i32, lanes, lanes, None, RoadParking::NONE, attributes)
        };
        let pocket = |on: bool| PocketRequest {
            edge: arm,
            end,
            left: false,
            right: on,
            body_m: 40.0,
        };
        let start = Instant::now();
        let applied = match name {
            "widening" | "narrowing" => {
                let (edge, fwd, bkw, median, parking, attributes) =
                    lanes(core, if name == "widening" { 2 } else { 1 });
                core.set_edge_cross_section_internal(edge, fwd, bkw, median, parking, attributes)
            }
            "pocket" | "pocket_off" => core.set_road_pocket_internal(pocket(name == "pocket")),
            "ring" => {
                let site = RoundaboutSite::Junction(node_at(core, 0.0, 350.0));
                core.place_roundabout_internal(RoundaboutRequest {
                    site,
                    diameter_m: default_diameter_m(site.lanes(&core.region_graph)),
                })
            }
            // Into the block to the T's south-east.
            _ => {
                let to = Vector3::new(-394.0, core.heightmap.sample_height_world(-394.0, 344.0), 344.0);
                core.move_network_node_taking_land_internal(tee as i32, to)
            }
        };
        let edit_ms = start.elapsed().as_secs_f64() * 1e3;
        assert!(
            applied,
            "the {name} is refused: {:?}",
            core.last_road_rejection.take()
        );
        let start = Instant::now();
        core.finalize_network_render_products();
        let publish_ms = start.elapsed().as_secs_f64() * 1e3;
        assert!(core.acknowledge_network_render_generation(core.cached_road_mesh_generation));
        assert_eq!(core.undo_stack.len(), depth + 1, "the {name} has one undo record");
        let demolished = buildings - core.allocator.buildings.len();
        let (erased, inherited) = core
            .undo_stack
            .back()
            .and_then(|record| record.cells.as_ref())
            .map_or((0, 0), |cells| (cells.erased().len(), cells.inherited().len()));
        eprintln!(
            "LOCALITY_IN_PLACE {}",
            serde_json::json!({"background": background, "edit": name,
                "buildings": buildings, "households": households, "agents": agents,
                "demolished": demolished, "erased_cells": erased, "inherited_cells": inherited,
                "edit_ms": edit_ms, "publish_ms": publish_ms})
        );
        if ["narrowing", "pocket_off"].contains(&name) {
            assert_eq!((demolished, erased), (0, 0), "the {name} takes nothing");
        } else {
            assert!(demolished >= 1, "the {name} takes built lots: {demolished}");
        }
        takes.push(serde_json::json!({"edit": name, "demolished": demolished,
            "erased_cells": erased, "inherited_cells": inherited}));
    }
    serde_json::Value::Array(takes)
}

// The plotted placements of `ZONE-11` on the grid's south row (z = -400), south curb, which no
// earlier edit touches and no parking lot fronts: a parking lot at x -100, a farm at x -300 and a
// mine at x 300, each over built lots; the farm's field behind the rows west of the lot, then
// resized over their rear row; the mine's area over the rear row west of it. First a parking lot
// on unzoned land beside the trunk west of the grid: the reference taking nothing. Each one is
// committed as its tool commits it; the take of its footprint is timed alone before (median of 9,
// on the city as the placement finds it, its site index prepared and that rebuild timed apart),
// with the explicit sites its query visits. Returns the
// takes and the field and area polygons, the products every tier must equal.
fn profile_placements(core: &mut SimCore, background: usize) -> serde_json::Value {
    use crate::nodes::sim::core::{CoreLand, RoadLandTake};
    use crate::simulation::agriculture::PolygonFootprint;
    const ROW_Z: f32 = -400.0;
    const REQUEST_Z: f32 = ROW_Z - 15.0;
    // Beside the trunk west of the grid, on unzoned land: the placement taking nothing, the
    // reference for what the take adds at each tier.
    const FREE_Z: f32 = 15.0;
    const FARM: &str = "kenney:building.industrial.small_farm";
    const MINE: &str = "kenney:building.coal.mine";
    // The grid's parking lot (`add_parking_lots`).
    let lot_asset = "parking:building.parking.small_lot";
    let rect = |x0: f32, x1: f32, z0: f32, z1: f32| {
        vec![
            Vector2::new(x0, z0),
            Vector2::new(x1, z0),
            Vector2::new(x1, z1),
            Vector2::new(x0, z1),
        ]
    };
    let mut takes = Vec::new();
    // The farm and mine by handle: a later placement's demolition may move their indices.
    let mut farm = None;
    let mut mine = None;
    let resolve = |core: &SimCore, handle: Option<_>| {
        core.allocator
            .building_handles
            .resolve(handle.expect("the owner stands"))
            .expect("an owner is never taken")
    };
    for name in [
        "service_free", "service", "farm", "field", "field_resize", "mine", "area",
    ] {
        let (buildings, households, agents) = (
            core.allocator.buildings.len(),
            core.households.households.len(),
            core.agents.len(),
        );
        let painted = core.zoning.cells.saved_cells().len();
        // The footprint the placement covers: a building's lot as its preview poses it, a
        // field's or area's polygon.
        let preview_lot = |core: &SimCore, asset: &str, x: f32| {
            let preview = if asset == lot_asset {
                core.get_service_building_placement_preview_internal(asset, x, REQUEST_Z)
            } else {
                core.get_industry_building_placement_preview_internal(asset, x, REQUEST_Z)
            };
            preview.unwrap_or_else(|error| panic!("{asset} at x {x}: {error:?}")).corners.to_vec()
        };
        let preview_lot_at = |core: &SimCore, x: f32, z: f32| {
            core.get_service_building_placement_preview_internal(lot_asset, x, z)
                .unwrap_or_else(|error| panic!("the lot at ({x}, {z}): {error:?}"))
                .corners
                .to_vec()
        };
        let lot_bounds = |core: &SimCore, owner: usize| {
            let lot = core.allocator.building_sites[owner].lot_footprint_world;
            lot.iter().fold(
                (f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY),
                |(x0, x1, z0), p| (x0.min(p.x), x1.max(p.x), z0.min(p.y)),
            )
        };
        let (footprint, owner) = match name {
            "service_free" => (preview_lot_at(core, -600.0, FREE_Z), None),
            "service" => (preview_lot(core, lot_asset, -100.0), None),
            "farm" => (preview_lot(core, FARM, -300.0), None),
            "mine" => (preview_lot(core, MINE, 300.0), None),
            "field" | "field_resize" => {
                let owner = resolve(core, farm);
                let (_, x1, _) = lot_bounds(core, owner);
                let top = if name == "field" { -430.0 } else { -420.0 };
                (rect(x1 + 1.0, -130.0, -700.0, top), Some(owner))
            }
            _ => {
                let owner = resolve(core, mine);
                let (x0, _, _) = lot_bounds(core, owner);
                (rect(120.0, x0 - 1.0, -560.0, -420.0), Some(owner))
            }
        };
        // The site index as the placement's first step prepares it (stale after the last
        // placement's new site), timed apart: an O(buildings) rebuild, `PERF-21`'s kind.
        let index_stale = !core.allocator.building_site_query_index_ready();
        let start = Instant::now();
        core.allocator
            .prepare_building_site_query_index(core.config.zone_cell_m);
        let index_rebuild_ms = start.elapsed().as_secs_f64() * 1e3;
        let polygon = PolygonFootprint::new(&footprint);
        let mut take = RoadLandTake::default();
        let take_ms = median_of_9(&mut || {
            take = RoadLandTake::of_footprint(&CoreLand::of(core), &polygon, owner);
        });
        let site_candidates = core.allocator.explicit_site_candidate_count(
            polygon.min,
            polygon.max,
            core.config.zone_cell_m,
        );
        let terrain = core.terrain_dirty_patch_states();
        core.acknowledge_terrain_render_patches(&terrain);
        let start = Instant::now();
        match name {
            "service_free" => core
                .place_service_building_internal(lot_asset, -600.0, FREE_Z)
                .map(|_| ())
                .map_err(|error| format!("{error:?}")),
            "service" => {
                core.place_service_building_internal(lot_asset, -100.0, REQUEST_Z)
                    .map(|_| ())
                    .map_err(|error| format!("{error:?}"))
            }
            "farm" | "mine" => {
                let asset = if name == "farm" { FARM } else { MINE };
                let x = if name == "farm" { -300.0 } else { 300.0 };
                core.place_industry_building_internal(asset, x, REQUEST_Z)
                    .map(|index| {
                        let handle = Some(core.allocator.building_handles.handle(index));
                        if name == "farm" {
                            farm = handle;
                        } else {
                            mine = handle;
                        }
                    })
                    .map_err(|error| format!("{error:?}"))
            }
            "field" | "field_resize" => core
                .commit_field_polygon_internal(owner.unwrap(), footprint.clone())
                .map(|_| ()),
            _ => core
                .commit_extractor_polygon_internal(owner.unwrap(), footprint.clone())
                .map(|_| ()),
        }
        .unwrap_or_else(|error| panic!("the {name} is refused: {error}"));
        let placement_ms = start.elapsed().as_secs_f64() * 1e3;
        let placed = usize::from(matches!(name, "service_free" | "service" | "farm" | "mine"));
        let demolished = buildings + placed - core.allocator.buildings.len();
        let erased = painted - core.zoning.cells.saved_cells().len();
        eprintln!(
            "LOCALITY_PLACEMENTS {}",
            serde_json::json!({"background": background, "placement": name,
                "buildings": buildings, "households": households, "agents": agents,
                "demolished": demolished, "erased_cells": erased,
                "take_buildings": take.buildings.len(), "take_lots": take.lots.len(),
                "take_cells": take.cells.len(), "site_candidates": site_candidates,
                "index_stale": index_stale, "index_rebuild_ms": index_rebuild_ms,
                "take_ms": take_ms, "placement_ms": placement_ms})
        );
        assert!(take.plotted.is_none(), "the {name} meets no plotted land");
        assert_eq!(demolished, take.buildings.len(), "the {name} demolishes its take");
        if ["service_free", "field"].contains(&name) {
            assert_eq!((demolished, erased), (0, 0), "the {name} takes nothing");
        } else {
            assert!(demolished >= 1, "the {name} takes built lots: {demolished}");
        }
        takes.push(serde_json::json!({"placement": name, "demolished": demolished,
            "erased_cells": erased, "take_lots": take.lots.len()}));
    }
    // The building placements' tail alone, again on the city as they left it: the lot plan,
    // attachment repair and entrance rebuild, then the site publication (`dev`'s path, no take).
    let start = Instant::now();
    core.rebuild_building_entrances_internal();
    let entrances_ms = start.elapsed().as_secs_f64() * 1e3;
    let start = Instant::now();
    core.publish_pending_building_site_changes();
    let publication_ms = start.elapsed().as_secs_f64() * 1e3;
    eprintln!(
        "LOCALITY_PLACEMENT_TAIL {}",
        serde_json::json!({"background": background, "entrances_ms": entrances_ms,
            "publication_ms": publication_ms})
    );
    // The field and area as committed, by their owners' lots (indices differ by tier).
    let bits = |points: &[Vector2]| {
        points
            .iter()
            .map(|p| [p.x.to_bits(), p.y.to_bits()])
            .collect::<Vec<_>>()
    };
    let owner_lot =
        |core: &SimCore, owner: usize| bits(&core.allocator.building_sites[owner].lot_footprint_world);
    let mut work_areas: Vec<_> = core
        .agriculture
        .sites()
        .iter()
        .map(|site| (owner_lot(core, site.building_idx), bits(&site.polygon_world)))
        .chain(
            core.resource_extraction
                .sites()
                .iter()
                .map(|site| (owner_lot(core, site.building_idx), bits(&site.polygon_world))),
        )
        .collect();
    work_areas.sort();
    serde_json::json!({"takes": takes,
        "work_areas": format!("{:016x}", crate::debug::work::debug_hash(&work_areas))})
}

// The median wall time of nine runs, in ms.
fn median_of_9(run: &mut dyn FnMut()) -> f64 {
    let mut samples: Vec<f64> = (0..9)
        .map(|_| {
            let start = Instant::now();
            run();
            start.elapsed().as_secs_f64() * 1e3
        })
        .collect();
    samples.sort_by(f64::total_cmp);
    samples[4]
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

// The road tool's hover on the same neighbourhood (`ZONE-09`, round RZ3): each demolishing
// stroke and one stroke along the rears of a block's lots, previewed `HOVERS` times on the road
// tool's context, its end moved a quarter metre each time as a pointer moves. A hover is
// `preview_road` alone (no core). Then the roundabout tool's hover (`ZONE-10`, round RZ10b): a
// ring of the default diameter in place of the crossing at (0, 350) (the commit benchmark's
// ring), tried `HOVERS` times on the context's edit site as the pointer moves a tenth of a
// metre at a time within the junction's reach, as `preview_roundabout` does. Their products (validity, reason
// and what they take) must not depend on the tier, their cost must stay flat
// (`PREVIEW_LOCALITY`).
const HOVERS: usize = 40;

#[test]
#[ignore = "release ZONE-09 hover preview locality; builds each tier outside timing"]
fn benchmark_preview_locality() {
    use crate::nodes::sim::core::road_preview::RoadPreviewRequest;
    use crate::nodes::sim::core::road_tool_context::RoadPreviewContext;
    super::contract::timing_mode();
    let tiers: Vec<usize> = std::env::var("METRUM_LOCALITY_TIERS")
        .map(|value| {
            value
                .split(',')
                .map(|t| t.trim().parse().unwrap())
                .collect()
        })
        .unwrap_or_else(|_| vec![0, 10_000, 30_000]);
    let mut reference = None;
    for background in tiers {
        let mut core = super::zoned_city_in_world(NEIGHBOURHOOD_SIDE_M, WORLD_M);
        add_background(&mut core, background);
        core.precompute_road_mesh_data();
        core.refresh_road_terrain_inputs();
        core.allocator
            .prepare_building_site_query_index(core.config.zone_cell_m);
        let view = crate::nodes::sim::core::road_tool_snapshot_from_core(&core)
            .expect("the fixture roadbed is current");
        let mut context = RoadPreviewContext::capture(view);
        preview_land(&mut core, &mut context);
        let half_z =
            (NEIGHBOURHOOD_SIDE_M / super::ROW_SPACING_M).floor() * super::ROW_SPACING_M * 0.5;
        let ground = |x: f32, z: f32| Vector3::new(x, core.heightmap.sample_height_world(x, z), z);
        let mut strokes: Vec<(String, [Vector3; 2])> = DEMOLITIONS
            .iter()
            .enumerate()
            .map(|(stroke, &(x, row))| {
                let z = row as f32 * super::ROW_SPACING_M - half_z;
                (
                    format!("demolition{stroke}"),
                    [ground(x, z), ground(x + DEMOLITION_RUN_M, z + super::ROW_SPACING_M)],
                )
            })
            .collect();
        // Along the rears of the lots between row streets 2 and 3, west of the block's parking
        // lot and clear of the cross street at x = -400.
        let rear_z = 2.5 * super::ROW_SPACING_M - half_z;
        strokes.push(("rears".into(), [ground(-380.0, rear_z), ground(-230.0, rear_z)]));
        let mut products = Vec::new();
        for (name, stroke) in &strokes {
            let mut total = Vec::with_capacity(HOVERS);
            let mut roadbed = Vec::with_capacity(HOVERS);
            let mut product = serde_json::Value::Null;
            for hover in 0..HOVERS {
                let mut points = stroke.to_vec();
                points[1].x += hover as f32 * 0.25;
                let request = RoadPreviewRequest {
                    surface_generation: context.view.surface_generation,
                    points,
                    fwd_lanes: 1,
                    bkw_lanes: 1,
                    snap_to_existing_roads: true,
                    tunnel: false,
                    median: None,
                    parking: crate::simulation::network::graph::RoadParking::NONE,
                    attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
                };
                let start = Instant::now();
                let preview = context.preview_road(request);
                total.push(start.elapsed().as_secs_f64() * 1e3);
                roadbed.push(preview.times.roadbed_ms);
                if hover == 0 {
                    product = preview_product(&preview);
                }
            }
            let stats = |samples: &mut Vec<f64>| {
                samples.sort_by(f64::total_cmp);
                let at = |q: f64| samples[((samples.len() - 1) as f64 * q).round() as usize];
                serde_json::json!({"p50": at(0.5), "p95": at(0.95), "max": at(1.0)})
            };
            eprintln!(
                "PREVIEW_LOCALITY {}",
                serde_json::json!({"background": background, "stroke": name,
                    "buildings": core.allocator.buildings.len(),
                    "parcels": core.zoning.parcels().len(),
                    "hovers": HOVERS, "preview_ms": stats(&mut total),
                    "roadbed_ms": stats(&mut roadbed), "product": product})
            );
            products.push(product);
        }
        products.push(ring_hover_product(&mut context, &core, background));
        match &reference {
            None => reference = Some(products),
            Some(reference) => assert_eq!(
                &products, reference,
                "a remote background of {background} changed the previews' products"
            ),
        }
    }
}

// The roundabout tool's hover at the crossing (0, 350), `HOVERS` times on the context with the
// pointer a tenth of a metre further each time, timed as `preview_roundabout` runs it (the site
// lookup and the edit site's try, no core). Returns what it shows: validity, reason and take.
fn ring_hover_product(
    context: &mut crate::nodes::sim::core::road_tool_context::RoadPreviewContext,
    core: &SimCore,
    background: usize,
) -> serde_json::Value {
    use crate::nodes::sim::roundabouts::roundabout_site_at;
    use crate::simulation::network::roundabouts::RoundaboutRequest;
    let centre = Vector3::new(0.0, core.heightmap.sample_height_world(0.0, 350.0), 350.0);
    let mut total = Vec::with_capacity(HOVERS);
    let mut product = serde_json::Value::Null;
    for hover in 0..HOVERS {
        let point = centre + Vector3::new(hover as f32 * 0.1, 0.0, 0.0);
        let start = Instant::now();
        let view = roundabout_site_at(&context.replica, point, 1);
        let request = RoundaboutRequest {
            site: view.site,
            diameter_m: view.default_diameter_m,
        };
        let verdict = context
            .road_edit_site()
            .try_roundabout_edit(request)
            .map(|tried| tried.verdict());
        total.push(start.elapsed().as_secs_f64() * 1e3);
        if hover == 0 {
            product = match &verdict {
                Ok(verdict) => serde_json::json!({"valid": true, "reason": null,
                    "cells": verdict.take.cells.len(), "lots": verdict.take.lots,
                    "buildings": verdict.take.buildings.len()}),
                Err(rejection) => serde_json::json!({"valid": false,
                    "reason": rejection.reason, "cells": 0, "lots": null, "buildings": 0}),
            };
        }
    }
    total.sort_by(f64::total_cmp);
    let at = |q: f64| total[((total.len() - 1) as f64 * q).round() as usize];
    eprintln!(
        "PREVIEW_LOCALITY {}",
        serde_json::json!({"background": background, "stroke": "ring",
            "buildings": core.allocator.buildings.len(),
            "parcels": core.zoning.parcels().len(), "hovers": HOVERS,
            "preview_ms": {"p50": at(0.5), "p95": at(0.95), "max": at(1.0)},
            "product": product})
    );
    assert_eq!(product["valid"], true, "the ring hover is valid: {product}");
    product
}

// The land the road tool's preview takes, as the simulation publishes it.
fn preview_land(
    core: &mut SimCore,
    context: &mut crate::nodes::sim::core::road_tool_context::RoadPreviewContext,
) {
    context.land = core.current_road_tool_land();
}

// What a hover shows: its verdict and what it takes (lots by id, which the neighbourhood's
// strokes keep across tiers).
fn preview_product(
    preview: &crate::nodes::sim::core::road_preview::RoadPreview,
) -> serde_json::Value {
    let take = preview.pieces.as_ref().map(|pieces| &pieces.take);
    serde_json::json!({"valid": preview.validation.is_valid,
        "reason": preview.validation.invalid_reason,
        "cells": take.map_or(0, |take| take.cells.len()),
        "lots": take.map(|take| take.lots.clone()),
        "buildings": take.map_or(0, |take| take.buildings.len())})
}
