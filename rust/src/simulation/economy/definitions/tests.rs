// SPDX-License-Identifier: GPL-2.0-only

//! Tests for the editor-facing economy definition JSON bridge.

use super::api::{export_project_json, load_project_json, run_sandbox_json};
use super::io::{CONTROLLERS_FILE, PROFILES_FILE, SCENARIOS_FILE};
use std::path::{Path, PathBuf};

fn project_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

fn write_fixture_project(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join(PROFILES_FILE),
        r#"
[[resources]]
id = "household_supplies"
runtime_id = 1
[[resources]]
id = "packaged_food"
runtime_id = 2
[[resources]]
id = "grain"
runtime_id = 3

[[profiles]]
id = "grain_farm_basic"
display_name = "Grain Farm"
kind = "field_producer"
description = "Starter farm"
worker_capacity = 4
base_rate_units_per_day = 160.0
wage_min_currency_per_day = 90.0
wage_max_currency_per_day = 110.0
unit_price_currency = 2.0

[[profiles.outputs]]
resource = "grain"
units_per_day = 160.0

[[profiles]]
id = "food_processor_basic"
display_name = "Food Processing Plant"
kind = "processor"
description = "Starter processor"
worker_capacity = 4
base_rate_units_per_day = 160.0
wage_min_currency_per_day = 90.0
wage_max_currency_per_day = 110.0
unit_price_currency = 4.0

[[profiles.inputs]]
resource = "grain"
units_per_day = 160.0

[[profiles.outputs]]
resource = "packaged_food"
units_per_day = 160.0

[[profiles]]
id = "grocery_basic"
display_name = "Grocery"
kind = "store"
description = "Starter grocery"
worker_capacity = 3
base_rate_units_per_day = 200.0
wage_min_currency_per_day = 80.0
wage_max_currency_per_day = 100.0
unit_price_currency = 6.0
stock_target_days = 3.0
reorder_threshold_days = 2.0
critical_threshold_days = 0.5
min_shipment_units = 40.0

[[profiles.inputs]]
resource = "packaged_food"
units_per_day = 160.0

[[profiles.outputs]]
resource = "household_supplies"
units_per_day = 200.0

[[profiles]]
id = "basic_household_demand"
display_name = "Household Demand Sink"
kind = "demand_sink"
description = "Starter household supply reserve sink"
consumption_rate_per_resident = 1.0
stock_target_days = 5.0
reorder_threshold_days = 2.5
critical_threshold_days = 1.0
min_shipment_units = 1.0

[[profiles.inputs]]
resource = "household_supplies"
units_per_day = 1.0

[runtime_tuning]
owa_import_price_multiplier = 1.75
owa_export_price_multiplier = 0.45
owa_distress_liquidation_multiplier = 0.25
startup_treasury_balance = 100000.0
unemployment_daily_benefit_per_member = 30.0
unemployment_max_days = 30
pension_daily_benefit_per_elder = 30.0
child_support_daily_benefit_per_child = 10.0

[runtime_tuning.logistics]
truck_load_units = 40.0
border_active_jobs_per_node = 4
border_queued_jobs_per_node = 4
queued_shipment_expiry_hours = 12
terminal_failure_attempts = 3
owa_export_saturation_loads_to_floor = 4.0
owa_export_saturation_floor_factor = 0.75
owa_export_saturation_recovery_hours = 24.0

[runtime_tuning.construction]
residential_hours_by_level = [6, 12, 18]
commercial_hours_by_level = [8, 16, 24]
industrial_hours_by_level = [12, 24, 36]

[runtime_tuning.fiscal]
income_tax_rate = 0.12
household_vat_rate = 0.08
business_profit_tax_rate = 0.10
residential_property_tax_per_home_per_day = 2.0
commercial_property_tax_per_building_per_day = 25.0
industrial_property_tax_per_building_per_day = 35.0
property_tax_level_multiplier = 1.75

[runtime_tuning.operational_clock]
seconds_per_day = 1440.0
travel_estimate_refresh_minutes = 360
household_replenishment_check_interval_hours = 6
household_replenishment_retry_cooldown_hours = 1
household_replenishment_terminal_failure_count = 3
household_shopping_leg_timeout_hours = 8
shipment_retry_cooldown_hours = 1

[[runtime_tuning.operational_clock.work_profiles]]
id = "daytime_work"
arrival_windows = [{ start_minute = 420, end_minute = 540 }]
departure_windows = [{ start_minute = 960, end_minute = 1080 }]
reliability_buffer_minutes = 15

[[runtime_tuning.operational_clock.work_profiles]]
id = "three_shift_work"
arrival_windows = [
    { start_minute = 330, end_minute = 390 },
    { start_minute = 810, end_minute = 870 },
    { start_minute = 1290, end_minute = 1350 },
]
departure_windows = [
    { start_minute = 780, end_minute = 840 },
    { start_minute = 1260, end_minute = 1320 },
    { start_minute = 300, end_minute = 360 },
]
reliability_buffer_minutes = 10

[[runtime_tuning.operational_clock.freight_profiles]]
id = "always_open"
preferred_windows = [{ start_minute = 0, end_minute = 1440 }]
outside_window_eta_penalty_minutes = 0
outside_window_cost_multiplier = 1.0

[[runtime_tuning.operational_clock.freight_profiles]]
id = "daytime_receive"
preferred_windows = [{ start_minute = 420, end_minute = 1080 }]
outside_window_eta_penalty_minutes = 60
outside_window_cost_multiplier = 1.1

[runtime_tuning.operational_clock.work_profile_by_zone_type]
commercial = "daytime_work"
industrial = "three_shift_work"

[runtime_tuning.operational_clock.freight_profile_by_zone_type]
commercial = "daytime_receive"
industrial = "always_open"

[runtime_tuning.households]
immigrant_starting_stock_days = 3.0
immigrant_starting_budget_per_member = 15.0
household_starting_budget_floor = 10.0
utility_cost_per_member_per_day = 3.0
residential_move_in_min_reserve_days_by_level = [0.5, 6.0, 12.0]
residential_stay_min_reserve_days_by_level = [0.5, 3.0, 6.0]
stay_failure_days_before_eviction = 2

[runtime_tuning.viability]
residential_min_occupancy_ratio_for_upgrade = [0.0, 0.65, 0.85]
residential_max_occupancy_ratio_for_downgrade = [1.0, 0.20, 0.15]
nonresidential_min_buffer_days_by_level = [0.0, 4.0, 8.0]
nonresidential_max_buffer_days_for_downgrade = [1.0, 1.5, 2.0]
nonresidential_min_staffing_ratio_for_upgrade = 0.85
nonresidential_max_staffing_ratio_for_downgrade = 0.25
industrial_min_input_coverage_for_upgrade = 0.75
industrial_min_output_headroom_for_upgrade = 0.25
industrial_max_input_coverage_for_downgrade = 0.20
industrial_max_output_headroom_for_downgrade = 0.10
"#,
    )
    .unwrap();

    std::fs::write(
        dir.join(CONTROLLERS_FILE),
        r#"
[[controllers]]
id = "household_restock_cost_basic"
display_name = "Household Replenishment Cost"
kind = "household_restock_cost"
description = "Starter household replenishment cost controller"
default_weight = 0.5
min_multiplier = 0.9
max_multiplier = 1.1
"#,
    )
    .unwrap();

    std::fs::write(
        dir.join(SCENARIOS_FILE),
        r#"
[[scenarios]]
id = "grocery_bottleneck"
display_name = "Grocery Bottleneck"
description = "Starter bottleneck test"
duration_days = 30
household_count = 60
average_household_size = 2.0
starting_household_stock_days = 3.0

[[scenarios.nodes]]
id = "grain_farm"
ref_kind = "profile"
ref_id = "grain_farm_basic"
position = [90.0, 180.0]

[[scenarios.nodes]]
id = "food_processor"
ref_kind = "profile"
ref_id = "food_processor_basic"
position = [330.0, 180.0]

[[scenarios.nodes]]
id = "grocery"
ref_kind = "profile"
ref_id = "grocery_basic"
position = [460.0, 180.0]

[[scenarios.nodes]]
id = "households"
ref_kind = "profile"
ref_id = "basic_household_demand"
position = [820.0, 180.0]

[[scenarios.nodes]]
id = "replenishment_cost"
ref_kind = "controller"
ref_id = "household_restock_cost_basic"
position = [820.0, 40.0]

[[scenarios.edges]]
from = "grain_farm"
to = "food_processor"
resource = "grain"

[[scenarios.edges]]
from = "food_processor"
to = "grocery"
resource = "packaged_food"

[[scenarios.edges]]
from = "grocery"
to = "households"
resource = "household_supplies"

[[scenarios.controller_links]]
controller_node_id = "replenishment_cost"
target_node_id = "households"
"#,
    )
    .unwrap();
}

