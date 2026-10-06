// SPDX-License-Identifier: GPL-2.0-only

//! Static starter-economy ledger for ECON-14, using the compiled shipped catalog.
//!
//! This is an accounting worksheet, not a second growth simulator. It assumes paid sales,
//! steady inventory, available inputs, level-one buildings and imported utilities. It excludes
//! delivery delays, startup purchases and cash-based profit tax. Scenario tests must establish
//! actual utilization, working capital, migration and survival before any tuning is accepted.

use super::*;
use crate::simulation::economy::agents::{HouseholdAgeComposition, household_age_composition};
use crate::simulation::economy::fiscal::{CityFiscalPolicy, daily_property_tax, tax_amount};
use crate::simulation::economy::households::candidate_immigrant_household_size_for_vacancy;
use crate::simulation::zoning::ZoneType;

struct Ledger<'a> {
    catalog: &'a RuntimeEconomyCatalog,
    tuning: &'a RuntimeEconomyTuning,
    policy: CityFiscalPolicy,
}

#[derive(Clone, Copy)]
struct Market {
    // Hypothetical import multiplier, including utility fallback. Never mutates live tuning.
    imports: f64,
    input_freight: f64,
    output_price: f64,
}

struct BusinessBalance {
    sales: f64,
    inputs: f64,
    wages: f64,
    overhead: f64,
}

impl BusinessBalance {
    fn margin(&self) -> f64 {
        self.sales - self.inputs - self.wages - self.overhead
    }

    fn required_sales(&self) -> f64 {
        self.inputs + self.wages + self.overhead
    }
}

struct HouseholdBalance {
    net_wages: f64,
    transfers: f64,
    spending: f64,
    taxes: f64,
}

impl HouseholdBalance {
    fn surplus(&self) -> f64 {
        self.net_wages + self.transfers - self.spending
    }
}

impl<'a> Ledger<'a> {
    fn new(catalog: &'a RuntimeEconomyCatalog, tuning: &'a RuntimeEconomyTuning) -> Self {
        Self {
            catalog,
            tuning,
            policy: CityFiscalPolicy::from_runtime_tuning(tuning),
        }
    }

    fn profile(&self, id: &str) -> &EconomyProfileRuntime {
        self.catalog.profile_for_id(id).unwrap()
    }

    fn utility_unit_cost(&self) -> f64 {
        ["power", "water", "sewage"]
            .iter()
            .map(|service| {
                f64::from(
                    self.catalog
                        .all_profiles()
                        .iter()
                        .find(|p| p.utility_service.as_deref() == Some(service))
                        .unwrap()
                        .unit_price_currency,
                )
            })
            .sum()
    }

    // One-output ordinary businesses only. Explicit area producers need their own area case.
    // sold_units is net saleable output; self-upkeep is removed before scaling external inputs.
    fn business(
        &self,
        profile: &EconomyProfileRuntime,
        workers: u32,
        sold_units: f64,
        market: Market,
    ) -> BusinessBalance {
        assert!(!profile_kind_uses_explicit_work_area(profile.kind));
        assert_eq!(profile.outputs.len(), 1);
        let output = &profile.outputs[0];
        let net_output = f64::from(profile.net_output_units_per_day(output));
        let scale = sold_units / net_output;
        assert!(workers <= profile.worker_capacity);
        assert!(
            scale >= 0.0 && scale <= f64::from(workers) / f64::from(profile.worker_capacity) + 1e-9
        );
        let inputs = purchased_inputs(profile)
            .map(|(resource, units)| {
                units
                    * scale
                    * unit_price(self.catalog, resource)
                    * market.imports
                    * market.input_freight
            })
            .sum();
        let zone = if matches!(
            profile.kind,
            EconomyProfileRuntimeKind::Store | EconomyProfileRuntimeKind::ServiceStore
        ) {
            ZoneType::Commercial
        } else {
            ZoneType::Industrial
        };
        BusinessBalance {
            sales: sold_units
                * unit_price(self.catalog, output.resource_runtime_id)
                * market.output_price,
            inputs,
            wages: f64::from(workers) * f64::from(profile.average_daily_wage()),
            overhead: self.utility_unit_cost() * market.imports
                + f64::from(daily_property_tax(zone, 1, &self.policy)),
        }
    }

    // All modeled consumer categories are available and paid. Inventory groceries charge VAT;
    // aggregate service sales currently do not. No rent payment exists in the current model.
    fn consumer_costs(&self, imports: f64) -> (f64, f64) {
        let mut spending =
            f64::from(self.tuning.households.utility_cost_per_member_per_day) * imports;
        let mut vat = 0.0;
        for sink in self
            .catalog
            .all_profiles()
            .iter()
            .filter(|p| p.kind == EconomyProfileRuntimeKind::DemandSink)
        {
            for port in &sink.inputs {
                let value = f64::from(sink.consumption_rate_per_resident)
                    * unit_price(self.catalog, port.resource_runtime_id);
                spending += value;
                let producer = local_producer(self.catalog, port.resource_runtime_id).unwrap();
                if producer.kind == EconomyProfileRuntimeKind::Store {
                    let tax = f64::from(tax_amount(value as f32, self.policy.household_vat_rate));
                    spending += tax;
                    vat += tax;
                } else {
                    assert_eq!(producer.kind, EconomyProfileRuntimeKind::ServiceStore);
                }
            }
        }
        (spending, vat)
    }

