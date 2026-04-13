//! Authored economy definitions used by the developer-facing economy editor.
//!
//! The runtime household/building simulation still owns live economic state, but
//! this module defines the authoritative TOML-backed profile/controller/scenario
//! data used to validate and tune the first-pass economy chains. The same data
//! will later feed the asset editor and a fuller compiled runtime representation.

use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

const PROFILES_FILE: &str = "profiles.toml";
const CONTROLLERS_FILE: &str = "controllers.toml";
const SCENARIOS_FILE: &str = "scenarios.toml";
const INDEX_FILE: &str = "economy.index.bin";

/// Loads an authored economy project from the canonical `economy/` folder and
/// returns a JSON envelope containing the parsed project plus validation output.
pub fn load_project_json(dir_path: &Path) -> Result<String, String> {
    let project = load_project(dir_path)?;
    let validation = validate_project(&project);
    let payload = serde_json::json!({
        "ok": true,
        "source_dir": dir_path,
        "project": project,
        "validation": validation,
    });
    serde_json::to_string(&payload)
        .map_err(|err| format!("could not encode economy project JSON: {err}"))
}

/// Validates and exports an authored economy project back to canonical TOML
/// files and regenerates the derived `economy.index.bin` cache.
pub fn export_project_json(project_json: &str, dir_path: &Path) -> Result<String, String> {
    let project: EconomyProject = serde_json::from_str(project_json)
        .map_err(|err| format!("economy project JSON parse error: {err}"))?;
    let validation = validate_project(&project);
    if validation.iter().any(|msg| msg.severity == "error") {
        let payload = serde_json::json!({
            "ok": false,
            "error": "validation failed; export aborted",
            "validation": validation,
        });
        return serde_json::to_string(&payload)
            .map_err(|err| format!("could not encode failed export JSON: {err}"));
    }

    std::fs::create_dir_all(dir_path).map_err(|err| {
        format!(
            "could not create economy dir '{}': {err}",
            dir_path.display()
        )
    })?;

    write_pretty_toml(
        &dir_path.join(PROFILES_FILE),
        &ProfilesFile {
            profiles: project.profiles.clone(),
            runtime_tuning: project.runtime_tuning.clone(),
        },
    )?;
    write_pretty_toml(
        &dir_path.join(CONTROLLERS_FILE),
        &ControllersFile {
            controllers: project.controllers.clone(),
        },
    )?;
    write_pretty_toml(
        &dir_path.join(SCENARIOS_FILE),
        &ScenariosFile {
            scenarios: project.scenarios.clone(),
        },
    )?;

    let compiled = build_index(&project);
    let compiled_bytes = serde_json::to_vec(&compiled)
        .map_err(|err| format!("could not encode economy cache: {err}"))?;
    std::fs::write(dir_path.join(INDEX_FILE), compiled_bytes)
        .map_err(|err| format!("could not write economy cache: {err}"))?;

    let payload = serde_json::json!({
        "ok": true,
        "validation": validation,
        "cache_path": dir_path.join(INDEX_FILE),
    });
    serde_json::to_string(&payload).map_err(|err| format!("could not encode export JSON: {err}"))
}

/// Runs the small authored-economy sandbox for a selected scenario and returns
/// a JSON envelope with summary metrics and daily series data.
pub fn run_sandbox_json(project_json: &str, scenario_id: &str) -> Result<String, String> {
    let project: EconomyProject = serde_json::from_str(project_json)
        .map_err(|err| format!("economy project JSON parse error: {err}"))?;
    let validation = validate_project(&project);
    if validation.iter().any(|msg| msg.severity == "error") {
        let payload = serde_json::json!({
            "ok": false,
            "error": "validation failed; sandbox aborted",
            "validation": validation,
        });
        return serde_json::to_string(&payload)
            .map_err(|err| format!("could not encode failed sandbox JSON: {err}"));
    }

    let result = run_sandbox(&project, scenario_id)?;
    let payload = serde_json::json!({
        "ok": true,
        "validation": validation,
        "result": result,
    });
    serde_json::to_string(&payload).map_err(|err| format!("could not encode sandbox JSON: {err}"))
}

#[derive(Clone, Serialize, Deserialize)]
struct EconomyProject {
    #[serde(default)]
    profiles: Vec<EconomyProfile>,
    #[serde(default)]
    runtime_tuning: RuntimeEconomyTuning,
    #[serde(default)]
    controllers: Vec<EconomyController>,
    #[serde(default)]
    scenarios: Vec<EconomyScenario>,
}

/// Authored economy-side runtime tuning used by the live simulation.
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub(crate) struct RuntimeEconomyTuning {
    /// Shared operational clock state and authored schedule profiles.
    pub operational_clock: OperationalClockRuntimeTuning,
    /// Household relocation and eviction thresholds.
    pub households: HouseholdRuntimeTuning,
    /// Building viability thresholds used by demand-owned level changes.
    pub viability: BuildingViabilityRuntimeTuning,
}

/// Shared operational-clock tuning used by labor, replenishment, and freight.
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub(crate) struct OperationalClockRuntimeTuning {
    /// Real seconds required to advance one authored operational day at `1.0x`.
    pub seconds_per_day: f64,
    /// Minutes between cached commute-estimate refreshes.
    pub travel_estimate_refresh_minutes: u16,
    /// Hours between household replenishment checks.
    pub household_replenishment_check_interval_hours: u16,
    /// Hours between reserve creation and household pickup completion.
    pub household_pickup_eta_hours: u16,
    /// Hours to wait before retrying a failed household replenishment.
    pub household_replenishment_retry_cooldown_hours: u16,
    /// Hours to wait before retrying a failed freight request.
    pub shipment_retry_cooldown_hours: u16,
    /// Named work timing profiles keyed by authored id.
    pub work_profiles: Vec<WorkTimingProfile>,
    /// Named freight timing preference profiles keyed by authored id.
    pub freight_profiles: Vec<FreightTimingProfile>,
    /// Broad zone-type to work profile mapping for the live baseline runtime.
    pub work_profile_by_zone_type: BTreeMap<String, String>,
    /// Broad zone-type to freight profile mapping for the live baseline runtime.
    pub freight_profile_by_zone_type: BTreeMap<String, String>,
}

/// One authored minute range from operational midnight.
#[derive(Clone, Copy, Serialize, Deserialize, Debug)]
pub(crate) struct MinuteWindow {
    /// Inclusive start minute from midnight.
    pub start_minute: u16,
    /// Exclusive end minute from midnight.
    pub end_minute: u16,
}

/// Authored arrival and departure windows for one repeated traveler profile.
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub(crate) struct WorkTimingProfile {
    /// Stable id used by the broad zone-type lookup table.
    pub id: String,
    /// Acceptable arrival windows for this schedule.
    pub arrival_windows: Vec<MinuteWindow>,
    /// Acceptable return-home departure windows for this schedule.
    pub departure_windows: Vec<MinuteWindow>,
    /// Fixed authored arrival buffer in minutes.
    pub reliability_buffer_minutes: u16,
}

/// Soft preferred freight receive or dispatch timing profile.
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub(crate) struct FreightTimingProfile {
    /// Stable id used by the broad zone-type lookup table.
    pub id: String,
    /// Preferred receive or dispatch windows for this profile.
    pub preferred_windows: Vec<MinuteWindow>,
    /// Extra ETA penalty applied outside the preferred window.
    pub outside_window_eta_penalty_minutes: u16,
    /// Cost multiplier applied outside the preferred window.
    pub outside_window_cost_multiplier: f32,
}

/// Household-side runtime tuning values derived from `economy/profiles.toml`.
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub(crate) struct HouseholdRuntimeTuning {
    /// Minimum reserve-days required to move into each residential level.
    pub residential_move_in_min_reserve_days_by_level: Vec<f32>,
    /// Minimum reserve-days required to remain in each residential level.
    pub residential_stay_min_reserve_days_by_level: Vec<f32>,
    /// Number of consecutive failed stay checks before eviction is allowed.
    pub stay_failure_days_before_eviction: u32,
}

/// Building-side viability thresholds derived from `economy/profiles.toml`.
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub(crate) struct BuildingViabilityRuntimeTuning {
    /// Minimum occupancy ratio required to upgrade into each residential target level.
    pub residential_min_occupancy_ratio_for_upgrade: Vec<f32>,
    /// Maximum occupancy ratio below which residential downgrade becomes viable for each level.
    pub residential_max_occupancy_ratio_for_downgrade: Vec<f32>,
    /// Minimum operating-buffer days required to upgrade into each non-residential target level.
    pub nonresidential_min_buffer_days_by_level: Vec<f32>,
    /// Maximum operating-buffer days below which downgrade becomes viable for each level.
    pub nonresidential_max_buffer_days_for_downgrade: Vec<f32>,
    /// Minimum staffing ratio required for non-residential upgrades.
    pub nonresidential_min_staffing_ratio_for_upgrade: f32,
    /// Maximum staffing ratio below which non-residential downgrade becomes viable.
    pub nonresidential_max_staffing_ratio_for_downgrade: f32,
    /// Minimum industrial input coverage required for industrial upgrades.
    pub industrial_min_input_coverage_for_upgrade: f32,
    /// Minimum industrial output headroom required for industrial upgrades.
    pub industrial_min_output_headroom_for_upgrade: f32,
    /// Maximum industrial input coverage below which industrial downgrade becomes viable.
    pub industrial_max_input_coverage_for_downgrade: f32,
    /// Maximum industrial output headroom below which industrial downgrade becomes viable.
    pub industrial_max_output_headroom_for_downgrade: f32,
}

