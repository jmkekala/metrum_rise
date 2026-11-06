// SPDX-License-Identifier: GPL-2.0-only

//! Joint proof of local input purchases and their suppliers' recurring sales.

use super::*;

/// Conservative staffed-buyer membership, independent of provisional purchases. A buyer
/// with a positive ceiling remains possible even if this pass could not finance it.
#[derive(Clone, Default)]
pub(super) struct PotentialCustomers {
    /// Membership written by the census and withdrawn when a buyer's ceiling reaches zero.
    pub(super) counted: Vec<bool>,
    /// Staffed input buyers in stable building order; retries skip homes and other nonbuyers.
    pub(super) owners: Vec<usize>,
    // Largest activity each counted buyer can still purchase at within this census.
    activity: Vec<f32>,
    pending: Vec<(u16, u32)>,
    // Resource/component unit totals marked stale, and the round being recounted.
    stale: Vec<(u16, u32)>,
    round: Vec<(u16, u32)>,
    sources: Vec<usize>,
    prepared: bool,
    /// Retains full-replay behavior for exact comparison tests.
    #[cfg(test)]
    pub(super) disabled: bool,
}

impl PotentialCustomers {
    /// Provisions O(buildings) retained membership. Component counts are initialized lazily
    /// over the existing customer index only when a joint financing proof needs a retry.
    pub(super) fn prepare(&mut self, buildings: usize) {
        self.counted.resize(buildings, false);
        // Closure only withdraws members from this original census list. Restore those
        // seed flags before changed production chunks overwrite their current membership.
        for &owner in &self.owners {
            if let Some(counted) = self.counted.get_mut(owner) {
                *counted = true;
            }
        }
        self.activity.resize(buildings, 0.0);
        self.owners.clear();
        self.pending.clear();
        self.stale.clear();
        self.prepared = false;
    }

    /// Each input can remove or mark at most two component memberships; prepare outside
    /// quotation. Distinct stale totals are bounded by the same membership.
    pub(super) fn reserve_inputs(&mut self, inputs: usize) {
        self.owners.reserve(inputs);
        self.pending.reserve(2 * inputs);
        self.stale.reserve(2 * inputs);
        self.round.reserve(2 * inputs);
    }

    fn remove(
        &mut self,
        owner: usize,
        profile: &EconomyProfileRuntime,
        business: &mut super::super::customers::business::BusinessMarket,
    ) {
        if !std::mem::replace(&mut self.counted[owner], false) {
            return;
        }
        self.activity[owner] = 0.0;
        for input in &profile.inputs {
            if !external_input(profile, input) {
                continue;
            }
            business.withdraw_possible_input(owner, input.resource_runtime_id, |component| {
                self.pending.push((input.resource_runtime_id, component))
            });
            business.mark_possible_input(owner, input.resource_runtime_id, |component| {
                self.stale.push((input.resource_runtime_id, component))
            });
        }
    }

    // A lower purchasing bound makes this buyer's input totals stale-high until recounted.
    fn lower(
        &mut self,
        owner: usize,
        activity: f32,
        profile: &EconomyProfileRuntime,
        business: &mut super::super::customers::business::BusinessMarket,
    ) {
        if !self.counted[owner] || activity >= self.activity[owner] {
            return;
        }
        self.activity[owner] = activity;
        for input in &profile.inputs {
            if !external_input(profile, input) {
                continue;
            }
            business.mark_possible_input(owner, input.resource_runtime_id, |component| {
                self.stale.push((input.resource_runtime_id, component))
            });
        }
    }
}

fn external_input(
    profile: &EconomyProfileRuntime,
    input: &super::super::definitions::RuntimeResourcePort,
) -> bool {
    input.units_per_day
        > profile
            .output_port(input.resource_runtime_id)
            .map_or(0.0, |output| output.units_per_day)
}

