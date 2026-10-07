// SPDX-License-Identifier: GPL-2.0-only

//! Tests for shipped economy runtime tuning validation.

#[test]
fn physical_recipes_use_kg_and_authored_sales_values() {
    use super::QuantityUnit;
    let catalog = super::load_runtime_economy_catalog().unwrap();
    assert_eq!(
        super::load_runtime_economy_tuning()
            .unwrap()
            .logistics
            .truck_payload_kg,
        20_000.0
    );
    for resource in [
        "household_supplies",
        "packaged_food",
        "grain",
        "coal",
        "machinery",
        "steel",
        "metals",
    ] {
        let id = catalog.resource_runtime_id_for_id(resource).unwrap();
        assert_eq!(
            catalog.resource_measure(id).unwrap().unit,
            QuantityUnit::Kilogram
        );
    }
    for resource in ["health_essentials", "personal_services"] {
        let id = catalog.resource_runtime_id_for_id(resource).unwrap();
        let measure = catalog.resource_measure(id).unwrap();
        assert_eq!(measure.unit, QuantityUnit::Service);
        assert_eq!(measure.shipment_batch_kg, 0.0);
    }
    for (id, sales) in [
        ("grain_farm_basic", 96.0),
        ("food_processor_basic", 2400.0),
        ("grocery_basic", 5000.0),
        ("machinery_factory_basic", 840.0),
    ] {
        let profile = catalog.profile_for_id(id).unwrap();
        assert!(
            (profile.outputs[0].units_per_day * profile.unit_price_currency - sales).abs() < 0.001
        );
    }
    for id in [
        "food_processor_basic",
        "grocery_basic",
        "machinery_factory_basic",
    ] {
        let profile = catalog.profile_for_id(id).unwrap();
        let input_kg: f32 = profile.inputs.iter().map(|p| p.units_per_day).sum();
        let output_kg: f32 = profile.outputs.iter().map(|p| p.units_per_day).sum();
        assert!(output_kg <= input_kg);
    }
}

#[test]
fn catalog_rejects_invalid_units_batches_and_created_mass() {
    let original: super::schema::ProfilesFile =
        toml::from_str(include_str!("../../../../../economy/profiles.toml")).unwrap();
    let compile = |data: &super::schema::ProfilesFile| {
        super::runtime_compile::compile_runtime_catalog(
            &data.profiles,
            &data.resources,
            &data.runtime_tuning,
        )
    };
    for invalid in [
        0.0,
        -1.0,
        f32::NAN,
        f32::INFINITY,
        original.runtime_tuning.logistics.truck_payload_kg + 1.0,
    ] {
        let mut data: super::schema::ProfilesFile =
            toml::from_str(include_str!("../../../../../economy/profiles.toml")).unwrap();
        data.resources[0].shipment_batch_kg = invalid;
        assert!(compile(&data).is_err());
    }
    let mut data = original;
    let grocery = data
        .profiles
        .iter_mut()
        .find(|p| p.id == "grocery_basic")
        .unwrap();
    grocery.outputs[0].units_per_day = 201.0;
    assert!(compile(&data).is_err(), "retail cannot create mass");
    data.profiles
        .iter_mut()
        .find(|p| p.id == "grocery_basic")
        .unwrap()
        .outputs[0]
        .units_per_day = 200.0;
    data.profiles
        .iter_mut()
        .find(|p| p.id == "power_plant_basic")
        .unwrap()
        .utility_quantity_unit = Some(super::QuantityUnit::CubicMetre);
    assert!(
        compile(&data).is_err(),
        "electrical energy cannot use a volume unit"
    );
}

#[test]
fn carrier_prices_must_be_finite_and_nonnegative() {
    let tuning = super::load_runtime_economy_tuning().unwrap();
    for invalid in [-1.0, f32::NAN, f32::INFINITY] {
        let mut tuning = (*tuning).clone();
        tuning.logistics.dispatch_fee_currency = invalid;
        assert!(super::validation::validate_runtime_tuning(&tuning).is_err());
        tuning.logistics.dispatch_fee_currency = 4.0;
        tuning.logistics.vehicle_hour_cost_currency = invalid;
        assert!(super::validation::validate_runtime_tuning(&tuning).is_err());
    }
}

#[test]
fn runtime_tuning_enforces_day_duration_without_narrowing() {
    use super::validation::validate_runtime_tuning;
    let mut tuning = (*super::load_runtime_economy_tuning().unwrap()).clone();
    for (duration, valid) in [
        (60.0, true),
        (1_440.0, true),
        (2_880.0, true),
        (60.0 - 1.0e-9, false),
        (0.0, false),
        (-1.0, false),
        (f64::NAN, false),
        (f64::INFINITY, false),
    ] {
        tuning.operational_clock.seconds_per_day = duration;
        assert_eq!(
            validate_runtime_tuning(&tuning).is_ok(),
            valid,
            "day duration {duration}"
        );
    }
}

#[test]
fn runtime_tuning_rejects_nonfinite_treasury_and_export_settings() {
    use super::validation::validate_runtime_tuning;
    let original = super::load_runtime_economy_tuning().unwrap();
    for value in [-1.0, f32::NAN, f32::INFINITY, 1.01] {
        let mut tuning = (*original).clone();
        tuning.owa_export_price_multiplier = value;
        assert!(
            validate_runtime_tuning(&tuning).is_err(),
            "export bid accepted {value}"
        );
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut tuning = (*original).clone();
        tuning.startup_treasury_balance = value;
        assert!(
            validate_runtime_tuning(&tuning).is_err(),
            "treasury accepted {value}"
        );
    }
}
