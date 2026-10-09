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
    FundedOperation, OperatingCosts, OperationPlan, OperationPrices, working_capital_assets,
    working_capital_assets_at_prices,
};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};

const MARKET_CHUNK: usize = 256;

mod closure;
mod household_claims;

/// Reused building/resource scratch. Employment and admission read the same funded capacities.
#[derive(Default)]
pub(crate) struct JobFunding {
    /// Maximum live employees supported by both recurring sales and working capital.
    pub(crate) capacities: Vec<u32>,
    /// Complete closure attempts in the latest census, for exact scaling diagnostics.
    #[cfg(test)]
    pub(crate) closure_passes: usize,
    /// Positive customer-limit propagation rounds summed over the latest census.
    #[cfg(test)]
    pub(crate) propagation_rounds: usize,
    // Accepted recipe activity, which can be smaller than the rounded worker fraction.
    activities: Vec<f32>,
    // Recurring profit and working-capital surplus of each accepted operation quote.
    // These are financing limits, not additional sales; customer validation remains separate.
    headroom: Vec<[f64; 2]>,
    // Whether the current-staff quote replaced its import alternative. Only that
    // accepted local operation can restore its receipts as an expansion fallback.
    local_input_backing: Vec<bool>,
    // Registry classification is immutable within one census, including closure retries.
    service_flags: Vec<bool>,
    // Provisional quotes remain private until the joint customer/source proof succeeds.
    closure_ceilings: Vec<u32>,
    // Joint customer limits retain fractional production independently of rounded payroll.
    closure_activity: Vec<f32>,
    // Necessary customer upper bounds propagate complete buyer withdrawal and partial
    // purchase limits without a fresh city-wide quotation for every supplier-chain link.
    potential_customers: closure::PotentialCustomers,
    offered_activity: Vec<f32>,
    // A physical repair touches only buyers of withdrawn provisional production.
    input_repairs: Vec<usize>,
    input_repair_seen: Vec<bool>,
    input_repair_headroom: Vec<[f64; 2]>,
    /// Jointly financed customer commitments, forked with the input book for projects.
    pub(crate) business: super::customers::business::BusinessMarket,
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
    // Exact quote/outcome prefixes, retained only across matching budget/access/request inputs.
    household_quotes: household_claims::HouseholdQuotes,
    // Staffed output already promised to households, excluded from business input offers.
    household_production: Vec<f64>,
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
    retained: (u32, f32),
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
    /// Forces ordinary household quotation in tests comparing the exact closure outcomes.
    #[cfg(test)]
    pub(crate) fn disable_household_quote_replay(&mut self) {
        self.household_quotes.disabled = true;
    }

    /// Uses ordinary full closure replay as the oracle for zero-customer propagation tests.
    #[cfg(test)]
    pub(crate) fn disable_customer_cascade(&mut self) {
        self.potential_customers.disabled = true;
    }

    /// Latest census's reused requests and changed-prefix reconstructions, for regressions.
    #[cfg(test)]
    pub(crate) fn household_quote_replay_counts(&self) -> (usize, usize) {
        (self.household_quotes.reused, self.household_quotes.rebuilt)
    }

    /// Requests replayed from the preceding census, excluding retries inside this census.
    #[cfg(test)]
    pub(crate) fn household_cross_census_replays(&self) -> usize {
        self.household_quotes.census_reused
    }