// Maximum staffed purchase of a counted buyer, replaying the joint proof's f32 arithmetic.
// Below full staffing, the rounded staffed share of a smaller activity can exceed the bound
// by a few ulps, so that case carries slack.
fn possible_input_units(
    customers: &PotentialCustomers,
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    owner: usize,
    resource: u16,
) -> Option<f64> {
    if !customers.counted[owner] {
        return None;
    }
    let building = &allocator.buildings[owner];
    let profile = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)?;
    let input = profile
        .input_port(resource)
        .filter(|input| external_input(profile, input))?;
    let activity = customers.activity[owner];
    if activity <= 0.0 {
        return Some(0.0);
    }
    let production = OperationPlan {
        workers: 0,
        activity,
        area: output_area_scale_for_building(building, profile),
    }
    .production(profile);
    let staffing = building.worker_count as f32
        / physical_worker_capacity_for_profile(building, profile).max(1) as f32;
    let units = super::super::operation::purchased_input_units(profile, input, production)
        * (staffing.min(activity) / activity);
    Some(f64::from(if staffing < activity {
        units * (1.0 + 4.0 * f32::EPSILON)
    } else {
        units
    }))
}

impl JobFunding {
    /// Propagates necessary customer limits. A source loses its whole operation when no
    /// potential staffed business customer remains for any output and neither households nor
    /// exports/public budgets can pay. Otherwise, a source selling only to businesses has its
    /// activity capped by its possible buyers' maximum staffed purchases, so partial
    /// reductions cross a supplier chain without a census for each link.
    ///
    /// Uses existing customer/supplier membership. O(input membership) lazy preparation,
    /// O(staffed input buyers × ports) seed checks per retry, then for each propagation round
    /// O(stale component membership + visited source output ports × log components); rounds
    /// follow the longest reduced dependency chain. Scratch is retained; no city scan or
    /// allocation is added for each dependency link.
    pub(super) fn propagate_impossible_customers(
        &mut self,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
    ) {
        #[cfg(test)]
        if self.potential_customers.disabled {
            return;
        }
        // Propagation only skips censuses that replay would repeat. A pass that failed before
        // reseeding the business book (an input overclaim the repair could not replace) leaves
        // the previous census's layout. Prepare this census's layout with an empty book, the
        // O(buildings) layout work the complete-set check does; that check reseeds the book.
        if !self.business.layout_matches(allocator, network) {
            let Some(components) = self.input_supply.freight_components() else {
                return;
            };
            self.business.reset(
                allocator,
                network,
                graph,
                components,
                catalog,
                0,
                self.customer_units.len(),
                |_| {},
            );
        }
        if !self.potential_customers.prepared {
            // Totals start from the census's initial limits; the seed check below then
            // marks every limit reduced by the failed proof that triggered this retry.
            for slot in 0..self.potential_customers.owners.len() {
                let owner = self.potential_customers.owners[slot];
                let building = &allocator.buildings[owner];
                self.potential_customers.activity[owner] = catalog
                    .profile_by_runtime_id(building.economy_profile_runtime_id)
                    .map_or(0.0, |profile| {
                        self.possible_activity(
                            owner,
                            building,
                            profile,
                            tuning,
                            physical_worker_capacity_for_profile(building, profile),
                            1.0,
                        )
                    });
            }
            let customers = &self.potential_customers;
            self.business.prepare_possible_buyers(|owner, resource| {
                possible_input_units(customers, allocator, catalog, owner, resource)
            });
            self.potential_customers.prepared = true;
        }
        for slot in 0..self.potential_customers.owners.len() {
            let owner = self.potential_customers.owners[slot];
            if !self.potential_customers.counted[owner] {
                continue;
            }
            let building = &allocator.buildings[owner];
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            if self.closure_ceilings[owner] == 0 {
                self.potential_customers
                    .remove(owner, profile, &mut self.business);
            } else {
                let activity = self.possible_activity(
                    owner,
                    building,
                    profile,
                    tuning,
                    self.closure_ceilings[owner],
                    self.closure_activity[owner],
                );
                self.potential_customers
                    .lower(owner, activity, profile, &mut self.business);
            }
        }
        loop {
            while let Some((resource, component)) = self.potential_customers.pending.pop() {
                self.input_supply
                    .visit_possible_sources(resource, component, |owner| {
                        if self.closure_ceilings[owner] == 0 || self.service_flags[owner] {
                            return;
                        }
                        let building = &allocator.buildings[owner];
                        if !live(building) {
                            return;
                        }
                        let Some(profile) =
                            catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
                        else {
                            return;
                        };
                        if building_outputs_can_export_to_owa(building, profile)
                            && self.carrier_quotes[owner].is_some()
                            && tuning.owa_export_price_multiplier > 0.0
                        {
                            return;
                        }
                        if profile.outputs.iter().any(|output| {
                            profile.net_output_units_per_day(output) > 0.0
                                && (self.customers.consumes(output.resource_runtime_id)
                                    || self
                                        .business
                                        .has_possible_buyers(owner, output.resource_runtime_id))
                        }) {
                            return;
                        }
                        self.closure_ceilings[owner] = 0;
                        self.closure_activity[owner] = 0.0;
                        self.potential_customers
                            .remove(owner, profile, &mut self.business);
                    });
            }
            if self.potential_customers.stale.is_empty() {
                break;
            }
            #[cfg(test)]
            {
                self.propagation_rounds += 1;
            }
            // Recount every stale total before reading any of them in this round. Totals
            // marked by this round's reductions stay stale-high until the next round.
            std::mem::swap(
                &mut self.potential_customers.stale,
                &mut self.potential_customers.round,
            );
            for slot in 0..self.potential_customers.round.len() {
                let (resource, component) = self.potential_customers.round[slot];
                let customers = &self.potential_customers;
                self.business
                    .recount_possible_input(resource, component, |owner| {
                        possible_input_units(customers, allocator, catalog, owner, resource)
                    });
            }
            for (resource, component) in self.potential_customers.round.drain(..) {
                let sources = &mut self.potential_customers.sources;
                self.input_supply
                    .visit_possible_sources(resource, component, |owner| sources.push(owner));
            }
            for slot in 0..self.potential_customers.sources.len() {
                let owner = self.potential_customers.sources[slot];
                self.limit_business_source(owner, allocator, catalog, tuning);
            }
            self.potential_customers.sources.clear();
        }
    }