    fn household(
        &self,
        composition: HouseholdAgeComposition,
        employed: u16,
        unemployment_active: bool,
        wage: f32,
        imports: f64,
    ) -> HouseholdBalance {
        assert!(employed <= composition.adult_count);
        let members =
            f64::from(composition.adult_count + composition.child_count + composition.elder_count);
        let income_tax =
            f64::from(tax_amount(wage, self.policy.income_tax_rate)) * f64::from(employed);
        let transfers = f64::from(composition.child_count)
            * f64::from(self.policy.child_support_per_child_per_day)
            + f64::from(composition.elder_count) * f64::from(self.policy.pension_per_elder_per_day)
            + if unemployment_active {
                f64::from(composition.adult_count - employed)
                    * f64::from(self.policy.unemployment_benefit_per_adult_per_day)
            } else {
                0.0
            };
        let (cost, vat) = self.consumer_costs(imports);
        let property = f64::from(daily_property_tax(ZoneType::Residential, 1, &self.policy));
        HouseholdBalance {
            net_wages: f64::from(wage) * f64::from(employed) - income_tax,
            transfers,
            spending: cost * members + property,
            taxes: income_tax + vat * members + property,
        }
    }
}

#[test]
fn ledger_nets_machinery_upkeep_before_pricing_external_inputs() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let ledger = Ledger::new(&catalog, &tuning);
    let profile = ledger.profile("machinery_factory_basic");
    let balance = ledger.business(
        profile,
        profile.worker_capacity,
        40.0,
        Market {
            imports: f64::from(tuning.owa_import_price_multiplier),
            input_freight: 1.0,
            output_price: 1.0,
        },
    );
    // Forty saleable units need one complete 42-unit production cycle, not 40/42 of its inputs.
    assert!(
        (balance.inputs
            - (12.0 * 8.0 + 4.0 * 16.0) * f64::from(tuning.owa_import_price_multiplier))
        .abs()
            < 1e-6
    );
    assert!((balance.sales - 40.0 * 20.0).abs() < 1e-6);
}

#[test]
fn household_ledger_reconciles_wages_vat_property_and_transfers() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let ledger = Ledger::new(&catalog, &tuning);
    let composition = HouseholdAgeComposition {
        adult_count: 1,
        child_count: 1,
        elder_count: 0,
    };
    let balance = ledger.household(composition, 1, false, 90.0, 1.75);
    // Net 79.20 + child support 10 - (2 * 33.55 + property 2).
    assert!((balance.surplus() - 20.1).abs() < 1e-4);
    // Treasury gets withheld income tax + grocery VAT + property, never VAT on service sales.
    assert!((balance.taxes - (10.8 + 4.0 + 2.0)).abs() < 1e-4);
    let unemployed = ledger.household(composition, 0, false, 90.0, 1.75);
    assert!((unemployed.transfers - 10.0).abs() < 1e-4);
}

