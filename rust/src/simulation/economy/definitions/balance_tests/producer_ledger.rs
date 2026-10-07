// SPDX-License-Identifier: GPL-2.0-only

//! Full-operation farm/mine worksheet, separate from the physical village experiments.
//! Assumes all output sells, inputs/utilities are imported, level-one property tax, and
//! a stated one-way route duration. Steady freight follows authored input reorder lots
//! and one-day export accumulation; real routes and queues require the physical experiment.

use super::*;
use crate::simulation::economy::fiscal::{CityFiscalPolicy, daily_property_tax};
use crate::simulation::economy::households::local_utility_service_costs;
use crate::simulation::work_area::scaled_work_area_worker_capacity;
use crate::simulation::zoning::ZoneType;

struct ProducerLedger {
    workers: u32,
    output_kg: f64,
    sales: f64,
    inputs: f64,
    wages: f64,
    utilities: f64,
    property: f64,
    freight: f64,
    investment: f64,
    after_tax: f64,
}

fn ledger(
    catalog: &RuntimeEconomyCatalog,
    tuning: &RuntimeEconomyTuning,
    profile: &EconomyProfileRuntime,
    hectares: f32,
    one_way_hours: f64,
) -> ProducerLedger {
    let workers = scaled_work_area_worker_capacity(
        profile.workers_per_hectare,
        hectares,
        minimum_work_area_workers(profile.kind),
        profile.area_worker_capacity_max,
    );
    let scale = f64::from(hectares);
    let output_kg = f64::from(profile.outputs[0].units_per_day) * scale;
    let sales =
        output_kg * f64::from(profile.unit_price_currency * tuning.owa_export_price_multiplier);
    let inputs: f64 = purchased_inputs(profile)
        .map(|(id, qty)| {
            qty * scale * unit_price(catalog, id) * f64::from(tuning.owa_import_price_multiplier)
        })
        .sum();
    let wages = f64::from(workers) * f64::from(profile.average_daily_wage());
    let utilities: f64 = profile
        .utility_demand_units_per_day(hectares)
        .into_iter()
        .zip(local_utility_service_costs(catalog))
        .map(|(qty, price)| f64::from(qty * price * tuning.owa_import_price_multiplier))
        .sum();
    let policy = CityFiscalPolicy::from_runtime_tuning(tuning);
    let property = f64::from(daily_property_tax(ZoneType::Industrial, 1, &policy));
    let input_dispatches: f64 = profile
        .inputs
        .iter()
        .map(|port| {
            let daily_kg = f64::from(port.units_per_day) * scale;
            let (target, reorder, _) = profile.input_inventory_targets(port, hectares);
            let batch = port.shipment_batch_kg;
            let lot = if reorder > 0.0 {
                target - reorder
            } else {
                batch
            };
            let lot =
                (lot.max(batch).min(tuning.logistics.truck_payload_kg) / batch).floor() * batch;
            daily_kg / f64::from(lot)
        })
        .sum();
    let export_dispatches: f64 = profile
        .outputs
        .iter()
        .map(|port| {
            let daily_kg = f64::from(profile.net_output_units_per_day(port)) * scale;
            let batch = f64::from(port.shipment_batch_kg);
            let full_load = (f64::from(tuning.logistics.truck_payload_kg) / batch).floor() * batch;
            let lot = ((daily_kg / batch).ceil().max(1.0) * batch).min(full_load);
            daily_kg / lot
        })
        .sum();
    let freight = (input_dispatches + export_dispatches)
        * (f64::from(tuning.logistics.dispatch_fee_currency)
            + 2.0 * one_way_hours * f64::from(tuning.logistics.vehicle_hour_cost_currency));
    // Current starter fixture footprints: 3x3 farm and 4x5 mine at 2,500 per cell.
    let footprint = if profile.kind == EconomyProfileRuntimeKind::Extractor {
        50_000.0
    } else {
        22_500.0
    };
    let investment = footprint + profile.area_expansion_cost(0.0, hectares * 10_000.0);
    let margin = sales - inputs - wages - utilities - property - freight;
    let after_tax = margin - margin.max(0.0) * f64::from(policy.business_profit_tax_rate);
    ProducerLedger {
        workers,
        output_kg,
        sales,
        inputs,
        wages,
        utilities,
        property,
        freight,
        investment,
        after_tax,
    }
}

#[test]
fn producer_sizes_keep_land_costs_and_output_scaling_when_farm_staffing_caps() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    for id in ["grain_farm_basic", "coal_mine_basic"] {
        let profile = catalog.profile_for_id(id).unwrap();
        let small = ledger(&catalog, &tuning, profile, 1.0, 0.5);
        for (hectares, farm_workers) in [(1.0, 2), (4.0, 2), (20.0, 2), (100.0, 6)] {
            let b = ledger(&catalog, &tuning, profile, hectares, 0.5);
            let farm = profile.kind == EconomyProfileRuntimeKind::FieldProducer;
            assert_eq!(
                b.workers,
                if farm {
                    farm_workers
                } else {
                    small.workers * hectares as u32
                }
            );
            assert_eq!(b.output_kg, small.output_kg * f64::from(hectares));
            assert_eq!(b.inputs, small.inputs * f64::from(hectares));
            // A one-hectare cereal plot cannot fund two full-time wages. Commercial
            // grain farms begin at 4 ha; mine staffing and its 1 ha margin are independent.
            assert_eq!(b.after_tax > 0.0, !farm || hectares >= 4.0);
            assert_eq!(
                b.investment - small.investment,
                f64::from(hectares - 1.0) * f64::from(profile.area_build_cost_per_hectare)
            );
            if hectares > 1.0 {
                assert!(b.utilities > small.utilities);
            }
            let remote = ledger(&catalog, &tuning, profile, hectares, 4.0);
            assert!(remote.freight > b.freight && remote.after_tax < b.after_tax);
            assert_eq!(remote.after_tax > 0.0, !farm || hectares >= 4.0);
        }
    }
}

#[test]
#[ignore = "manual producer worksheet; CSV on stderr, no simulation or timing claims"]
fn producer_balance_sheet() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    eprintln!(
        "profile,ha,one_way_hours,workers,kg_per_day,sales,inputs,wages,utilities,property,freight,investment,after_tax,payback_days"
    );
    for id in ["grain_farm_basic", "coal_mine_basic"] {
        for hectares in [1.0, 4.0, 20.0, 100.0] {
            for hours in [0.5, 4.0] {
                let b = ledger(
                    &catalog,
                    &tuning,
                    catalog.profile_for_id(id).unwrap(),
                    hectares,
                    hours,
                );
                eprintln!(
                    "{id},{hectares},{hours},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2}",
                    b.workers,
                    b.output_kg,
                    b.sales,
                    b.inputs,
                    b.wages,
                    b.utilities,
                    b.property,
                    b.freight,
                    b.investment,
                    b.after_tax,
                    if b.after_tax > 0.0 {
                        b.investment / b.after_tax
                    } else {
                        f64::INFINITY
                    }
                );
            }
        }
    }
}