#[test]
fn load_project_returns_valid_json() {
    let dir = project_dir("metrum_economy_editor_load");
    let _ = std::fs::remove_dir_all(&dir);
    write_fixture_project(&dir);

    let json = load_project_json(&dir).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed["ok"].as_bool().unwrap());
    assert_eq!(parsed["project"]["profiles"].as_array().unwrap().len(), 4);
    assert!(parsed["validation"].as_array().unwrap().is_empty());
}

#[test]
fn export_project_preserves_authored_values() {
    let dir = project_dir("metrum_economy_editor_export");
    let _ = std::fs::remove_dir_all(&dir);
    write_fixture_project(&dir);
    let loaded = load_project_json(&dir).unwrap();
    let mut project =
        serde_json::from_str::<serde_json::Value>(&loaded).unwrap()["project"].clone();
    project["profiles"][0]["worker_capacity_area_m2"] = serde_json::json!(100_000.0);
    let project_json = serde_json::to_string(&project).unwrap();

    let out_dir = project_dir("metrum_economy_editor_export_out");
    let _ = std::fs::remove_dir_all(&out_dir);
    let result = export_project_json(&project_json, &out_dir).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(parsed["ok"].as_bool().unwrap());
    assert!(out_dir.join(PROFILES_FILE).exists());
    assert!(out_dir.join(CONTROLLERS_FILE).exists());
    assert!(out_dir.join(SCENARIOS_FILE).exists());
    let reloaded: serde_json::Value =
        serde_json::from_str(&load_project_json(&out_dir).unwrap()).unwrap();
    assert_eq!(
        reloaded["project"]["profiles"][0]["worker_capacity_area_m2"],
        100_000.0
    );
}

