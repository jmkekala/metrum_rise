// SPDX-License-Identifier: GPL-2.0-only

//! Demand spawn-need accounting and hard spawn gates.

use super::actions::{DemandSpawnAction, DemandSpawnCandidate};
use super::admission::HouseholdAdmissionBudget;
use super::snapshot::DailyDemandSnapshot;
use super::types::{EPSILON, RESIDENTIAL_SPAWN_VACANT_SLOT_RESERVE_RATIO};
use crate::simulation::buildings::allocator::{
    Building, BuildingAllocator, baseline_private_zone_slot,
};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, ResourceRuntimeId, RuntimeEconomyCatalog,
};
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use crate::simulation::zoning::ZoneType;

use crate::simulation::economy::operation::{
    InputRequirement, InputSupplyCosts, OperatingCosts, OperationPlan, OperationPrices,
    working_capital_assets_at_prices,
};

/// Household capacity still needed after ready and committed homes are counted.
/// `incoming_households` comes from [`incoming_household_need`].
pub(super) fn residential_missing_household_slots(
    snapshot: &DailyDemandSnapshot,
    incoming_households: u32,
) -> u32 {
    let reserve_slots = residential_vacancy_reserve(snapshot.total_household_count);
    snapshot
        .unhoused_household_count
        .saturating_add(incoming_households)
        .saturating_add(reserve_slots)
        .saturating_sub(
            snapshot
                .vacant_household_slots
                .saturating_add(snapshot.under_construction_household_slots),
        )
}

/// Arriving households whose adults staff the snapshot's open jobs (`DEM-02-C`).
///
/// One home per open job overbuilt housing: admission charges every adult of a household
/// against the jobs. This walks admission's own applicant stream instead. Offers sample the
/// residential vacancies admission will fill; with none, this pass's legal home candidates in
/// attachment order stand in for the homes the offers would reach. An empty connected town
/// still offers its first home. O(offers walked) <= O(max(16, 4 x open jobs)), no allocation.
pub(super) fn incoming_household_need(
    snapshot: &DailyDemandSnapshot,
    allocator: &BuildingAllocator,
    budget: &HouseholdAdmissionBudget,
    policy: &CityFiscalPolicy,
    seed: usize,
    candidates: &[DemandSpawnCandidate],
) -> u32 {
    if snapshot.connected_border_count == 0 {
        return 0;
    }
    let vacancies = baseline_private_zone_slot(ZoneType::Residential)
        .filter(|_| !allocator.dirty_index)
        .map_or(&[][..], |slot| allocator.vacancy_index[slot].as_slice());
    let households = if !vacancies.is_empty() {
        budget.households_to_staff(policy, seed, vacancies.len(), |position| {
            let home = vacancies[position];
            (
                allocator.applicant_seed(home),
                allocator.flat_size_m2(home),
                allocator.buildings[home].level,
            )
        })
    } else {
        budget.households_to_staff(policy, seed, candidates.len(), |position| {
            let action = &candidates[position].action;
            let level = allocator
                .registry
                .get(&action.asset_id)
                .and_then(|entry| entry.manifest.building.as_ref())
                .map_or(1, |building| building.level);
            (
                action.parcel_id as usize,
                allocator.registry.flat_size_m2(&action.asset_id),
                level,
            )
        })
    };
    households.max(u32::from(snapshot.total_household_count == 0))
}

/// Vacant household places retained by both housing construction and downsizing.
pub(super) fn residential_vacancy_reserve(households: u32) -> u32 {
    (households as f32 * RESIDENTIAL_SPAWN_VACANT_SLOT_RESERVE_RATIO).ceil() as u32
}

