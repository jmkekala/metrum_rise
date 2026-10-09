// SPDX-License-Identifier: GPL-2.0-only

//! Spatially indexed household claims with exact directed shopping access.

use super::ConsumerBudgets;
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::economy::accessibility::{
    BuildingModeComponents, ModeComponentIndex, ReachableBucketEntry, ReachableBucketIndex,
    ReachableBucketScanEvent, ReachableReservation, chunk_for_point,
};
use crate::simulation::economy::agents::tick::{
    BuildingTripEnds, PedestrianReachability,
};
use crate::simulation::economy::definitions::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use crate::simulation::economy::households::Household;
use crate::simulation::network::{TransitNetwork, graph::RegionGraph, types::TransitFlags};
use rayon::prelude::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Access {
    home: usize,
    has_car: bool,
    can_shop: bool,
    chunk: (i32, i32),
    components: [BuildingModeComponents; 2],
    reservations: [[Option<ReachableReservation>; 2]; 2],
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
    query: crate::simulation::pathing::cch::CchQueryScratch,
    revision: Option<(u64, u64, u32, u64)>,
    frontage_revision: u64,
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
    #[cfg(test)]
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
        if self.refresh(households, allocator, catalog, tuning, policy, treasury, network, graph) {
            self.reset_claims();
        }
    }

    /// Refreshes base budgets and access, retaining claims only when every input is exact.
    /// Returns whether the preceding book survived. Seller requests and financing decisions
    /// must still match before its quantities can be reused. O(H + recipe ports) warm checks.
    pub(crate) fn refresh(
        &mut self,
        households: &[Household],
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        policy: &CityFiscalPolicy,
        treasury: f64,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        let budgets_unchanged = self.budgets
            .prepare_budgets(households, allocator, catalog, tuning, policy, treasury);
        self.query.prepare(&network.cch_graph, graph);
        let output_ports = allocator
            .buildings
            .par_iter()
            .map(|building| {
                catalog
                    .profile_by_runtime_id(building.economy_profile_runtime_id)
                    .map_or(0, |profile| profile.outputs.len())
            })
            .sum();
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
            // Every category has identical membership and stable sorted bucket positions.
            // Availability differs, so only the immutable positions are shared across them.
            if let Some(indices) = self.indices.first() {
                self.access.par_iter_mut().enumerate().for_each(|(household, access)| {
                    access.reservations = std::array::from_fn(|mode| std::array::from_fn(|rank| {
                        access.components[mode].as_slice().get(rank).and_then(|&component| {
                            indices[mode].reservation(ReachableBucketEntry::new(
                                component, access.chunk, household))
                        })
                    }));
                });
            }
        }
        let retained = budgets_unchanged && !changed && !length_changed && !access_changed
            && self.frontage_revision == network.frontage_delay_revision;
        if !retained {
            self.reset_claims();
        }
        self.budgets.prepare_owners(allocator.buildings.len(), output_ports);
        self.visited.resize(households.len(), 0);
        self.revision = Some(revision);
        self.frontage_revision = network.frontage_delay_revision;
        retained
    }

    /// Returns all accounts to their prepared recurring budgets in O(H*K + owners).
    /// The caller must keep this census's household, basket and access inputs immutable.
    /// Receipt and bucket storage remain allocated; no budgets or routes are requoted.
    pub(crate) fn reset_claims(&mut self) {
        self.budgets.reset_claims();
        let count = self.budgets.categories.len();
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
            self.walking.prepare_query();
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

    /// Prepares temporary project receipts once before ordered construction quotation.
    pub(crate) fn prepare_project_owners(&mut self, owners: usize, output_ports: usize) {
        self.budgets.prepare_owners(owners, output_ports);
        self.budgets.prepare_revalidation();
    }

    /// Starts the household part of an enclosing replacement/revalidation proof.
    pub(crate) fn begin_revalidation(&mut self) { self.budgets.begin_revalidation(); }

    /// Publishes all accepted owner changes in the enclosing proof.
    pub(crate) fn accept_revalidation(&mut self) { self.budgets.accept_revalidation(); }

    /// Restores exact accounts, owner receipts and only affected spatial eligibility.
    pub(crate) fn reject_revalidation(&mut self) {
        self.reject();
        let count = self.budgets.categories.len();
        let access = &self.access;
        let indices = &mut self.indices;
        self.budgets.visit_revalidation_funds(|slot, funds| {
            sync_account(indices, access, count, slot, funds);
        });
        self.budgets.reject_revalidation();
    }

    /// Household production already promised by one owner; O(its current customer receipts).
    pub(crate) fn reserved_units(&self, owner: usize, resource: u16) -> f64 {
        self.budgets.reserved_units(owner, resource)
    }

    /// Retains this live owner's receipt so a later replacement can reclaim its own customers.
    pub(crate) fn accept_for(&mut self, owner: usize) {
        self.budgets.accept_for(owner);
    }

    /// Census commit outside any revalidation transaction: preflights reusable cells and grows
    /// the retained arena exactly when the census estimate is short. O(staged claims + reusable
    /// cells); warm censuses keep the grown capacity and do not allocate.
    pub(crate) fn accept_growing_for(&mut self, owner: usize) {
        if let Err(required) = self.budgets.check_accept_capacity(owner) {
            self.budgets.reserve_receipts(required);
        }
        self.budgets.accept_for(owner);
    }

    /// Starts every census from an empty receipt arena to exercise exact commit growth.
    #[cfg(test)]
    pub(crate) fn discard_receipt_estimate_for_test(&mut self) {
        self.budgets.discard_receipt_estimate = true;
    }

    /// Preflights exact receipt capacity in O(staged claims + reusable cells), without mutation.
    pub(crate) fn check_accept_capacity(&self, owner: usize) -> Result<(), usize> {
        self.budgets.check_accept_capacity(owner)
    }

    /// Grows receipt/journal storage only after the enclosing proposal has been restored.
    pub(crate) fn reserve_receipts(&mut self, required: usize) {
        self.budgets.reserve_receipts(required);
    }

    /// Exercises exact runtime preflight with no spare cells and all existing claims intact.
    #[cfg(test)]
    pub(crate) fn shrink_receipt_storage_for_test(&mut self) {
        debug_assert!(!self.budgets.revalidation.is_active());
        self.budgets.committed.shrink_to_fit();
        self.budgets.prepare_revalidation();
    }

    /// Observes actual arena growth in forced runtime replay tests.
    #[cfg(test)]
    pub(crate) fn receipt_capacity_for_test(&self) -> usize { self.budgets.committed.capacity() }

    /// Compares exact family balances and owner purchases independently of receipt cell reuse.
    #[cfg(test)]
    pub(crate) fn assert_same_accounts_for_test(&self, other: &Self) {
        let balances = |market: &Self| market.budgets.funds.iter()
            .map(|funds| (funds.units.to_bits(), funds.gross_spending.to_bits())).collect::<Vec<_>>();
        assert_eq!(balances(self), balances(other));
        let purchases = |market: &Self| {
            let mut result = Vec::new();
            for (owner, &first) in market.budgets.owners.iter().enumerate() {
                let mut cell = first;
                while cell != usize::MAX {
                    let receipt = market.budgets.committed[cell];
                    result.push((owner, receipt.slot, receipt.acquired.units.to_bits(),
                        receipt.acquired.gross_spending.to_bits()));
                    cell = receipt.next;
                }
            }
            result.sort_unstable();
            result
        };
        assert_eq!(purchases(self), purchases(other));
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
                    let mut reachable = |from: &BuildingTripEnds, to: &BuildingTripEnds| {
                        from.has_proven_walk_to(to, &self.walking)
                            || from.can_reach(to, access.has_car, network, graph,
                                &mut self.walking, &mut self.query)
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
            let funds = if previous {
                claim.previous
            } else {
                self.budgets.funds[claim.slot]
            };
            sync_account(&mut self.indices, &self.access, categories, claim.slot, funds);
        }
    }

    /// Restores exact pre-quote accounts and their spatial eligibility after rejection.
    pub(crate) fn reject(&mut self) {
        self.sync_staged(true);
        self.budgets.reject();
    }
}

fn sync_account(indices: &mut [[ReachableBucketIndex; 2]], access: &[Access],
    categories: usize, slot: usize, funds: super::Funds) {
    let household = slot / categories;
    let category = slot % categories;
    let available = funds.units > 0.0 && funds.gross_spending > 0.0;
    let access = access[household];
    for (mode, reservations) in access.reservations.iter().enumerate() {
        for &reservation in reservations.iter().flatten() {
            indices[category][mode].set_reserved_available(reservation, available);
        }
    }
}

fn shares_walk_group(from: [usize; 2], from_nodes: [u32; 2], to: [usize; 2], to_nodes: [u32; 2]) -> bool {
    from.into_iter().enumerate().any(|(origin, group)| {
        group != usize::MAX && to.into_iter().enumerate()
            .any(|(destination, target)| group == target && from_nodes[origin] != to_nodes[destination])
    })
}