    /// Retains one accepted live operation for affected-input repricing during construction.
    /// O(1) apart from authored recipe-port area lookup; callers copy rows once per pass.
    pub(crate) fn funded_operation(&self, index: usize, building: &Building,
        profile: &EconomyProfileRuntime, utilities: [f32; 3], tuning: &RuntimeEconomyTuning,
        fiscal: &CityFiscalPolicy, minute: u16) -> Option<FundedOperation> {
        let workers = self.capacities.get(index).copied().unwrap_or(0);
        if workers == 0 { return None; }
        Some(FundedOperation {
            profile: profile.runtime_id,
            plan: OperationPlan { workers, activity: self.activities[index],
                area: output_area_scale_for_building(building, profile) },
            prices: closure::operation_prices(building, self.carrier_quotes[index],
                freight_goods_multiplier(profile, building.zone_type, tuning, minute), utilities, tuning, fiscal),
            headroom: self.headroom[index],
        })
    }

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
        self.headroom.clear();
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
        if let Some(headroom) = self.headroom.get_mut(index) { *headroom = [0.0; 2]; }
        self.quote_revision = None;
        self.input_supply.invalidate();
    }

    /// Moves jobs and fiscal/cargo records in O(mapping size + new slots), including undo growth.
    /// Scratch is reused; unmoved buildings need no scan or copy.
    pub(crate) fn remap_buildings(&mut self, mapping: &std::collections::HashMap<usize, usize>) {
        // A remap invalidates the complete input/customer quotation below.
        self.headroom.clear();
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
        let timing = crate::debug::category_enabled("economy");
        let mut phase = timing.then(std::time::Instant::now);
        let mut times = [0.0; 7];
        let mut lap = |slot: usize| {
            if let Some(previous) = phase {
                times[slot] += previous.elapsed().as_secs_f64() * 1000.0;
                phase = Some(std::time::Instant::now());
            }
        };
        self.pricing_phase = freight_price_phase(tuning, minute_of_day);
        let count = allocator.buildings.len();
        self.capacities.resize(count, 0);
        self.activities.resize(count, 0.0);
        self.headroom.resize(count, [0.0; 2]);
        self.local_input_backing.resize(count, false);
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
        lap(0);
        let resources = catalog.resource_count() + 1;
        self.potential_customers.prepare(count);
        self.market.resize(resources, [0.0; 2]);
        self.market.fill([0.0; 2]);
        self.market_chunks
            .resize(count.div_ceil(MARKET_CHUNK) * resources, [0.0; 2]);
        self.market_chunks
            .par_chunks_mut(resources)
            .zip(self.potential_customers.counted.par_chunks_mut(MARKET_CHUNK))
            .enumerate()
            .for_each(|(chunk, (totals, counted))| {
                totals.fill([0.0; 2]);
                let first = chunk * MARKET_CHUNK;
                for (building, counted) in allocator.buildings[first..(first + MARKET_CHUNK).min(count)]
                    .iter().zip(counted) {
                    *counted = false;
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
                    *counted = building.worker_count > 0 && physical > 0 && !profile.inputs.is_empty();
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
        lap(1);
        let retained_customers = self.customers.refresh(
            households,
            allocator,
            catalog,
            tuning,
            fiscal,
            treasury_balance,
            network,
            graph,
        );
        lap(2);
        self.customer_offsets.resize(count + 1, 0);
        self.local_order.resize(count, 0);
        self.phase_offsets.resize(catalog.funding_phase_count(), 0);
        self.phase_offsets.fill(0);
        let mut ports = 0;
        let mut inputs = 0;
        let mut consumer_firms = 0;
        let mut consumer_ports = 0;
        let mut maximum_consumer_ports = 0;
        for (index, building) in allocator.buildings.iter().enumerate() {
            self.customer_offsets[index] = ports;
            self.phase_offsets[catalog.funding_phase(building.economy_profile_runtime_id)] += 1;
            if let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id) {
                ports += profile.outputs.len();
                inputs += profile.inputs.len();
                if profile.outputs.iter().any(|port| self.customers.consumes(port.resource_runtime_id)) {
                    consumer_firms += 1;
                    consumer_ports += profile.outputs.len();
                    maximum_consumer_ports = maximum_consumer_ports.max(profile.outputs.len());
                }
            }
        }
        self.customer_offsets[count] = ports;
        // Immutable recipe order is prepared once, not once per closure attempt. Counts
        // and scatter cursors are dependent; stable building order keeps claims deterministic.
        let mut first = 0;
        for offset in &mut self.phase_offsets {
            let end = first + *offset;
            *offset = first;
            first = end;
        }
        self.potential_customers.reserve_inputs(inputs);
        for (index, building) in allocator.buildings.iter().enumerate() {
            if self.potential_customers.counted[index] { self.potential_customers.owners.push(index); }
            let next = &mut self.phase_offsets[catalog.funding_phase(building.economy_profile_runtime_id)];
            self.local_order[*next] = index;
            *next += 1;
        }
        self.customer_units.resize(ports, -1.0);
        let household_census = household_claims::Census::new(allocator, catalog, fiscal, network, graph);
        #[cfg(test)]
        let retained_customers = if self.household_quotes.disabled {
            if retained_customers {
                self.customers.reset(households, allocator, catalog, tuning, fiscal,
                    treasury_balance, network, graph);
            }
            false
        } else { retained_customers };
        self.household_quotes.prepare(consumer_firms, consumer_ports, maximum_consumer_ports, retained_customers);
        self.household_production.resize(ports, 0.0);
        self.customer_units.par_iter_mut().zip(&mut self.household_production)
            .with_min_len(MARKET_CHUNK).for_each(|(units, production)| {
                *units = -1.0;
                *production = 0.0;
            });
        let utilities = local_utility_service_costs(catalog)
            .map(|price| price * tuning.owa_import_price_multiplier);
        self.closure_ceilings.resize(count, 0);
        self.closure_activity.resize(count, 1.0);
        self.service_flags.resize(count, false);
        self.offered_activity.resize(count, 1.0);
        self.closure_ceilings.par_iter_mut().zip(&mut self.closure_activity)
            .zip(&mut self.offered_activity).zip(&mut self.service_flags)
            .zip(&allocator.buildings).zip(&self.freight_capital).with_min_len(MARKET_CHUNK)
            .for_each(|(((((ceiling, activity), offered), service), building), &cargo)| {
                *service = allocator.is_city_service_building(building);
                *ceiling = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
                    .filter(|profile| *service || !cannot_fund_one_worker(building, profile, cargo))
                    .map_or(0, |profile| physical_worker_capacity_for_profile(building, profile));
                *activity = 1.0;
                *offered = 1.0;
            });
        #[cfg(test)]
        { self.closure_passes = 0; self.propagation_rounds = 0; }
        let mut first = true;
        loop {
        #[cfg(test)]
        { self.closure_passes += 1; }
        if !first {
            self.household_quotes.restart(&mut self.customers, &household_census);
            self.customer_units.fill(-1.0);
        }
        first = false;
        self.capacities
            .par_iter_mut()
            .zip(self.service_capacities.par_iter_mut())
            .zip(self.activities.par_iter_mut())
            .zip(self.headroom.par_iter_mut())
            .zip(self.local_input_backing.par_iter_mut())
            .enumerate()
            .for_each(|(index, ((((capacity, service), activity), headroom), local_backing))| {
                *capacity = 0;
                *service = 0;
                *activity = 0.0;
                *headroom = [0.0; 2];
                *local_backing = false;
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
                if self.service_flags[index] {
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
                if !live(building) || physical == 0 || self.closure_ceilings[index] == 0 {
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
                    headroom,
                    self.closure_ceilings[index],
                    self.closure_activity[index],
                );
            });
        lap(3);
        for (index, building) in allocator.buildings.iter().enumerate() {
            if !live(building) || self.service_flags[index] { continue; }
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id) else { continue; };
            if !profile.outputs.iter().any(|port| self.customers.consumes(port.resource_runtime_id)) { continue; }
            let physical = physical_worker_capacity_for_profile(building, profile);
            if physical == 0 || self.closure_ceilings[index] == 0 { continue; }
            let units = &mut self.customer_units[self.customer_offsets[index]..self.customer_offsets[index + 1]];
            self.household_quotes.reserve(index, &mut self.customers, units, &household_census);
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
                None, Some(units), &mut self.headroom[index], self.closure_ceilings[index],
                self.closure_activity[index]);
            self.household_quotes.finish(self.capacities[index] > 0,
                &mut self.customers, &household_census);
        }
        lap(4);
        self.input_supply
            .reset(allocator, graph, network, catalog, &self.offered_activity);
        self.reserve_household_production(allocator, catalog, graph, true);
        lap(5);
        // Reserve current operations before offering inputs to prospective employees.
        // Supplier phases and building order remain stable within both dependent passes.
        // Fully staffed operations already searched their complete range in the first pass.
        for current_staff in [true, false] {
            for &index in &self.local_order {
                let building = &allocator.buildings[index];
                if !live(building) {
                    continue;
                }
                let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
                else {
                    continue;
                };
                let physical = physical_worker_capacity_for_profile(building, profile);
                let service = self.service_flags[index];
                let full_ceiling = if service { self.capacities[index] } else { self.closure_ceilings[index] };
                if physical == 0 || full_ceiling == 0
                    || (current_staff && building.worker_count == 0)
                    || (!current_staff && building.worker_count >= full_ceiling) {
                    continue;
                }
                if !current_staff {
                    // Only this operation's current claim is returned for its larger quote.
                    // Other staffed buyers retain their inputs throughout the expansion search.
                    self.input_supply.release_owner(index, allocator, graph);
                    // A rejected expansion restores the receipts backing its retained plan.
                    // A partial local quote that lost to imports has no such backing: its
                    // cheaper inventory valuation cannot silently replace the import proof.
                    if !self.local_input_backing[index] { self.input_supply.accept(); }
                }
                if !self.input_supply.offers_inputs(profile) {
                    self.input_supply.reject(allocator, graph);
                    continue;
                }
                let ceiling = if current_staff { full_ceiling.min(building.worker_count) } else { full_ceiling };
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
                let consumer = profile.outputs.iter().any(|port| self.customers.consumes(port.resource_runtime_id));
                let units = &mut self.customer_units[self.customer_offsets[index]..self.customer_offsets[index + 1]];
                let new_customers = consumer && self.capacities[index] == 0;
                if new_customers {
                    self.household_quotes.reserve(index, &mut self.customers, units, &household_census);
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
                    retained: if current_staff { (0, 0.0) } else { (self.capacities[index], self.activities[index]) },
                    goods_multiplier,
                };
                if service {
                    // Public payroll follows fiscal policy even when material delivery limits
                    // production. Reserve only the feasible input-backed part of that operation.
                    let activity = reserve_service_inputs(
                        profile,
                        catalog,
                        ceiling,
                        physical,
                        prices,
                        input_funding,
                    );
                    self.activities[index] = if current_staff { self.activities[index].max(activity) } else { activity };
                    continue;
                }
                let mut headroom = self.headroom[index];
                let operation = funded_private_operation(
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
                    &mut headroom,
                    ceiling,
                    self.closure_activity[index],
                );
                if !current_staff || operation.0 >= self.capacities[index] {
                    (self.capacities[index], self.activities[index]) = operation;
                    self.headroom[index] = headroom;
                    if current_staff { self.local_input_backing[index] = operation.0 > 0; }
                } else if building.worker_count >= full_ceiling {
                    // The complete local search lost to the existing import alternative.
                    self.input_supply.release_owner(index, allocator, graph);
                    self.input_supply.accept();
                }
                if new_customers {
                    self.household_quotes.finish(self.capacities[index] > 0,
                        &mut self.customers, &household_census);
                }
            }
        }
        self.household_quotes.finish_pass(&mut self.customers, &household_census);
        for index in 0..count {
            self.input_supply.publish_funded_activity(index, self.offered_activity[index],
                self.activities[index], allocator, catalog, graph);
        }
        self.reserve_household_production(allocator, catalog, graph, false);
        // Withdrawn provisional suppliers can still own input receipts until the next
        // quotation. Those invalid purchases hide customers from alternative suppliers;
        // using them to lower permanent worker ceilings can strand a fully financed chain.
        // Requote physical claims first, then derive ceilings from a consistent input book.
        if (!self.input_supply.has_overclaims()
            || self.repair_provisional_inputs(allocator, logistics, network, graph,
                catalog, tuning, fiscal, minute_of_day))
            && !self.validate_business_customers(allocator, logistics, network, graph,
                catalog, tuning, fiscal, minute_of_day, inputs) {
            self.publish_business_deliveries(allocator, catalog, graph);
            break;
        }
        self.propagate_impossible_customers(allocator, catalog, tuning);
        self.offered_activity.clone_from(&self.activities);
        // Attribute failed local proofs to this pass before the next import pass starts.
        lap(6);
        }
        lap(6);
        if timing { log_funding_times(count, minute_of_day, &times); }
    }
}