/// Selects actual homes in attachment order, stopping when their capacity covers the need.
/// Unselected offers remain available for coordinated workforce planning. O(candidates), with
/// buffers reserved before iteration and no per-candidate allocation or city scan.
pub(super) fn select_residential_spawns(
    allocator: &BuildingAllocator,
    candidates: Vec<DemandSpawnCandidate>,
    mut missing_slots: u32,
    optional_homes: &mut Vec<DemandSpawnAction>,
) -> Vec<DemandSpawnAction> {
    let mut selected = Vec::with_capacity(candidates.len().min(missing_slots as usize));
    optional_homes.reserve(candidates.len());
    for candidate in candidates {
        let slots = allocator
            .registry
            .household_capacity(&candidate.action.asset_id);
        if slots == 0 {
            continue;
        }
        if missing_slots > 0 {
            missing_slots = missing_slots.saturating_sub(slots);
            selected.push(candidate.action);
        } else {
            optional_homes.push(candidate.action);
        }
    }
    selected
}

pub(super) fn candidate_economy_profile<'a>(
    allocator: &BuildingAllocator,
    catalog: &'a RuntimeEconomyCatalog,
    asset_id: &str,
) -> Option<&'a EconomyProfileRuntime> {
    let profile_id = allocator.registry.economy_profile(asset_id)?;
    catalog.profile_for_id(profile_id)
}

#[derive(Clone, Debug, Default)]
pub(super) struct OutputAbsorptionContext {
    placed_output_units_by_resource: Vec<f32>,
    demand_units_by_resource: Vec<f32>,
    export_expansion_available: bool,
}

/// Read-only market after optionally releasing the capacity of one replaced building.
/// Rejected proposals do not mutate shared totals or introduce subtract/add rounding drift.
#[derive(Clone, Copy)]
pub(super) struct OutputMarket<'a> {
    context: &'a OutputAbsorptionContext,
    replaced: Option<(&'a EconomyProfileRuntime, f32)>,
}

impl OutputMarket<'_> {
    fn unmet_units(&self, resource: ResourceRuntimeId) -> f32 {
        let released = self.replaced.map_or(0.0, |(profile, scale)| {
            profile
                .output_port(resource)
                .map_or(0.0, |port| profile.net_output_units_per_day(port) * scale)
        });
        (self.context.consumer_demand(resource)
            - (self.context.placed_capacity(resource) - released).max(0.0))
        .max(0.0)
    }

    fn export_expansion_available(&self) -> bool {
        self.context.export_expansion_available
    }

    /// Target workforce supported by customers, including the replaced building's customers.
    pub(super) fn candidate_workers(&self, profile: &EconomyProfileRuntime) -> u32 {
        self.context.market_workers(profile, |resource, output| {
            self.unmet_units(resource) / output
        })
    }

    fn attainable_revenue(
        &self,
        resource: ResourceRuntimeId,
        output: f32,
        local_price: f32,
        export_price: f32,
    ) -> f64 {
        let local = output.min(self.unmet_units(resource));
        f64::from(local) * f64::from(local_price)
            + if self.export_expansion_available() {
                f64::from(output - local) * f64::from(export_price)
            } else {
                0.0
            }
    }
}

impl OutputAbsorptionContext {
    /// Copies resource totals into reusable replay storage in O(resources), retaining capacity.
    pub(super) fn copy_from(&mut self, source: &Self) {
        self.placed_output_units_by_resource
            .clone_from(&source.placed_output_units_by_resource);
        self.demand_units_by_resource
            .clone_from(&source.demand_units_by_resource);
        self.export_expansion_available = source.export_expansion_available;
    }

    /// Staffing for the remaining market; factories require their full viable operation.
    pub(super) fn candidate_workers(&self, profile: &EconomyProfileRuntime) -> u32 {
        self.market_workers(profile, |resource, output| {
            self.unmet_units(resource) / output
        })
    }

    /// Staffing for a placed shop's share of demand, including constructing competitors.
    pub(super) fn committed_workers(&self, profile: &EconomyProfileRuntime) -> u32 {
        self.market_workers(profile, |resource, output| {
            self.consumer_demand(resource) / self.placed_capacity(resource).max(output)
        })
    }

    fn market_workers(
        &self,
        profile: &EconomyProfileRuntime,
        share: impl Fn(ResourceRuntimeId, f32) -> f32,
    ) -> u32 {
        if !matches!(
            profile.kind,
            EconomyProfileRuntimeKind::Store | EconomyProfileRuntimeKind::ServiceStore
        ) {
            return profile.worker_capacity;
        }
        let scale = profile
            .outputs
            .iter()
            .filter(|port| port.units_per_day > EPSILON)
            .map(|port| share(port.resource_runtime_id, port.units_per_day).clamp(0.0, 1.0))
            .fold(0.0, f32::max);
        (profile.worker_capacity as f32 * scale).ceil() as u32
    }

