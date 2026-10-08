// SPDX-License-Identifier: GPL-2.0-only

//! Shared live-job forecast: recurring sales pay the operation; assets bridge settlement.

use rayon::prelude::*;

use super::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, RuntimeEconomyCatalog, RuntimeEconomyTuning,
};
use super::fiscal::{CityFiscalPolicy, daily_property_tax};
use super::households::{
    Household, local_utility_service_costs,
    output_area_scale_for_building, physical_worker_capacity_for_profile,
    service_funded_worker_capacity,
};
use super::logistics::{RecurringSupply, ShipmentSystem, building_outputs_can_export_to_owa, freight_goods_multiplier, freight_price_phase};
use super::operation::{
    OperatingCosts, OperationPlan, OperationPrices, working_capital_assets,
    working_capital_assets_at_prices,
};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};

const MARKET_CHUNK: usize = 256;

/// Reused building/resource scratch. Employment and admission read the same funded capacities.
#[derive(Default)]
pub(crate) struct JobFunding {
    /// Maximum live employees supported by both recurring sales and working capital.
    pub(crate) capacities: Vec<u32>,
    // Accepted recipe activity, which can be smaller than the rounded worker fraction.
    activities: Vec<f32>,
    /// Fiscal staffing ceilings, including unfinished services; these are not live jobs.
    pub(crate) service_capacities: Vec<u32>,
    /// Paid goods and unsettled sales available to the corresponding building.
    pub(crate) freight_capital: Vec<f64>,
    carrier_quotes: Vec<Option<f32>>,
    pricing_phase: u16,
    quote_revision: Option<(u64, u64, u32, u64, u64)>,
    border_nodes: Vec<u32>,
    quoted_border_nodes: Vec<u32>,
    // Resource slots contain full output capacity and recurring local demand, respectively.
    market_chunks: Vec<[f64; 2]>,
    market: Vec<[f64; 2]>,
    // Sparse output-port quotas; negative entries retain the non-household buyer path.
    customer_offsets: Vec<usize>,
    customer_units: Vec<f64>,
    // Stable counting-sort scratch: authored supplier phases, then live building index.
    local_order: Vec<usize>,
    phase_offsets: Vec<usize>,
    remapped_capacities: Vec<(usize, u32, u32, f64, f32)>,
    /// Finite recurring supply left after funded live operations have made their input claims.
    pub(crate) input_supply: RecurringSupply,
    /// Individual recurring household accounts prepared with this funding census.
    pub(crate) customers: super::customers::ConsumerMarket,
}

struct InputFunding<'a> {
    supply: &'a mut RecurringSupply,
    logistics: &'a mut ShipmentSystem,
    allocator: &'a BuildingAllocator,
    network: &'a TransitNetwork,
    graph: &'a RegionGraph,
    tuning: &'a RuntimeEconomyTuning,
    buyer: usize,
    outside: Option<f32>,
    imported: (u32, f32),
    goods_multiplier: f32,
}

impl InputFunding<'_> {
    fn costs(
        &mut self,
        profile: &EconomyProfileRuntime,
        catalog: &RuntimeEconomyCatalog,
        plan: OperationPlan,
        prices: OperationPrices,
        export_units: impl Fn(u16, f32) -> f32,
    ) -> Option<OperatingCosts> {
        self.supply.retry(self.allocator, self.graph);
        let entrance = self.allocator.entrances.get(self.buyer)?;
        Some(OperatingCosts::for_plan_with_input_quotes(
            profile,
            plan,
            prices,
            export_units,
            |input| {
                self.logistics.quote_recurring_input(
                    self.supply,
                    input,
                    Some(self.buyer),
                    entrance,
                    self.outside,
                    self.allocator,
                    catalog,
                    self.tuning,
                    self.network,
                    self.graph, self.goods_multiplier,
                )
            },
        ))
    }
}

fn live(building: &Building) -> bool {
    !building.broken
        && !building.economy_broken
        && !building.is_deserted
        && !building.is_under_construction()
        && building.edge_idx != usize::MAX
}

