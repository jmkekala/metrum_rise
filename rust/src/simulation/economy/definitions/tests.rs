//! Tests for the editor-facing economy definition JSON bridge.

use super::api::{export_project_json, load_project_json, run_sandbox_json};
use super::io::{CONTROLLERS_FILE, INDEX_FILE, PROFILES_FILE, SCENARIOS_FILE};
use std::path::{Path, PathBuf};

fn project_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

fn write_fixture_project(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join(PROFILES_FILE),
        r#"
[[profiles]]
id = "food_processor_basic"
display_name = "Food Processor"
kind = "producer"
description = "Starter processor"
worker_capacity = 4
base_rate_units_per_day = 160.0
wage_min_currency_per_day = 90.0
wage_max_currency_per_day = 110.0
unit_price_currency = 4.0

[[profiles.outputs]]
resource = "staple_food"
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
resource = "staple_food"
units_per_day = 160.0

[[profiles.outputs]]
resource = "household_supplies"
units_per_day = 200.0

[[profiles]]
id = "basic_household_demand"
display_name = "Household Demand Sink"
kind = "demand_sink"
description = "Starter sink"
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
commercial_owa_utility_cost_per_day = 8.0
industrial_owa_utility_cost_per_day = 12.0
startup_treasury_balance = 100000.0
unemployment_daily_benefit_per_member = 30.0
unemployment_max_days = 30

[runtime_tuning.logistics]
truck_load_units = 40.0
border_active_jobs_per_node = 4
border_queued_jobs_per_node = 4
queued_shipment_expiry_hours = 12
terminal_failure_attempts = 3

[runtime_tuning.construction]
residential_hours_by_level = [6, 12, 18]
commercial_hours_by_level = [8, 16, 24]
industrial_hours_by_level = [12, 24, 36]

[runtime_tuning.fiscal]
income_tax_rate = 0.12
household_vat_rate = 0.08
business_purchase_tax_rate = 0.03
business_profit_tax_rate = 0.10
residential_property_tax_base = 250.0
commercial_property_tax_base = 500.0
industrial_property_tax_base = 750.0
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
display_name = "Household Restock Cost"
kind = "household_restock_cost"
description = "Starter household cost controller"
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
replenishment_target_days = 3.0
replenishment_trigger_days = 1.5
pickup_cadence_hours = 6.0

[[scenarios.nodes]]
id = "food_processor"
ref_kind = "profile"
ref_id = "food_processor_basic"
position = [120.0, 180.0]

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
from = "food_processor"
to = "grocery"
resource = "staple_food"

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
    assert_eq!(parsed["project"]["profiles"].as_array().unwrap().len(), 3);
    assert!(parsed["validation"].as_array().unwrap().is_empty());
}

#[test]
fn export_project_writes_cache_file() {
    let dir = project_dir("metrum_economy_editor_export");
    let _ = std::fs::remove_dir_all(&dir);
    write_fixture_project(&dir);
    let loaded = load_project_json(&dir).unwrap();
    let project_json = serde_json::to_string(
        &serde_json::from_str::<serde_json::Value>(&loaded).unwrap()["project"],
    )
    .unwrap();

    let out_dir = project_dir("metrum_economy_editor_export_out");
    let _ = std::fs::remove_dir_all(&out_dir);
    let result = export_project_json(&project_json, &out_dir).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(parsed["ok"].as_bool().unwrap());
    assert!(out_dir.join(PROFILES_FILE).exists());
    assert!(out_dir.join(CONTROLLERS_FILE).exists());
    assert!(out_dir.join(SCENARIOS_FILE).exists());
    assert!(out_dir.join(INDEX_FILE).exists());
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

    project["profiles"][0]["worker_capacity"] = serde_json::json!(4.0);
    project["profiles"][1]["worker_capacity"] = serde_json::json!(3.0);
    project["scenarios"][0]["duration_days"] = serde_json::json!(30.0);
    project["scenarios"][0]["household_count"] = serde_json::json!(60.0);
    project["runtime_tuning"]["unemployment_max_days"] = serde_json::json!(30.0);

    let project_json = serde_json::to_string(project).unwrap();
    let result = run_sandbox_json(&project_json, "grocery_bottleneck").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(parsed["ok"].as_bool().unwrap());
}