#[test]
fn sandbox_returns_daily_metrics() {
    let dir = project_dir("metrum_economy_editor_sandbox");
    let _ = std::fs::remove_dir_all(&dir);
    write_fixture_project(&dir);
    let loaded = load_project_json(&dir).unwrap();
    let loaded_value: serde_json::Value = serde_json::from_str(&loaded).unwrap();
    let project_json = serde_json::to_string(&loaded_value["project"]).unwrap();

    let result = run_sandbox_json(&project_json, "grocery_bottleneck").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(parsed["ok"].as_bool().unwrap());
    assert_eq!(parsed["result"]["daily"].as_array().unwrap().len(), 30);
    assert!(
        parsed["result"]["final_household_stock_days"]
            .as_f64()
            .unwrap()
            >= 0.0
    );
}

#[test]
fn sandbox_accepts_integer_like_float_fields_from_editor_json() {
    let dir = project_dir("metrum_economy_editor_sandbox_float_ints");
    let _ = std::fs::remove_dir_all(&dir);
    write_fixture_project(&dir);
    let loaded = load_project_json(&dir).unwrap();
    let mut loaded_value: serde_json::Value = serde_json::from_str(&loaded).unwrap();
    let project = loaded_value
        .get_mut("project")
        .and_then(serde_json::Value::as_object_mut)
        .unwrap();

    project["resources"][0]["runtime_id"] = serde_json::json!(1.0);
    project["profiles"][0]["worker_capacity"] = serde_json::json!(4.0);
    project["profiles"][1]["worker_capacity"] = serde_json::json!(3.0);
    project["scenarios"][0]["duration_days"] = serde_json::json!(30.0);
    project["scenarios"][0]["household_count"] = serde_json::json!(60.0);
    project["runtime_tuning"]["unemployment_max_days"] = serde_json::json!(30.0);
    for key in [
        "residential_hours_by_level",
        "commercial_hours_by_level",
        "industrial_hours_by_level",
    ] {
        project["runtime_tuning"]["construction"][key] = serde_json::json!([6.0, 12.0, 18.0]);
    }

    let project_json = serde_json::to_string(project).unwrap();
    let result = run_sandbox_json(&project_json, "grocery_bottleneck").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(parsed["ok"].as_bool().unwrap());

    project["runtime_tuning"]["construction"]["residential_hours_by_level"] =
        serde_json::json!([6.5]);
    assert!(
        run_sandbox_json(
            &serde_json::to_string(project).unwrap(),
            "grocery_bottleneck"
        )
        .unwrap_err()
        .contains("whole number")
    );
}

#[test]
fn sandbox_uses_only_the_first_authored_household_sink() {
    let dir = project_dir("metrum_economy_editor_multiple_sinks");
    write_fixture_project(&dir);
    let loaded: serde_json::Value =
        serde_json::from_str(&load_project_json(&dir).unwrap()).unwrap();
    let mut project = loaded["project"].clone();
    let before: serde_json::Value = serde_json::from_str(
        &run_sandbox_json(&project.to_string(), "grocery_bottleneck").unwrap(),
    )
    .unwrap();
    let scenario = &mut project["scenarios"][0];
    let mut second_sink = scenario["nodes"][3].clone();
    second_sink["id"] = serde_json::json!("extra_households");
    scenario["nodes"].as_array_mut().unwrap().push(second_sink);
    scenario["edges"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "from": "grocery", "to": "extra_households", "resource": "household_supplies"
        }));
    for _ in 0..2 {
        let after: serde_json::Value = serde_json::from_str(
            &run_sandbox_json(&project.to_string(), "grocery_bottleneck").unwrap(),
        )
        .unwrap();
        assert_eq!(after["ok"], true);
        assert!(
            after["validation"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["code"] == "multiple_demand_sinks")
        );
        for metric in [
            "final_household_stock_days",
            "total_unmet_units",
            "average_household_cost_per_day",
        ] {
            assert_eq!(
                after["result"][metric], before["result"][metric],
                "{metric}"
            );
        }
        project["scenarios"][0]["edges"]
            .as_array_mut()
            .unwrap()
            .reverse();
    }
}