/// Compact runtime resource id resolved from authored resource names.
pub(crate) type ResourceRuntimeId = u16;

/// One compiled runtime resource port.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RuntimeResourcePort {
    /// Runtime resource id carried through this port.
    pub resource_runtime_id: ResourceRuntimeId,
    /// Authored throughput in units per day.
    pub units_per_day: f32,
}

/// Broad runtime behavior class compiled from one authored economy profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EconomyProfileRuntimeKind {
    /// Upstream producing building that writes directly into its output stock buffer.
    Producer,
    /// Throughput building that converts one input buffer into one output buffer.
    Store,
    /// Non-building aggregate sink profile used by sandbox and household-side graphs.
    DemandSink,
    /// City utility building that generates a service signal (power, water).
    UtilityProducer,
    /// City utility building that processes a service output (sewage treatment).
    UtilityProcessor,
    /// Authored profile kind that the live starter runtime does not execute yet.
    Unsupported,
}

/// Compiled runtime view of one authored `economy_profile`.
#[derive(Clone, Debug)]
pub(crate) struct EconomyProfileRuntime {
    /// Stable compact runtime id used on live buildings.
    pub runtime_id: u16,
    /// Authored stable profile id.
    pub id: String,
    /// Broad runtime behavior kind used by the starter live economy.
    #[allow(dead_code)]
    pub kind: EconomyProfileRuntimeKind,
    /// Optional authored work schedule profile id.
    pub work_schedule_profile: Option<String>,
    /// Optional authored freight timing profile id.
    pub freight_timing_profile: Option<String>,
    /// Fixed baseline unit sale price for the profile's main output.
    pub unit_price_currency: f32,
    /// Fixed minimum daily wage offered by this profile.
    pub wage_min_currency_per_day: f32,
    /// Fixed maximum daily wage offered by this profile.
    pub wage_max_currency_per_day: f32,
    /// Authored target stock horizon in days for the starter live runtime.
    pub stock_target_days: f32,
    /// Output units pre-seeded into inventory when the building is first placed.
    /// Expressed as days of output throughput. Zero disables seeding.
    pub starting_inventory_days: f32,
    /// Authored reorder threshold in days for the starter live runtime.
    pub reorder_threshold_days: f32,
    /// Authored critical threshold in days for emergency freight.
    pub critical_threshold_days: f32,
    /// Authored lower shipment bound for ordinary freight requests.
    pub min_shipment_units: f32,
    /// Household-demand consumption rate used by abstract sink profiles.
    #[allow(dead_code)]
    pub consumption_rate_per_resident: f32,
    /// Which utility service this building provides ("power", "water", or "sewage").
    /// `None` for non-utility profiles.
    pub utility_service: Option<String>,
    /// Compiled typed input ports used by the live runtime.
    pub inputs: Vec<RuntimeResourcePort>,
    /// Compiled typed output ports used by the live runtime.
    pub outputs: Vec<RuntimeResourcePort>,
    /// False when the authored profile exists but the live runtime cannot execute it yet.
    pub runtime_supported: bool,
}

impl EconomyProfileRuntime {
    /// Returns the authored average daily wage for the profile.
    pub(crate) fn average_daily_wage(&self) -> f32 {
        ((self.wage_min_currency_per_day + self.wage_max_currency_per_day) * 0.5).max(0.0)
    }

    /// Returns one compiled input port by resource id.
    pub(crate) fn input_port(
        &self,
        resource_runtime_id: ResourceRuntimeId,
    ) -> Option<&RuntimeResourcePort> {
        self.inputs
            .iter()
            .find(|port| port.resource_runtime_id == resource_runtime_id)
    }

    /// Returns one compiled output port by resource id.
    pub(crate) fn output_port(
        &self,
        resource_runtime_id: ResourceRuntimeId,
    ) -> Option<&RuntimeResourcePort> {
        self.outputs
            .iter()
            .find(|port| port.resource_runtime_id == resource_runtime_id)
    }

    /// Returns the runtime target units for one tracked inventory port.
    pub(crate) fn inventory_target_units_for(&self, port: &RuntimeResourcePort) -> f32 {
        (port.units_per_day.max(0.0) * self.stock_target_days.max(0.0)).max(0.0)
    }

    /// Returns the runtime reorder threshold in units for one tracked inventory port.
    pub(crate) fn inventory_reorder_units_for(&self, port: &RuntimeResourcePort) -> f32 {
        (port.units_per_day.max(0.0) * self.reorder_threshold_days.max(0.0)).max(0.0)
    }

    /// Returns the runtime emergency threshold in units for one tracked inventory port.
    pub(crate) fn inventory_critical_units_for(&self, port: &RuntimeResourcePort) -> f32 {
        (port.units_per_day.max(0.0) * self.critical_threshold_days.max(0.0)).max(0.0)
    }

    /// Returns the runtime output buffer cap in units for one output resource.
    pub(crate) fn output_buffer_capacity_units_for(&self, port: &RuntimeResourcePort) -> f32 {
        let capacity = port.units_per_day.max(0.0) * self.stock_target_days.max(0.0);
        if capacity <= 0.0 { f32::MAX } else { capacity }
    }
}

/// Cached compiled runtime economy catalog derived from authored `economy_profile` data.
#[derive(Clone, Debug, Default)]
pub(crate) struct RuntimeEconomyCatalog {
    profiles: Vec<EconomyProfileRuntime>,
    by_id: BTreeMap<String, u16>,
    resource_by_id: BTreeMap<String, ResourceRuntimeId>,
    price_by_resource: BTreeMap<ResourceRuntimeId, f32>,
}

impl RuntimeEconomyCatalog {
    /// Returns one compiled runtime profile by authored id.
    pub(crate) fn profile_for_id(&self, profile_id: &str) -> Option<&EconomyProfileRuntime> {
        let runtime_id = *self.by_id.get(profile_id)?;
        self.profile_by_runtime_id(runtime_id)
    }

    /// Returns one compiled runtime profile by compact runtime id.
    pub(crate) fn profile_by_runtime_id(&self, runtime_id: u16) -> Option<&EconomyProfileRuntime> {
        runtime_id
            .checked_sub(1)
            .and_then(|idx| self.profiles.get(idx as usize))
    }

    /// Returns one compiled runtime resource by authored id.
    pub(crate) fn resource_runtime_id_for_id(
        &self,
        resource_id: &str,
    ) -> Option<ResourceRuntimeId> {
        self.resource_by_id.get(resource_id).copied()
    }

    /// Returns the number of compiled runtime resources in the catalog.
    pub(crate) fn resource_count(&self) -> usize {
        self.resource_by_id.len()
    }

    /// Returns the authored resource id string for a given compact runtime id, or `None`.
    ///
    /// Linear scan over the resource map — only call on user-triggered events, not hot paths.
    pub(crate) fn resource_id_for_runtime_id(&self, runtime_id: u16) -> Option<&str> {
        self.resource_by_id
            .iter()
            .find(|&(_, id)| *id == runtime_id)
            .map(|(name, _)| name.as_str())
    }

    /// Returns the default runtime unit price for one resource when `OWA` imports it.
    pub(crate) fn unit_price_for_resource(&self, resource: ResourceRuntimeId) -> Option<f32> {
        self.price_by_resource.get(&resource).copied()
    }
}

impl Default for RuntimeEconomyTuning {
    fn default() -> Self {
        Self {
            operational_clock: OperationalClockRuntimeTuning::default(),
            households: HouseholdRuntimeTuning::default(),
            viability: BuildingViabilityRuntimeTuning::default(),
        }
    }
}

impl Default for OperationalClockRuntimeTuning {
    fn default() -> Self {
        let daytime = WorkTimingProfile {
            id: "daytime_work".to_owned(),
            arrival_windows: vec![MinuteWindow {
                start_minute: 7 * 60,
                end_minute: 9 * 60,
            }],
            departure_windows: vec![MinuteWindow {
                start_minute: 16 * 60,
                end_minute: 18 * 60,
            }],
            reliability_buffer_minutes: 15,
        };
        let three_shift = WorkTimingProfile {
            id: "three_shift_work".to_owned(),
            arrival_windows: vec![
                MinuteWindow {
                    start_minute: 5 * 60 + 30,
                    end_minute: 6 * 60 + 30,
                },
                MinuteWindow {
                    start_minute: 13 * 60 + 30,
                    end_minute: 14 * 60 + 30,
                },
                MinuteWindow {
                    start_minute: 21 * 60 + 30,
                    end_minute: 22 * 60 + 30,
                },
            ],
            departure_windows: vec![
                MinuteWindow {
                    start_minute: 13 * 60,
                    end_minute: 14 * 60,
                },
                MinuteWindow {
                    start_minute: 21 * 60,
                    end_minute: 22 * 60,
                },
                MinuteWindow {
                    start_minute: 5 * 60,
                    end_minute: 6 * 60,
                },
            ],
            reliability_buffer_minutes: 10,
        };
        Self {
            seconds_per_day: 24.0 * 60.0,
            travel_estimate_refresh_minutes: 360,
            household_replenishment_check_interval_hours: 6,
            household_pickup_eta_hours: 1,
            household_replenishment_retry_cooldown_hours: 1,
            shipment_retry_cooldown_hours: 1,
            work_profiles: vec![daytime, three_shift],
            freight_profiles: vec![
                FreightTimingProfile {
                    id: "always_open".to_owned(),
                    preferred_windows: vec![MinuteWindow {
                        start_minute: 0,
                        end_minute: 24 * 60,
                    }],
                    outside_window_eta_penalty_minutes: 0,
                    outside_window_cost_multiplier: 1.0,
                },
                FreightTimingProfile {
                    id: "daytime_receive".to_owned(),
                    preferred_windows: vec![MinuteWindow {
                        start_minute: 7 * 60,
                        end_minute: 18 * 60,
                    }],
                    outside_window_eta_penalty_minutes: 60,
                    outside_window_cost_multiplier: 1.1,
                },
                FreightTimingProfile {
                    id: "early_morning_preferred".to_owned(),
                    preferred_windows: vec![MinuteWindow {
                        start_minute: 4 * 60,
                        end_minute: 8 * 60,
                    }],
                    outside_window_eta_penalty_minutes: 60,
                    outside_window_cost_multiplier: 1.05,
                },
            ],
            work_profile_by_zone_type: BTreeMap::from([
                ("commercial".to_owned(), "daytime_work".to_owned()),
                ("industrial".to_owned(), "three_shift_work".to_owned()),
            ]),
            freight_profile_by_zone_type: BTreeMap::from([
                ("commercial".to_owned(), "daytime_receive".to_owned()),
                ("industrial".to_owned(), "always_open".to_owned()),
            ]),
        }
    }
}

