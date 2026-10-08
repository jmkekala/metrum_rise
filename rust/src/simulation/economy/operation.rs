// SPDX-License-Identifier: GPL-2.0-only

//! Recurring operating costs and working capital, independent of owner startup equity.

use super::definitions::{EconomyProfileRuntime, ResourceRuntimeId, RuntimeEconomyCatalog};
use super::logistics::{export_dispatch_target_kg, quantize_requested_amount};
use crate::simulation::buildings::allocator::Building;

/// Physical operation proposed by either hiring or construction planning.
#[derive(Clone, Copy)]
pub(crate) struct OperationPlan {
    /// Workers whose gross daily wages must be covered by sales.
    pub workers: u32,
    /// Fraction of full production; integer staffing may exceed this demand-limited fraction.
    pub activity: f32,
    /// Explicit production area in hectares; ordinary buildings use one.
    pub area: f32,
}

/// Site-specific prices, taxes and carrier quotes supplied by the caller.
#[derive(Clone, Copy)]
pub(crate) struct OperationPrices {
    /// Actual or fallback prices for electricity, water and sewage, in that order.
    pub utilities: [f32; 3],
    /// Daily property bill at this site's zone and level.
    pub property_tax: f32,
    /// Multiplier for externally purchased input goods, excluding carrier fees.
    pub imports: f32,
    /// Price of one paid round-trip carrier contract on the site's supply/export route.
    pub carrier_quote: f32,
    /// Physical payload limit used by the freight dispatcher.
    pub truck_payload: f32,
}

/// Net purchased input and circulation stock required by one proposed operation.
#[derive(Clone, Copy, Debug)]
pub(crate) struct InputRequirement {
    /// Resource to be supplied; internally produced units are excluded from daily purchases.
    pub resource: ResourceRuntimeId,
    /// Recurring purchased quantity at the proposed staffing and work area.
    pub daily_units: f32,
    /// Opening buffer, including stock needed to start a self-replenishing recipe.
    pub buffer_units: f32,
    /// Replenishment quantity after the dispatcher's batch/payload rounding.
    pub replenishment_batch_units: Option<f32>,
}

/// Source-specific input costs; a quote may combine finite local supply and outside purchases.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct InputSupplyCosts {
    /// Recurring goods payments, excluding carriers and opening inventory.
    pub goods: f32,
    /// Recurring carrier payments amortized over the actual quoted deliveries.
    pub freight: f32,
    /// Replacement value of the complete required input buffer.
    pub buffer: f32,
}

impl InputRequirement {
    /// Prices the outside option through a real carrier quote supplied by the caller.
    pub(crate) fn imported(
        self,
        catalog: &RuntimeEconomyCatalog,
        prices: OperationPrices,
    ) -> InputSupplyCosts {
        self.at_prices(
            catalog
                .unit_price_for_resource(self.resource)
                .unwrap_or(0.0)
                * prices.imports,
            prices.carrier_quote,
        )
    }

    /// Prices one source using the shared dispatch batch, without allocating or claiming stock.
    pub(crate) fn at_prices(self, unit_price: f32, carrier_quote: f32) -> InputSupplyCosts {
        InputSupplyCosts {
            goods: self.daily_units * unit_price,
            buffer: self.buffer_units * unit_price,
            freight: if self.daily_units > 0.0 {
                self.replenishment_batch_units
                    .map_or(f32::INFINITY, |batch| {
                        self.daily_units / batch * carrier_quote
                    })
            } else {
                0.0
            },
        }
    }
}

/// Values cash, paid cargo and stock that the proposed recipe can use or sell.
/// Shared by live hiring and upgrades; input/output overlap is counted only once.
pub(crate) fn working_capital_assets(
    building: &Building,
    profile: &EconomyProfileRuntime,
    catalog: &RuntimeEconomyCatalog,
    import_multiplier: f32,
    paid_cargo: f64,
    output_price: impl Fn(ResourceRuntimeId) -> f32,
) -> f64 {
    working_capital_assets_at_prices(
        building,
        profile,
        paid_cargo,
        |resource| catalog.unit_price_for_resource(resource).unwrap_or(0.0) * import_multiplier,
        output_price,
    )
}