#[test]
fn editor_rejects_invalid_scenario_and_controller_scalars() {
    use super::validation::validate_project;
    let dir = project_dir("metrum_economy_editor_scalar_validation");
    write_fixture_project(&dir);
    let original = super::io::load_project(&dir).unwrap();
    for case in 0..9 {
        let mut project = original.clone();
        match case {
            0 => project.scenarios[0].duration_days = 0,
            1 => project.scenarios[0].average_household_size = 0.0,
            2 => project.scenarios[0].average_household_size = f32::NAN,
            3 => project.scenarios[0].starting_household_stock_days = -1.0,
            4 => project.scenarios[0].starting_household_stock_days = f32::INFINITY,
            5 => project.controllers[0].default_weight = -0.1,
            6 => project.controllers[0].default_weight = f32::NAN,
            7 => project.controllers[0].min_multiplier = -1.0,
            8 => project.controllers[0].max_multiplier = 0.5,
            _ => unreachable!(),
        }
        assert!(
            validate_project(&project).iter().any(|m| m.is_error()),
            "invalid scalar case {case} was accepted"
        );
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
    for field in 0..2 {
        for value in [-1.0, 0.0, f32::NAN, f32::INFINITY] {
            let mut tuning = (*original).clone();
            if field == 0 {
                tuning.logistics.owa_export_saturation_loads_to_floor = value;
            } else {
                tuning.logistics.owa_export_saturation_recovery_hours = value;
            }
            assert!(
                validate_runtime_tuning(&tuning).is_err(),
                "field {field} accepted {value}"
            );
        }
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

#[test]
fn sandbox_field_payroll_uses_one_hectare_of_staffing_density() {
    let dir = project_dir("metrum_economy_editor_field_staffing");
    let _ = std::fs::remove_dir_all(&dir);
    write_fixture_project(&dir);
    let loaded: serde_json::Value =
        serde_json::from_str(&load_project_json(&dir).unwrap()).unwrap();
    let mut project = loaded["project"].clone();
    for (area_m2, insolvent) in [(10_000.0, true), (100_000.0, false)] {
        project["profiles"][0]["worker_capacity_area_m2"] = serde_json::json!(area_m2);
        let result = run_sandbox_json(
            &serde_json::to_string(&project).unwrap(),
            "grocery_bottleneck",
        )
        .unwrap();
        let result: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            result["result"]["bottlenecks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| {
                    message
                        .as_str()
                        .unwrap()
                        .starts_with("Node 'grain_farm' is insolvent:")
                }),
            insolvent
        );
    }
}

#[test]
fn shipped_project_exports_resources_and_runs_food_chain_with_machinery_imports() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../economy");
    let payload: serde_json::Value =
        serde_json::from_str(&load_project_json(&source).unwrap()).unwrap();
    let project = payload["project"].clone();
    let dest = project_dir("metrum_machinery_editor_roundtrip");
    let result: serde_json::Value =
        serde_json::from_str(&export_project_json(&project.to_string(), &dest).unwrap()).unwrap();
    assert_eq!(result["ok"], true, "{result}");
    let reloaded: serde_json::Value =
        serde_json::from_str(&load_project_json(&dest).unwrap()).unwrap();
    assert_eq!(reloaded["project"]["resources"], project["resources"]);
    assert_eq!(reloaded["project"]["scenarios"], project["scenarios"]);
    let sandbox: serde_json::Value = serde_json::from_str(
        &run_sandbox_json(&project.to_string(), "grocery_bottleneck").unwrap(),
    )
    .unwrap();
    assert_eq!(sandbox["ok"], true, "{sandbox}");
    assert!(sandbox["result"]["total_delivered_units"].as_f64().unwrap() > 0.0);
    let mut disconnected = project.clone();
    disconnected["scenarios"][0]["owa_import_resources"] = serde_json::json!([]);
    let result: serde_json::Value = serde_json::from_str(
        &run_sandbox_json(&disconnected.to_string(), "grocery_bottleneck").unwrap(),
    )
    .unwrap();
    assert_eq!(
        result["ok"], false,
        "unconnected Machinery cannot silently become free supply"
    );
    std::fs::remove_dir_all(dest).unwrap();
}
