// SPDX-License-Identifier: GPL-2.0-only

//! Spatially indexed household claims with exact directed shopping access.

use super::ConsumerBudgets;
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::economy::accessibility::{
    BuildingModeComponents, ModeComponentIndex, ReachableBucketEntry, ReachableBucketIndex,
    ReachableBucketScanEvent, chunk_for_point,
};
use crate::simulation::economy::agents::tick::{
    BuildingTripEnds, PedestrianReachability, TripEstimate, estimate_trip_seconds_between,
};
use crate::simulation::economy::definitions::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use crate::simulation::economy::households::Household;
use crate::simulation::network::{TransitNetwork, graph::RegionGraph, types::TransitFlags};
use rayon::prelude::*;
use std::sync::atomic::AtomicU32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Access {
    home: usize,
    has_car: bool,
    can_shop: bool,
    chunk: (i32, i32),
    components: [BuildingModeComponents; 2],
    walk_origins: [usize; 2],
    walk_destinations: [usize; 2],
    // Same node-rank order as BuildingTripEnds. Equal-node pairs need the direct connector.
    walk_nodes: [u32; 2],
}

/// Finite family accounts and their reusable component/chunk candidate indices.
/// Component membership only prunes candidates; both directed trips must still succeed.
#[derive(Clone, Debug, Default)]
pub(crate) struct ConsumerMarket {
    budgets: ConsumerBudgets,
    access: Vec<Access>,
    indices: Vec<[ReachableBucketIndex; 2]>,
    components: Option<[ModeComponentIndex; 2]>,
    walking: PedestrianReachability,
    revision: Option<(u64, u64, u32, u64)>,
    visited: Vec<u64>,
    visit: u64,
}

impl ConsumerMarket {
    /// Withdraws all customer accounts and access, retaining reusable account allocations.
    pub(crate) fn clear(&mut self) {
        self.budgets.clear();
        self.revision = None;
        self.access.clear();
        self.indices.clear();
    }

    /// Refreshes O(H*K) budget/tree entries in parallel across consumer categories.
    /// Access changes rebuild the existing bucket structure in O(H*K log H + roads);
    /// unchanged layouts retain it. Neither phase scans the city once per seller.
    pub(crate) fn reset(
        &mut self,
        households: &[Household],
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        policy: &CityFiscalPolicy,
        treasury: f64,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        self.budgets
            .reset(households, allocator, catalog, tuning, policy, treasury);
        let output_ports = allocator
            .buildings
            .par_iter()
            .map(|building| {
                catalog
                    .profile_by_runtime_id(building.economy_profile_runtime_id)
                    .map_or(0, |profile| profile.outputs.len())
            })
            .sum();
        self.budgets
            .prepare_owners(allocator.buildings.len(), output_ports);
        let revision = (
            allocator.building_ref_revision(),
            allocator.entrance_ref_revision(),
            network.cch_graph.build_generation,
            network.cch_graph.metric_revision,
        );
        let changed = self.revision != Some(revision);
        if changed {
            self.walking = PedestrianReachability::build(network, graph);
            self.components = Some([
                ModeComponentIndex::build(graph, TransitFlags::FOOT),
                ModeComponentIndex::build(graph, TransitFlags::CAR),
            ]);
        }
        let components = self.components.as_ref().unwrap();
        let length_changed = self.access.len() != households.len();
        self.access.resize(households.len(), Access::default());
        let access_changed = self
            .access
            .par_iter_mut()
            .zip(households)
            .map(|(access, household)| {
                let can_shop = household.adult_count > 0 || household.elder_count > 0;
                if !changed
                    && access.home == household.home_building_id
                    && access.has_car == household.shopping_has_car
                    && access.can_shop == can_shop
                {
                    return false;
                }
                let mut next = Access {
                    home: household.home_building_id,
                    has_car: household.shopping_has_car,
                    can_shop,
                    ..Default::default()
                };
                if can_shop && let Some(building) = allocator.buildings.get(next.home) {
                    if let Some(edge) = allocator.entrances.get(next.home)
                        .and_then(|entrance| graph.get_edge(entrance.edge_idx)) {
                        next.walk_nodes = [edge.start_node, edge.end_node];
                    }
                    next.walk_origins = BuildingTripEnds::origin(next.home, allocator, network, graph)
                        .walking_groups(&self.walking);
                    next.walk_destinations = BuildingTripEnds::destination(next.home, allocator, network, graph)
                        .walking_groups(&self.walking);
                    next.chunk = chunk_for_point(building.center_x, building.center_y);
                    next.components[0] = components[0].building_components(
                        allocator,
                        graph,
                        next.home,
                        TransitFlags::FOOT,
                    );
                    if next.has_car {
                        next.components[1] = components[1].building_components(
                            allocator,
                            graph,
                            next.home,
                            TransitFlags::CAR,
                        );
                    }
                }
                *access = next;
                true
            })
            .reduce(|| false, |a, b| a | b);
        let count = self.budgets.categories.len();
        if changed || length_changed || access_changed || self.indices.len() != count {
            self.indices.resize_with(count, Default::default);
            self.indices.par_iter_mut().for_each(|indices| {
                for (mode, index) in indices.iter_mut().enumerate() {
                    let mut entries = Vec::with_capacity(self.access.len() * 2);
                    for (household, access) in self.access.iter().enumerate() {
                        access.components[mode].append_bucket_entries(
                            &mut entries,
                            access.chunk,
                            household,
                        );
                    }
                    *index = ReachableBucketIndex::from_entries(entries);
                    index.enable_reservations();
                }
            });
        }
        self.indices
            .par_iter_mut()
            .enumerate()
            .for_each(|(category, indices)| {
                for index in indices {
                    index.reset_available(|household| {
                        let funds = self.budgets.funds[household * count + category];
                        funds.units > 0.0 && funds.gross_spending > 0.0
                    });
                }
            });
        self.visited.resize(households.len(), 0);
        self.revision = Some(revision);
    }