fn log_funding_times(buildings: usize, minute: u16, times: &[f64; 7]) {
    crate::debug_log!("economy", "funding_detail buildings={} minute={} routes_ms={:.3} market_ms={:.3} customers_ms={:.3} import_funding_ms={:.3} household_claims_ms={:.3} sources_ms={:.3} local_funding_ms={:.3}",
        buildings, minute, times[0], times[1], times[2], times[3], times[4], times[5], times[6]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_business_sales_use_seller_prices_while_exports_use_the_outside_tariff() {
        use crate::simulation::buildings::allocator::indexed_test_building;
        use crate::simulation::economy::definitions::load_runtime_economy_catalog;
        use crate::simulation::zoning::ZoneType;

        let catalog = load_runtime_economy_catalog().unwrap();
        let mut profile = catalog.profile_for_id("food_processor_basic").unwrap().clone();
        let mut building = indexed_test_building(String::new(), ZoneType::Industrial, 0);
        building.operating_budget = 1_000_000.0;
        let physical = profile.worker_capacity;
        let prices = OperationPrices {
            utilities: [0.0; 3], property_tax: 0.0, imports: 1.0,
            carrier_quote: 0.0, truck_payload: 20_000.0,
        };
        let mut market = vec![[0.0; 2]; catalog.resource_count() + 1];
        for output in &profile.outputs {
            let units = f64::from(profile.net_output_units_per_day(output));
            market[output.resource_runtime_id as usize] = [units, units];
        }
        profile.unit_price_currency = 0.0;
        let mut headroom = [0.0; 2];
        assert_eq!(funded_private_operation(&building, &profile, &catalog, physical,
            prices, &market, None, 0.0, None, None, &mut headroom, physical, 1.0), (0, 0.0));
        assert_eq!(headroom, [0.0; 2]);
        profile.unit_price_currency = 1_000.0;
        assert_eq!(funded_private_operation(&building, &profile, &catalog, physical,
            prices, &market, None, 0.0, None, None, &mut headroom, physical, 1.0), (physical, 1.0));
        assert!(headroom[0] > 0.0 && headroom[1] >= 0.0);
        market.iter_mut().for_each(|total| total[1] = 0.0);
        let expensive = funded_private_operation(&building, &profile, &catalog, physical,
            prices, &market, Some(1.0), 0.0, None, None, &mut headroom, physical, 1.0);
        profile.unit_price_currency = 0.0;
        assert_eq!(funded_private_operation(&building, &profile, &catalog, physical,
            prices, &market, Some(1.0), 0.0, None, None, &mut headroom, physical, 1.0), expensive);
        assert!(expensive.0 > 0);
    }

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
            supply.supply.accept_growing_for(supply.buyer);
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

// The same provisional sales and stock valuation support both a staffing search and
// an unchanged operation whose withdrawn input supplier is being replaced.
struct ProvisionalSales<'a> {
    profile: &'a EconomyProfileRuntime,
    catalog: &'a RuntimeEconomyCatalog,
    area: f32,
    market: &'a [[f64; 2]],
    household: Option<&'a [f64]>,
    export_multiplier: Option<f32>,
}

impl ProvisionalSales<'_> {
    fn share(&self, resource: u16) -> f32 {
        provisional_local_share(self.profile, self.area, resource, self.household, self.market)
    }

    fn local_units(&self, resource: u16, units: f32) -> f32 {
        let quota = self.profile.output_port(resource).map_or(0.0, |output| {
            self.profile.net_output_units_per_day(output) * self.area * self.share(resource)
        });
        units.min(quota)
    }

    fn export_units(&self, resource: u16, units: f32) -> f32 {
        if self.export_multiplier.is_some() { units - self.local_units(resource, units) }
        else { 0.0 }
    }

    fn output_price(&self, resource: u16) -> f64 {
        let share = self.share(resource);
        f64::from(self.profile.unit_price_currency) * f64::from(share)
            + f64::from(self.catalog.unit_price_for_resource(resource).unwrap_or(0.0)
                * self.export_multiplier.unwrap_or(0.0)) * f64::from(1.0 - share)
    }

    fn revenue(&self, activity: f32) -> f64 {
        self.profile.outputs.iter().map(|output| {
            let resource = output.resource_runtime_id;
            let units = self.profile.net_output_units_per_day(output) * self.area * activity;
            f64::from(self.profile.unit_price_currency) * f64::from(self.local_units(resource, units))
                + f64::from(self.catalog.unit_price_for_resource(resource).unwrap_or(0.0)
                    * self.export_multiplier.unwrap_or(0.0)) * f64::from(self.export_units(resource, units))
        }).sum()
    }
}