impl JobFunding {
    /// Net-input multiplier for an established funded buyer, capped by its current staff.
    /// Public invoices use the fiscal ceiling independently of material availability; private
    /// buyers use their accepted recipe activity. O(1), including explicit work-area scaling.
    pub(crate) fn input_demand_scale(
        &self,
        index: usize,
        building: &Building,
        profile: &EconomyProfileRuntime,
        service: bool,
    ) -> f32 {
        let physical = physical_worker_capacity_for_profile(building, profile).max(1);
        let activity = if service {
            self.capacities.get(index).copied().unwrap_or(0) as f32 / physical as f32
        } else {
            self.activities.get(index).copied().unwrap_or(0.0)
        };
        super::households::funded_input_demand_scale(building, profile, activity)
    }

    /// Registers a newly placed service's future input ceiling before the next hourly census.
    /// Existing entries stay intact. O(new indexed slots), O(1) when warm; creates no live job.
    pub(crate) fn register_pending_service(
        &mut self,
        index: usize,
        building: &Building,
        profile: &EconomyProfileRuntime,
        funding: f32,
    ) {
        self.service_capacities
            .resize(self.service_capacities.len().max(index + 1), 0);
        self.service_capacities[index] = service_funded_worker_capacity(
            physical_worker_capacity_for_profile(building, profile),
            profile,
            0,
            &[funding],
        );
        self.quote_revision = None;
    }

    /// Paid cargo assets prepared with the current live-job forecast, excluding carrier charges.
    pub(crate) fn paid_cargo_capital(&self, index: usize) -> f64 {
        self.freight_capital.get(index).copied().unwrap_or(0.0)
    }

    /// Whether an hourly construction pass can reuse the funded source book, including its
    /// frontage-priced freight costs. Geometry alone cannot validate recurring profitability.
    /// O(connected borders), called once per planning pass.
    pub(crate) fn has_current_input_supply(
        &self,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        borders: &[u32],
        pricing_phase: u16,
    ) -> bool {
        self.input_supply.matches_layout(allocator, network)
            && self.pricing_phase == pricing_phase
            && self.quote_revision
                == Some((
                    allocator.building_ref_revision(),
                    allocator.entrance_ref_revision(),
                    network.cch_graph.build_generation,
                    network.cch_graph.metric_revision,
                    network.frontage_delay_revision,
                ))
            && self.quoted_border_nodes == borders
    }

    /// Clears the live view without discarding reusable forecast storage.
    pub(crate) fn clear(&mut self) {
        self.capacities.clear();
        self.activities.clear();
        self.service_capacities.clear();
        self.freight_capital.clear();
        self.quote_revision = None;
        self.input_supply.invalidate();
        self.customers.clear();
    }

    /// Withdraws a removed employer immediately, before the next hourly forecast.
    pub(crate) fn invalidate_building(&mut self, index: usize) {
        if let Some(capacity) = self.capacities.get_mut(index) {
            *capacity = 0;
        }
        if let Some(activity) = self.activities.get_mut(index) {
            *activity = 0.0;
        }
        if let Some(capacity) = self.service_capacities.get_mut(index) {
            *capacity = 0;
        }
        if let Some(capital) = self.freight_capital.get_mut(index) {
            *capital = 0.0;
        }
        self.quote_revision = None;
        self.input_supply.invalidate();
    }

    /// Moves jobs and fiscal/cargo records in O(mapping size + new slots), including undo growth.
    /// Scratch is reused; unmoved buildings need no scan or copy.
    pub(crate) fn remap_buildings(&mut self, mapping: &std::collections::HashMap<usize, usize>) {
        self.remapped_capacities.clear();
        let mut required = 0;
        for (&old, &new) in mapping {
            required = required.max(new + 1);
            self.remapped_capacities.push((
                new,
                self.capacities.get(old).copied().unwrap_or(0),
                self.service_capacities.get(old).copied().unwrap_or(0),
                self.freight_capital.get(old).copied().unwrap_or(0.0),
                self.activities.get(old).copied().unwrap_or(0.0),
            ));
        }
        // Undo can reintroduce a last index after an hourly forecast shrank these arrays.
        // Empty, unprepared views stay empty rather than allocating city-sized scratch.
        for capacities in [&mut self.capacities, &mut self.service_capacities] {
            if !capacities.is_empty() {
                capacities.resize(capacities.len().max(required), 0);
            }
        }
        if !self.freight_capital.is_empty() {
            self.freight_capital
                .resize(self.freight_capital.len().max(required), 0.0);
        }
        if !self.activities.is_empty() {
            self.activities
                .resize(self.activities.len().max(required), 0.0);
        }
        for &old in mapping.keys() {
            if let Some(capacity) = self.capacities.get_mut(old) {
                *capacity = 0;
            }
            if let Some(activity) = self.activities.get_mut(old) {
                *activity = 0.0;
            }
            if let Some(capacity) = self.service_capacities.get_mut(old) {
                *capacity = 0;
            }
            if let Some(capital) = self.freight_capital.get_mut(old) {
                *capital = 0.0;
            }
        }
        for &(new, value, service, capital, activity) in &self.remapped_capacities {
            if let Some(capacity) = self.capacities.get_mut(new) {
                *capacity = value;
            }
            if let Some(capacity) = self.service_capacities.get_mut(new) {
                *capacity = service;
            }
            if let Some(value) = self.freight_capital.get_mut(new) {
                *value = capital;
            }
            if let Some(value) = self.activities.get_mut(new) {
                *value = activity;
            }
        }
        self.quote_revision = None;
        self.input_supply.invalidate();
    }