    /// Legacy catalog-price diagnostic; individual claims use the actual seller price.
    pub(crate) fn affordable_residents(&self) -> f64 {
        self.budgets.affordable_residents()
    }

    /// Whether the authored household basket consumes this resource, in O(1).
    pub(crate) fn consumes(&self, resource: u16) -> bool {
        self.budgets
            .category_by_resource
            .get(resource as usize)
            .is_some_and(|&category| category != usize::MAX)
    }

    /// Forks live accounts and owner receipts once per construction pass. Matching access
    /// retains bucket storage; warm copying is O(household accounts + accepted claims).
    pub(crate) fn copy_reservations_from(&mut self, source: &Self) {
        if self.revision != source.revision
            || self.access != source.access
            || self.indices.len() != source.indices.len()
        {
            *self = source.clone();
        }
        self.budgets.copy_reservations_from(&source.budgets);
        let count = self.budgets.categories.len();
        self.indices
            .par_iter_mut()
            .enumerate()
            .for_each(|(category, indices)| {
                for index in indices {
                    index.reset_available(|household| {
                        let account = self.budgets.funds[household * count + category];
                        account.units > 0.0 && account.gross_spending > 0.0
                    });
                }
            });
    }

    /// Temporarily returns only this incumbent's accepted purchases for a replacement quote.
    /// Rejection restores the exact pre-release accounts and spatial eligibility.
    pub(crate) fn release_owner(&mut self, owner: usize) {
        self.reject();
        self.budgets.release_owner(owner);
        self.sync_staged(false);
    }

    /// Retains this live owner's receipt so a later replacement can reclaim its own customers.
    pub(crate) fn accept_for(&mut self, owner: usize) {
        self.budgets.accept_for(owner);
    }