    pub(super) fn empty(resource_count: usize) -> Self {
        Self {
            placed_output_units_by_resource: vec![0.0; resource_count + 1],
            demand_units_by_resource: vec![0.0; resource_count + 1],
            export_expansion_available: false,
        }
    }

    pub(super) fn from_resource_amounts(
        resource_count: usize,
        placed_output_capacity_by_resource: &[(ResourceRuntimeId, f32)],
        resident_demand_rates_by_resource: &[(ResourceRuntimeId, f32)],
        housed_resident_count: u32,
        business_input_need_by_resource: &[(ResourceRuntimeId, f32)],
    ) -> Self {
        let mut context = Self::empty(resource_count);

        for &(resource_runtime_id, capacity) in placed_output_capacity_by_resource {
            context.add_placed_capacity(resource_runtime_id, capacity.max(0.0));
        }

        for &(resource_runtime_id, rate_per_resident) in resident_demand_rates_by_resource {
            context.add_demand_units(
                resource_runtime_id,
                rate_per_resident.max(0.0) * housed_resident_count as f32,
            );
        }

        for &(resource_runtime_id, need_units) in business_input_need_by_resource {
            context.add_demand_units(resource_runtime_id, need_units.max(0.0));
        }

        context
    }

    /// Daily demand remaining after existing and selected output capacity is reserved.
    pub(super) fn unmet_units(&self, resource: ResourceRuntimeId) -> f32 {
        (self.consumer_demand(resource) - self.placed_capacity(resource)).max(0.0)
    }

    /// Whether outside buyers are available independently of existing firms' cash balances.
    /// Outside buyers themselves impose no quantity limit and never enter local demand totals.
    pub(super) fn set_export_expansion_available(&mut self, available: bool) {
        self.export_expansion_available = available;
    }

    /// Whether export-led openings can be considered in this pass.
    #[cfg(test)]
    pub(super) fn export_expansion_available(&self) -> bool {
        self.export_expansion_available
    }

    /// Reads the current market without releasing any incumbent capacity.
    pub(super) fn market(&self) -> OutputMarket<'_> {
        OutputMarket {
            context: self,
            replaced: None,
        }
    }

    /// Credits only the incumbent's exact snapshot capacity while assessing a replacement.
    pub(super) fn replacement_market<'a>(
        &'a self,
        previous: &'a EconomyProfileRuntime,
        scale: f32,
    ) -> OutputMarket<'a> {
        OutputMarket {
            context: self,
            replaced: Some((previous, scale)),
        }
    }

    /// Replaces one selected building's commitment without changing any other firm's customers.
    pub(super) fn reserve_replacement(
        &mut self,
        previous: &EconomyProfileRuntime,
        scale: f32,
        target: &EconomyProfileRuntime,
    ) {
        self.release_candidate(previous, scale);
        self.reserve_candidate(target);
    }

    /// Releases a committed capacity in a private replay market before ordered reconstruction.
    /// Ordinary rejected proposals use read-only replacement views and never call this mutator.
    pub(super) fn release_candidate(&mut self, previous: &EconomyProfileRuntime, scale: f32) {
        for port in &previous.outputs {
            let slot = &mut self.placed_output_units_by_resource[port.resource_runtime_id as usize];
            *slot = (*slot - previous.net_output_units_per_day(port) * scale).max(0.0);
        }
    }

    /// Reserve selected output immediately; a new factory cannot justify its own customers.
    pub(super) fn reserve_candidate(&mut self, profile: &EconomyProfileRuntime) {
        self.reserve_scaled_candidate(profile, 1.0);
    }

    /// Restores the exact full output commitment of an existing construction site.
    pub(super) fn reserve_scaled_candidate(&mut self, profile: &EconomyProfileRuntime, scale: f32) {
        for port in &profile.outputs {
            self.add_placed_capacity(
                port.resource_runtime_id,
                profile.net_output_units_per_day(port) * scale,
            );
        }
    }

    /// Adds financed consumer purchases or independent business input demand.
    pub(super) fn add_demand_units(&mut self, resource_runtime_id: ResourceRuntimeId, units: f32) {
        if let Some(slot) = self
            .demand_units_by_resource
            .get_mut(resource_runtime_id as usize)
        {
            *slot += units;
        }
    }

    fn add_placed_capacity(&mut self, resource_runtime_id: ResourceRuntimeId, capacity: f32) {
        if let Some(slot) = self
            .placed_output_units_by_resource
            .get_mut(resource_runtime_id as usize)
        {
            *slot += capacity;
        }
    }

    fn placed_capacity(&self, resource_runtime_id: ResourceRuntimeId) -> f32 {
        self.placed_output_units_by_resource
            .get(resource_runtime_id as usize)
            .copied()
            .unwrap_or(0.0)
    }

    fn consumer_demand(&self, resource_runtime_id: ResourceRuntimeId) -> f32 {
        self.demand_units_by_resource
            .get(resource_runtime_id as usize)
            .copied()
            .unwrap_or(0.0)
    }
}

