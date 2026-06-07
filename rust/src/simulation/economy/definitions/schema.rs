//! Authored TOML and JSON schema for economy profiles, controllers, and scenarios.

use super::runtime::RuntimeEconomyTuning;
use super::serde_helpers::{default_duration_days, default_one, deserialize_u32_from_number};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct EconomyProject {
    #[serde(default)]
    pub(super) profiles: Vec<EconomyProfile>,
    pub(super) runtime_tuning: RuntimeEconomyTuning,
    #[serde(default)]
    pub(super) controllers: Vec<EconomyController>,
    #[serde(default)]
    pub(super) scenarios: Vec<EconomyScenario>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct EconomyProfile {
    pub(super) id: String,
    pub(super) display_name: String,
    #[serde(default)]
    pub(super) kind: String,
    #[serde(default)]
    pub(super) description: String,
    #[serde(default, deserialize_with = "deserialize_u32_from_number")]
    pub(super) worker_capacity: u32,
    #[serde(default)]
    pub(super) base_rate_units_per_day: f32,
    #[serde(default)]
    pub(super) wage_min_currency_per_day: f32,
    #[serde(default)]
    pub(super) wage_max_currency_per_day: f32,
    #[serde(default)]
    pub(super) unit_price_currency: f32,
    #[serde(default)]
    pub(super) stock_target_days: f32,
    #[serde(default)]
    pub(super) reorder_threshold_days: f32,
    #[serde(default)]
    pub(super) critical_threshold_days: f32,
    #[serde(default)]
    pub(super) min_shipment_units: f32,
    #[serde(default)]
    pub(super) consumption_rate_per_resident: f32,
    #[serde(default)]
    pub(super) starting_inventory_days: f32,
    #[serde(default)]
    pub(super) utility_service: Option<String>,
    #[serde(default)]
    pub(super) work_schedule_profile: Option<String>,
    #[serde(default)]
    pub(super) freight_timing_profile: Option<String>,
    #[serde(default)]
    pub(super) inputs: Vec<ResourcePort>,
    #[serde(default)]
    pub(super) outputs: Vec<ResourcePort>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct ResourcePort {
    pub(super) resource: String,
    pub(super) units_per_day: f32,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct EconomyController {
    pub(super) id: String,
    pub(super) display_name: String,
    #[serde(default)]
    pub(super) kind: String,
    #[serde(default)]
    pub(super) description: String,
    #[serde(default = "default_one")]
    pub(super) default_weight: f32,
    #[serde(default = "default_one")]
    pub(super) min_multiplier: f32,
    #[serde(default = "default_one")]
    pub(super) max_multiplier: f32,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct EconomyScenario {
    pub(super) id: String,
    pub(super) display_name: String,
    #[serde(default)]
    pub(super) description: String,
    #[serde(
        default = "default_duration_days",
        deserialize_with = "deserialize_u32_from_number"
    )]
    pub(super) duration_days: u32,
    #[serde(default, deserialize_with = "deserialize_u32_from_number")]
    pub(super) household_count: u32,
    #[serde(default = "default_one")]
    pub(super) average_household_size: f32,
    #[serde(default)]
    pub(super) starting_household_stock_days: f32,
    #[serde(default)]
    pub(super) replenishment_target_days: f32,
    #[serde(default)]
    pub(super) replenishment_trigger_days: f32,
    #[serde(default)]
    pub(super) pickup_cadence_hours: f32,
    #[serde(default)]
    pub(super) nodes: Vec<ScenarioNode>,
    #[serde(default)]
    pub(super) edges: Vec<ScenarioEdge>,
    #[serde(default)]
    pub(super) controller_links: Vec<ScenarioControllerLink>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct ScenarioNode {
    pub(super) id: String,
    pub(super) ref_kind: String,
    pub(super) ref_id: String,
    pub(super) position: [f32; 2],
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct ScenarioEdge {
    pub(super) from: String,
    pub(super) to: String,
    pub(super) resource: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct ScenarioControllerLink {
    pub(super) controller_node_id: String,
    pub(super) target_node_id: String,
}

#[derive(Serialize, Deserialize)]
pub(super) struct ProfilesFile {
    #[serde(default)]
    pub(super) profiles: Vec<EconomyProfile>,
    pub(super) runtime_tuning: RuntimeEconomyTuning,
}

#[derive(Serialize, Deserialize)]
pub(super) struct ControllersFile {
    #[serde(default)]
    pub(super) controllers: Vec<EconomyController>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct ScenariosFile {
    #[serde(default)]
    pub(super) scenarios: Vec<EconomyScenario>,
}