    /// Stages at most `requested` recurring units from actual reached families, in stable
    /// foot/car, chunk-ring and household order. A family is checked once per call, including
    /// homes appearing in multiple components. Exhausted accounts leave the index immediately.
    /// Cost is visited bucket candidates plus their directed route queries, not all households.
    pub(crate) fn claim_reachable(
        &mut self,
        entrance: &BuildingEntrance,
        resource: u16,
        requested: f64,
        gross_price: f32,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> f64 {
        let Some(&category) = self.budgets.category_by_resource.get(resource as usize) else {
            return 0.0;
        };
        if category >= self.indices.len()
            || requested <= 0.0
            || !gross_price.is_finite()
            || gross_price <= 0.0
        {
            return 0.0;
        }
        let Some(components) = self.components.as_ref() else {
            return 0.0;
        };
        if !self.indices[category].iter().any(ReachableBucketIndex::has_available) {
            return 0.0;
        }
        self.visit = self.visit.wrapping_add(1);
        if self.visit == 0 {
            self.visited.par_iter_mut().for_each(|value| *value = 0);
            self.visit = 1;
        }
        let origin = BuildingTripEnds::at_entrance(entrance, true, network, graph);
        let destination = BuildingTripEnds::at_entrance(entrance, false, network, graph);
        let origin_groups = origin.walking_groups(&self.walking);
        let destination_groups = destination.walking_groups(&self.walking);
        let nodes = graph.get_edge(entrance.edge_idx)
            .map_or([u32::MAX; 2], |edge| [edge.start_node, edge.end_node]);
        let pathfinds = AtomicU32::new(0);
        let mut claimed = 0.0;
        for (mode, flag) in [TransitFlags::FOOT, TransitFlags::CAR]
            .into_iter()
            .enumerate()
        {
            let reached = components[mode].entrance_components(entrance, graph, flag);
            self.indices[category][mode].scan_nearest(
                reached,
                entrance.door_pos.x,
                entrance.door_pos.y,
                |event| {
                    if claimed >= requested {
                        return false;
                    }
                    let ReachableBucketScanEvent::Item {
                        item_idx: household,
                    } = event
                    else {
                        return true;
                    };
                    if self.visited[household] == self.visit {
                        return true;
                    }
                    self.visited[household] = self.visit;
                    if self.budgets.available(household, resource, gross_price) <= 0.0 {
                        return true;
                    }
                    let access = self.access[household];
                    let outward = shares_walk_group(access.walk_origins, access.walk_nodes, destination_groups, nodes);
                    let homeward = shares_walk_group(origin_groups, nodes, access.walk_destinations, access.walk_nodes);
                    if outward && homeward {
                        claimed += self.budgets.claim(household, resource, requested - claimed, gross_price);
                        return claimed < requested;
                    }
                    let home_origin =
                        BuildingTripEnds::origin(access.home, allocator, network, graph);
                    let home_destination =
                        BuildingTripEnds::destination(access.home, allocator, network, graph);
                    let reachable = |from: &BuildingTripEnds, to: &BuildingTripEnds| {
                        from.has_proven_walk_to(to, &self.walking)
                            || matches!(
                                estimate_trip_seconds_between(
                                    from,
                                    to,
                                    access.has_car,
                                    network,
                                    graph,
                                    &pathfinds,
                                    f32::INFINITY,
                                    f32::INFINITY
                                ),
                                TripEstimate::Seconds(Some(_))
                            )
                    };
                    if (outward || reachable(&home_origin, &destination))
                        && (homeward || reachable(&origin, &home_destination))
                    {
                        claimed += self.budgets.claim(
                            household,
                            resource,
                            requested - claimed,
                            gross_price,
                        );
                    }
                    claimed < requested
                },
            );
        }
        self.sync_staged(false);
        claimed
    }

    fn sync_staged(&mut self, previous: bool) {
        let categories = self.budgets.categories.len();
        for claim in &self.budgets.staged {
            let household = claim.slot / categories;
            let category = claim.slot % categories;
            let funds = if previous {
                claim.previous
            } else {
                self.budgets.funds[claim.slot]
            };
            let available = funds.units > 0.0 && funds.gross_spending > 0.0;
            let access = self.access[household];
            for (mode, components) in access.components.iter().enumerate() {
                for &component in components.as_slice() {
                    self.indices[category][mode].set_available(
                        ReachableBucketEntry::new(component, access.chunk, household),
                        available,
                    );
                }
            }
        }
    }

    /// Commits staged household claims after the complete operation is funded.
    pub(crate) fn accept(&mut self) {
        self.budgets.accept();
    }

    /// Restores exact pre-quote accounts and their spatial eligibility after rejection.
    pub(crate) fn reject(&mut self) {
        self.sync_staged(true);
        self.budgets.reject();
    }
}

fn shares_walk_group(from: [usize; 2], from_nodes: [u32; 2], to: [usize; 2], to_nodes: [u32; 2]) -> bool {
    from.into_iter().enumerate().any(|(origin, group)| {
        group != usize::MAX && to.into_iter().enumerate()
            .any(|(destination, target)| group == target && from_nodes[origin] != to_nodes[destination])
    })
}