/// Applies recurring profitability and quoted working-capital valuation to a replacement.
#[allow(clippy::too_many_arguments)]
pub(super) fn replacement_market_covers_operation(
    profile: &EconomyProfileRuntime,
    catalog: &RuntimeEconomyCatalog,
    market: OutputMarket<'_>,
    prices: OperationPrices,
    exports: f32,
    building: &Building,
    paid_cargo: f64,
    supply: &mut impl super::supply::ProjectSupply,
) -> bool {
    let Some(costs) = project_operation_costs(
        profile,
        catalog,
        market,
        prices,
        exports,
        building.worker_count,
        supply,
    ) else {
        return false;
    };
    let capital = working_capital_assets_at_prices(
        building,
        profile,
        paid_cargo,
        |resource| supply.input_price(resource, catalog, prices),
        |resource| {
            if let Some(price) = supply.output_price(resource) {
                return price;
            }
            let output = profile
                .output_port(resource)
                .map_or(0.0, |port| profile.net_output_units_per_day(port));
            if output <= EPSILON {
                return 0.0;
            }
            let exports = if matches!(
                profile.kind,
                EconomyProfileRuntimeKind::Processor | EconomyProfileRuntimeKind::Producer
            ) {
                exports
            } else {
                0.0
            };
            market.attainable_revenue(
                resource,
                output,
                profile.unit_price_currency,
                catalog.unit_price_for_resource(resource).unwrap_or(0.0) * exports,
            ) / f64::from(output)
        },
    );
    supply.fund(profile, prices, costs, capital)
}

/// Evaluates one candidate against source-specific finite quotes without committing its claims.
/// The caller commits only after customer, workforce, and placement checks also succeed.
pub(super) fn project_covers_operation(
    profile: &EconomyProfileRuntime,
    catalog: &RuntimeEconomyCatalog,
    market: OutputMarket<'_>,
    prices: OperationPrices,
    exports: f32,
    supply: &mut impl super::supply::ProjectSupply,
) -> bool {
    let capital = supply.startup_capital(profile, catalog, prices);
    project_operation_costs(profile, catalog, market, prices, exports, 0, supply)
        .is_some_and(|costs| supply.fund(profile, prices, costs, capital))
}