    /// Rebuilds once per employment pass, including passes with no current resident applicants.
    /// Market aggregation is O(B*ports + ceil(B/256)*R + households*consumer categories), with fixed parallel
    /// chunks and ordered f64 merges. Staffing checks descend from the market ceiling:
    /// commonly one operation per business, worst-case O(total job capacity * ports²),
    /// parallel for independent import ceilings. Household and supplier claims commit serially
    /// through finite component/chunk indices, plus directed route queries for visited buyers.
    /// Freight rounding makes costs non-monotone, so binary
    /// search cannot safely reject smaller operations. Consumers read capacities in O(1).
    pub(crate) fn refresh(
        &mut self,
        allocator: &BuildingAllocator,
        households: &[Household],
        logistics: &mut ShipmentSystem,
        network: &TransitNetwork,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        fiscal: &CityFiscalPolicy,
        service_funding: &[f32],
        treasury_balance: f64,
        minute_of_day: u16,
    ) {
        self.refresh_with_service_capacity(
            allocator,
            households,
            logistics,
            network,
            graph,
            catalog,
            tuning,
            fiscal,
            treasury_balance,
            |index, physical, profile| {
                service_funded_worker_capacity(physical, profile, index, service_funding)
            }, minute_of_day,
        );
    }

    /// Rebuilds the same live input market using the caller's authoritative service job limits.
    /// Construction execution uses current funded service capacities after index remaps, while
    /// employment supplies the fiscal funding fractions through `refresh` above.
    pub(crate) fn refresh_with_service_capacity(
        &mut self,
        allocator: &BuildingAllocator,
        households: &[Household],
        logistics: &mut ShipmentSystem,
        network: &TransitNetwork,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        fiscal: &CityFiscalPolicy,
        treasury_balance: f64,
        service_capacity: impl Fn(usize, u32, &EconomyProfileRuntime) -> u32 + Sync,
        minute_of_day: u16,
    ) {
        self.pricing_phase = freight_price_phase(tuning, minute_of_day);
        let count = allocator.buildings.len();
        self.capacities.resize(count, 0);
        self.activities.resize(count, 0.0);
        self.service_capacities.resize(count, 0);
        self.freight_capital.resize(count, 0.0);
        let revision = (
            allocator.building_ref_revision(),
            allocator.entrance_ref_revision(),
            network.cch_graph.build_generation,
            network.cch_graph.metric_revision,
            network.frontage_delay_revision,
        );
        super::logistics::collect_connected_border_nodes(graph, &mut self.border_nodes);
        if self.quote_revision != Some(revision)
            || self.carrier_quotes.len() != count
            || self.border_nodes != self.quoted_border_nodes
        {
            self.carrier_quotes.resize(count, None);
            logistics.write_border_carrier_quotes(
                allocator,
                network,
                graph,
                &self.border_nodes,
                catalog,
                tuning,
                &mut self.carrier_quotes,
            );
            self.quote_revision = Some(revision);
            self.quoted_border_nodes.clone_from(&self.border_nodes);
        }
        logistics.write_freight_working_capital(&mut self.freight_capital);
        let resources = catalog.resource_count() + 1;
        self.market.resize(resources, [0.0; 2]);
        self.market.fill([0.0; 2]);
        self.market_chunks
            .resize(count.div_ceil(MARKET_CHUNK) * resources, [0.0; 2]);
        self.market_chunks
            .par_chunks_mut(resources)
            .enumerate()
            .for_each(|(chunk, totals)| {
                totals.fill([0.0; 2]);
                let first = chunk * MARKET_CHUNK;
                for building in &allocator.buildings[first..(first + MARKET_CHUNK).min(count)] {
                    if !live(building) {
                        continue;
                    }
                    let Some(profile) =
                        catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
                    else {
                        continue;
                    };
                    let area = output_area_scale_for_building(building, profile);
                    let physical = physical_worker_capacity_for_profile(building, profile);
                    let activity = (building.worker_count as f32 / physical.max(1) as f32).min(1.0);
                    for output in &profile.outputs {
                        totals[output.resource_runtime_id as usize][0] +=
                            f64::from(profile.net_output_units_per_day(output) * area);
                    }
                    // Existing staffed buyers supply input demand. A proposed job cannot
                    // justify its own revenue by promising another unfunded employer's purchases.
                    for input in &profile.inputs {
                        let internal = profile
                            .output_port(input.resource_runtime_id)
                            .map_or(0.0, |port| port.units_per_day);
                        totals[input.resource_runtime_id as usize][1] +=
                            f64::from((input.units_per_day - internal).max(0.0) * area * activity);
                    }
                }
            });
        self.market
            .par_iter_mut()
            .enumerate()
            .for_each(|(resource, total)| {
                for chunk in self.market_chunks.chunks(resources) {
                    total[0] += chunk[resource][0];
                    total[1] += chunk[resource][1];
                }
            });
        self.customers.reset(
            households,
            allocator,
            catalog,
            tuning,
            fiscal,
            treasury_balance,
            network,
            graph,
        );
        let residents = self.customers.affordable_residents();
        for profile in catalog.all_profiles() {
            if profile.kind == EconomyProfileRuntimeKind::DemandSink {
                for input in &profile.inputs {
                    self.market[input.resource_runtime_id as usize][1] +=
                        f64::from(profile.consumption_rate_per_resident) * residents;
                }
            }
        }
        self.customer_offsets.resize(count + 1, 0);
        let mut ports = 0;
        for (index, building) in allocator.buildings.iter().enumerate() {
            self.customer_offsets[index] = ports;
            ports += catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
                .map_or(0, |profile| profile.outputs.len());
        }
        self.customer_offsets[count] = ports;
        self.customer_units.resize(ports, -1.0);
        self.customer_units.par_iter_mut().for_each(|units| *units = -1.0);
        let utilities = local_utility_service_costs(catalog)
            .map(|price| price * tuning.owa_import_price_multiplier);
        self.capacities
            .par_iter_mut()
            .zip(self.service_capacities.par_iter_mut())
            .zip(self.activities.par_iter_mut())
            .enumerate()
            .for_each(|(index, ((capacity, service), activity))| {
                *capacity = 0;
                *service = 0;
                *activity = 0.0;
                let building = &allocator.buildings[index];
                let Some(profile) =
                    catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
                else {
                    return;
                };
                let physical = physical_worker_capacity_for_profile(building, profile);
                let goods_multiplier = freight_goods_multiplier(profile, building.zone_type, tuning, minute_of_day);
                let prices = OperationPrices {
                    utilities,
                    property_tax: daily_property_tax(building.zone_type, building.level, fiscal),
                    imports: tuning.owa_import_price_multiplier * if goods_multiplier.is_finite() { goods_multiplier } else { 1.0 },
                    carrier_quote: self.carrier_quotes[index].filter(|_| goods_multiplier.is_finite()).unwrap_or(f32::INFINITY),
                    truck_payload: tuning.logistics.truck_payload_kg,
                };
                if allocator.is_city_service_building(building) {
                    *service = service_capacity(index, physical, profile).min(physical);
                    if live(building) {
                        *capacity = *service;
                        let planned = *service as f32 / physical.max(1) as f32;
                        let costs = OperatingCosts::for_plan(
                            profile,
                            catalog,
                            OperationPlan {
                                workers: *service,
                                activity: planned,
                                area: output_area_scale_for_building(building, profile),
                            },
                            prices,
                            |_, _| 0.0,
                        );
                        if costs.inputs.is_finite() && costs.freight.is_finite() {
                            *activity = planned;
                        }
                    }
                    return;
                }
                if !live(building) || physical == 0 {
                    return;
                }
                // Finite family claims commit serially below. Independent non-consumer
                // firms and public funding retain the parallel import-ceiling pass.
                if profile.outputs.iter().any(|port| self.customers.consumes(port.resource_runtime_id)) {
                    return;
                }
                let exports = building_outputs_can_export_to_owa(building, profile)
                    && self.carrier_quotes[index].is_some()
                    && tuning.owa_export_price_multiplier > 0.0;
                (*capacity, *activity) = funded_private_operation(
                    building,
                    profile,
                    catalog,
                    physical,
                    prices,
                    &self.market,
                    exports.then_some(tuning.owa_export_price_multiplier),
                    self.freight_capital[index],
                    None,
                    None,
                );
            });
        for (index, building) in allocator.buildings.iter().enumerate() {
            if !live(building) || allocator.is_city_service_building(building) { continue; }
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id) else { continue; };
            if !profile.outputs.iter().any(|port| self.customers.consumes(port.resource_runtime_id)) { continue; }
            let physical = physical_worker_capacity_for_profile(building, profile);
            if physical == 0 { continue; }
            let units = &mut self.customer_units[self.customer_offsets[index]..self.customer_offsets[index + 1]];
            reserve_consumer_outputs(&mut self.customers, units, index, building, profile,
                fiscal, allocator, network, graph);
            let goods_multiplier = freight_goods_multiplier(profile, building.zone_type, tuning, minute_of_day);
            let prices = OperationPrices {
                utilities,
                property_tax: daily_property_tax(building.zone_type, building.level, fiscal),
                imports: tuning.owa_import_price_multiplier * if goods_multiplier.is_finite() { goods_multiplier } else { 1.0 },
                carrier_quote: self.carrier_quotes[index].filter(|_| goods_multiplier.is_finite()).unwrap_or(f32::INFINITY),
                truck_payload: tuning.logistics.truck_payload_kg,
            };
            let exports = building_outputs_can_export_to_owa(building, profile)
                && self.carrier_quotes[index].is_some() && tuning.owa_export_price_multiplier > 0.0;
            (self.capacities[index], self.activities[index]) = funded_private_operation(
                building, profile, catalog, physical, prices, &self.market,
                exports.then_some(tuning.owa_export_price_multiplier), self.freight_capital[index],
                None, Some(units));
            if self.capacities[index] > 0 {
                self.customers.accept_for(index);
            } else {
                self.customers.reject();
            }
        }
        self.input_supply
            .reset(allocator, graph, network, catalog, &self.activities);
        if !self.input_supply.has_offers() {
            return;
        }
        // Stable counting sort is O(buildings + authored phases), with reusable scratch.
        // Counts and scatter cursors are dependent; keeping each phase in building order
        // makes finite acceptance deterministic without a comparison sort or citywide retry.
        self.local_order.resize(count, 0);
        self.phase_offsets.resize(catalog.funding_phase_count(), 0);
        self.phase_offsets.fill(0);
        for building in &allocator.buildings {
            self.phase_offsets[catalog.funding_phase(building.economy_profile_runtime_id)] += 1;
        }
        let mut first = 0;
        for offset in &mut self.phase_offsets {
            let end = first + *offset;
            *offset = first;
            first = end;
        }
        for (index, building) in allocator.buildings.iter().enumerate() {
            let next = &mut self.phase_offsets[catalog.funding_phase(building.economy_profile_runtime_id)];
            self.local_order[*next] = index;
            *next += 1;
        }
        // Finite source claims are dependent: suppliers precede downstream recipes;
        // cyclic recipes and sibling buyers retain stable building priority.
        // Independent import-funded ceilings above remain parallel; they establish which
        // currently staffed sources can offer recurring production without owner-funded sales.
        for &index in &self.local_order {
            let building = &allocator.buildings[index];
            if !live(building) {
                continue;
            }
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            if !self.input_supply.offers_inputs(profile) {
                continue;
            }
            let physical = physical_worker_capacity_for_profile(building, profile);
            if physical == 0 {
                continue;
            }
            let exports = building_outputs_can_export_to_owa(building, profile)
                && self.carrier_quotes[index].is_some()
                && tuning.owa_export_price_multiplier > 0.0;
            let goods_multiplier = freight_goods_multiplier(profile, building.zone_type, tuning, minute_of_day);
            let prices = OperationPrices {
                utilities,
                property_tax: daily_property_tax(building.zone_type, building.level, fiscal),
                imports: tuning.owa_import_price_multiplier * if goods_multiplier.is_finite() { goods_multiplier } else { 1.0 },
                carrier_quote: self.carrier_quotes[index].filter(|_| goods_multiplier.is_finite()).unwrap_or(f32::INFINITY),
                truck_payload: tuning.logistics.truck_payload_kg,
            };
            let imported_activity = self.activities[index];
            let consumer = profile.outputs.iter().any(|port| self.customers.consumes(port.resource_runtime_id));
            let units = &mut self.customer_units[self.customer_offsets[index]..self.customer_offsets[index + 1]];
            let new_customers = consumer && self.capacities[index] == 0;
            if new_customers {
                reserve_consumer_outputs(&mut self.customers, units, index, building, profile,
                    fiscal, allocator, network, graph);
            }
            let input_funding = InputFunding {
                supply: &mut self.input_supply,
                logistics,
                allocator,
                network,
                graph,
                tuning,
                buyer: index,
                outside: self.carrier_quotes[index],
                imported: (self.capacities[index], self.activities[index]),
                goods_multiplier,
            };
            if allocator.is_city_service_building(building) {
                // Public payroll follows fiscal policy even when material delivery limits
                // production. Reserve only the feasible input-backed part of that operation.
                self.activities[index] = reserve_service_inputs(
                    profile,
                    catalog,
                    self.capacities[index],
                    physical,
                    prices,
                    input_funding,
                );
                continue;
            }
            (self.capacities[index], self.activities[index]) = funded_private_operation(
                building,
                profile,
                catalog,
                physical,
                prices,
                &self.market,
                exports.then_some(tuning.owa_export_price_multiplier),
                self.freight_capital[index],
                Some(input_funding),
                consumer.then_some(units),
            );
            if new_customers {
                if self.capacities[index] > 0 { self.customers.accept_for(index); }
                else { self.customers.reject(); }
            }
            self.input_supply.publish_funded_increase(index, imported_activity,
                self.activities[index], allocator, catalog, graph);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_remap_grows_prepared_funding_without_allocating_unprepared_views() {
        let mapping = [(0, 2)].into();
        let mut jobs = JobFunding {
            capacities: vec![7, 3],
            activities: vec![0.25, 0.4],
            service_capacities: vec![7, 0],
            freight_capital: vec![9.0, 2.0],
            ..Default::default()
        };
        jobs.remap_buildings(&mapping);
        assert_eq!(jobs.capacities, [0, 3, 7]);
        assert_eq!(jobs.activities, [0.0, 0.4, 0.25]);
        assert_eq!(jobs.service_capacities, [0, 0, 7]);
        assert_eq!(jobs.freight_capital, [0.0, 2.0, 9.0]);
        jobs.clear();
        jobs.remap_buildings(&mapping);
        assert!(jobs.capacities.is_empty());
        assert!(jobs.activities.is_empty());
        assert!(jobs.service_capacities.is_empty());
        assert!(jobs.freight_capital.is_empty());
    }

    #[test]
    fn funded_jobs_follow_remapped_buildings_and_clear_removed_slots() {
        let mut jobs = JobFunding {
            capacities: vec![3, 7, 11],
            activities: vec![0.25, 0.4, 0.5],
            service_capacities: vec![0, 7, 0],
            freight_capital: vec![2.0, 5.0, 9.0],
            ..Default::default()
        };
        jobs.remap_buildings(&[(0, 1), (1, 0)].into());
        assert_eq!(jobs.capacities, [7, 3, 11]);
        assert_eq!(jobs.activities, [0.4, 0.25, 0.5]);
        assert_eq!(jobs.service_capacities, [7, 0, 0]);
        assert_eq!(jobs.freight_capital, [5.0, 2.0, 9.0]);
        jobs.invalidate_building(0);
        jobs.remap_buildings(&[(2, 0)].into());
        assert_eq!(jobs.capacities, [11, 3, 0]);
        assert_eq!(jobs.activities, [0.5, 0.25, 0.0]);
        assert_eq!(jobs.service_capacities, [0, 0, 0]);
        assert_eq!(jobs.freight_capital, [9.0, 2.0, 0.0]);
        jobs.clear();
        assert!(jobs.capacities.is_empty());
        assert!(jobs.activities.is_empty());
        assert!(jobs.service_capacities.is_empty());
        assert!(jobs.freight_capital.is_empty());
    }
}

// Public service input invoices draw on the treasury under the existing fiscal policy.
// They do not require private sales or owner equity. Integer activity probes reuse the same
// finite quote/retry scratch; worst-case O(funded workers * recipe ports² + source searches).
fn reserve_service_inputs(
    profile: &EconomyProfileRuntime,
    catalog: &RuntimeEconomyCatalog,
    funded: u32,
    physical: u32,
    prices: OperationPrices,
    mut supply: InputFunding<'_>,
) -> f32 {
    for workers in (1..=funded.min(physical)).rev() {
        let Some(costs) = supply.costs(
            profile,
            catalog,
            OperationPlan {
                workers,
                activity: workers as f32 / physical.max(1) as f32,
                area: 1.0,
            },
            prices,
            |_, _| 0.0,
        ) else {
            return 0.0;
        };
        if costs.inputs.is_finite() && costs.freight.is_finite() {
            supply.supply.accept_for(supply.buyer);
            return workers as f32 / physical.max(1) as f32;
        }
    }
    supply.supply.reject(supply.allocator, supply.graph);
    0.0
}

// Keep the full reachable net-output reservation through staffing searches. A rejected
// operation releases every family claim; a cheaper local-input retry can reuse accepted quotas.
fn reserve_consumer_outputs(
    customers: &mut super::customers::ConsumerMarket,
    units: &mut [f64],
    index: usize,
    building: &Building,
    profile: &EconomyProfileRuntime,
    fiscal: &CityFiscalPolicy,
    allocator: &BuildingAllocator,
    network: &TransitNetwork,
    graph: &RegionGraph,
) {
    let area = output_area_scale_for_building(building, profile);
    for (quota, output) in units.iter_mut().zip(&profile.outputs) {
        if !customers.consumes(output.resource_runtime_id) {
            *quota = -1.0;
            continue;
        }
        *quota = allocator.entrances.get(index).map_or(0.0, |entrance| {
            customers.claim_reachable(entrance, output.resource_runtime_id,
                f64::from(profile.net_output_units_per_day(output) * area),
                profile.unit_price_currency * (1.0 + fiscal.household_vat_rate),
                allocator, network, graph)
        });
    }
}

fn local_output_price(
    profile: &EconomyProfileRuntime,
    catalog: &RuntimeEconomyCatalog,
    consumer_units: Option<&[f64]>,
    resource: u16,
) -> f32 {
    if profile.outputs.iter().position(|port| port.resource_runtime_id == resource)
        .is_some_and(|port| consumer_units.is_some_and(|units| units[port] >= 0.0)) {
        profile.unit_price_currency
    } else {
        catalog.unit_price_for_resource(resource).unwrap_or(0.0)
    }
}

fn funded_private_operation(
    building: &Building,
    profile: &EconomyProfileRuntime,
    catalog: &RuntimeEconomyCatalog,
    physical: u32,
    prices: OperationPrices,
    market: &[[f64; 2]],
    export_multiplier: Option<f32>,
    freight_capital: f64,
    mut supply: Option<InputFunding<'_>>,
    consumer_units: Option<&[f64]>,
) -> (u32, f32) {
    let area = output_area_scale_for_building(building, profile);
    let local_share = |resource: u16| {
        if let Some((port, output)) = profile.outputs.iter().enumerate()
            .find(|(_, output)| output.resource_runtime_id == resource)
            && let Some(units) = consumer_units.map(|units| units[port]).filter(|units| *units >= 0.0) {
                let capacity = profile.net_output_units_per_day(output) * area;
                return if capacity > 0.0 { (units / f64::from(capacity)).clamp(0.0, 1.0) as f32 } else { 0.0 };
            }
        let [capacity, demand] = market.get(resource as usize).copied().unwrap_or([0.0; 2]);
        if capacity > 0.0 {
            (demand / capacity).clamp(0.0, 1.0) as f32
        } else {
            0.0
        }
    };
    let market_activity = if export_multiplier.is_some() {
        1.0
    } else {
        profile
            .outputs
            .iter()
            .map(|port| local_share(port.resource_runtime_id))
            .fold(0.0, f32::max)
    };
    let desired = (physical as f32 * market_activity).ceil() as u32;
    if desired == 0 {
        return (0, 0.0);
    }
    let local_units = |resource, units: f32| {
        let quota = profile.output_port(resource).map_or(0.0, |output| {
            profile.net_output_units_per_day(output) * area * local_share(resource)
        });
        units.min(quota)
    };
    let export_units = |resource, units: f32| {
        if export_multiplier.is_some() {
            units - local_units(resource, units)
        } else {
            0.0
        }
    };
    let output_price = |resource| {
        let share = local_share(resource);
        if consumer_units.is_none() {
            return catalog.unit_price_for_resource(resource).unwrap_or(0.0)
                * (share + (1.0 - share) * export_multiplier.unwrap_or(0.0));
        }
        local_output_price(profile, catalog, consumer_units, resource) * share
            + catalog.unit_price_for_resource(resource).unwrap_or(0.0)
                * (1.0 - share) * export_multiplier.unwrap_or(0.0)
    };
    let imported_capital = working_capital_assets(
        building,
        profile,
        catalog,
        prices.imports,
        freight_capital,
        output_price,
    );
    let costs = |workers| {
        OperatingCosts::for_plan(
            profile,
            catalog,
            OperationPlan {
                workers,
                activity: market_activity.min(workers as f32 / physical.max(1) as f32),
                area,
            },
            prices,
            export_units,
        )
    };
    // Integer payroll, local sales quotas and quantized freight do not form a monotone
    // profitability test. Evaluate the largest feasible operation first, without allowing
    // an unprofitable full-capacity export plan to erase a profitable smaller business.
    // A cheaper source also lowers the replacement value of retained stock. Keep the
    // already validated import plan when that repricing cannot finance equal staffing;
    // the source book may already have promised its import-backed production to buyers.
    let imported = supply.as_ref().map_or((0, 0.0), |context| context.imported);
    for workers in (imported.0.max(1)..=desired).rev() {
        let costs = if let Some(context) = supply.as_mut() {
            let Some(costs) = context.costs(
                profile,
                catalog,
                OperationPlan {
                    workers,
                    activity: market_activity.min(workers as f32 / physical.max(1) as f32),
                    area,
                },
                prices,
                export_units,
            ) else {
                break;
            };
            costs
        } else {
            costs(workers)
        };
        let capital = if let Some(context) = supply.as_ref() {
            working_capital_assets_at_prices(
                building,
                profile,
                freight_capital,
                |resource| context.supply.input_price(resource),
                output_price,
            )
        } else {
            imported_capital
        };
        if f64::from(costs.working_capital) > capital {
            continue;
        }
        let activity = market_activity.min(workers as f32 / physical.max(1) as f32);
        let revenue: f32 = profile
            .outputs
            .iter()
            .map(|output| {
                let units = profile.net_output_units_per_day(output) * area * activity;
                let external = export_units(output.resource_runtime_id, units);
                let local = local_units(output.resource_runtime_id, units);
                if consumer_units.is_none() {
                    return catalog.unit_price_for_resource(output.resource_runtime_id).unwrap_or(0.0)
                        * (local + external * export_multiplier.unwrap_or(0.0));
                }
                local_output_price(profile, catalog, consumer_units, output.resource_runtime_id) * local + catalog
                    .unit_price_for_resource(output.resource_runtime_id)
                    .unwrap_or(0.0)
                    * external * export_multiplier.unwrap_or(0.0)
            })
            .sum();
        if revenue > costs.total() {
            if let Some(context) = supply.as_mut() {
                context.supply.accept_for(context.buyer);
            }
            return (workers, activity);
        }
    }
    if let Some(context) = supply.as_mut() {
        context.supply.reject(context.allocator, context.graph);
    }
    imported
}
