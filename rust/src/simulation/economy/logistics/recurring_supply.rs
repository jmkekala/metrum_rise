// SPDX-License-Identifier: GPL-2.0-only

//! Finite production-rate quotes, separate from dispatch's physical-stock reservations.

use super::route_cache::FreightRouteCache;
use super::supplier_index::SupplierCandidateIndex;
use super::timing::freight_cost;
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::economy::accessibility::{
    ModeComponentIndex, ReachableBucketEntry, ReachableBucketScanEvent, chunk_for_point,
    lower_bound_travel_seconds, max_speed_for_modes,
};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, ResourceRuntimeId, RuntimeEconomyCatalog, RuntimeEconomyTuning,
};
use crate::simulation::economy::households::{
    output_area_scale_for_building, physical_worker_capacity_for_profile,
};
use crate::simulation::economy::operation::{InputRequirement, InputSupplyCosts};
use crate::simulation::network::{TransitNetwork, graph::RegionGraph, types::TransitFlags};
use rayon::prelude::*;

#[derive(Clone, Copy, Debug)]
struct Claim {
    source: usize,
    resource: ResourceRuntimeId,
    slot: usize,
    previous: f64,
    units: f64,
}

#[derive(Clone, Copy, Debug)]
struct Offer {
    source: usize,
    slot: usize,
    seconds: f32,
    next: usize,
}

#[derive(Clone, Copy, Debug)]
struct InputOffers {
    resource: ResourceRuntimeId,
    first: usize,
    last: usize,
    next: usize,
    exhausted: bool,
}

/// Reusable finite input market. Quotes stage claims; only accepted operations commit them.
/// Source quantities are net daily production of the funded recipe activity, capped by current staff.
#[derive(Clone, Debug, Default)]
pub(crate) struct RecurringSupply {
    offsets: Vec<usize>,
    active: Vec<bool>,
    revision: Option<(u64, u64, u32, u64, usize)>,
    remaining: Vec<f64>,
    staged: Vec<Claim>,
    // Only live owners need receipts: each incumbent can be replaced once in a planning pass.
    // Each source can be exhausted at its import seed and again after its one local
    // funding increase: at most twice the output ports plus accepted input ports.
    committed: Vec<Claim>,
    owner_ranges: Vec<(usize, usize)>,
    released: Vec<Claim>,
    releasing_owner: Option<usize>,
    receipt_capacity: usize,
    prices: Vec<(ResourceRuntimeId, f32)>,
    // A staffing search retries one operation against exactly restored quantities.
    // Replay its already-ranked offers instead of repeating spatial/route searches.
    offers: Vec<Offer>,
    input_offers: Vec<InputOffers>,
    nearest_incumbents: Vec<Option<usize>>,
    index: SupplierCandidateIndex,
    components: Option<ModeComponentIndex>,
    max_speed: f32,
}

impl RecurringSupply {
    /// Invalidates the source view after job-state removal, remapping, or reload.
    pub(crate) fn invalidate(&mut self) {
        self.revision = None;
    }

    /// Whether supplier indices and geometry still refer to this allocator/network layout.
    pub(crate) fn matches_layout(
        &self,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
    ) -> bool {
        self.revision
            == Some((
                allocator.building_ref_revision(),
                allocator.entrance_ref_revision(),
                network.cch_graph.build_generation,
                network.cch_graph.metric_revision,
                allocator.buildings.len(),
            ))
    }

    /// Forks accepted live reservations once for a construction pass. Unchanged geometry
    /// retains its index allocation; warm copying costs O(buildings + output ports + accepted
    /// input flows), never a city-sized copy per project.
    pub(crate) fn copy_reservations_from(
        &mut self,
        source: &Self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        graph: &RegionGraph,
    ) {
        debug_assert!(source.staged.is_empty());
        if self.revision != source.revision {
            *self = source.clone();
            // Vec cloning preserves contents, not spare capacity. Prepare the empty quote
            // journals here so the first proposal does not allocate inside its port loop.
            self.staged.reserve(self.remaining.len());
            self.released.reserve(self.remaining.len());
            self.offers.reserve(self.remaining.len());
            self.prices.reserve(catalog.resource_count());
            self.input_offers.reserve(catalog.resource_count());
            return;
        }
        self.staged.clear();
        self.released.clear();
        self.releasing_owner = None;
        self.committed.clone_from(&source.committed);
        self.owner_ranges.clone_from(&source.owner_ranges);
        self.prices.clear();
        self.offers.clear();
        self.input_offers.clear();
        self.remaining.clone_from(&source.remaining);
        for index in 0..source.offsets.len().saturating_sub(1) {
            let Some(profile) = catalog
                .profile_by_runtime_id(allocator.buildings[index].economy_profile_runtime_id)
            else {
                continue;
            };
            for (port, output) in profile.outputs.iter().enumerate() {
                let slot = source.offsets[index] + port;
                if self.active[slot] != source.active[slot] {
                    self.set_available(
                        index,
                        output.resource_runtime_id,
                        source.active[slot],
                        allocator,
                        graph,
                    );
                    self.active[slot] = source.active[slot];
                }
            }
        }
    }