fn project_operation_costs(
    profile: &EconomyProfileRuntime,
    catalog: &RuntimeEconomyCatalog,
    market: OutputMarket<'_>,
    prices: OperationPrices,
    exports: f32,
    minimum_workers: u32,
    supply: &mut impl super::supply::ProjectSupply,
) -> Option<OperatingCosts> {
    // A replacement retains incumbents only within the target's physical job slots.
    let minimum_workers = minimum_workers.min(profile.worker_capacity);
    // A store whose full crew cannot be housed may plan for the crew that can (`DEM-02-C`):
    // it claims only the share of its output that crew serves, so the remaining customers
    // stay available to another store. Other kinds keep their whole recipe.
    let crew_cap = supply.crew_cap();
    let share = if matches!(
        profile.kind,
        EconomyProfileRuntimeKind::Store | EconomyProfileRuntimeKind::ServiceStore
    ) && crew_cap < profile.worker_capacity
    {
        crew_cap as f32 / profile.worker_capacity as f32
    } else {
        1.0
    };
    if let Some(customers) = supply.customers(profile, share, prices, exports) {
        let activity = customers.activity * share;
        let workers = (profile.worker_capacity as f32 * activity).ceil() as u32;
        let workers = workers.max(minimum_workers).min(crew_cap);
        let plan = OperationPlan {
            workers,
            activity,
            area: 1.0,
        };
        supply.set_supported_plan(plan);
        if workers == 0 {
            return None;
        }
        let mut costs = OperatingCosts::for_plan_with_input_quotes(
            profile,
            plan,
            prices,
            |_, _| 0.0,
            |input| supply.quote(input, catalog, prices),
        );
        costs.freight += customers.export_freight;
        costs.working_capital += customers.export_freight;
        return (customers.revenue > costs.total()).then_some(costs);
    }
    market_operation_costs(
        profile,
        catalog,
        market,
        prices,
        exports,
        minimum_workers,
        |input| supply.quote(input, catalog, prices),
    )
}

/// Isolated tariff fixture; runtime project quotes also claim reachable customers.
#[cfg(test)]
pub(super) fn market_covers_operation_with_input_quotes(
    profile: &EconomyProfileRuntime,
    catalog: &RuntimeEconomyCatalog,
    market: OutputMarket<'_>,
    prices: OperationPrices,
    exports: f32,
    input_quote: impl FnMut(InputRequirement) -> InputSupplyCosts,
) -> bool {
    // Opening equity pays inventory and settlement timing; it never enters sales.
    market_operation_costs(profile, catalog, market, prices, exports, 0, input_quote).is_some_and(
        |costs| {
            f64::from(profile.zoned_startup_budget(catalog, prices.imports))
                >= costs.working_capital
        },
    )
}

fn market_operation_costs(
    profile: &EconomyProfileRuntime,
    catalog: &RuntimeEconomyCatalog,
    market: OutputMarket<'_>,
    prices: OperationPrices,
    exports: f32,
    minimum_workers: u32,
    input_quote: impl FnMut(InputRequirement) -> InputSupplyCosts,
) -> Option<OperatingCosts> {
    match profile.kind {
        EconomyProfileRuntimeKind::Store | EconomyProfileRuntimeKind::ServiceStore => {
            commercial_operation_in_market(profile, market, prices, minimum_workers, input_quote)
        }
        EconomyProfileRuntimeKind::Processor | EconomyProfileRuntimeKind::Producer => {
            industrial_operation_in_market(profile, catalog, market, prices, exports, input_quote)
        }
        _ => Some(OperatingCosts::default()),
    }
}

#[cfg(test)]
pub(super) fn commercial_market_covers_operation(
    profile: &EconomyProfileRuntime,
    catalog: &RuntimeEconomyCatalog,
    absorption: &OutputAbsorptionContext,
    prices: OperationPrices,
) -> bool {
    commercial_operation_in_market(profile, absorption.market(), prices, 0, |input| {
        input.imported(catalog, prices)
    })
    .is_some()
}

