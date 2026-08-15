//! Authored TOML and JSON schema for economy profiles, controllers, and scenarios.

use super::runtime::RuntimeEconomyTuning;
use super::serde_helpers::{default_duration_days, default_one, deserialize_u32_from_number};
use serde::{Deserialize, Serialize};

pub(super) const PROFILE_KIND_PRODUCER: &str = "producer";
pub(super) const PROFILE_KIND_FIELD_PRODUCER: &str = "field_producer";
pub(super) const PROFILE_KIND_PROCESSOR: &str = "processor";
pub(super) const PROFILE_KIND_STORE: &str = "store";
pub(super) const PROFILE_KIND_SERVICE_STORE: &str = "service_store";
pub(super) const PROFILE_KIND_DEMAND_SINK: &str = "demand_sink";
pub(super) const PROFILE_KIND_EXTRACTOR: &str = "extractor";
pub(super) const PROFILE_KIND_UTILITY_PRODUCER: &str = "utility_producer";
pub(super) const PROFILE_KIND_UTILITY_PROCESSOR: &str = "utility_processor";
pub(super) const NODE_REF_KIND_PROFILE: &str = "profile";
pub(super) const NODE_REF_KIND_CONTROLLER: &str = "controller";
pub(super) const CONTROLLER_KIND_HOUSEHOLD_RESTOCK_COST: &str = "household_restock_cost";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AuthoredProfileKind {
    Producer,
    FieldProducer,
    Processor,
    Store,
    ServiceStore,
    DemandSink,
    Extractor,
    UtilityProducer,
    UtilityProcessor,
    Unsupported,
}

impl AuthoredProfileKind {
    pub(super) fn from_str(kind: &str) -> Self {
        match kind {
            PROFILE_KIND_PRODUCER => Self::Producer,
            PROFILE_KIND_FIELD_PRODUCER => Self::FieldProducer,
            PROFILE_KIND_PROCESSOR => Self::Processor,
            PROFILE_KIND_STORE => Self::Store,
            PROFILE_KIND_SERVICE_STORE => Self::ServiceStore,
            PROFILE_KIND_DEMAND_SINK => Self::DemandSink,
            PROFILE_KIND_EXTRACTOR => Self::Extractor,
            PROFILE_KIND_UTILITY_PRODUCER => Self::UtilityProducer,
            PROFILE_KIND_UTILITY_PROCESSOR => Self::UtilityProcessor,
            _ => Self::Unsupported,
        }
    }
}

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

impl EconomyProfile {
    pub(super) fn authored_kind(&self) -> AuthoredProfileKind {
        AuthoredProfileKind::from_str(self.kind.as_str())
    }
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