    // Upper bound on the activity at which a counted buyer can purchase in any later pass of
    // this census. Mirrors the top plan in `funded_private_operation`; household quotas can
    // still grow when a rival fails, so consumer sellers rely on their closure limit alone.
    fn possible_activity(
        &self,
        owner: usize,
        building: &Building,
        profile: &EconomyProfileRuntime,
        tuning: &RuntimeEconomyTuning,
        ceiling: u32,
        activity_ceiling: f32,
    ) -> f32 {
        let physical = physical_worker_capacity_for_profile(building, profile);
        if self.service_flags[owner] {
            return self.capacities[owner] as f32 / physical.max(1) as f32;
        }
        if physical == 0 || ceiling == 0 {
            return 0.0;
        }
        let area = output_area_scale_for_building(building, profile);
        let unbounded = profile
            .outputs
            .iter()
            .any(|port| self.customers.consumes(port.resource_runtime_id))
            || (building_outputs_can_export_to_owa(building, profile)
                && self.carrier_quotes[owner].is_some()
                && tuning.owa_export_price_multiplier > 0.0);
        let market_activity = if unbounded {
            1.0
        } else {
            profile
                .outputs
                .iter()
                .map(|port| {
                    provisional_local_share(
                        profile,
                        area,
                        port.resource_runtime_id,
                        None,
                        &self.market,
                    )
                })
                .fold(0.0, f32::max)
        }
        .min(activity_ceiling);
        let workers = ((physical as f32 * market_activity).ceil() as u32).min(ceiling);
        market_activity.min(workers as f32 / physical.max(1) as f32)
    }

    // Caps a source that sells only to businesses by its possible buyers' total purchases,
    // using the joint proof's own demand-activity arithmetic. A single possible buyer gives
    // the value that proof will find; several buyers carry rounding slack, so the ordinary
    // closure can still lower that conservative cap.
    fn limit_business_source(
        &mut self,
        owner: usize,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
    ) {
        if self.closure_ceilings[owner] == 0 || self.service_flags[owner] {
            return;
        }
        let building = &allocator.buildings[owner];
        if !live(building) {
            return;
        }
        let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
        else {
            return;
        };
        if (building_outputs_can_export_to_owa(building, profile)
            && self.carrier_quotes[owner].is_some()
            && tuning.owa_export_price_multiplier > 0.0)
            || profile
                .outputs
                .iter()
                .any(|output| self.customers.consumes(output.resource_runtime_id))
        {
            return;
        }
        let area = output_area_scale_for_building(building, profile);
        let mut demand_activity = 0.0_f32;
        for output in &profile.outputs {
            let full = profile.net_output_units_per_day(output) * area;
            if full <= 0.0 {
                continue;
            }
            let (buyers, units) = self
                .business
                .possible_demand(owner, output.resource_runtime_id);
            let sold = units.min(f64::from(full)) as f32;
            let sold = if buyers > 1 {
                sold * (1.0 + 4.0 * f32::EPSILON)
            } else {
                sold
            };
            demand_activity = demand_activity.max(sold / full);
        }
        if demand_activity >= self.closure_activity[owner] {
            return;
        }
        let physical = physical_worker_capacity_for_profile(building, profile);
        self.closure_activity[owner] = demand_activity;
        self.closure_ceilings[owner] =
            self.closure_ceilings[owner].min((physical as f32 * demand_activity).ceil() as u32);
        if self.closure_ceilings[owner] == 0 {
            self.closure_activity[owner] = 0.0;
            self.potential_customers
                .remove(owner, profile, &mut self.business);
        } else {
            let activity = self.possible_activity(
                owner,
                building,
                profile,
                tuning,
                self.closure_ceilings[owner],
                demand_activity,
            );
            self.potential_customers
                .lower(owner, activity, profile, &mut self.business);
        }
    }