/// Values stock using the same per-resource replacement prices as the operation's supply quote.
/// Input/output overlap is counted once; carrier charges are never inventory value. O(I + O*I),
/// plus caller-owned price lookups, without allocations or changes to the building's cash.
pub(crate) fn working_capital_assets_at_prices(
    building: &Building,
    profile: &EconomyProfileRuntime,
    paid_cargo: f64,
    input_price: impl Fn(ResourceRuntimeId) -> f32,
    output_price: impl Fn(ResourceRuntimeId) -> f32,
) -> f64 {
    let mut capital = f64::from(building.operating_budget) + paid_cargo;
    for input in &profile.inputs {
        capital += f64::from(building.inventory_units(input.resource_runtime_id))
            * f64::from(input_price(input.resource_runtime_id));
    }
    for output in &profile.outputs {
        if profile.input_port(output.resource_runtime_id).is_none() {
            capital += f64::from(
                building.inventory_units(output.resource_runtime_id)
                    * output_price(output.resource_runtime_id),
            );
        }
    }
    capital
}

/// Recurring expenses for one operation; opening equity never enters these quantities.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OperatingCosts {
    /// Gross wages due each operational day.
    pub payroll: f32,
    /// Purchased input goods consumed daily, excluding internally replenished resources.
    pub inputs: f32,
    /// Utilities and property tax, including costs incurred while production is idle.
    pub overhead: f32,
    /// Carrier charges amortized over actual replenishment/export batch quantities.
    pub freight: f32,
    /// Input buffers plus the next payroll/overhead payment, valued at replacement cost.
    pub working_capital: f32,
}

impl OperatingCosts {
    /// Evaluates ports without allocations: O(I*O + (I+O)*log(resources)) with price lookups.
    /// The callback allocates each net output's units to its own outside buyers.
    pub(crate) fn for_plan(
        profile: &EconomyProfileRuntime,
        catalog: &RuntimeEconomyCatalog,
        plan: OperationPlan,
        prices: OperationPrices,
        export_units: impl Fn(ResourceRuntimeId, f32) -> f32,
    ) -> Self {
        Self::for_plan_with_input_quotes(profile, plan, prices, export_units, |input| {
            input.at_prices(
                catalog
                    .unit_price_for_resource(input.resource)
                    .unwrap_or(0.0)
                    * prices.imports,
                prices.carrier_quote,
            )
        })
    }

    /// Evaluates the same recipe with a resource-specific supply quotation for every input.
    /// Quotes must cover the whole requirement and must not consume shared supply on a rejected
    /// plan. Unavailable supply has infinite cost. This adds O(I) quote calls, no allocations;
    /// source lookup and finite-capacity reservations remain owned by the market caller.
    pub(crate) fn for_plan_with_input_quotes(
        profile: &EconomyProfileRuntime,
        plan: OperationPlan,
        prices: OperationPrices,
        export_units: impl Fn(ResourceRuntimeId, f32) -> f32,
        mut input_quote: impl FnMut(InputRequirement) -> InputSupplyCosts,
    ) -> Self {
        let activity = plan.activity.clamp(0.0, 1.0);
        let area =
            if crate::simulation::work_area::profile_kind_uses_explicit_work_area(profile.kind) {
                crate::simulation::work_area::sanitize_work_area_scale(plan.area)
            } else {
                1.0
            };
        let production = activity * area;
        let demand = profile.utility_demand_units_per_day(plan.area);
        let electricity = profile.electricity_demand_kwh_per_day(plan.area, activity, 0);
        let mut costs = Self {
            payroll: plan.workers as f32 * profile.average_daily_wage(),
            overhead: electricity * prices.utilities[0]
                + demand[1] * prices.utilities[1]
                + demand[2] * prices.utilities[2]
                + prices.property_tax,
            ..Default::default()
        };
        for input in &profile.inputs {
            let internal = profile
                .output_port(input.resource_runtime_id)
                .map_or(0.0, |output| output.units_per_day);
            let daily_units = (input.units_per_day - internal).max(0.0) * production;
            let (target, reorder, _) = profile.input_inventory_targets(input, production);
            let quote = input_quote(InputRequirement {
                resource: input.resource_runtime_id,
                daily_units,
                buffer_units: target,
                replenishment_batch_units: if daily_units > 0.0 {
                    quantize_requested_amount(
                        (target - reorder).max(input.shipment_batch_kg),
                        f32::INFINITY,
                        f32::INFINITY,
                        input.shipment_batch_kg,
                        true,
                        prices.truck_payload,
                    )
                } else {
                    None
                },
            });
            costs.inputs += quote.goods;
            costs.working_capital += quote.buffer;
            costs.freight += quote.freight;
        }
        for output in &profile.outputs {
            let daily_units = profile.net_output_units_per_day(output) * production;
            let exported_units =
                export_units(output.resource_runtime_id, daily_units).clamp(0.0, daily_units);
            if exported_units > 0.0 {
                // The dispatcher batches a production day, rounded up to the resource's
                // legal shipment quantum. It does not require filling the entire truck.
                // Ordinary factories retain a full authored day's batch even while
                // understaffed; area producers dispatch their realized daily output.
                let batch_output =
                    if crate::simulation::work_area::profile_kind_uses_explicit_work_area(
                        profile.kind,
                    ) {
                        daily_units
                    } else {
                        profile.net_output_units_per_day(output)
                    };
                let batch = export_dispatch_target_kg(
                    batch_output,
                    output.shipment_batch_kg,
                    prices.truck_payload,
                );
                costs.freight += exported_units / batch * prices.carrier_quote;
            }
        }
        costs.working_capital += costs.payroll + costs.overhead + costs.freight;
        costs
    }