#[test]
#[ignore = "manual ECON-14 accounting worksheet; prints current and hypothetical cases, not a growth acceptance test"]
fn starter_town_balance_sheet() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let ledger = Ledger::new(&catalog, &tuning);
    let current = f64::from(tuning.owa_import_price_multiplier);
    let grocery = ledger.profile("grocery_basic");
    let wage = grocery.average_daily_wage();
    println!(
        "Static ledger: full payment, steady inventories, level 1, imported utilities.\nMargins exclude cash-based profit tax. Hypothetical import values do not modify tuning."
    );
    for imports in [current, 1.50, 1.25] {
        let market = Market {
            imports,
            input_freight: 1.0,
            output_price: 1.0,
        };
        let (per_resident, _) = ledger.consumer_costs(imports);
        println!(
            "\nIMPORT {imports:.2}; consumer cost/resident {per_resident:.3} plus household property tax"
        );
        println!(
            "HOUSEHOLD adults children elders employed support net_wages transfers costs surplus"
        );
        for (adults, children, elders, employed, support) in [
            (1, 0, 0, 1, false),
            (2, 0, 0, 1, false),
            (2, 0, 0, 2, false),
            (1, 1, 0, 1, false),
            (1, 2, 0, 1, false),
            (1, 3, 0, 1, false),
            (1, 5, 0, 1, false),
            (2, 2, 0, 1, false),
            (2, 2, 0, 2, false),
            (0, 0, 1, 0, false),
            (1, 0, 0, 0, true),
            (1, 0, 0, 0, false),
        ] {
            let composition = HouseholdAgeComposition {
                adult_count: adults,
                child_count: children,
                elder_count: elders,
            };
            let h = ledger.household(composition, employed, support, wage, imports);
            println!(
                "HH {adults} {children} {elders} {employed} {support} {:.3} {:.3} {:.3} {:.3}",
                h.net_wages,
                h.transfers,
                h.spending,
                h.surplus()
            );
        }
        println!(
            "BUSINESS full-staff local sales: sales inputs payroll overhead margin required_effective_bid"
        );
        for id in [
            "grocery_basic",
            "food_processor_basic",
            "machinery_factory_basic",
        ] {
            let p = ledger.profile(id);
            let output = f64::from(p.net_output_units_per_day(&p.outputs[0]));
            let b = ledger.business(p, p.worker_capacity, output, market);
            println!(
                "BUSINESS {id} {:.3} {:.3} {:.3} {:.3} {:.3} {:.6}",
                b.sales,
                b.inputs,
                b.wages,
                b.overhead,
                b.margin(),
                b.required_sales() / b.sales
            );
            if id == "grocery_basic" {
                let freight = tuning
                    .operational_clock
                    .freight_profiles
                    .iter()
                    .find(|f| Some(f.id.as_str()) == p.freight_timing_profile.as_deref())
                    .unwrap();
                let peak = ledger.business(
                    p,
                    p.worker_capacity,
                    output,
                    Market {
                        input_freight: f64::from(freight.outside_window_cost_multiplier),
                        ..market
                    },
                );
                println!("GROCERY outside-window margin {:.3}", peak.margin());
            } else {
                let export = ledger.business(
                    p,
                    p.worker_capacity,
                    output,
                    Market {
                        output_price: f64::from(tuning.owa_export_price_multiplier)
                            * f64::from(tuning.logistics.owa_export_saturation_floor_factor),
                        ..market
                    },
                );
                println!(
                    "EXPORT {id} current saturated margin {:.3}",
                    export.margin()
                );
            }
        }
        let capacity = f64::from(grocery.net_output_units_per_day(&grocery.outputs[0]));
        println!("GROCERY residents workers ordinary-margin outside-window-margin");
        let freight = tuning
            .operational_clock
            .freight_profiles
            .iter()
            .find(|f| Some(f.id.as_str()) == grocery.freight_timing_profile.as_deref())
            .unwrap();
        for residents in [10, 13, 14, 20, 26, 27, 40, 80, 100, 200] {
            let units = f64::from(residents)
                * f64::from(
                    ledger
                        .profile("basic_household_demand")
                        .consumption_rate_per_resident,
                );
            let workers = (units / capacity * f64::from(grocery.worker_capacity)).ceil() as u32;
            let b = ledger.business(grocery, workers, units, market);
            let peak = ledger.business(
                grocery,
                workers,
                units,
                Market {
                    input_freight: f64::from(freight.outside_window_cost_multiplier),
                    ..market
                },
            );
            println!(
                "GROCERY {residents} {workers} {:.3} {:.3}",
                b.margin(),
                peak.margin()
            );
        }
        println!("SERVICE minimum whole residents with nonnegative margin, full payment");
        for (profile_id, sink_id) in [
            ("personal_service_small", "personal_service_demand"),
            ("health_essentials_small", "health_essentials_demand"),
        ] {
            let p = ledger.profile(profile_id);
            let rate = f64::from(ledger.profile(sink_id).consumption_rate_per_resident);
            let capacity = f64::from(p.outputs[0].units_per_day);
            let first = (1..=(capacity / rate).floor() as u32).find(|&residents| {
                let units = f64::from(residents) * rate;
                let workers = (units / capacity * f64::from(p.worker_capacity)).ceil() as u32;
                ledger.business(p, workers, units, market).margin() >= 0.0
            });
            println!("SERVICE {profile_id} {first:?}");
        }
        println!(
            "TOWN 100m2 homes, home_id=household_id, slot=0; all adults earn grocery wage; no unemployment benefit"
        );
        println!(
            "households residents adults children elders required_jobs local_chain_jobs households_in_deficit household_surplus taxes_minus_transfers"
        );
        for households in [20, 50, 100] {
            let mut people = [0u32; 3];
            let (mut spending, mut transfers, mut taxes, mut surplus, mut failures) =
                (0.0, 0.0, 0.0, 0.0, 0);
            for id in 0..households {
                let size = candidate_immigrant_household_size_for_vacancy(100.0, id, 0).unwrap();
                let composition = household_age_composition(id, id, size);
                people[0] += u32::from(composition.adult_count);
                people[1] += u32::from(composition.child_count);
                people[2] += u32::from(composition.elder_count);
                let h =
                    ledger.household(composition, composition.adult_count, false, wage, imports);
                spending += h.spending;
                transfers += h.transfers;
                taxes += h.taxes;
                surplus += h.surplus();
                failures += usize::from(h.surplus() < 0.0);
            }
            let residents: u32 = people.iter().sum();
            let net_wage = f64::from(wage - tax_amount(wage, ledger.policy.income_tax_rate));
            let required_jobs = (spending - transfers).max(0.0) / net_wage;
            println!(
                "TOWN {households} {residents} {} {} {} {required_jobs:.3} {:.3} {failures} {surplus:.3} {:.3}",
                people[0],
                people[1],
                people[2],
                closed_chain_jobs_per_resident(&catalog) * f64::from(residents),
                taxes - transfers
            );
        }
    }
}