impl Default for WorkTimingProfile {
    fn default() -> Self {
        Self {
            id: String::new(),
            arrival_windows: Vec::new(),
            departure_windows: Vec::new(),
            reliability_buffer_minutes: 0,
        }
    }
}

impl Default for FreightTimingProfile {
    fn default() -> Self {
        Self {
            id: String::new(),
            preferred_windows: Vec::new(),
            outside_window_eta_penalty_minutes: 0,
            outside_window_cost_multiplier: 1.0,
        }
    }
}

impl OperationalClockRuntimeTuning {
    /// Returns the authored work profile configured for one broad zone type.
    pub fn work_profile_for_zone_type(&self, zone_type: &str) -> Option<&WorkTimingProfile> {
        let profile_id = self.work_profile_by_zone_type.get(zone_type)?;
        self.work_profiles
            .iter()
            .find(|profile| profile.id == *profile_id)
    }

    /// Returns the authored freight timing profile configured for one broad zone type.
    pub fn freight_profile_for_zone_type(&self, zone_type: &str) -> Option<&FreightTimingProfile> {
        let profile_id = self.freight_profile_by_zone_type.get(zone_type)?;
        self.freight_profiles
            .iter()
            .find(|profile| profile.id == *profile_id)
    }
}

impl Default for HouseholdRuntimeTuning {
    fn default() -> Self {
        Self {
            residential_move_in_min_reserve_days_by_level: vec![0.0, 6.0, 12.0],
            residential_stay_min_reserve_days_by_level: vec![0.0, 4.0, 8.0],
            stay_failure_days_before_eviction: 2,
        }
    }
}

impl Default for BuildingViabilityRuntimeTuning {
    fn default() -> Self {
        Self {
            residential_min_occupancy_ratio_for_upgrade: vec![0.0, 0.65, 0.85],
            residential_max_occupancy_ratio_for_downgrade: vec![1.0, 0.20, 0.15],
            nonresidential_min_buffer_days_by_level: vec![0.0, 4.0, 8.0],
            nonresidential_max_buffer_days_for_downgrade: vec![1.0, 1.5, 2.0],
            nonresidential_min_staffing_ratio_for_upgrade: 0.85,
            nonresidential_max_staffing_ratio_for_downgrade: 0.25,
            industrial_min_input_coverage_for_upgrade: 0.75,
            industrial_min_output_headroom_for_upgrade: 0.25,
            industrial_max_input_coverage_for_downgrade: 0.20,
            industrial_max_output_headroom_for_downgrade: 0.10,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct EconomyProfile {
    id: String,
    display_name: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    description: String,
    #[serde(default, deserialize_with = "deserialize_u32_from_number")]
    worker_capacity: u32,
    #[serde(default)]
    base_rate_units_per_day: f32,
    #[serde(default)]
    wage_min_currency_per_day: f32,
    #[serde(default)]
    wage_max_currency_per_day: f32,
    #[serde(default)]
    unit_price_currency: f32,
    #[serde(default)]
    stock_target_days: f32,
    #[serde(default)]
    reorder_threshold_days: f32,
    #[serde(default)]
    critical_threshold_days: f32,
    #[serde(default)]
    min_shipment_units: f32,
    #[serde(default)]
    consumption_rate_per_resident: f32,
    #[serde(default)]
    starting_inventory_days: f32,
    #[serde(default)]
    utility_service: Option<String>,
    #[serde(default)]
    work_schedule_profile: Option<String>,
    #[serde(default)]
    freight_timing_profile: Option<String>,
    #[serde(default)]
    inputs: Vec<ResourcePort>,
    #[serde(default)]
    outputs: Vec<ResourcePort>,
}

#[derive(Clone, Serialize, Deserialize)]
struct ResourcePort {
    resource: String,
    units_per_day: f32,
}

#[derive(Clone, Serialize, Deserialize)]
struct EconomyController {
    id: String,
    display_name: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    description: String,
    #[serde(default = "default_one")]
    default_weight: f32,
    #[serde(default = "default_one")]
    min_multiplier: f32,
    #[serde(default = "default_one")]
    max_multiplier: f32,
}

#[derive(Clone, Serialize, Deserialize)]
struct EconomyScenario {
    id: String,
    display_name: String,
    #[serde(default)]
    description: String,
    #[serde(
        default = "default_duration_days",
        deserialize_with = "deserialize_u32_from_number"
    )]
    duration_days: u32,
    #[serde(default, deserialize_with = "deserialize_u32_from_number")]
    household_count: u32,
    #[serde(default = "default_one")]
    average_household_size: f32,
    #[serde(default)]
    starting_household_stock_days: f32,
    #[serde(default)]
    replenishment_target_days: f32,
    #[serde(default)]
    replenishment_trigger_days: f32,
    #[serde(default)]
    pickup_cadence_hours: f32,
    #[serde(default)]
    nodes: Vec<ScenarioNode>,
    #[serde(default)]
    edges: Vec<ScenarioEdge>,
    #[serde(default)]
    controller_links: Vec<ScenarioControllerLink>,
}

#[derive(Clone, Serialize, Deserialize)]
struct ScenarioNode {
    id: String,
    ref_kind: String,
    ref_id: String,
    position: [f32; 2],
}

#[derive(Clone, Serialize, Deserialize)]
struct ScenarioEdge {
    from: String,
    to: String,
    resource: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct ScenarioControllerLink {
    controller_node_id: String,
    target_node_id: String,
}

#[derive(Serialize, Deserialize)]
struct ProfilesFile {
    #[serde(default)]
    profiles: Vec<EconomyProfile>,
    #[serde(default)]
    runtime_tuning: RuntimeEconomyTuning,
}

#[derive(Serialize, Deserialize)]
struct ControllersFile {
    #[serde(default)]
    controllers: Vec<EconomyController>,
}

#[derive(Serialize, Deserialize)]
struct ScenariosFile {
    #[serde(default)]
    scenarios: Vec<EconomyScenario>,
}

#[derive(Serialize)]
struct ValidationMessage {
    severity: &'static str,
    code: &'static str,
    scope: String,
    message: String,
}

#[derive(Serialize)]
struct CompiledEconomyIndex {
    profile_ids: Vec<String>,
    controller_ids: Vec<String>,
    scenario_ids: Vec<String>,
    compatibility: Vec<CompiledCompatibility>,
}

#[derive(Serialize)]
struct CompiledCompatibility {
    resource: String,
    source_profile_id: String,
    target_profile_ids: Vec<String>,
}

#[derive(Serialize)]
struct SandboxResult {
    scenario_id: String,
    display_name: String,
    duration_days: u32,
    daily_household_demand_units: f32,
    final_household_stock_days: f32,
    lowest_household_stock_days: f32,
    total_delivered_units: f32,
    total_unmet_units: f32,
    average_household_cost_per_day: f32,
    bottlenecks: Vec<String>,
    daily: Vec<DailySandboxMetric>,
}

#[derive(Serialize)]
struct DailySandboxMetric {
    day: u32,
    household_stock_days: f32,
    delivered_units: f32,
    unmet_units: f32,
    average_household_cost: f32,
}

fn default_duration_days() -> u32 {
    30
}

fn default_one() -> f32 {
    1.0
}

fn deserialize_u32_from_number<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(number) => {
            if let Some(unsigned) = number.as_u64() {
                return u32::try_from(unsigned)
                    .map_err(|_| serde::de::Error::custom("numeric value exceeds u32 range"));
            }
            if let Some(signed) = number.as_i64() {
                return u32::try_from(signed).map_err(|_| {
                    serde::de::Error::custom("numeric value must be >= 0 and within u32 range")
                });
            }
            if let Some(float) = number.as_f64() {
                if !float.is_finite() || float < 0.0 {
                    return Err(serde::de::Error::custom(
                        "numeric value must be finite and >= 0",
                    ));
                }
                let rounded = float.round();
                if (float - rounded).abs() > f64::EPSILON {
                    return Err(serde::de::Error::custom(
                        "numeric value must be a whole number",
                    ));
                }
                return u32::try_from(rounded as i64)
                    .map_err(|_| serde::de::Error::custom("numeric value exceeds u32 range"));
            }
            Err(serde::de::Error::custom(
                "unsupported numeric representation",
            ))
        }
        serde_json::Value::Null => Ok(0),
        other => Err(serde::de::Error::custom(format!(
            "expected numeric value for u32 field, got {other}"
        ))),
    }
}