    /// Daily sales must cover this amount without spending owner equity.
    pub(crate) fn total(self) -> f32 {
        self.payroll + self.inputs + self.overhead + self.freight
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::economy::definitions::{
        load_runtime_economy_catalog, load_runtime_economy_tuning,
    };

    #[test]
    fn resource_supply_quotes_can_fund_a_recipe_that_import_only_prices_reject() {
        let catalog = load_runtime_economy_catalog().unwrap();
        let profile = catalog.profile_for_id("machinery_factory_basic").unwrap();
        let steel = catalog.resource_runtime_id_for_id("steel").unwrap();
        let machinery = catalog.resource_runtime_id_for_id("machinery").unwrap();
        let plan = OperationPlan {
            workers: 4,
            activity: 1.0,
            area: 1.0,
        };
        let mut prices = OperationPrices {
            utilities: [0.0; 3],
            property_tax: 0.0,
            imports: 1.25,
            carrier_quote: 40.0,
            truck_payload: 20_000.0,
        };
        let quoted_input = |input: InputRequirement| {
            if input.resource == machinery {
                assert_eq!(input.daily_units, 0.0);
                assert!(
                    input.buffer_units > 0.0,
                    "self-supply still needs circulation stock"
                );
            }
            let (multiplier, carrier) = if input.resource == steel {
                (1.0, 2.0)
            } else {
                (1.25, 40.0)
            };
            input.at_prices(
                catalog.unit_price_for_resource(input.resource).unwrap() * multiplier,
                carrier,
            )
        };
        let imported = OperatingCosts::for_plan(profile, &catalog, plan, prices, |_, n| n);
        let local = OperatingCosts::for_plan_with_input_quotes(
            profile,
            plan,
            prices,
            |_, n| n,
            quoted_input,
        );
        let revenue: f32 = profile
            .outputs
            .iter()
            .map(|output| {
                profile.net_output_units_per_day(output)
                    * catalog
                        .unit_price_for_resource(output.resource_runtime_id)
                        .unwrap()
            })
            .sum();
        prices.property_tax = revenue - (imported.total() + local.total()) / 2.0;
        assert!(prices.property_tax > 0.0);
        let imported = OperatingCosts::for_plan(profile, &catalog, plan, prices, |_, n| n);
        let local = OperatingCosts::for_plan_with_input_quotes(
            profile,
            plan,
            prices,
            |_, n| n,
            quoted_input,
        );
        assert!(imported.total() > revenue);
        assert!(local.total() < revenue);
        assert!(local.working_capital < imported.working_capital);
        assert_eq!(local.payroll, imported.payroll);
        assert_eq!(local.overhead, imported.overhead);
        let local_sales = OperatingCosts::for_plan_with_input_quotes(
            profile,
            plan,
            prices,
            |_, _| 0.0,
            quoted_input,
        );
        let imported_local_sales =
            OperatingCosts::for_plan(profile, &catalog, plan, prices, |_, _| 0.0);
        assert_eq!(
            local.freight - local_sales.freight,
            imported.freight - imported_local_sales.freight,
            "local input freight cannot discount the separate export route"
        );
    }

    #[test]
    fn unavailable_input_supply_cannot_be_paid_for_by_startup_cash() {
        let catalog = load_runtime_economy_catalog().unwrap();
        let prices = OperationPrices {
            utilities: [0.0; 3],
            property_tax: 0.0,
            imports: 1.25,
            carrier_quote: 0.0,
            truck_payload: 20_000.0,
        };
        let plan = OperationPlan {
            workers: 4,
            activity: 1.0,
            area: 1.0,
        };
        let factory = catalog.profile_for_id("food_processor_basic").unwrap();
        let unavailable = |_| InputSupplyCosts {
            goods: f32::INFINITY,
            freight: f32::INFINITY,
            buffer: f32::INFINITY,
        };
        let blocked = OperatingCosts::for_plan_with_input_quotes(
            factory,
            plan,
            prices,
            |_, n| n,
            unavailable,
        );
        assert!(blocked.total().is_infinite());
        assert!(blocked.working_capital.is_infinite());
        let service = catalog.profile_for_id("personal_service_small").unwrap();
        let supported = OperatingCosts::for_plan_with_input_quotes(
            service,
            plan,
            prices,
            |_, _| 0.0,
            |_| panic!("an input-free service needs no supply quote"),
        );
        assert!(supported.total().is_finite());
    }

    #[test]
    fn factory_costs_include_profile_power_and_daily_export_contracts() {
        let catalog = load_runtime_economy_catalog().unwrap();
        let tuning = load_runtime_economy_tuning().unwrap();
        let profile = catalog.profile_for_id("food_processor_basic").unwrap();
        let plan = OperationPlan {
            workers: profile.worker_capacity,
            activity: 1.0,
            area: 1.0,
        };
        let prices = OperationPrices {
            utilities: [2.0, 3.0, 4.0],
            property_tax: 5.0,
            imports: 1.25,
            carrier_quote: 12.0,
            truck_payload: tuning.logistics.truck_payload_kg,
        };
        let domestic = OperatingCosts::for_plan(profile, &catalog, plan, prices, |_, _| 0.0);
        let exports = OperatingCosts::for_plan(profile, &catalog, plan, prices, |_, units| units);
        assert_eq!(exports.overhead, 32.0 * 2.0 + 3.0 + 4.0 + 5.0);
        assert!(
            (exports.freight - domestic.freight - 12.0).abs() < 0.001,
            "one daily food export pays a complete carrier contract"
        );
        assert!(
            exports.working_capital > exports.total(),
            "opening buffers require working capital"
        );
        let distant = OperatingCosts::for_plan(
            profile,
            &catalog,
            plan,
            OperationPrices {
                carrier_quote: 24.0,
                ..prices
            },
            |_, units| units,
        );
        assert!((distant.freight - 2.0 * exports.freight).abs() < 0.001);
    }

    #[test]
    fn internally_produced_machinery_is_not_bought_again() {
        let catalog = load_runtime_economy_catalog().unwrap();
        let profile = catalog.profile_for_id("machinery_factory_basic").unwrap();
        let costs = OperatingCosts::for_plan(
            profile,
            &catalog,
            OperationPlan {
                workers: 4,
                activity: 1.0,
                area: 1.0,
            },
            OperationPrices {
                utilities: [0.0; 3],
                property_tax: 0.0,
                imports: 1.25,
                carrier_quote: 0.0,
                truck_payload: 20_000.0,
            },
            |_, units| units,
        );
        let steel = catalog.resource_runtime_id_for_id("steel").unwrap();
        let metals = catalog.resource_runtime_id_for_id("metals").unwrap();
        let expected = (30.0 * catalog.unit_price_for_resource(steel).unwrap()
            + 10.0 * catalog.unit_price_for_resource(metals).unwrap())
            * 1.25;
        assert!((costs.inputs - expected).abs() < 0.001);
    }
}
