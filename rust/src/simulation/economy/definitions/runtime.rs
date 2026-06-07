//! Runtime economy tuning and compiled catalog contracts used by live simulation systems.

use super::serde_helpers::{deserialize_u16_from_number, deserialize_u32_from_number};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Authored economy-side runtime tuning used by the live simulation.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub(crate) struct RuntimeEconomyTuning {
    /// Shared operational clock state and authored schedule profiles.
    pub operational_clock: OperationalClockRuntimeTuning,
    /// Household relocation and eviction thresholds.
    pub households: HouseholdRuntimeTuning,
    /// Building viability thresholds used by demand-owned level changes.
    pub viability: BuildingViabilityRuntimeTuning,
    /// Daily OWA utility charge for one commercial building when local utilities are incomplete.
    pub commercial_owa_utility_cost_per_day: f32,
    /// Daily OWA utility charge for one industrial building when local utilities are incomplete.
    pub industrial_owa_utility_cost_per_day: f32,
    /// Multiplier applied to the local resource price when the OWA supplies an import.
    /// Values above 1.0 make OWA imports more expensive than local sourcing, giving local
    /// producers a cost advantage once they are operational. Must be >= 1.0; values below
    /// 1.0 are rejected at validation time.
    pub owa_import_price_multiplier: f32,
    /// Multiplier applied to the local resource price when an industrial building exports surplus
    /// output to the OWA. Values below 1.0 make OWA exports less profitable than local sales,
    /// keeping the OWA as a safety-valve rather than a primary revenue source. Must be in
    /// `[0.0, 1.0]`; values outside this range are rejected at validation time.
    pub owa_export_price_multiplier: f32,
    /// Starting city treasury balance at new-game creation.
    /// Migrated from the `STARTUP_TREASURY_BALANCE` Rust constant to make it tunable per-profile.
    pub startup_treasury_balance: f64,
    /// Currency paid per unemployed household member per day from the city treasury.
    /// Must cover at least one day's household supply cost to generate real purchase activity.
    pub unemployment_daily_benefit_per_member: f32,
    /// Days a household may receive unemployment benefit before becoming emigration-eligible.
    /// Prevents infinite treasury drain when no jobs exist in the city.
    #[serde(default, deserialize_with = "deserialize_u32_from_number")]
    pub unemployment_max_days: u32,
}

/// Shared operational-clock tuning used by labor, replenishment, and freight.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub(crate) struct OperationalClockRuntimeTuning {
    /// Real seconds required to advance one authored operational day at `1.0x`.
    pub seconds_per_day: f64,
    /// Minutes between cached commute-estimate refreshes.
    #[serde(default, deserialize_with = "deserialize_u16_from_number")]
    pub travel_estimate_refresh_minutes: u16,
    /// Hours between household replenishment checks.
    #[serde(default, deserialize_with = "deserialize_u16_from_number")]
    pub household_replenishment_check_interval_hours: u16,
    /// Hours between reserve creation and household pickup completion.
    #[serde(default, deserialize_with = "deserialize_u16_from_number")]
    pub household_pickup_eta_hours: u16,
    /// Hours to wait before retrying a failed household replenishment.
    #[serde(default, deserialize_with = "deserialize_u16_from_number")]
    pub household_replenishment_retry_cooldown_hours: u16,
    /// Hours to wait before retrying a failed freight request.
    #[serde(default, deserialize_with = "deserialize_u16_from_number")]
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
    #[serde(default, deserialize_with = "deserialize_u16_from_number")]
    pub start_minute: u16,
    /// Exclusive end minute from midnight.
    #[serde(default, deserialize_with = "deserialize_u16_from_number")]
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
    #[serde(default, deserialize_with = "deserialize_u16_from_number")]
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
    #[serde(default, deserialize_with = "deserialize_u16_from_number")]
    pub outside_window_eta_penalty_minutes: u16,
    /// Cost multiplier applied outside the preferred window.
    pub outside_window_cost_multiplier: f32,
}

/// Household-side runtime tuning values derived from `economy/profiles.toml`.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub(crate) struct HouseholdRuntimeTuning {
    /// Pantry days granted to newly-arrived immigrant households.
    pub immigrant_starting_stock_days: f32,
    /// Starting currency granted per household member when an immigrant household arrives.
    pub immigrant_starting_budget_per_member: f32,
    /// Minimum budget for materialized households that already have a home but no record.
    pub household_starting_budget_floor: f32,
    /// Daily household utility charge per resident.
    pub utility_cost_per_member_per_day: f32,
    /// Minimum reserve-days required to move into each residential level.
    pub residential_move_in_min_reserve_days_by_level: Vec<f32>,
    /// Minimum reserve-days required to remain in each residential level.
    pub residential_stay_min_reserve_days_by_level: Vec<f32>,
    /// Number of consecutive failed stay checks before eviction is allowed.
    #[serde(deserialize_with = "deserialize_u32_from_number")]
    pub stay_failure_days_before_eviction: u32,
}

/// Building-side viability thresholds derived from `economy/profiles.toml`.
#[derive(Clone, Serialize, Deserialize, Debug)]
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
    /// Maximum number of workers this building can employ, as authored in the economy profile.
    pub worker_capacity: u32,
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
    pub(super) profiles: Vec<EconomyProfileRuntime>,
    pub(super) by_id: BTreeMap<String, u16>,
    pub(super) resource_by_id: BTreeMap<String, ResourceRuntimeId>,
    pub(super) resource_id_by_runtime_id: Vec<String>,
    pub(super) price_by_resource: BTreeMap<ResourceRuntimeId, f32>,
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
    pub(crate) fn resource_id_for_runtime_id(&self, runtime_id: u16) -> Option<&str> {
        runtime_id
            .checked_sub(1)
            .and_then(|idx| self.resource_id_by_runtime_id.get(idx as usize))
            .map(String::as_str)
    }

    /// Returns the default runtime unit price for one resource when `OWA` imports it.
    pub(crate) fn unit_price_for_resource(&self, resource: ResourceRuntimeId) -> Option<f32> {
        self.price_by_resource.get(&resource).copied()
    }

    /// Returns a slice of all compiled runtime profiles in this catalog.
    pub(crate) fn all_profiles(&self) -> &[EconomyProfileRuntime] {
        &self.profiles
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