    /// Retains supported plans while replacing invalid provisional deliveries atomically.
    /// One failed plan rejects all repairs; the ordinary closure can then resize operations.
    /// Preparation is O(B + output slots); warm repair is O(output slots + affected receipt
    /// chains + A log A + affected input quotes), without allocating inside owner quotation.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn repair_provisional_inputs(
        &mut self,
        allocator: &BuildingAllocator,
        logistics: &mut ShipmentSystem,
        network: &TransitNetwork,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        fiscal: &CityFiscalPolicy,
        minute: u16,
    ) -> bool {
        let count = allocator.buildings.len();
        self.input_repairs
            .reserve(count.saturating_sub(self.input_repairs.len()));
        self.input_repair_seen.resize(count, false);
        self.input_repair_headroom.resize(count, [0.0; 2]);
        self.input_supply.prepare_owners(count, 0);
        let utilities = local_utility_service_costs(catalog)
            .map(|price| price * tuning.owa_import_price_multiplier);
        loop {
            self.input_supply.begin_revalidation();
            self.input_supply.withdraw_overclaims(
                &mut self.input_repairs,
                &mut self.input_repair_seen,
                allocator,
                graph,
            );
            self.input_repairs.sort_unstable_by_key(|&owner| {
                (
                    catalog.funding_phase(allocator.buildings[owner].economy_profile_runtime_id),
                    owner,
                )
            });
            let mut result = Ok(!self.input_supply.has_overclaims());
            for slot in 0..self.input_repairs.len() {
                if result != Ok(true) {
                    break;
                }
                let owner = self.input_repairs[slot];
                result = self.repair_provisional_buyer(
                    owner, allocator, logistics, network, graph, catalog, tuning, fiscal, minute,
                    utilities,
                );
            }
            if result == Ok(true) {
                self.input_supply.accept_revalidation();
                for &owner in &self.input_repairs {
                    self.headroom[owner] = self.input_repair_headroom[owner];
                }
            } else {
                self.input_supply.reject_revalidation(allocator, graph);
            }
            for owner in self.input_repairs.drain(..) {
                self.input_repair_seen[owner] = false;
            }
            match result {
                Ok(funded) => return funded,
                Err(required) => self.input_supply.reserve_receipts(required),
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn repair_provisional_buyer(
        &mut self,
        owner: usize,
        allocator: &BuildingAllocator,
        logistics: &mut ShipmentSystem,
        network: &TransitNetwork,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        fiscal: &CityFiscalPolicy,
        minute: u16,
        utilities: [f32; 3],
    ) -> Result<bool, usize> {
        let building = &allocator.buildings[owner];
        let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
        else {
            return Ok(false);
        };
        let Some(entrance) = allocator.entrances.get(owner) else {
            return Ok(false);
        };
        let public = self.service_flags[owner];
        let multiplier = freight_goods_multiplier(profile, building.zone_type, tuning, minute);
        let outside = self.carrier_quotes[owner].filter(|_| multiplier.is_finite());
        let prices = operation_prices(building, outside, multiplier, utilities, tuning, fiscal);
        let plan = OperationPlan {
            workers: self.capacities[owner],
            activity: self.activities[owner],
            area: output_area_scale_for_building(building, profile),
        };
        let exports = !public
            && building_outputs_can_export_to_owa(building, profile)
            && outside.is_some()
            && tuning.owa_export_price_multiplier > 0.0;
        let sales = ProvisionalSales {
            profile,
            catalog,
            area: plan.area,
            market: &self.market,
            household: Some(
                &self.customer_units
                    [self.customer_offsets[owner]..self.customer_offsets[owner + 1]],
            ),
            export_multiplier: exports.then_some(tuning.owa_export_price_multiplier),
        };
        let costs = OperatingCosts::for_plan_with_input_quotes(
            profile,
            plan,
            prices,
            |resource, units| sales.export_units(resource, units),
            |input| {
                logistics.quote_missing_recurring_input(
                    &mut self.input_supply,
                    input,
                    owner,
                    Some(owner),
                    entrance,
                    outside,
                    allocator,
                    catalog,
                    tuning,
                    network,
                    graph,
                    multiplier,
                )
            },
        );
        if !costs.total().is_finite() || !costs.working_capital.is_finite() {
            return Ok(false);
        }
        let headroom = if public {
            [f64::INFINITY; 2]
        } else {
            let capital = working_capital_assets_at_prices(
                building,
                profile,
                self.freight_capital[owner],
                |resource| self.input_supply.input_price(resource),
                |resource| sales.output_price(resource),
            );
            let revenue = sales.revenue(plan.activity);
            if revenue <= costs.total() || costs.working_capital > capital {
                return Ok(false);
            }
            [revenue - costs.total(), capital - costs.working_capital]
        };
        self.input_supply.check_extension_capacity()?;
        self.input_supply.extend_inputs_for(owner);
        self.input_repair_headroom[owner] = headroom;
        Ok(true)
    }

    /// Household promises and input purchases draw from the same staffed production. Carry
    /// the preceding proof's household allocations into retries so withdrawn input offers
    /// cannot be promised again before the household-dependent operation is requoted.
    /// Covers sources `rows`; the census steps through them in order (`SIM-02-I`).
    pub(super) fn reserve_household_production(
        &mut self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        graph: &RegionGraph,
        reset: bool,
        rows: std::ops::Range<usize>,
    ) {
        // Availability mutations share resource buckets, so commit in stable source order.
        for (source, building) in allocator.buildings[rows.clone()]
            .iter()
            .enumerate()
            .map(|(offset, building)| (rows.start + offset, building))
        {
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            let physical = physical_worker_capacity_for_profile(building, profile).max(1);
            let activity = if live(building) && self.capacities[source] > 0 {
                (building.worker_count as f32 / physical as f32).min(self.activities[source])
            } else {
                0.0
            };
            let area = output_area_scale_for_building(building, profile);
            for (port, output) in profile.outputs.iter().enumerate() {
                let slot = self.customer_offsets[source] + port;
                let previous = if reset {
                    0.0
                } else {
                    self.household_production[slot]
                };
                let current = if reset {
                    self.household_production[slot]
                } else {
                    self.customer_units[slot].max(0.0).min(f64::from(
                        profile.net_output_units_per_day(output) * area * activity,
                    ))
                };
                self.input_supply.reserve_household_output(
                    source,
                    output.resource_runtime_id,
                    previous,
                    current,
                    allocator,
                    catalog,
                    graph,
                );
                self.household_production[slot] = current;
            }
        }
    }

    /// Provisional operations may depend on one another. Only this complete-set check can
    /// publish them: each business purchase has one staffed buyer and one financing account.
    /// The census runs it as steps (`SIM-02-I`): this start of the seeding of every buyer's
    /// accounts, [`Self::seed_business_rows`] over every building range in order and the
    /// business book's layout phases (`SIM-02-K`), [`Self::claim_business_customers`] over
    /// every seller in order (a storage shortfall grows the arena between attempts and seeds
    /// again; warm passes need no retry), then [`Self::prove_business_customers`] over every
    /// building and [`Self::publish_business_financing`]. Returns false, a failed proof, when
    /// the input book has no freight components. O(B) fills.
    pub(super) fn begin_business_seeding(
        &mut self,
        allocator: &BuildingAllocator,
        inputs: usize,
    ) -> bool {
        if self.input_supply.freight_components().is_none() {
            return false;
        }
        // Financial limits can split a buyer among more sellers than the initial linear
        // preparation estimate; see the retry above.
        let input_slots = self.input_supply.input_tranche_capacity(inputs);
        self.business.begin_seeding(
            allocator.buildings.len(),
            input_slots,
            self.customer_units.len(),
        );
        true
    }

    /// Seeds the accounts of buyers `rows`, the seeding's next ones, in buyer order
    /// (`SIM-02-K`). O(rows + their input tranches), serial.
    pub(super) fn seed_business_rows(
        &mut self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        fiscal: &CityFiscalPolicy,
        minute: u16,
        rows: std::ops::Range<usize>,
    ) {
        let supplies = &self.input_supply;
        let capacities = &self.capacities;
        let activities = &self.activities;
        let headroom = &self.headroom;
        let carriers = &self.carrier_quotes;
        let service_flags = &self.service_flags;
        let utilities = local_utility_service_costs(catalog)
            .map(|price| price * tuning.owa_import_price_multiplier);
        let book = self.business.seeding_book();
        // Source receipts link distinct buyers, so their ordered seeding is dependent.
        for (index, building) in allocator.buildings[rows.clone()]
            .iter()
            .enumerate()
            .map(|(offset, building)| (rows.start + offset, building))
        {
            if capacities[index] == 0 || !live(building) {
                continue;
            }
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            let service = service_flags[index];
            book.set_finance(
                index,
                if service {
                    [f64::INFINITY; 2]
                } else {
                    headroom[index]
                },
            );
            if building.worker_count == 0 {
                continue;
            }
            let physical = physical_worker_capacity_for_profile(building, profile).max(1);
            let activity = if service {
                capacities[index] as f32 / physical as f32
            } else {
                activities[index]
            };
            if activity <= 0.0 {
                continue;
            }
            let staffed = (building.worker_count as f32 / physical as f32).min(activity) / activity;
            let multiplier = freight_goods_multiplier(profile, building.zone_type, tuning, minute);
            let outside = carriers[index].filter(|_| multiplier.is_finite());
            let prices = operation_prices(building, outside, multiplier, utilities, tuning, fiscal);
            OperatingCosts::for_plan_with_input_quotes(
                profile,
                OperationPlan {
                    workers: capacities[index],
                    activity,
                    area: output_area_scale_for_building(building, profile),
                },
                prices,
                |_, _| 0.0,
                |input| {
                    let imported = super::super::logistics::imported_input_unit_price(
                        catalog,
                        tuning,
                        input.resource,
                        multiplier,
                    );
                    let mut supplied = 0.0;
                    let staffed_input = super::super::operation::InputRequirement {
                        daily_units: input.daily_units * staffed,
                        ..input
                    };
                    let complete = supplies.visit_input_sources(
                        index,
                        staffed_input,
                        imported,
                        outside,
                        |source, units, goods, carrier| {
                            supplied += units;
                            if let Some(slot) = book.push_input(
                                index,
                                input,
                                goods,
                                carrier,
                                building.inventory_units(input.resource),
                                units,
                                multiplier,
                            ) && let Some(source) = source
                            {
                                book.seed_for(source, slot);
                            }
                        },
                    );
                    // A funded public invoice can seek a first local supplier without
                    // currently receiving its input. Private accounts never get this seed.
                    if service && !complete {
                        book.push_input(
                            index,
                            input,
                            imported,
                            0.0,
                            building.inventory_units(input.resource),
                            f64::from(staffed_input.daily_units) - supplied,
                            multiplier,
                        );
                    }
                    Default::default()
                },
            );
        }
    }

    /// Claims the customers of sellers from `start` on, in building order after the seeding,
    /// until `work` (of the book, the route cache and the sellers claimed) reaches `budget`;
    /// returns the seller it stopped before and the work used (`SIM-02-K`), so a slice holds
    /// at most one seller past its budget. A storage shortfall returns the arena size required
    /// and the work used: the caller reserves it, seeds again and claims again from the first
    /// seller, so no partial matching result escapes.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn claim_business_customers(
        &mut self,
        allocator: &BuildingAllocator,
        logistics: &mut ShipmentSystem,
        network: &TransitNetwork,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        start: usize,
        budget: u64,
        work: impl Fn(&Self, &ShipmentSystem, usize) -> u64,
    ) -> Result<(usize, u64), (usize, u64)> {
        let total = allocator.buildings.len();
        let Some(components) = self.input_supply.freight_components() else {
            return Ok((total, work(self, logistics, total - start)));
        };
        for index in start..total {
            let used = work(self, logistics, index - start);
            if used >= budget {
                return Ok((index, used));
            }
            let building = &allocator.buildings[index];
            if self.capacities[index] == 0 || !live(building) {
                continue;
            }
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            if !super::super::logistics::building_outputs_can_supply_local_inputs(building, profile)
            {
                continue;
            }
            let Some(entrance) = allocator.entrances.get(index) else {
                continue;
            };
            let area = output_area_scale_for_building(building, profile);
            for (port, output) in profile.outputs.iter().enumerate() {
                let full = f64::from(profile.net_output_units_per_day(output) * area);
                let household = self.customer_units[self.customer_offsets[index] + port].max(0.0);
                // The input ledger also contains prospective-worker commitments. Only the
                // staffed part is established business demand, but all of it occupies output.
                let committed = self.input_supply.committed_output(
                    index,
                    output.resource_runtime_id,
                    allocator,
                    catalog,
                );
                let remaining = (full - household - committed).max(0.0);
                self.business.claim_reachable(
                    entrance,
                    Some(index),
                    output.resource_runtime_id,
                    remaining,
                    profile.unit_price_currency,
                    components,
                    allocator,
                    logistics,
                    network,
                    graph,
                    tuning,
                );
            }
            if let Err(required) = self.business.try_extend_for(index) {
                return Err((required, work(self, logistics, index - start)));
            }
        }
        Ok((total, work(self, logistics, total - start)))
    }

    /// The proof over buildings `rows`, independent per building and parallel: a provisional
    /// customer shortage lowers no ceiling, a failed joint proof does. Returns whether any
    /// ceiling was lowered; the census ORs the slices.
    pub(super) fn prove_business_customers(
        &mut self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        fiscal: &CityFiscalPolicy,
        minute: u16,
        rows: std::ops::Range<usize>,
    ) -> bool {
        let utilities = local_utility_service_costs(catalog)
            .map(|price| price * tuning.owa_import_price_multiplier);
        self.headroom[rows.clone()]
            .par_iter_mut()
            .zip(self.closure_ceilings[rows.clone()].par_iter_mut())
            .zip(self.closure_activity[rows.clone()].par_iter_mut())
            .enumerate()
            .map(|(offset, ((headroom, ceiling), activity_ceiling))| {
                let index = rows.start + offset;
                let building = &allocator.buildings[index];
                if self.service_flags[index] {
                    *headroom = [f64::INFINITY; 2];
                    return false;
                }
                // A provisional customer shortage is not a permanent staffing limit:
                // another rejected firm can return those customers during this census.
                // Retain only limits imposed by a failed joint proof, not ordinary quotes.
                let previous = self.capacities[index];
                if self.capacities[index] == 0 {
                    return false;
                }
                let Some(profile) =
                    catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
                else {
                    return false;
                };
                let physical = physical_worker_capacity_for_profile(building, profile);
                let area = output_area_scale_for_building(building, profile);
                let exports = building_outputs_can_export_to_owa(building, profile)
                    && self.carrier_quotes[index].is_some()
                    && tuning.owa_export_price_multiplier > 0.0;
                let export_price = |resource| {
                    if exports {
                        catalog.unit_price_for_resource(resource).unwrap_or(0.0)
                            * tuning.owa_export_price_multiplier
                    } else {
                        0.0
                    }
                };
                let quotas = &self.customer_units
                    [self.customer_offsets[index]..self.customer_offsets[index + 1]];
                let before_share = |resource| {
                    provisional_local_share(profile, area, resource, Some(quotas), &self.market)
                };
                let actual_local = |resource, full: f32| {
                    let port = profile
                        .outputs
                        .iter()
                        .position(|output| output.resource_runtime_id == resource)
                        .unwrap();
                    let household = (quotas[port].max(0.0) as f32).min(full);
                    let sales = self.business.sales(
                        index,
                        resource,
                        f64::from((full - household).max(0.0)),
                    );
                    (
                        household + sales.units as f32,
                        f64::from(household) * f64::from(profile.unit_price_currency)
                            + sales.revenue,
                    )
                };
                let mut old_revenue = 0.0_f64;
                let mut revenue = 0.0_f64;
                let mut capital_change = 0.0_f64;
                let mut demand_activity = 0.0_f32;
                for output in &profile.outputs {
                    let resource = output.resource_runtime_id;
                    let full = profile.net_output_units_per_day(output) * area;
                    let units = full * self.activities[index];
                    let before = (full * before_share(resource)).min(units);
                    old_revenue += f64::from(before) * f64::from(profile.unit_price_currency)
                        + f64::from(units - before) * f64::from(export_price(resource));
                    let (local, goods) = actual_local(resource, units);
                    revenue += goods + f64::from(units - local) * f64::from(export_price(resource));
                    let (quota, full_goods) = actual_local(resource, full);
                    let share = if full > 0.0 { quota / full } else { 0.0 };
                    demand_activity = demand_activity.max(share);
                    if profile.input_port(resource).is_none() {
                        let old_price = f64::from(profile.unit_price_currency)
                            * f64::from(before_share(resource))
                            + f64::from(export_price(resource))
                                * f64::from(1.0 - before_share(resource));
                        let new_price = if full > 0.0 {
                            full_goods / f64::from(full)
                        } else {
                            0.0
                        } + f64::from(export_price(resource))
                            * f64::from(1.0 - share);
                        let inventory = building.inventory_units(resource);
                        capital_change +=
                            f64::from(inventory) * new_price - f64::from(inventory) * old_price;
                    }
                }
                let multiplier =
                    freight_goods_multiplier(profile, building.zone_type, tuning, minute);
                let prices = operation_prices(
                    building,
                    self.carrier_quotes[index],
                    multiplier,
                    utilities,
                    tuning,
                    fiscal,
                );
                let plan = OperationPlan {
                    workers: self.capacities[index],
                    activity: self.activities[index],
                    area,
                };
                let freight = |actual: bool| {
                    OperatingCosts::for_plan_with_input_quotes(
                        profile,
                        plan,
                        prices,
                        |resource, units| {
                            if exports {
                                let local = if actual {
                                    actual_local(resource, units).0
                                } else {
                                    let full = profile.net_output_units_per_day(
                                        profile.output_port(resource).unwrap(),
                                    ) * area;
                                    (full * before_share(resource)).min(units)
                                };
                                units - local
                            } else {
                                0.0
                            }
                        },
                        |_| Default::default(),
                    )
                    .freight
                };
                let freight_change = freight(true) - freight(false);
                let financed = self.business.financing(index);
                *headroom = [
                    financed[0] + revenue - old_revenue - freight_change,
                    financed[1] + capital_change - freight_change,
                ];
                if !exports && demand_activity < self.activities[index] {
                    *ceiling = (*ceiling).min((physical as f32 * demand_activity).ceil() as u32);
                    *activity_ceiling = (*activity_ceiling).min(demand_activity);
                }
                if !(headroom[0] > 0.0 && headroom[1] >= 0.0) {
                    *ceiling = (*ceiling).min(self.capacities[index] - 1);
                }
                *ceiling < previous || *activity_ceiling < self.activities[index]
            })
            .reduce(|| false, |a, b| a | b)
    }

    /// Publishes the proven financing to the business book, after every proof slice.
    pub(super) fn publish_business_financing(&mut self) {
        self.business.publish_financing(&self.headroom);
    }

    /// Links established deliveries to both buyers and suppliers before construction forks
    /// the residual book. Existing inputs keep their original order and replacement ownership.
    pub(super) fn publish_business_deliveries(
        &mut self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        graph: &RegionGraph,
    ) {
        let supply = &mut self.input_supply;
        supply.prepare_customer_deliveries(self.business.receipt_count());
        self.business
            .visit_additional_sales(|seller, buyer, resource, units, quote| {
                let stored = supply.commit_customer_delivery(
                    seller, buyer, resource, units, quote, allocator, catalog, graph,
                );
                assert!(
                    stored.is_ok(),
                    "one physical receipt per prepared financial delivery"
                );
            });
    }
}

/// Prices shared by the live proof and its retained construction operation records.
pub(super) fn operation_prices(
    building: &Building,
    outside: Option<f32>,
    multiplier: f32,
    utilities: [f32; 3],
    tuning: &RuntimeEconomyTuning,
    fiscal: &CityFiscalPolicy,
) -> OperationPrices {
    OperationPrices {
        utilities,
        property_tax: daily_property_tax(building.zone_type, building.level, fiscal),
        imports: tuning.owa_import_price_multiplier
            * if multiplier.is_finite() {
                multiplier
            } else {
                1.0
            },
        carrier_quote: outside
            .filter(|_| multiplier.is_finite())
            .unwrap_or(f32::INFINITY),
        truck_payload: tuning.logistics.truck_payload_kg,
    }
}