fn commercial_operation_in_market(
    profile: &EconomyProfileRuntime,
    absorption: OutputMarket<'_>,
    prices: OperationPrices,
    minimum_workers: u32,
    input_quote: impl FnMut(InputRequirement) -> InputSupplyCosts,
) -> Option<OperatingCosts> {
    if !matches!(
        profile.kind,
        EconomyProfileRuntimeKind::Store | EconomyProfileRuntimeKind::ServiceStore
    ) {
        return Some(OperatingCosts::default());
    }
    let workers = absorption.candidate_workers(profile).max(minimum_workers);
    if workers == 0 {
        return None;
    }
    let activity = profile
        .outputs
        .iter()
        .filter(|port| port.units_per_day > EPSILON)
        .map(|port| {
            (absorption.unmet_units(port.resource_runtime_id) / port.units_per_day).clamp(0.0, 1.0)
        })
        .fold(0.0, f32::max);
    let revenue: f64 = profile
        .outputs
        .iter()
        .map(|port| {
            f64::from(
                absorption
                    .unmet_units(port.resource_runtime_id)
                    .min(profile.net_output_units_per_day(port) * activity),
            ) * f64::from(profile.unit_price_currency)
        })
        .sum();
    let costs = OperatingCosts::for_plan_with_input_quotes(
        profile,
        OperationPlan {
            workers,
            activity,
            area: 1.0,
        },
        prices,
        |_, _| 0.0,
        input_quote,
    );
    (revenue > costs.total()).then_some(costs)
}

#[cfg(test)]
pub(super) fn industrial_market_covers_operation(
    profile: &EconomyProfileRuntime,
    catalog: &RuntimeEconomyCatalog,
    absorption: &OutputAbsorptionContext,
    prices: OperationPrices,
    exports: f32,
) -> bool {
    industrial_operation_in_market(
        profile,
        catalog,
        absorption.market(),
        prices,
        exports,
        |input| input.imported(catalog, prices),
    )
    .is_some()
}

fn industrial_operation_in_market(
    profile: &EconomyProfileRuntime,
    catalog: &RuntimeEconomyCatalog,
    absorption: OutputMarket<'_>,
    prices: OperationPrices,
    exports: f32,
    input_quote: impl FnMut(InputRequirement) -> InputSupplyCosts,
) -> Option<OperatingCosts> {
    if !matches!(
        profile.kind,
        EconomyProfileRuntimeKind::Processor | EconomyProfileRuntimeKind::Producer
    ) {
        return Some(OperatingCosts::default());
    }
    industrial_operation_for_plan(
        profile,
        catalog,
        absorption,
        OperationPlan {
            workers: profile.worker_capacity,
            activity: 1.0,
            area: 1.0,
        },
        prices,
        exports,
        input_quote,
    )
}

/// Reserves a placed explicit producer's full operation using its actual area and capital.
/// O(recipe ports² + source queries); no per-site storage or citywide lookup is needed.
#[allow(clippy::too_many_arguments)]
pub(super) fn constructing_area_covers_operation(
    building: &Building,
    profile: &EconomyProfileRuntime,
    catalog: &RuntimeEconomyCatalog,
    market: OutputMarket<'_>,
    prices: OperationPrices,
    exports: f32,
    paid_cargo: f64,
    supply: &mut impl super::supply::ProjectSupply,
) -> bool {
    let area =
        crate::simulation::economy::households::output_area_scale_for_building(building, profile);
    let workers = crate::simulation::economy::households::physical_worker_capacity_for_profile(
        building, profile,
    );
    if workers == 0 || area <= 0.0 {
        supply.set_supported_plan(OperationPlan {
            workers: 0,
            activity: 0.0,
            area,
        });
        return false;
    }
    let costs = if let Some(customers) = supply.customers(profile, area, prices, exports) {
        let workers = (workers as f32 * customers.activity).ceil() as u32;
        let plan = OperationPlan {
            workers,
            activity: customers.activity,
            area,
        };
        supply.set_supported_plan(plan);
        if workers == 0 {
            return false;
        }
        let mut costs = OperatingCosts::for_plan_with_input_quotes(
            profile,
            plan,
            prices,
            |_, _| 0.0,
            |input| supply.quote(input, catalog, prices),
        );
        costs.freight += customers.export_freight;
        costs.working_capital += customers.export_freight;
        (customers.revenue > costs.total()).then_some(costs)
    } else {
        supply.set_supported_plan(OperationPlan {
            workers,
            activity: 1.0,
            area,
        });
        industrial_operation_for_plan(
            profile,
            catalog,
            market,
            OperationPlan {
                workers,
                activity: 1.0,
                area,
            },
            prices,
            exports,
            |input| supply.quote(input, catalog, prices),
        )
    };
    let Some(costs) = costs else {
        return false;
    };
    let capital = working_capital_assets_at_prices(
        building,
        profile,
        paid_cargo,
        |resource| supply.input_price(resource, catalog, prices),
        |resource| {
            if let Some(price) = supply.output_price(resource) {
                return price;
            }
            let units = profile
                .output_port(resource)
                .map_or(0.0, |port| profile.net_output_units_per_day(port) * area);
            if units <= EPSILON {
                return 0.0;
            }
            market.attainable_revenue(
                resource,
                units,
                profile.unit_price_currency,
                catalog.unit_price_for_resource(resource).unwrap_or(0.0) * exports,
            ) / f64::from(units)
        },
    );
    supply.fund(profile, prices, costs, capital)
}