fn load_project(dir_path: &Path) -> Result<EconomyProject, String> {
    let profiles: ProfilesFile = parse_toml_file(&dir_path.join(PROFILES_FILE))?;
    let controllers: ControllersFile = parse_toml_file(&dir_path.join(CONTROLLERS_FILE))?;
    let scenarios: ScenariosFile = parse_toml_file(&dir_path.join(SCENARIOS_FILE))?;
    Ok(EconomyProject {
        profiles: profiles.profiles,
        runtime_tuning: profiles.runtime_tuning,
        controllers: controllers.controllers,
        scenarios: scenarios.scenarios,
    })
}

static BUILTIN_RUNTIME_TUNING: OnceLock<Result<RuntimeEconomyTuning, String>> = OnceLock::new();
static BUILTIN_RUNTIME_CATALOG: OnceLock<Result<RuntimeEconomyCatalog, String>> = OnceLock::new();

/// Loads the shipped economy-side runtime tuning from `economy/profiles.toml`.
pub(crate) fn load_runtime_economy_tuning() -> Result<Arc<RuntimeEconomyTuning>, String> {
    match BUILTIN_RUNTIME_TUNING.get_or_init(load_runtime_economy_tuning_from_disk) {
        Ok(config) => Ok(Arc::new(config.clone())),
        Err(err) => Err(err.clone()),
    }
}

fn load_runtime_economy_tuning_from_disk() -> Result<RuntimeEconomyTuning, String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("economy")
        .join(PROFILES_FILE);
    let profiles: ProfilesFile = parse_toml_file(&path)?;
    validate_runtime_tuning(&profiles.runtime_tuning)?;
    Ok(profiles.runtime_tuning)
}

/// Loads the shipped compiled runtime economy catalog from `economy/profiles.toml`.
pub(crate) fn load_runtime_economy_catalog() -> Result<Arc<RuntimeEconomyCatalog>, String> {
    match BUILTIN_RUNTIME_CATALOG.get_or_init(load_runtime_economy_catalog_from_disk) {
        Ok(catalog) => Ok(Arc::new(catalog.clone())),
        Err(err) => Err(err.clone()),
    }
}

fn load_runtime_economy_catalog_from_disk() -> Result<RuntimeEconomyCatalog, String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("economy")
        .join(PROFILES_FILE);
    let profiles: ProfilesFile = parse_toml_file(&path)?;
    validate_runtime_tuning(&profiles.runtime_tuning)?;
    compile_runtime_catalog(&profiles.profiles, &profiles.runtime_tuning)
}

fn compile_runtime_catalog(
    authored_profiles: &[EconomyProfile],
    runtime_tuning: &RuntimeEconomyTuning,
) -> Result<RuntimeEconomyCatalog, String> {
    let work_profile_ids: BTreeSet<&str> = runtime_tuning
        .operational_clock
        .work_profiles
        .iter()
        .map(|profile| profile.id.as_str())
        .collect();
    let freight_profile_ids: BTreeSet<&str> = runtime_tuning
        .operational_clock
        .freight_profiles
        .iter()
        .map(|profile| profile.id.as_str())
        .collect();

    let mut catalog = RuntimeEconomyCatalog::default();
    let mut resource_ids = BTreeSet::new();
    for profile in authored_profiles {
        for input in &profile.inputs {
            resource_ids.insert(input.resource.clone());
        }
        for output in &profile.outputs {
            resource_ids.insert(output.resource.clone());
        }
    }
    for (idx, resource_id) in resource_ids.into_iter().enumerate() {
        let runtime_id = u16::try_from(idx + 1)
            .map_err(|_| "runtime economy catalog exceeds u16 resource id range".to_owned())?;
        catalog
            .resource_by_id
            .insert(resource_id.clone(), runtime_id);
    }

    for (idx, profile) in authored_profiles.iter().enumerate() {
        if catalog.by_id.contains_key(&profile.id) {
            return Err(format!(
                "runtime economy catalog contains duplicate profile id '{}'",
                profile.id
            ));
        }
        if let Some(work_profile) = profile.work_schedule_profile.as_deref()
            && !work_profile_ids.contains(work_profile)
        {
            return Err(format!(
                "profile '{}' references missing work_schedule_profile '{}'",
                profile.id, work_profile
            ));
        }
        if let Some(freight_profile) = profile.freight_timing_profile.as_deref()
            && !freight_profile_ids.contains(freight_profile)
        {
            return Err(format!(
                "profile '{}' references missing freight_timing_profile '{}'",
                profile.id, freight_profile
            ));
        }

        let runtime_id = u16::try_from(idx + 1)
            .map_err(|_| "runtime economy catalog exceeds u16 profile id range".to_owned())?;
        let compiled = compile_runtime_profile(runtime_id, profile, &catalog.resource_by_id)?;
        if compiled.unit_price_currency > 0.0 {
            for output in &compiled.outputs {
                catalog
                    .price_by_resource
                    .entry(output.resource_runtime_id)
                    .or_insert(compiled.unit_price_currency);
            }
        }
        catalog.by_id.insert(compiled.id.clone(), runtime_id);
        catalog.profiles.push(compiled);
    }

    Ok(catalog)
}