    /// Whether any funded staffed source has a positive remaining production rate.
    pub(crate) fn has_offers(&self) -> bool {
        self.index.has_offers()
    }

    /// Whether this recipe can claim any remaining local offer, in O(input ports * log resources).
    /// A false result permits reuse of the already computed import-only funding result.
    pub(crate) fn offers_inputs(&self, profile: &EconomyProfileRuntime) -> bool {
        profile.inputs.iter().any(|input| {
            let internal = profile
                .output_port(input.resource_runtime_id)
                .map_or(0.0, |output| output.units_per_day);
            input.units_per_day > internal
                && self
                    .index
                    .buckets_for_resource(input.resource_runtime_id)
                    .is_some_and(|buckets| buckets.has_available())
        })
    }

    /// Prepares production rates and the existing resource/component/chunk index once per pass.
    /// Warm work is O(B + outputs + changed availability * log bucket size). Spatial
    /// rebuilding costs O(outputs log outputs + roads) only when building/access topology changes.
    /// Quantities use sparse output-port storage, not B*R.
    /// Physical shipment stock never creates a recurring offer.
    pub(crate) fn reset(
        &mut self,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        network: &TransitNetwork,
        catalog: &RuntimeEconomyCatalog,
        funded_activity: &[f32],
    ) {
        self.staged.clear();
        self.committed.clear();
        self.released.clear();
        self.releasing_owner = None;
        self.owner_ranges.resize(allocator.buildings.len(), (0, 0));
        self.owner_ranges
            .par_iter_mut()
            .for_each(|range| *range = (0, 0));
        self.prices.clear();
        self.offers.clear();
        self.input_offers.clear();
        self.prices.reserve(catalog.resource_count());
        self.input_offers.reserve(catalog.resource_count());
        let revision = (
            allocator.building_ref_revision(),
            allocator.entrance_ref_revision(),
            network.cch_graph.build_generation,
            network.cch_graph.metric_revision,
            allocator.buildings.len(),
        );
        if self.revision != Some(revision) {
            self.nearest_incumbents
                .resize(catalog.resource_count() + 1, None);
            self.nearest_incumbents.fill(None);
            self.offsets.clear();
            self.offsets.reserve(allocator.buildings.len() + 1);
            self.offsets.push(0);
            self.receipt_capacity = 0;
            for building in &allocator.buildings {
                self.receipt_capacity += catalog
                    .profile_by_runtime_id(building.economy_profile_runtime_id)
                    .map_or(0, |profile| profile.inputs.len());
                let outputs = catalog
                    .profile_by_runtime_id(building.economy_profile_runtime_id)
                    .map_or(0, |profile| profile.outputs.len());
                self.offsets
                    .push(self.offsets.last().copied().unwrap() + outputs);
            }
            self.remaining.resize(*self.offsets.last().unwrap(), 0.0);
            self.receipt_capacity += 2 * self.remaining.len();
            let components = ModeComponentIndex::build(graph, TransitFlags::CAR);
            // Inactive/unfinished sources remain geometrically indexed. Hourly quantities
            // enable them only when they have actual staffed, funded net production.
            self.index = SupplierCandidateIndex::build_filtered(
                allocator,
                graph,
                catalog,
                &components,
                false,
                |_, _| true,
            );
            self.index.enable_reservations();
            self.active.resize(self.remaining.len(), true);
            self.active.fill(true);
            self.components = Some(components);
            self.max_speed = max_speed_for_modes(graph, TransitFlags::CAR)
                .max(crate::config::AGENT_DRIVEWAY_SPEED_MS);
            self.revision = Some(revision);
        }
        fill_production_rates(
            allocator,
            catalog,
            funded_activity,
            &self.offsets,
            0,
            allocator.buildings.len(),
            &mut self.remaining,
        );
        for source in 0..allocator.buildings.len() {
            let Some(profile) = catalog
                .profile_by_runtime_id(allocator.buildings[source].economy_profile_runtime_id)
            else {
                continue;
            };
            for (port, output) in profile.outputs.iter().enumerate() {
                let slot = self.offsets[source] + port;
                let available = self.remaining[slot] > 0.0;
                if available != self.active[slot] {
                    self.set_available(
                        source,
                        output.resource_runtime_id,
                        available,
                        allocator,
                        graph,
                    );
                    self.active[slot] = available;
                }
            }
        }
        // At most one outstanding claim per eligible output port in an operation quote.
        self.staged.reserve(self.remaining.len());
        self.offers.reserve(self.remaining.len());
        self.committed.reserve(self.receipt_capacity);
        self.released.reserve(self.remaining.len());
    }

