// SPDX-License-Identifier: GPL-2.0-only

//! Commute bounds used by job search must never change an exhaustive commute estimate.

use super::super::populated_city::zoned_city;
use crate::config::MIN_ROUTE_SPEED_MS;
use crate::simulation::economy::accessibility::max_speed_for_modes;
use crate::simulation::economy::agents::tick::{
    BuildingTripEnds, TripEstimate, estimate_building_origin_trip_seconds,
    estimate_trip_seconds_between,
};
use crate::simulation::network::types::TransitFlags;
use crate::simulation::zoning::ZoneType;
use std::sync::atomic::AtomicU32;

const CITY_SIDE_M: f32 = 800.0;
// Every home would take minutes at test optimization; a stride still covers every row and column.
const HOME_STRIDE: usize = 23;

#[test]
fn bounded_commute_estimates_match_exhaustive_estimates() {
    let mut core = zoned_city(CITY_SIDE_M);
    core.transit_network
        .rebuild_pathing_if_dirty(&mut core.region_graph);
    let allocator = &core.allocator;
    let network = &core.transit_network;
    let graph = &core.region_graph;
    let speed_bound =
        max_speed_for_modes(graph, TransitFlags::FOOT | TransitFlags::CAR).max(MIN_ROUTE_SPEED_MS);
    let pathfinds = AtomicU32::new(0);

    let buildings_of = |zone: fn(ZoneType) -> bool| -> Vec<usize> {
        (0..allocator.buildings.len())
            .filter(|&idx| zone(allocator.buildings[idx].zone_type))
            .collect()
    };
    let homes = buildings_of(|zone| zone == ZoneType::Residential);
    let jobs = buildings_of(|zone| matches!(zone, ZoneType::Commercial | ZoneType::Industrial));
    assert!(homes.len() > 500 && jobs.len() > 100, "{} homes, {} jobs", homes.len(), jobs.len());

    let job_ends: Vec<_> = jobs
        .iter()
        .map(|&job| BuildingTripEnds::destination(job, allocator, network, graph))
        .collect();
    let node_slack_m = jobs
        .iter()
        .zip(&job_ends)
        .map(|(&job, ends)| {
            let building = &allocator.buildings[job];
            ends.max_node_distance(building.center_x, building.center_y)
        })
        .fold(0.0, f32::max);

    let mut routed = 0;
    let mut over_budget = 0;
    for &home in homes.iter().step_by(HOME_STRIDE) {
        let home_ends = BuildingTripEnds::origin(home, allocator, network, graph);
        let home_building = &allocator.buildings[home];
        for (&job, ends) in jobs.iter().zip(&job_ends) {
            for has_car in [false, true] {
                let exact = estimate_building_origin_trip_seconds(
                    home, job, has_car, allocator, network, graph, &pathfinds,
                );
                let Some(exact_s) = exact else {
                    continue;
                };
                routed += 1;
                let job_building = &allocator.buildings[job];
                let distance_m = (job_building.center_x - home_building.center_x)
                    .hypot(job_building.center_y - home_building.center_y);
                let ring_bound_s = home_ends.lower_bound_seconds_to_distance(
                    has_car,
                    home_building.center_x,
                    home_building.center_y,
                    distance_m,
                    node_slack_m,
                    speed_bound,
                );
                assert!(
                    ring_bound_s <= f32::from(exact_s),
                    "home {home} -> job {job} (car {has_car}): ring bound {ring_bound_s} s \
                     exceeds the {exact_s} s commute"
                );
                for budget_s in [
                    f32::INFINITY,
                    f32::from(exact_s) + 1.0,
                    f32::from(exact_s),
                    f32::from(exact_s) * 0.5,
                ] {
                    match estimate_trip_seconds_between(
                        &home_ends, ends, has_car, network, graph, &pathfinds, speed_bound,
                        budget_s,
                    ) {
                        TripEstimate::Seconds(seconds) => assert_eq!(
                            seconds, exact,
                            "home {home} -> job {job} (car {has_car}, budget {budget_s} s)"
                        ),
                        TripEstimate::OverBudget => {
                            over_budget += 1;
                            assert!(
                                f32::from(exact_s) > budget_s,
                                "home {home} -> job {job} (car {has_car}): {exact_s} s commute \
                                 reported over the {budget_s} s budget"
                            );
                        }
                    }
                }
            }
        }
    }
    assert!(routed > 10_000, "only {routed} reachable pairs checked");
    assert!(over_budget > 0, "no budget ever pruned a pair");
}