fn compile_runtime_profile(
    runtime_id: u16,
    profile: &EconomyProfile,
    resource_by_id: &BTreeMap<String, ResourceRuntimeId>,
) -> Result<EconomyProfileRuntime, String> {
    let kind = match profile.kind.as_str() {
        "producer" => EconomyProfileRuntimeKind::Producer,
        "store" => EconomyProfileRuntimeKind::Store,
        "demand_sink" => EconomyProfileRuntimeKind::DemandSink,
        "utility_producer" => EconomyProfileRuntimeKind::UtilityProducer,
        "utility_processor" => EconomyProfileRuntimeKind::UtilityProcessor,
        _ => EconomyProfileRuntimeKind::Unsupported,
    };

    let compiled_inputs = profile
        .inputs
        .iter()
        .map(|input| {
            let Some(&resource_runtime_id) = resource_by_id.get(input.resource.as_str()) else {
                return Err(format!(
                    "profile '{}' references unresolved input resource '{}'",
                    profile.id, input.resource
                ));
            };
            Ok(RuntimeResourcePort {
                resource_runtime_id,
                units_per_day: input.units_per_day.max(0.0),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let compiled_outputs = profile
        .outputs
        .iter()
        .map(|output| {
            let Some(&resource_runtime_id) = resource_by_id.get(output.resource.as_str()) else {
                return Err(format!(
                    "profile '{}' references unresolved output resource '{}'",
                    profile.id, output.resource
                ));
            };
            Ok(RuntimeResourcePort {
                resource_runtime_id,
                units_per_day: output.units_per_day.max(0.0),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let runtime_supported = match kind {
        EconomyProfileRuntimeKind::Producer => !compiled_outputs.is_empty(),
        EconomyProfileRuntimeKind::Store => {
            !compiled_inputs.is_empty() && !compiled_outputs.is_empty()
        }
        EconomyProfileRuntimeKind::UtilityProducer | EconomyProfileRuntimeKind::UtilityProcessor => {
            true
        }
        EconomyProfileRuntimeKind::DemandSink | EconomyProfileRuntimeKind::Unsupported => false,
    };

    Ok(EconomyProfileRuntime {
        runtime_id,
        id: profile.id.clone(),
        kind,
        work_schedule_profile: profile.work_schedule_profile.clone(),
        freight_timing_profile: profile.freight_timing_profile.clone(),
        unit_price_currency: profile.unit_price_currency.max(0.0),
        wage_min_currency_per_day: profile.wage_min_currency_per_day.max(0.0),
        wage_max_currency_per_day: profile.wage_max_currency_per_day.max(0.0),
        stock_target_days: profile.stock_target_days.max(0.0),
        starting_inventory_days: profile.starting_inventory_days.max(0.0),
        reorder_threshold_days: profile.reorder_threshold_days.max(0.0),
        critical_threshold_days: profile.critical_threshold_days.max(0.0),
        min_shipment_units: profile.min_shipment_units.max(0.0),
        consumption_rate_per_resident: profile.consumption_rate_per_resident.max(0.0),
        utility_service: profile.utility_service.clone(),
        inputs: compiled_inputs,
        outputs: compiled_outputs,
        runtime_supported,
    })
}

fn parse_toml_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|err| format!("could not read '{}': {err}", path.display()))?;
    toml::from_str(&content).map_err(|err| format!("could not parse '{}': {err}", path.display()))
}

fn write_pretty_toml<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let encoded = toml::to_string_pretty(value)
        .map_err(|err| format!("could not encode '{}': {err}", path.display()))?;
    std::fs::write(path, encoded)
        .map_err(|err| format!("could not write '{}': {err}", path.display()))
}

fn validate_project(project: &EconomyProject) -> Vec<ValidationMessage> {
    let mut messages = Vec::new();
    let profile_ids = duplicate_ids(project.profiles.iter().map(|profile| profile.id.as_str()));
    for duplicate in profile_ids {
        messages.push(error(
            "duplicate_profile_id",
            "project.profiles",
            format!("profile id '{duplicate}' is defined more than once"),
        ));
    }

    let controller_ids = duplicate_ids(
        project
            .controllers
            .iter()
            .map(|controller| controller.id.as_str()),
    );
    for duplicate in controller_ids {
        messages.push(error(
            "duplicate_controller_id",
            "project.controllers",
            format!("controller id '{duplicate}' is defined more than once"),
        ));
    }

    let scenario_ids = duplicate_ids(
        project
            .scenarios
            .iter()
            .map(|scenario| scenario.id.as_str()),
    );
    for duplicate in scenario_ids {
        messages.push(error(
            "duplicate_scenario_id",
            "project.scenarios",
            format!("scenario id '{duplicate}' is defined more than once"),
        ));
    }

    let profile_map: BTreeMap<&str, &EconomyProfile> = project
        .profiles
        .iter()
        .map(|profile| (profile.id.as_str(), profile))
        .collect();
    let controller_map: BTreeMap<&str, &EconomyController> = project
        .controllers
        .iter()
        .map(|controller| (controller.id.as_str(), controller))
        .collect();

    for profile in &project.profiles {
        if profile.display_name.trim().is_empty() {
            messages.push(error(
                "missing_profile_display_name",
                format!("profile.{}", profile.id),
                "profile display_name must not be empty".to_owned(),
            ));
        }
        if profile.kind.trim().is_empty() {
            messages.push(error(
                "missing_profile_kind",
                format!("profile.{}", profile.id),
                "profile kind must not be empty".to_owned(),
            ));
        }
    }
    validate_runtime_tuning_messages(&project.runtime_tuning, &mut messages);

    for controller in &project.controllers {
        if controller.display_name.trim().is_empty() {
            messages.push(error(
                "missing_controller_display_name",
                format!("controller.{}", controller.id),
                "controller display_name must not be empty".to_owned(),
            ));
        }
        if controller.kind.trim().is_empty() {
            messages.push(error(
                "missing_controller_kind",
                format!("controller.{}", controller.id),
                "controller kind must not be empty".to_owned(),
            ));
        }
    }

    for scenario in &project.scenarios {
        validate_scenario(scenario, &profile_map, &controller_map, &mut messages);
    }

    messages
}

fn validate_runtime_tuning_messages(
    tuning: &RuntimeEconomyTuning,
    messages: &mut Vec<ValidationMessage>,
) {
    if let Err(err) = validate_runtime_tuning(tuning) {
        messages.push(error(
            "invalid_runtime_tuning",
            "project.runtime_tuning",
            err,
        ));
    }
}

fn validate_runtime_tuning(tuning: &RuntimeEconomyTuning) -> Result<(), String> {
    validate_range(
        tuning.operational_clock.seconds_per_day as f32,
        60.0,
        f32::INFINITY,
        "runtime_tuning.operational_clock.seconds_per_day",
    )?;
    if tuning.operational_clock.travel_estimate_refresh_minutes == 0 {
        return Err(
            "runtime_tuning.operational_clock.travel_estimate_refresh_minutes must be > 0"
                .to_owned(),
        );
    }
    if tuning
        .operational_clock
        .household_replenishment_check_interval_hours
        == 0
    {
        return Err(
            "runtime_tuning.operational_clock.household_replenishment_check_interval_hours must be > 0"
                .to_owned(),
        );
    }
    if tuning.operational_clock.household_pickup_eta_hours == 0 {
        return Err(
            "runtime_tuning.operational_clock.household_pickup_eta_hours must be > 0".to_owned(),
        );
    }
    if tuning
        .operational_clock
        .household_replenishment_retry_cooldown_hours
        == 0
    {
        return Err(
            "runtime_tuning.operational_clock.household_replenishment_retry_cooldown_hours must be > 0"
                .to_owned(),
        );
    }
    if tuning.operational_clock.shipment_retry_cooldown_hours == 0 {
        return Err(
            "runtime_tuning.operational_clock.shipment_retry_cooldown_hours must be > 0".to_owned(),
        );
    }
    validate_work_profiles(&tuning.operational_clock)?;
    validate_freight_profiles(&tuning.operational_clock)?;
    validate_nonempty_level_array(
        &tuning
            .households
            .residential_move_in_min_reserve_days_by_level,
        "runtime_tuning.households.residential_move_in_min_reserve_days_by_level",
        0.0,
        f32::INFINITY,
    )?;
    validate_nonempty_level_array(
        &tuning.households.residential_stay_min_reserve_days_by_level,
        "runtime_tuning.households.residential_stay_min_reserve_days_by_level",
        0.0,
        f32::INFINITY,
    )?;
    if tuning.households.stay_failure_days_before_eviction == 0 {
        return Err(
            "runtime_tuning.households.stay_failure_days_before_eviction must be > 0".to_owned(),
        );
    }
    validate_nonempty_level_array(
        &tuning.viability.residential_min_occupancy_ratio_for_upgrade,
        "runtime_tuning.viability.residential_min_occupancy_ratio_for_upgrade",
        0.0,
        1.0,
    )?;
    validate_nonempty_level_array(
        &tuning
            .viability
            .residential_max_occupancy_ratio_for_downgrade,
        "runtime_tuning.viability.residential_max_occupancy_ratio_for_downgrade",
        0.0,
        1.0,
    )?;
    validate_nonempty_level_array(
        &tuning.viability.nonresidential_min_buffer_days_by_level,
        "runtime_tuning.viability.nonresidential_min_buffer_days_by_level",
        0.0,
        f32::INFINITY,
    )?;
    validate_nonempty_level_array(
        &tuning
            .viability
            .nonresidential_max_buffer_days_for_downgrade,
        "runtime_tuning.viability.nonresidential_max_buffer_days_for_downgrade",
        0.0,
        f32::INFINITY,
    )?;
    validate_range(
        tuning
            .viability
            .nonresidential_min_staffing_ratio_for_upgrade,
        0.0,
        1.0,
        "runtime_tuning.viability.nonresidential_min_staffing_ratio_for_upgrade",
    )?;
    validate_range(
        tuning
            .viability
            .nonresidential_max_staffing_ratio_for_downgrade,
        0.0,
        1.0,
        "runtime_tuning.viability.nonresidential_max_staffing_ratio_for_downgrade",
    )?;
    validate_range(
        tuning.viability.industrial_min_input_coverage_for_upgrade,
        0.0,
        1.0,
        "runtime_tuning.viability.industrial_min_input_coverage_for_upgrade",
    )?;
    validate_range(
        tuning.viability.industrial_min_output_headroom_for_upgrade,
        0.0,
        1.0,
        "runtime_tuning.viability.industrial_min_output_headroom_for_upgrade",
    )?;
    validate_range(
        tuning.viability.industrial_max_input_coverage_for_downgrade,
        0.0,
        1.0,
        "runtime_tuning.viability.industrial_max_input_coverage_for_downgrade",
    )?;
    validate_range(
        tuning
            .viability
            .industrial_max_output_headroom_for_downgrade,
        0.0,
        1.0,
        "runtime_tuning.viability.industrial_max_output_headroom_for_downgrade",
    )?;
    Ok(())
}

fn validate_work_profiles(clock: &OperationalClockRuntimeTuning) -> Result<(), String> {
    if clock.work_profiles.is_empty() {
        return Err("runtime_tuning.operational_clock.work_profiles must not be empty".to_owned());
    }
    let duplicates = duplicate_ids(
        clock
            .work_profiles
            .iter()
            .map(|profile| profile.id.as_str()),
    );
    if let Some(duplicate) = duplicates.into_iter().next() {
        return Err(format!(
            "runtime_tuning.operational_clock.work_profiles contains duplicate id '{duplicate}'"
        ));
    }
    for profile in &clock.work_profiles {
        if profile.id.trim().is_empty() {
            return Err(
                "runtime_tuning.operational_clock.work_profiles[*].id must not be empty".to_owned(),
            );
        }
        if profile.arrival_windows.is_empty() || profile.departure_windows.is_empty() {
            return Err(format!(
                "runtime_tuning.operational_clock.work_profiles.{} must define arrival_windows and departure_windows",
                profile.id
            ));
        }
        if profile.arrival_windows.len() != profile.departure_windows.len() {
            return Err(format!(
                "runtime_tuning.operational_clock.work_profiles.{} must use matching arrival/departure window counts",
                profile.id
            ));
        }
        if profile.reliability_buffer_minutes == 0 {
            return Err(format!(
                "runtime_tuning.operational_clock.work_profiles.{}.reliability_buffer_minutes must be > 0",
                profile.id
            ));
        }
        for (idx, window) in profile.arrival_windows.iter().enumerate() {
            validate_minute_window(
                *window,
                &format!(
                    "runtime_tuning.operational_clock.work_profiles.{}.arrival_windows[{idx}]",
                    profile.id
                ),
            )?;
        }
        for (idx, window) in profile.departure_windows.iter().enumerate() {
            validate_minute_window(
                *window,
                &format!(
                    "runtime_tuning.operational_clock.work_profiles.{}.departure_windows[{idx}]",
                    profile.id
                ),
            )?;
        }
    }
    for (zone_type, profile_id) in &clock.work_profile_by_zone_type {
        validate_zone_type_key(
            zone_type,
            "runtime_tuning.operational_clock.work_profile_by_zone_type",
        )?;
        if !clock
            .work_profiles
            .iter()
            .any(|profile| profile.id == *profile_id)
        {
            return Err(format!(
                "runtime_tuning.operational_clock.work_profile_by_zone_type.{zone_type} references missing profile '{profile_id}'"
            ));
        }
    }
    Ok(())
}

fn validate_freight_profiles(clock: &OperationalClockRuntimeTuning) -> Result<(), String> {
    if clock.freight_profiles.is_empty() {
        return Err(
            "runtime_tuning.operational_clock.freight_profiles must not be empty".to_owned(),
        );
    }
    let duplicates = duplicate_ids(
        clock
            .freight_profiles
            .iter()
            .map(|profile| profile.id.as_str()),
    );
    if let Some(duplicate) = duplicates.into_iter().next() {
        return Err(format!(
            "runtime_tuning.operational_clock.freight_profiles contains duplicate id '{duplicate}'"
        ));
    }
    for profile in &clock.freight_profiles {
        if profile.id.trim().is_empty() {
            return Err(
                "runtime_tuning.operational_clock.freight_profiles[*].id must not be empty"
                    .to_owned(),
            );
        }
        if profile.preferred_windows.is_empty() {
            return Err(format!(
                "runtime_tuning.operational_clock.freight_profiles.{} must define preferred_windows",
                profile.id
            ));
        }
        for (idx, window) in profile.preferred_windows.iter().enumerate() {
            validate_minute_window(
                *window,
                &format!(
                    "runtime_tuning.operational_clock.freight_profiles.{}.preferred_windows[{idx}]",
                    profile.id
                ),
            )?;
        }
        validate_range(
            profile.outside_window_cost_multiplier,
            1.0,
            10.0,
            &format!(
                "runtime_tuning.operational_clock.freight_profiles.{}.outside_window_cost_multiplier",
                profile.id
            ),
        )?;
    }
    for (zone_type, profile_id) in &clock.freight_profile_by_zone_type {
        validate_zone_type_key(
            zone_type,
            "runtime_tuning.operational_clock.freight_profile_by_zone_type",
        )?;
        if !clock
            .freight_profiles
            .iter()
            .any(|profile| profile.id == *profile_id)
        {
            return Err(format!(
                "runtime_tuning.operational_clock.freight_profile_by_zone_type.{zone_type} references missing profile '{profile_id}'"
            ));
        }
    }
    Ok(())
}

fn validate_minute_window(window: MinuteWindow, label: &str) -> Result<(), String> {
    let day_minutes: u16 = 24 * 60;
    if window.start_minute >= day_minutes
        || window.end_minute > day_minutes
        || window.start_minute >= window.end_minute
    {
        return Err(format!(
            "{label} must satisfy 0 <= start_minute < end_minute <= {}",
            day_minutes
        ));
    }
    Ok(())
}

fn validate_zone_type_key(zone_type: &str, label: &str) -> Result<(), String> {
    match zone_type {
        "commercial" | "industrial" | "residential" => Ok(()),
        other => Err(format!(
            "{label} contains unsupported zone_type key '{other}'"
        )),
    }
}

fn validate_nonempty_level_array(
    values: &[f32],
    label: &str,
    min_value: f32,
    max_value: f32,
) -> Result<(), String> {
    if values.is_empty() {
        return Err(format!("{label} must contain at least one level value"));
    }
    for (idx, value) in values.iter().copied().enumerate() {
        validate_range(value, min_value, max_value, &format!("{label}[{idx}]"))?;
    }
    Ok(())
}

fn validate_range(value: f32, min_value: f32, max_value: f32, label: &str) -> Result<(), String> {
    if !value.is_finite() || value < min_value || value > max_value {
        Err(format!(
            "{label} must be finite and in [{}..={}]",
            min_value, max_value
        ))
    } else {
        Ok(())
    }
}

fn validate_scenario(
    scenario: &EconomyScenario,
    profile_map: &BTreeMap<&str, &EconomyProfile>,
    controller_map: &BTreeMap<&str, &EconomyController>,
    messages: &mut Vec<ValidationMessage>,
) {
    if scenario.nodes.is_empty() {
        messages.push(error(
            "empty_scenario",
            format!("scenario.{}", scenario.id),
            "scenario has no graph nodes".to_owned(),
        ));
        return;
    }

    let duplicate_node_ids = duplicate_ids(scenario.nodes.iter().map(|node| node.id.as_str()));
    for duplicate in duplicate_node_ids {
        messages.push(error(
            "duplicate_scenario_node_id",
            format!("scenario.{}", scenario.id),
            format!("scenario node id '{duplicate}' is defined more than once"),
        ));
    }

    let node_map: BTreeMap<&str, &ScenarioNode> = scenario
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();
    let mut demand_sink_count = 0u32;

    for node in &scenario.nodes {
        match node.ref_kind.as_str() {
            "profile" => {
                let Some(profile) = profile_map.get(node.ref_id.as_str()) else {
                    messages.push(error(
                        "missing_profile_ref",
                        format!("scenario.{}.node.{}", scenario.id, node.id),
                        format!("scenario node references missing profile '{}'", node.ref_id),
                    ));
                    continue;
                };
                if profile.kind == "demand_sink" {
                    demand_sink_count += 1;
                }
            }
            "controller" => {
                if !controller_map.contains_key(node.ref_id.as_str()) {
                    messages.push(error(
                        "missing_controller_ref",
                        format!("scenario.{}.node.{}", scenario.id, node.id),
                        format!(
                            "scenario node references missing controller '{}'",
                            node.ref_id
                        ),
                    ));
                }
            }
            other => messages.push(error(
                "invalid_node_kind",
                format!("scenario.{}.node.{}", scenario.id, node.id),
                format!(
                    "scenario node kind '{other}' is invalid; expected 'profile' or 'controller'"
                ),
            )),
        }
    }

    if demand_sink_count == 0 {
        messages.push(error(
            "missing_demand_sink",
            format!("scenario.{}", scenario.id),
            "scenario must include one household demand sink node".to_owned(),
        ));
    } else if demand_sink_count > 1 {
        messages.push(warning(
            "multiple_demand_sinks",
            format!("scenario.{}", scenario.id),
            "scenario includes multiple demand sinks; sandbox playback uses the first one"
                .to_owned(),
        ));
    }

    let mut incoming_resources: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut profile_graph_outgoing: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut profile_graph_incoming_degree: BTreeMap<&str, u32> = BTreeMap::new();

    for edge in &scenario.edges {
        let Some(from_node) = node_map.get(edge.from.as_str()) else {
            messages.push(error(
                "missing_edge_source_node",
                format!("scenario.{}.edge.{}->{}", scenario.id, edge.from, edge.to),
                format!("edge source node '{}' does not exist", edge.from),
            ));
            continue;
        };
        let Some(to_node) = node_map.get(edge.to.as_str()) else {
            messages.push(error(
                "missing_edge_target_node",
                format!("scenario.{}.edge.{}->{}", scenario.id, edge.from, edge.to),
                format!("edge target node '{}' does not exist", edge.to),
            ));
            continue;
        };
        if from_node.ref_kind != "profile" || to_node.ref_kind != "profile" {
            messages.push(error(
                "invalid_edge_endpoint_kind",
                format!("scenario.{}.edge.{}->{}", scenario.id, edge.from, edge.to),
                "scenario edges may connect profile nodes only".to_owned(),
            ));
            continue;
        }

        let Some(from_profile) = profile_map.get(from_node.ref_id.as_str()) else {
            continue;
        };
        let Some(to_profile) = profile_map.get(to_node.ref_id.as_str()) else {
            continue;
        };

        if !port_exists(&from_profile.outputs, edge.resource.as_str()) {
            messages.push(error(
                "edge_resource_not_produced",
                format!("scenario.{}.edge.{}->{}", scenario.id, edge.from, edge.to),
                format!(
                    "source profile '{}' does not output resource '{}'",
                    from_profile.id, edge.resource
                ),
            ));
        }
        if !port_exists(&to_profile.inputs, edge.resource.as_str()) {
            messages.push(error(
                "edge_resource_not_consumed",
                format!("scenario.{}.edge.{}->{}", scenario.id, edge.from, edge.to),
                format!(
                    "target profile '{}' does not consume resource '{}'",
                    to_profile.id, edge.resource
                ),
            ));
        }

        incoming_resources
            .entry(edge.to.as_str())
            .or_default()
            .insert(edge.resource.as_str());

        profile_graph_outgoing
            .entry(edge.from.as_str())
            .or_default()
            .insert(edge.to.as_str());
        *profile_graph_incoming_degree
            .entry(edge.to.as_str())
            .or_insert(0) += 1;
        profile_graph_incoming_degree
            .entry(edge.from.as_str())
            .or_insert(0);
    }

    for node in &scenario.nodes {
        if node.ref_kind != "profile" {
            continue;
        }
        let Some(profile) = profile_map.get(node.ref_id.as_str()) else {
            continue;
        };
        let received = incoming_resources.get(node.id.as_str());
        for input in &profile.inputs {
            if received.is_none_or(|resources| !resources.contains(input.resource.as_str())) {
                messages.push(error(
                    "disconnected_required_input",
                    format!("scenario.{}.node.{}", scenario.id, node.id),
                    format!(
                        "profile '{}' requires input '{}' but no edge supplies it",
                        profile.id, input.resource
                    ),
                ));
            }
        }
    }

    for link in &scenario.controller_links {
        let Some(controller_node) = node_map.get(link.controller_node_id.as_str()) else {
            messages.push(error(
                "missing_controller_link_source",
                format!(
                    "scenario.{}.controller_link.{}->{}",
                    scenario.id, link.controller_node_id, link.target_node_id
                ),
                format!(
                    "controller node '{}' does not exist",
                    link.controller_node_id
                ),
            ));
            continue;
        };
        let Some(target_node) = node_map.get(link.target_node_id.as_str()) else {
            messages.push(error(
                "missing_controller_link_target",
                format!(
                    "scenario.{}.controller_link.{}->{}",
                    scenario.id, link.controller_node_id, link.target_node_id
                ),
                format!("target node '{}' does not exist", link.target_node_id),
            ));
            continue;
        };
        if controller_node.ref_kind != "controller" {
            messages.push(error(
                "invalid_controller_link_source_kind",
                format!(
                    "scenario.{}.controller_link.{}->{}",
                    scenario.id, link.controller_node_id, link.target_node_id
                ),
                "controller link source must be a controller node".to_owned(),
            ));
        }
        if target_node.ref_kind != "profile" {
            messages.push(error(
                "invalid_controller_link_target_kind",
                format!(
                    "scenario.{}.controller_link.{}->{}",
                    scenario.id, link.controller_node_id, link.target_node_id
                ),
                "controller links must target profile nodes".to_owned(),
            ));
        }
    }

    let cycle_detected = has_profile_cycle(
        &scenario.nodes,
        &profile_graph_outgoing,
        &profile_graph_incoming_degree,
    );
    if cycle_detected {
        messages.push(error(
            "cyclic_scenario_graph",
            format!("scenario.{}", scenario.id),
            "scenario graph contains a cycle; bootstrap playback requires an acyclic profile chain"
                .to_owned(),
        ));
    }
}

fn run_sandbox(project: &EconomyProject, scenario_id: &str) -> Result<SandboxResult, String> {
    let scenario = project
        .scenarios
        .iter()
        .find(|scenario| scenario.id == scenario_id)
        .ok_or_else(|| format!("scenario '{scenario_id}' not found"))?;

    let profile_map: BTreeMap<&str, &EconomyProfile> = project
        .profiles
        .iter()
        .map(|profile| (profile.id.as_str(), profile))
        .collect();
    let controller_map: BTreeMap<&str, &EconomyController> = project
        .controllers
        .iter()
        .map(|controller| (controller.id.as_str(), controller))
        .collect();
    let node_map: BTreeMap<&str, &ScenarioNode> = scenario
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();

    let topo_order = topological_profile_node_order(scenario)?;
    let demand_sink_node = scenario
        .nodes
        .iter()
        .find(|node| {
            node.ref_kind == "profile"
                && profile_map
                    .get(node.ref_id.as_str())
                    .is_some_and(|profile| profile.kind == "demand_sink")
        })
        .ok_or_else(|| format!("scenario '{}' has no demand sink", scenario.id))?;
    let demand_sink_profile = profile_map
        .get(demand_sink_node.ref_id.as_str())
        .copied()
        .ok_or_else(|| {
            format!(
                "demand sink profile '{}' is missing",
                demand_sink_node.ref_id
            )
        })?;

    let household_demand_per_day = scenario.household_count as f32
        * scenario.average_household_size
        * demand_sink_profile.consumption_rate_per_resident.max(0.0);

    let mut household_stock_units =
        household_demand_per_day * scenario.starting_household_stock_days.max(0.0);
    let mut lowest_stock_days = if household_demand_per_day > 0.0 {
        household_stock_units / household_demand_per_day
    } else {
        0.0
    };
    let mut total_delivered_units = 0.0;
    let mut total_unmet_units = 0.0;
    let mut total_household_cost = 0.0;
    let mut daily = Vec::with_capacity(scenario.duration_days as usize);
    let mut inventories: BTreeMap<String, BTreeMap<String, f32>> = BTreeMap::new();

    let outgoing_edges = build_outgoing_edges(scenario);
    let household_price_multiplier = household_cost_multiplier(
        scenario,
        demand_sink_node.id.as_str(),
        &node_map,
        &controller_map,
    );

    for day in 1..=scenario.duration_days {
        let mut delivered_today = 0.0;
        let mut unmet_today = 0.0;
        let mut household_cost_today = 0.0;

        for node_id in &topo_order {
            let node = node_map.get(node_id.as_str()).copied().ok_or_else(|| {
                format!(
                    "scenario node '{}' missing during sandbox playback",
                    node_id
                )
            })?;
            let profile = profile_map
                .get(node.ref_id.as_str())
                .copied()
                .ok_or_else(|| {
                    format!("profile '{}' missing during sandbox playback", node.ref_id)
                })?;

            if profile.kind == "demand_sink" {
                let delivered_to_sink =
                    take_all_incoming_stock(&mut inventories, node.id.as_str(), &profile.inputs);
                delivered_today += delivered_to_sink;
                household_stock_units += delivered_to_sink;
                let consumed = household_stock_units.min(household_demand_per_day);
                unmet_today = household_demand_per_day - consumed;
                household_stock_units -= consumed;
                household_cost_today += delivered_to_sink
                    * inferred_unit_price(
                        scenario,
                        node.id.as_str(),
                        &outgoing_edges,
                        &node_map,
                        &profile_map,
                    )
                    * household_price_multiplier;
                continue;
            }

            let throughput = compute_throughput(profile, &inventories, node.id.as_str());
            if profile.inputs.is_empty() {
                add_outputs_to_inventory(&mut inventories, node.id.as_str(), &profile.outputs, 1.0);
            } else if throughput > 0.0 && profile.base_rate_units_per_day > 0.0 {
                let scale = throughput / profile.base_rate_units_per_day;
                consume_inputs_from_inventory(
                    &mut inventories,
                    node.id.as_str(),
                    &profile.inputs,
                    scale,
                );
                add_outputs_to_inventory(
                    &mut inventories,
                    node.id.as_str(),
                    &profile.outputs,
                    scale,
                );
            }

            transfer_outgoing_stock(
                &mut inventories,
                node.id.as_str(),
                outgoing_edges.get(node.id.as_str()),
            );
        }

        let stock_days = if household_demand_per_day > 0.0 {
            household_stock_units / household_demand_per_day
        } else {
            0.0
        };
        lowest_stock_days = lowest_stock_days.min(stock_days);
        total_delivered_units += delivered_today;
        total_unmet_units += unmet_today;
        total_household_cost += household_cost_today;
        daily.push(DailySandboxMetric {
            day,
            household_stock_days: stock_days,
            delivered_units: delivered_today,
            unmet_units: unmet_today,
            average_household_cost: if scenario.household_count > 0 {
                household_cost_today / scenario.household_count as f32
            } else {
                0.0
            },
        });
    }

    let final_stock_days = if household_demand_per_day > 0.0 {
        household_stock_units / household_demand_per_day
    } else {
        0.0
    };

    let mut bottlenecks = Vec::new();
    if lowest_stock_days < 1.0 {
        bottlenecks.push(format!(
            "Household stock drops below 1.0 days and bottoms out at {:.2} days.",
            lowest_stock_days
        ));
    }
    if total_unmet_units > 0.0 {
        bottlenecks.push(format!(
            "Sandbox leaves {:.1} household_supplies units unmet across {} days.",
            total_unmet_units, scenario.duration_days
        ));
    }
    if bottlenecks.is_empty() {
        bottlenecks.push("Starter chain remains stocked for the whole sandbox run.".to_owned());
    }

    Ok(SandboxResult {
        scenario_id: scenario.id.clone(),
        display_name: scenario.display_name.clone(),
        duration_days: scenario.duration_days,
        daily_household_demand_units: household_demand_per_day,
        final_household_stock_days: final_stock_days,
        lowest_household_stock_days: lowest_stock_days,
        total_delivered_units,
        total_unmet_units,
        average_household_cost_per_day: if scenario.duration_days > 0
            && scenario.household_count > 0
        {
            total_household_cost / (scenario.duration_days as f32 * scenario.household_count as f32)
        } else {
            0.0
        },
        bottlenecks,
        daily,
    })
}

fn build_index(project: &EconomyProject) -> CompiledEconomyIndex {
    let mut compatibility = Vec::new();
    for source in &project.profiles {
        for output in &source.outputs {
            let mut targets = Vec::new();
            for target in &project.profiles {
                if port_exists(&target.inputs, output.resource.as_str()) {
                    targets.push(target.id.clone());
                }
            }
            if !targets.is_empty() {
                compatibility.push(CompiledCompatibility {
                    resource: output.resource.clone(),
                    source_profile_id: source.id.clone(),
                    target_profile_ids: targets,
                });
            }
        }
    }

    CompiledEconomyIndex {
        profile_ids: project
            .profiles
            .iter()
            .map(|profile| profile.id.clone())
            .collect(),
        controller_ids: project
            .controllers
            .iter()
            .map(|controller| controller.id.clone())
            .collect(),
        scenario_ids: project
            .scenarios
            .iter()
            .map(|scenario| scenario.id.clone())
            .collect(),
        compatibility,
    }
}

fn build_outgoing_edges<'a>(
    scenario: &'a EconomyScenario,
) -> BTreeMap<&'a str, Vec<&'a ScenarioEdge>> {
    let mut outgoing: BTreeMap<&str, Vec<&ScenarioEdge>> = BTreeMap::new();
    for edge in &scenario.edges {
        outgoing.entry(edge.from.as_str()).or_default().push(edge);
    }
    outgoing
}

fn household_cost_multiplier(
    scenario: &EconomyScenario,
    demand_sink_node_id: &str,
    node_map: &BTreeMap<&str, &ScenarioNode>,
    controller_map: &BTreeMap<&str, &EconomyController>,
) -> f32 {
    for link in &scenario.controller_links {
        if link.target_node_id != demand_sink_node_id {
            continue;
        }
        let Some(controller_node) = node_map.get(link.controller_node_id.as_str()) else {
            continue;
        };
        if controller_node.ref_kind != "controller" {
            continue;
        }
        let Some(controller) = controller_map.get(controller_node.ref_id.as_str()) else {
            continue;
        };
        if controller.kind != "household_restock_cost" {
            continue;
        }
        let t = controller.default_weight.clamp(0.0, 1.0);
        return controller.min_multiplier
            + (controller.max_multiplier - controller.min_multiplier) * t;
    }
    1.0
}

fn inferred_unit_price(
    scenario: &EconomyScenario,
    demand_sink_node_id: &str,
    outgoing_edges: &BTreeMap<&str, Vec<&ScenarioEdge>>,
    node_map: &BTreeMap<&str, &ScenarioNode>,
    profile_map: &BTreeMap<&str, &EconomyProfile>,
) -> f32 {
    for edge in &scenario.edges {
        if edge.to != demand_sink_node_id {
            continue;
        }
        let Some(source_node) = node_map.get(edge.from.as_str()) else {
            continue;
        };
        let Some(source_profile) = profile_map.get(source_node.ref_id.as_str()) else {
            continue;
        };
        if source_profile.unit_price_currency > 0.0 {
            return source_profile.unit_price_currency;
        }
    }
    for (node_id, edges) in outgoing_edges {
        if edges.iter().any(|edge| edge.to == demand_sink_node_id) {
            let Some(source_node) = node_map.get(node_id) else {
                continue;
            };
            let Some(source_profile) = profile_map.get(source_node.ref_id.as_str()) else {
                continue;
            };
            if source_profile.unit_price_currency > 0.0 {
                return source_profile.unit_price_currency;
            }
        }
    }
    0.0
}

fn compute_throughput(
    profile: &EconomyProfile,
    inventories: &BTreeMap<String, BTreeMap<String, f32>>,
    node_id: &str,
) -> f32 {
    if profile.inputs.is_empty() {
        return profile.base_rate_units_per_day.max(0.0);
    }
    if profile.base_rate_units_per_day <= 0.0 {
        return 0.0;
    }

    let mut throughput = profile.base_rate_units_per_day;
    for input in &profile.inputs {
        let available = inventories
            .get(node_id)
            .and_then(|stock| stock.get(input.resource.as_str()))
            .copied()
            .unwrap_or(0.0);
        if input.units_per_day <= 0.0 {
            continue;
        }
        let allowed = available / input.units_per_day * profile.base_rate_units_per_day;
        throughput = throughput.min(allowed);
    }
    throughput.max(0.0)
}

fn add_outputs_to_inventory(
    inventories: &mut BTreeMap<String, BTreeMap<String, f32>>,
    node_id: &str,
    outputs: &[ResourcePort],
    scale: f32,
) {
    let stock = inventories.entry(node_id.to_owned()).or_default();
    for output in outputs {
        *stock.entry(output.resource.clone()).or_default() += output.units_per_day * scale;
    }
}

fn consume_inputs_from_inventory(
    inventories: &mut BTreeMap<String, BTreeMap<String, f32>>,
    node_id: &str,
    inputs: &[ResourcePort],
    scale: f32,
) {
    let stock = inventories.entry(node_id.to_owned()).or_default();
    for input in inputs {
        let entry = stock.entry(input.resource.clone()).or_default();
        *entry = (*entry - input.units_per_day * scale).max(0.0);
    }
}

fn take_all_incoming_stock(
    inventories: &mut BTreeMap<String, BTreeMap<String, f32>>,
    node_id: &str,
    inputs: &[ResourcePort],
) -> f32 {
    let stock = inventories.entry(node_id.to_owned()).or_default();
    let mut total = 0.0;
    for input in inputs {
        if let Some(entry) = stock.get_mut(input.resource.as_str()) {
            total += *entry;
            *entry = 0.0;
        }
    }
    total
}

fn transfer_outgoing_stock(
    inventories: &mut BTreeMap<String, BTreeMap<String, f32>>,
    from_node_id: &str,
    outgoing_edges: Option<&Vec<&ScenarioEdge>>,
) {
    let Some(outgoing_edges) = outgoing_edges else {
        return;
    };

    let mut transfers: Vec<(String, String, f32)> = Vec::new();
    {
        let Some(stock) = inventories.get_mut(from_node_id) else {
            return;
        };
        for edge in outgoing_edges {
            let Some(amount) = stock.get_mut(edge.resource.as_str()) else {
                continue;
            };
            if *amount <= 0.0 {
                continue;
            }
            let moved = *amount;
            *amount = 0.0;
            transfers.push((edge.to.clone(), edge.resource.clone(), moved));
        }
    }

    for (target_node, resource, amount) in transfers {
        *inventories
            .entry(target_node)
            .or_default()
            .entry(resource)
            .or_default() += amount;
    }
}

fn topological_profile_node_order(scenario: &EconomyScenario) -> Result<Vec<String>, String> {
    let mut outgoing: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut indegree: BTreeMap<&str, u32> = scenario
        .nodes
        .iter()
        .filter(|node| node.ref_kind == "profile")
        .map(|node| (node.id.as_str(), 0))
        .collect();

    for edge in &scenario.edges {
        outgoing
            .entry(edge.from.as_str())
            .or_default()
            .push(edge.to.as_str());
        *indegree.entry(edge.to.as_str()).or_insert(0) += 1;
    }

    let mut queue = VecDeque::new();
    for (&node_id, &degree) in &indegree {
        if degree == 0 {
            queue.push_back(node_id);
        }
    }

    let mut ordered = Vec::with_capacity(indegree.len());
    while let Some(node_id) = queue.pop_front() {
        ordered.push(node_id.to_owned());
        if let Some(next) = outgoing.get(node_id) {
            for &target in next {
                if let Some(degree) = indegree.get_mut(target) {
                    *degree -= 1;
                    if *degree == 0 {
                        queue.push_back(target);
                    }
                }
            }
        }
    }

    if ordered.len() != indegree.len() {
        return Err(format!(
            "scenario '{}' contains a profile-cycle",
            scenario.id
        ));
    }

    Ok(ordered)
}

fn has_profile_cycle(
    nodes: &[ScenarioNode],
    outgoing: &BTreeMap<&str, BTreeSet<&str>>,
    indegree: &BTreeMap<&str, u32>,
) -> bool {
    let profile_node_count = nodes
        .iter()
        .filter(|node| node.ref_kind == "profile")
        .count();
    if profile_node_count == 0 {
        return false;
    }

    let mut indegree = indegree.clone();
    let mut queue = VecDeque::new();
    for node in nodes.iter().filter(|node| node.ref_kind == "profile") {
        if indegree.get(node.id.as_str()).copied().unwrap_or(0) == 0 {
            queue.push_back(node.id.as_str());
        }
    }

    let mut visited = 0usize;
    while let Some(node_id) = queue.pop_front() {
        visited += 1;
        if let Some(targets) = outgoing.get(node_id) {
            for &target in targets {
                if let Some(entry) = indegree.get_mut(target) {
                    *entry -= 1;
                    if *entry == 0 {
                        queue.push_back(target);
                    }
                }
            }
        }
    }

    visited != profile_node_count
}

fn duplicate_ids<'a>(ids: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut duplicates = BTreeSet::new();
    for id in ids {
        if !seen.insert(id.to_owned()) {
            duplicates.insert(id.to_owned());
        }
    }
    duplicates.into_iter().collect()
}

fn port_exists(ports: &[ResourcePort], resource: &str) -> bool {
    ports.iter().any(|port| port.resource == resource)
}

fn error(
    code: &'static str,
    scope: impl Into<String>,
    message: impl Into<String>,
) -> ValidationMessage {
    ValidationMessage {
        severity: "error",
        code,
        scope: scope.into(),
        message: message.into(),
    }
}

fn warning(
    code: &'static str,
    scope: impl Into<String>,
    message: impl Into<String>,
) -> ValidationMessage {
    ValidationMessage {
        severity: "warning",
        code,
        scope: scope.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
id = "household_demand_sink"
display_name = "Household Demand Sink"
kind = "demand_sink"
description = "Starter sink"
consumption_rate_per_resident = 1.0

[[profiles.inputs]]
resource = "household_supplies"
units_per_day = 1.0
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
ref_id = "household_demand_sink"
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

        let project_json = serde_json::to_string(project).unwrap();
        let result = run_sandbox_json(&project_json, "grocery_bottleneck").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(parsed["ok"].as_bool().unwrap());
    }
}