    /// Publishes one accepted live operation's increase over its import-funded seed.
    /// Only currently staffed net production is added; stock and unfilled jobs add nothing.
    /// Called once per source, between complete operation quotes. O(output ports plus
    /// newly available ports * log bucket size), using the existing index and sparse slots.
    pub(crate) fn publish_funded_increase(
        &mut self,
        source: usize,
        before: f32,
        after: f32,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        graph: &RegionGraph,
    ) {
        debug_assert!(self.staged.is_empty() && self.released.is_empty());
        debug_assert!(after >= before);
        if after == before {
            return;
        }
        let building = &allocator.buildings[source];
        let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
        else {
            return;
        };
        let previous = production_activity(building, profile, before);
        let current = production_activity(building, profile, after);
        if previous == current {
            return;
        }
        for (port, output) in profile.outputs.iter().enumerate() {
            let rate = profile.net_output_units_per_day(output) * output_area_scale_for_building(building, profile);
            let added = f64::from(rate * current) - f64::from(rate * previous);
            let slot = self.offsets[source] + port;
            self.remaining[slot] += added;
            if !self.active[slot] && self.remaining[slot] > 0.0 {
                self.active[slot] = true;
                self.set_available(source, output.resource_runtime_id, true, allocator, graph);
            }
        }
    }