fn industrial_operation_for_plan(
    profile: &EconomyProfileRuntime,
    catalog: &RuntimeEconomyCatalog,
    absorption: OutputMarket<'_>,
    plan: OperationPlan,
    prices: OperationPrices,
    exports: f32,
    input_quote: impl FnMut(InputRequirement) -> InputSupplyCosts,
) -> Option<OperatingCosts> {
    let revenue: f64 = profile
        .outputs
        .iter()
        .map(|port| {
            absorption.attainable_revenue(
                port.resource_runtime_id,
                profile.net_output_units_per_day(port) * plan.area * plan.activity,
                profile.unit_price_currency,
                exports
                    * catalog
                        .unit_price_for_resource(port.resource_runtime_id)
                        .unwrap_or(0.0),
            )
        })
        .sum();
    let costs = OperatingCosts::for_plan_with_input_quotes(
        profile,
        plan,
        prices,
        |resource, units| {
            if absorption.export_expansion_available() {
                (units - absorption.unmet_units(resource)).max(0.0)
            } else {
                0.0
            }
        },
        input_quote,
    );
    (revenue > costs.total()).then_some(costs)
}

#[cfg(test)]
pub(super) fn nonresidential_passes_absorption_gate(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    absorption: &OutputAbsorptionContext,
    asset_id: &str,
) -> bool {
    let Some(profile_id) = allocator.registry.economy_profile(asset_id) else {
        return false;
    };
    let Some(candidate_profile) = catalog.profile_for_id(profile_id) else {
        return false;
    };
    candidate_profile.outputs.iter().any(|port| {
        candidate_profile.net_output_units_per_day(port) > EPSILON
            && (absorption.unmet_units(port.resource_runtime_id) > EPSILON
                || (absorption.export_expansion_available()
                    && matches!(
                        candidate_profile.kind,
                        EconomyProfileRuntimeKind::Processor | EconomyProfileRuntimeKind::Producer
                    )))
    })
}

#[cfg(test)]
mod replacement_tests {
    use super::*;
    use crate::simulation::economy::definitions::load_runtime_economy_catalog;

    #[test]
    fn replacement_reuses_only_its_own_customers_without_mutating_rejected_quotes() {
        let catalog = load_runtime_economy_catalog().unwrap();
        let profile = catalog.profile_for_id("grocery_basic").unwrap();
        let port = &profile.outputs[0];
        let units = port.units_per_day;
        let mut market = OutputAbsorptionContext::from_resource_amounts(
            catalog.resource_count(),
            &[(port.resource_runtime_id, units * 1.5)],
            &[(port.resource_runtime_id, units * 1.75)],
            1,
            &[],
        );
        let before = market.placed_output_units_by_resource.clone();
        // This shop accounts for half a unit of capacity; its competitor owns the rest.
        for _ in 0..100 {
            let quote = market.replacement_market(profile, 0.5);
            assert_eq!(quote.unmet_units(port.resource_runtime_id), units * 0.75);
        }
        assert_eq!(market.placed_output_units_by_resource, before);
        market.reserve_replacement(profile, 0.5, profile);
        assert_eq!(market.unmet_units(port.resource_runtime_id), 0.0);
        assert_eq!(
            market
                .replacement_market(profile, 1.0)
                .unmet_units(port.resource_runtime_id),
            units * 0.75,
            "the next replacement cannot reuse the first one's new capacity"
        );
    }
}
