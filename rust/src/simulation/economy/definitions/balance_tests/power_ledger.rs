// SPDX-License-Identifier: GPL-2.0-only

//! Recurring power margin with imported fuel, fixed staffed upkeep and amortized freight.
//! This worksheet is independent of physical trips, startup capital and construction costs.

use super::*;

fn daily_margin(workers: u32, external_kwh: f32) -> f32 {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let plant = catalog.profile_for_id("power_plant_basic").unwrap();
    let staffing = workers as f32 / plant.worker_capacity as f32;
    let own_kwh = plant.electricity_demand_kwh_per_day(0.0, staffing, 0);
    let generation = (external_kwh + own_kwh).min(plant.power_capacity_kw * 24.0 * staffing);
    let revenue = (generation - own_kwh).max(0.0) * plant.unit_price_currency;
    let fuel = catalog.resource_runtime_id_for_id("coal").unwrap();
    let mut inputs = 0.0;
    let mut freight = 0.0;
    for port in &plant.inputs {
        // Coal follows dispatch; machinery upkeep follows staffed capacity even at low sales.
        let quantity = if port.resource_runtime_id == fuel {
            generation / plant.base_rate_units_per_day * port.units_per_day
        } else {
            port.units_per_day * staffing
        };
        inputs += quantity
            * catalog
                .unit_price_for_resource(port.resource_runtime_id)
                .unwrap()
            * tuning.owa_import_price_multiplier;
        let (target, reorder, _) = plant.input_inventory_targets(port, staffing);
        let batch = port.shipment_batch_kg;
        let lot = ((target - reorder)
            .max(batch)
            .min(tuning.logistics.truck_payload_kg)
            / batch)
            .floor()
            * batch;
        // Half-hour each way: one vehicle-hour per dispatch.
        freight += quantity / lot
            * (tuning.logistics.dispatch_fee_currency
                + tuning.logistics.vehicle_hour_cost_currency);
    }
    let water_sewage: f32 = ["water_plant_basic", "wastewater_treatment_basic"]
        .iter()
        .map(|id| {
            catalog.profile_for_id(id).unwrap().unit_price_currency
                * tuning.owa_import_price_multiplier
        })
        .sum();
    revenue - inputs - freight - workers as f32 * plant.average_daily_wage() - water_sewage
}

#[test]
fn reference_towns_cover_staffed_power_without_startup_capital() {
    // 170 residents * 5 + 85 occupied homes * 1 + 65 workplace kWh = 1,000 kWh/day.
    for (workers, expected_margin) in [(1, 13.0125), (2, 35.62), (3, 58.2125)] {
        let margin = daily_margin(workers, workers as f32 * 1000.0);
        assert!(
            (margin - expected_margin).abs() < 0.01,
            "{workers} workers: {margin}"
        );
    }
    assert!(
        daily_margin(2, 1000.0) < 0.0,
        "spare capacity alone cannot fund a second worker"
    );
    assert!(
        daily_margin(1, 0.0) < -115.0,
        "idle plants still pay upkeep and wages"
    );
}

#[test]
fn electricity_profiles_preserve_standby_area_and_resident_demand() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let home = catalog.profile_for_id("basic_household_demand").unwrap();
    assert_eq!(home.electricity_demand_kwh_per_day(0.0, 0.0, 2), 11.0);
    let mine = catalog.profile_for_id("coal_mine_basic").unwrap();
    assert_eq!(mine.electricity_demand_kwh_per_day(1.0, 0.0, 0), 62.0);
    assert_eq!(mine.electricity_demand_kwh_per_day(1.0, 1.0, 0), 602.0);
    assert_eq!(mine.electricity_demand_kwh_per_day(2.0, 0.5, 0), 662.0);
    let plant = catalog.profile_for_id("power_plant_basic").unwrap();
    assert_eq!(
        plant.base_rate_units_per_day,
        plant.power_capacity_kw * 24.0
    );
    assert_eq!(plant.utility_quantity_unit.unwrap().symbol(), "kWh");
}