// Every private plan's working capital holds at least one wage: input buffers, overhead and
// freight are non-negative. Without stock, capital is cash plus paid cargo, fixed for the whole
// census, so no later market change can fund a single worker. A zero closure ceiling states
// that proof up front; financing would reject each plan anyway, so capacities are unchanged.
fn cannot_fund_one_worker(building: &Building, profile: &EconomyProfileRuntime, cargo: f64) -> bool {
    profile.inputs.iter().chain(&profile.outputs)
        .all(|port| building.inventory_units(port.resource_runtime_id) == 0.0)
        && f64::from(building.operating_budget) + cargo < f64::from(profile.average_daily_wage())
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
    headroom: &mut [f64; 2],
    ceiling: u32,
    activity_ceiling: f32,
) -> (u32, f32) {
    if supply.is_none() { *headroom = [0.0; 2]; }
    let area = output_area_scale_for_building(building, profile);
    let sales = ProvisionalSales { profile, catalog, area, market, household: consumer_units, export_multiplier };
    let market_activity = (if export_multiplier.is_some() {
        1.0
    } else {
        profile
            .outputs
            .iter()
            .map(|port| sales.share(port.resource_runtime_id))
            .fold(0.0, f32::max)
    }).min(activity_ceiling);
    let desired = ((physical as f32 * market_activity).ceil() as u32).min(ceiling);
    if desired == 0 {
        return (0, 0.0);
    }
    let imported_capital = working_capital_assets(
        building,
        profile,
        catalog,
        prices.imports,
        freight_capital,
        |resource| sales.output_price(resource),
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
            |resource, units| sales.export_units(resource, units),
        )
    };
    // Integer payroll, local sales quotas and quantized freight do not form a monotone
    // profitability test. Evaluate the largest feasible operation first, without allowing
    // an unprofitable full-capacity export plan to erase a profitable smaller business.
    // A cheaper source also lowers the replacement value of retained stock. Keep the
    // already validated plan when that repricing cannot finance equal staffing. The caller
    // keeps a local plan's released receipts provisional until replacement is accepted.
    let retained = supply.as_ref().map_or((0, 0.0), |context| context.retained);
    for workers in (retained.0.max(1)..=desired).rev() {
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
                |resource, units| sales.export_units(resource, units),
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
                |resource| sales.output_price(resource),
            )
        } else {
            imported_capital
        };
        if f64::from(costs.working_capital) > capital {
            continue;
        }
        let activity = market_activity.min(workers as f32 / physical.max(1) as f32);
        let revenue = sales.revenue(activity);
        if revenue > costs.total() {
            *headroom = [f64::from(revenue) - f64::from(costs.total()),
                capital - f64::from(costs.working_capital)];
            if let Some(context) = supply.as_mut() {
                context.supply.accept_growing_for(context.buyer);
            }
            return (workers, activity);
        }
    }
    if let Some(context) = supply.as_mut() {
        context.supply.reject(context.allocator, context.graph);
    }
    retained
}

// Household quotas and raw staffed business needs share an output. The business amount is
// only an upper bound here: joint financing and reachability must prove it before publication.
fn provisional_local_share(profile: &EconomyProfileRuntime, area: f32, resource: u16,
    household: Option<&[f64]>, market: &[[f64; 2]]) -> f32 {
    let Some((port, output)) = profile.outputs.iter().enumerate()
        .find(|(_, output)| output.resource_runtime_id == resource) else { return 0.0; };
    let capacity = f64::from(profile.net_output_units_per_day(output) * area);
    if capacity <= 0.0 { return 0.0; }
    let household = household.map_or(0.0, |units| units[port].max(0.0));
    let business = if profile.kind == EconomyProfileRuntimeKind::ServiceStore { 0.0 }
        else { market.get(resource as usize).map_or(0.0, |total| total[1]) };
    ((household + business) / capacity).clamp(0.0, 1.0) as f32
}