    fn slot(
        &self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        source: usize,
        resource: ResourceRuntimeId,
    ) -> Option<usize> {
        let building = allocator.buildings.get(source)?;
        let profile = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)?;
        let port = profile
            .outputs
            .iter()
            .position(|port| port.resource_runtime_id == resource)?;
        Some(*self.offsets.get(source)? + port)
    }

    fn set_available(
        &mut self,
        source: usize,
        resource: ResourceRuntimeId,
        available: bool,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
    ) {
        let Some(components) = &self.components else {
            return;
        };
        let building = &allocator.buildings[source];
        let chunk = chunk_for_point(building.center_x, building.center_y);
        for &component in components
            .building_components(allocator, graph, source, TransitFlags::CAR)
            .as_slice()
        {
            self.index.set_available(
                resource,
                ReachableBucketEntry::new(component, chunk, source),
                available,
            );
        }
    }

    /// Rolls back a rejected operation in O(claims * log suppliers), retaining scratch capacity.
    pub(crate) fn reject(&mut self, allocator: &BuildingAllocator, graph: &RegionGraph) {
        self.retry(allocator, graph);
        while let Some(claim) = self.released.pop() {
            self.restore(claim, allocator, graph);
        }
        self.releasing_owner = None;
        self.offers.clear();
        self.input_offers.clear();
    }

    /// Restores the same operation's claims while retaining its exact supplier ordering.
    /// No other operation may mutate this book between retries. Each cached list advances
    /// in O(1) per offer; clearing a retry costs O(claims * log bucket size + input ports).
    pub(crate) fn retry(&mut self, allocator: &BuildingAllocator, graph: &RegionGraph) {
        while let Some(claim) = self.staged.pop() {
            self.restore(claim, allocator, graph);
        }
        self.prices.clear();
        for offers in &mut self.input_offers {
            offers.next = offers.first;
        }
    }

    /// Keeps accepted quantities reserved for all subsequent operations and projects.
    pub(crate) fn accept(&mut self) {
        if let Some(owner) = self.releasing_owner.take() {
            self.owner_ranges[owner] = (0, 0);
        }
        self.released.clear();
        self.staged.clear();
        self.prices.clear();
        self.offers.clear();
        self.input_offers.clear();
    }

    fn restore(&mut self, claim: Claim, allocator: &BuildingAllocator, graph: &RegionGraph) {
        self.remaining[claim.slot] = claim.previous;
        let available = claim.previous > 0.0;
        if self.active[claim.slot] != available {
            self.active[claim.slot] = available;
            self.set_available(claim.source, claim.resource, available, allocator, graph);
        }
    }

    /// Records a live owner's accepted input flows for a later replacement quote.
    /// O(claims), using the pass's preallocated sparse receipt arena.
    pub(crate) fn accept_for(&mut self, owner: usize) {
        let first = self.committed.len();
        self.committed.extend_from_slice(&self.staged);
        self.accept();
        self.owner_ranges[owner] = (first, self.committed.len());
    }

    /// Temporarily releases only this incumbent's accepted inputs. Rejection restores exact
    /// prior quantities; acceptance retires the receipt so it cannot be credited twice.
    /// O(owner claims * log bucket size), without scanning other owners or suppliers.
    pub(crate) fn release_owner(
        &mut self,
        owner: usize,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
    ) {
        debug_assert!(self.staged.is_empty() && self.released.is_empty());
        let Some(&(first, end)) = self.owner_ranges.get(owner) else {
            return;
        };
        self.releasing_owner = Some(owner);
        for index in first..end {
            let receipt = self.committed[index];
            let previous = self.remaining[receipt.slot];
            self.released.push(Claim {
                previous,
                ..receipt
            });
            self.remaining[receipt.slot] += receipt.units;
            if previous == 0.0 && receipt.units > 0.0 {
                self.active[receipt.slot] = true;
                self.set_available(receipt.source, receipt.resource, true, allocator, graph);
            }
        }
    }

    /// Replacement price of an input in the staged operation, including finite source mixing.
    pub(crate) fn input_price(&self, resource: ResourceRuntimeId) -> f32 {
        self.prices
            .iter()
            .find(|&&(id, _)| id == resource)
            .map_or(0.0, |&(_, price)| price)
    }

    /// Quotes and stages one input using exact directed routes and actual supplier prices.
    /// Repeated source searches skip depleted entries; an exhausted component costs O(log C).
    /// Imported residual supply requires a real border quote. Caller accepts or rejects the
    /// complete operation, so a later input/capital/income failure cannot consume local supply.
    pub(super) fn quote(
        &mut self,
        input: InputRequirement,
        buyer: Option<usize>,
        entrance: &BuildingEntrance,
        outside_quote: Option<f32>,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        network: &TransitNetwork,
        graph: &RegionGraph,
        routes: &mut FreightRouteCache,
        goods_multiplier: f32,
    ) -> InputSupplyCosts {
        let imported_price = catalog
            .unit_price_for_resource(input.resource)
            .unwrap_or(0.0)
            * tuning.owa_import_price_multiplier;
        if input.daily_units > 0.0 && !goods_multiplier.is_finite() {
            return InputSupplyCosts { goods: f32::INFINITY, buffer: f32::INFINITY,
                ..Default::default() };
        }
        let imported_price = imported_price
            * if goods_multiplier.is_finite() { goods_multiplier } else { 1.0 };
        if input.daily_units <= 0.0 {
            // Internally replenished circulation stock needs valuation, not a recurring order.
            self.prices.push((input.resource, imported_price));
            return input.at_prices(imported_price, 0.0);
        }
        let Some(batch) = input.replenishment_batch_units else {
            return InputSupplyCosts {
                goods: f32::INFINITY,
                ..Default::default()
            };
        };
        let mut needed = f64::from(input.daily_units);
        let mut result = InputSupplyCosts::default();
        while needed > 0.0 {
            let choice = self.next_offer(
                input.resource,
                buyer,
                entrance,
                allocator,
                catalog,
                network,
                graph,
                routes,
            );
            let Some((source, slot, seconds)) = choice else {
                break;
            };
            let supplier = &allocator.buildings[source];
            let profile = catalog
                .profile_by_runtime_id(supplier.economy_profile_runtime_id)
                .unwrap();
            let carrier = freight_cost(
                seconds,
                tuning.operational_clock.seconds_per_day,
                &tuning.logistics,
            );
            let local_price = profile.unit_price_currency * goods_multiplier;
            // A viable outside route remains available when the local offer costs more.
            if outside_quote.is_some_and(|outside| {
                imported_price + outside / batch < local_price + carrier / batch
            }) {
                break;
            }
            let previous = self.remaining[slot];
            let units = needed.min(previous);
            needed -= units;
            self.remaining[slot] -= units;
            if self.remaining[slot] == 0.0 {
                self.active[slot] = false;
                self.set_available(source, input.resource, false, allocator, graph);
            }
            self.staged.push(Claim {
                source,
                resource: input.resource,
                slot,
                previous,
                units,
            });
            result.goods += units as f32 * local_price;
            result.freight += units as f32 / batch * carrier;
            result.buffer += input.buffer_units
                * (units as f32 / input.daily_units)
                * local_price;
        }
        if needed > 0.0 {
            if let Some(carrier) = outside_quote {
                result.goods += needed as f32 * imported_price;
                result.freight += needed as f32 / batch * carrier;
                result.buffer +=
                    input.buffer_units * (needed as f32 / input.daily_units) * imported_price;
            } else {
                result.goods = f32::INFINITY;
            }
        }
        self.prices.push((
            input.resource,
            if result.goods.is_finite() {
                result.goods / input.daily_units
            } else {
                imported_price
            },
        ));
        result
    }

    fn next_offer(
        &mut self,
        resource: ResourceRuntimeId,
        buyer: Option<usize>,
        entrance: &BuildingEntrance,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        network: &TransitNetwork,
        graph: &RegionGraph,
        routes: &mut FreightRouteCache,
    ) -> Option<(usize, usize, f32)> {
        let list = self
            .input_offers
            .iter()
            .position(|offers| offers.resource == resource)
            .unwrap_or_else(|| {
                self.input_offers.push(InputOffers {
                    resource,
                    first: usize::MAX,
                    last: usize::MAX,
                    next: usize::MAX,
                    exhausted: false,
                });
                self.input_offers.len() - 1
            });
        while self.input_offers[list].next != usize::MAX {
            let offer = self.offers[self.input_offers[list].next];
            self.input_offers[list].next = offer.next;
            if self.remaining[offer.slot] > 0.0 {
                return Some((offer.source, offer.slot, offer.seconds));
            }
        }
        if self.input_offers[list].exhausted {
            return None;
        }
        let choice = self.nearest(
            resource, buyer, entrance, allocator, catalog, network, graph, routes,
        );
        if let Some((source, slot, seconds)) = choice {
            let next = self.offers.len();
            let previous = self.input_offers[list].last;
            if previous != usize::MAX {
                self.offers[previous].next = next;
            } else {
                self.input_offers[list].first = next;
            }
            self.input_offers[list].last = next;
            self.offers.push(Offer {
                source,
                slot,
                seconds,
                next: usize::MAX,
            });
        } else {
            self.input_offers[list].exhausted = true;
        }
        choice
    }

    fn nearest(
        &mut self,
        resource: ResourceRuntimeId,
        buyer: Option<usize>,
        entrance: &BuildingEntrance,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        network: &TransitNetwork,
        graph: &RegionGraph,
        routes: &mut FreightRouteCache,
    ) -> Option<(usize, usize, f32)> {
        let buckets = self.index.buckets_for_resource(resource)?;
        let components =
            self.components
                .as_ref()?
                .entrance_components(entrance, graph, TransitFlags::CAR);
        // Reprice the preceding buyer's supplier as an initial upper bound. Every
        // candidate that can beat it is still checked, so this changes search cost,
        // never route eligibility or deterministic route/price/index ordering.
        let incumbent = self.nearest_incumbents[resource as usize];
        let excluded = self.releasing_owner;
        let mut best = incumbent
            .filter(|&source| Some(source) != buyer && Some(source) != excluded)
            .and_then(|source| {
                let slot = self.slot(allocator, catalog, source, resource)?;
                if self.remaining[slot] <= 0.0 {
                    return None;
                }
                let seconds = if let Some(buyer) = buyer {
                    routes.between_buildings(source, buyer, allocator, network, graph)
                } else {
                    routes.to_site(source, entrance, allocator, network, graph)
                }?;
                Some((source, slot, seconds))
            });
        buckets.scan_nearest(
            components,
            entrance.door_pos.x,
            entrance.door_pos.y,
            |event| match event {
                ReachableBucketScanEvent::Item { item_idx: source } => {
                    if Some(source) == buyer
                        || Some(source) == excluded
                        || best.is_some_and(|best| best.0 == source)
                    {
                        return true;
                    }
                    let Some(slot) = self.slot(allocator, catalog, source, resource) else {
                        return true;
                    };
                    if self.remaining[slot] <= 0.0 {
                        return true;
                    }
                    if let Some((_, _, seconds)) = best {
                        let Some(origin) = allocator.entrances.get(source) else {
                            return true;
                        };
                        // The complete door route cannot beat straight-line travel at
                        // the fastest road/driveway speed. Avoid even a cached route
                        // lookup when this candidate cannot improve the incumbent.
                        let distance_sq = (origin.door_pos - entrance.door_pos).length_squared();
                        if lower_bound_travel_seconds(distance_sq, self.max_speed) > seconds {
                            return true;
                        }
                    }
                    let seconds = if let Some(buyer) = buyer {
                        routes.between_buildings(source, buyer, allocator, network, graph)
                    } else {
                        routes.to_site(source, entrance, allocator, network, graph)
                    };
                    if let Some(seconds) = seconds {
                        // Equal routes have the same carrier fee. Only read prices on
                        // a time tie; ordinary nearest-route searches need no extra lookup.
                        if best.is_none_or(|current| {
                            seconds.total_cmp(&current.2)
                                .then_with(|| {
                                    let price = |index: usize| {
                                        catalog.profile_by_runtime_id(
                                            allocator.buildings[index].economy_profile_runtime_id,
                                        ).unwrap().unit_price_currency
                                    };
                                    price(source).total_cmp(&price(current.0))
                                })
                                .then_with(|| source.cmp(&current.0))
                                .is_lt()
                        }) {
                            best = Some((source, slot, seconds));
                        }
                    }
                    true
                }
                ReachableBucketScanEvent::RingComplete {
                    next_min_distance_sq,
                } => best.is_none_or(|(_, _, seconds)| {
                    lower_bound_travel_seconds(next_min_distance_sq, self.max_speed) <= seconds
                }),
            },
        );
        self.nearest_incumbents[resource as usize] = best.map(|(source, _, _)| source);
        best
    }
}

// Prefix offsets split variable-length output rows into disjoint mutable ranges safely.
// Independent production arithmetic runs on Rayon; the dependent prefix itself stays serial.
fn fill_production_rates(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    funded: &[f32],
    offsets: &[usize],
    first: usize,
    end: usize,
    remaining: &mut [f64],
) {
    if end - first > 256 {
        let middle = first + (end - first) / 2;
        let (left, right) = remaining.split_at_mut(offsets[middle] - offsets[first]);
        rayon::join(
            || fill_production_rates(allocator, catalog, funded, offsets, first, middle, left),
            || fill_production_rates(allocator, catalog, funded, offsets, middle, end, right),
        );
        return;
    }
    for index in first..end {
        let building = &allocator.buildings[index];
        let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
        else {
            continue;
        };
        let activity = production_activity(building, profile, funded.get(index).copied().unwrap_or(0.0));
        let area = output_area_scale_for_building(building, profile);
        for (output, value) in profile.outputs.iter().zip(
            &mut remaining[offsets[index] - offsets[first]..offsets[index + 1] - offsets[first]],
        ) {
            *value = f64::from(profile.net_output_units_per_day(output) * area * activity);
        }
    }
}

fn production_activity(
    building: &crate::simulation::buildings::allocator::Building,
    profile: &EconomyProfileRuntime,
    funded: f32,
) -> f32 {
    if building.broken
        || building.economy_broken
        || building.is_deserted
        || building.is_under_construction()
        || building.edge_idx == usize::MAX
    {
        return 0.0;
    }
    let physical = physical_worker_capacity_for_profile(building, profile);
    (building.worker_count as f32 / physical.max(1) as f32)
        .min(funded)
        .clamp(0.0, 1.0)
}
