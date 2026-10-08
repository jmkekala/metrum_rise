// SPDX-License-Identifier: GPL-2.0-only

//! Runtime economy tuning and compiled catalog contracts used by live simulation systems.

use super::serde_helpers::{
    deserialize_u16_from_number, deserialize_u16_vec_from_numbers, deserialize_u32_from_number,
};
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
    /// Freight batching, border terminal, and failure escalation limits.
    pub logistics: LogisticsRuntimeTuning,
    /// Private building construction durations for fresh demand-owned spawns.
    pub construction: ConstructionRuntimeTuning,
    /// City revenue policy applied to wages, consumption, freight purchases, and new buildings.
    pub fiscal: FiscalRuntimeTuning,
    /// Multiplier applied to the local resource price when the OWA supplies an import.
    /// Values above 1.0 make OWA imports more expensive than local sourcing, giving local
    /// producers a cost advantage once they are operational. Must be >= 1.0; values below
    /// 1.0 are rejected at validation time.
    pub owa_import_price_multiplier: f32,
    /// Multiplier applied to the local resource price when an industrial building exports surplus
    /// output to the finite regional market. Must be in
    /// `[0.0, 1.0]`; values outside this range are rejected at validation time.
    pub owa_export_price_multiplier: f32,
    /// Multiplier applied when a distressed building fire-sells unreserved output inventory.
    /// Must be no higher than scheduled exports; shipped tuning keeps it lower so bankruptcy
    /// liquidation is a last resort, not a profitable operating model.
    pub owa_distress_liquidation_multiplier: f32,
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
    /// Currency paid per elder resident per day from the city treasury.
    pub pension_daily_benefit_per_elder: f32,
    /// Currency paid per child resident per day from the city treasury.
    pub child_support_daily_benefit_per_child: f32,
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
    /// Hours to wait before retrying a failed household replenishment.
    #[serde(default, deserialize_with = "deserialize_u16_from_number")]
    pub household_replenishment_retry_cooldown_hours: u16,
    /// Consecutive failed household replenishment attempts before terminal shortage.
    #[serde(default, deserialize_with = "deserialize_u16_from_number")]
    pub household_replenishment_terminal_failure_count: u16,
    /// Extra operational hours allowed beyond estimated physical travel for each shopping leg.
    #[serde(default, deserialize_with = "deserialize_u16_from_number")]
    pub household_shopping_leg_timeout_hours: u16,
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

/// Runtime controls for batched freight and border-terminal behavior.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub(crate) struct LogisticsRuntimeTuning {
    /// Maximum cargo mass carried by one physical freight vehicle, independent of batching.
    pub truck_payload_kg: f32,
    /// Carrier contract fee per dispatched vehicle, independent of cargo mass.
    pub dispatch_fee_currency: f32,
    /// Carrier price per operational vehicle-hour, including the return journey.
    pub vehicle_hour_cost_currency: f32,
    /// Maximum active dispatched OWA freight jobs allowed at one border node.
    #[serde(default, deserialize_with = "deserialize_u16_from_number")]
    pub border_active_jobs_per_node: u16,
    /// Maximum queued OWA freight jobs allowed at one border node.
    #[serde(default, deserialize_with = "deserialize_u16_from_number")]
    pub border_queued_jobs_per_node: u16,
    /// Operational hours a queued border shipment may wait before expiring.
    #[serde(default, deserialize_with = "deserialize_u16_from_number")]
    pub queued_shipment_expiry_hours: u16,
    /// Failed request attempts before a building/resource request becomes terminal.
    #[serde(default, deserialize_with = "deserialize_u16_from_number")]
    pub terminal_failure_attempts: u16,
}

/// Runtime controls for fresh private building construction.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub(crate) struct ConstructionRuntimeTuning {
    /// Fresh residential construction hours by target level, indexed from level 1.
    #[serde(deserialize_with = "deserialize_u16_vec_from_numbers")]
    pub residential_hours_by_level: Vec<u16>,
    /// Fresh commercial construction hours by target level, indexed from level 1.
    #[serde(deserialize_with = "deserialize_u16_vec_from_numbers")]
    pub commercial_hours_by_level: Vec<u16>,
    /// Fresh industrial construction hours by target level, indexed from level 1.
    #[serde(deserialize_with = "deserialize_u16_vec_from_numbers")]
    pub industrial_hours_by_level: Vec<u16>,
}

/// Runtime fiscal policy that controls city treasury revenue flows.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub(crate) struct FiscalRuntimeTuning {
    /// Fraction of gross daily wages withheld before household income is received.
    pub income_tax_rate: f32,
    /// Fraction added to household store purchases and remitted to the city at pickup.
    pub household_vat_rate: f32,
    /// Fraction of positive daily business operating-budget growth remitted to the city.
    pub business_profit_tax_rate: f32,
    /// Currency charged per occupied residential home per day.
    pub residential_property_tax_per_home_per_day: f32,
    /// Currency charged per active private commercial building per day.
    pub commercial_property_tax_per_building_per_day: f32,
    /// Currency charged per active private industrial building per day.
    pub industrial_property_tax_per_building_per_day: f32,
    /// Per-level multiplier applied to property tax above level 1.
    pub property_tax_level_multiplier: f32,
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
    /// Combined daily water/sewage charge per resident; electricity is metered separately.
    pub water_sewage_cost_per_member_per_day: f32,
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
    /// Minimum staffing ratio required for non-residential upgrades.
    pub nonresidential_min_staffing_ratio_for_upgrade: f32,
    /// Minimum industrial input coverage required for industrial upgrades.
    pub industrial_min_input_coverage_for_upgrade: f32,
    /// Minimum industrial output headroom required for industrial upgrades.
    pub industrial_min_output_headroom_for_upgrade: f32,
}

/// Compact runtime resource id resolved from authored resource names.
pub(crate) type ResourceRuntimeId = u16;

/// Canonical resource or utility quantity; rates use these quantities per operational day.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum QuantityUnit {
    /// Physical goods are accounted and carried by mass.
    #[serde(rename = "kg")]
    Kilogram,
    /// Electrical energy supplied during an interval, not instantaneous power.
    #[serde(rename = "kWh")]
    KilowattHour,
    /// Supplied water or treated wastewater volume.
    #[serde(rename = "m3")]
    CubicMetre,
    /// Completed service equivalents, which cannot be stored or shipped as cargo.
    #[serde(rename = "service")]
    Service,
}

impl QuantityUnit {
    /// Display symbol for an already canonical quantity; no numeric conversion is needed.
    pub(crate) fn symbol(self) -> &'static str {
        match self {
            Self::Kilogram => "kg",
            Self::KilowattHour => "kWh",
            Self::CubicMetre => "m³",
            Self::Service => "services",
        }
    }
}

/// Immutable dimensional and batching metadata indexed by stable resource id.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ResourceMeasure {
    /// Canonical quantity for inventory, rates and prices.
    pub unit: QuantityUnit,
    /// Minimum freight batch in kg; services have no freight batch.
    pub shipment_batch_kg: f32,
}

/// One compiled runtime resource port.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RuntimeResourcePort {
    /// Runtime resource id carried through this port.
    pub resource_runtime_id: ResourceRuntimeId,
    /// Authored throughput in the resource's canonical quantity per operational day.
    pub units_per_day: f32,
    /// Resource-specific ordinary freight batch, precompiled to avoid hot-path lookups.
    pub shipment_batch_kg: f32,
}

/// Broad runtime behavior class compiled from one authored economy profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EconomyProfileRuntimeKind {
    /// Upstream producing building that writes directly into its output stock buffer.
    Producer,
    /// Renewable producer whose output is scaled by a committed player-drawn field polygon.
    FieldProducer,
    /// Industrial converter that consumes input inventory and writes processed output inventory.
    Processor,
    /// Throughput building that converts one input buffer into one output buffer.
    Store,
    /// Commercial service-capacity building whose outputs are aggregate visits, not inventory.
    ServiceStore,
    /// Non-building aggregate sink profile used by sandbox and household-side graphs.
    DemandSink,
    /// Deposit-backed producer scaled by a committed player-drawn extraction polygon.
    Extractor,
    /// City utility building that generates a service signal (power, water).
    UtilityProducer,
    /// City utility building that processes a service output (sewage treatment).
    UtilityProcessor,
    /// Authored profile kind that the live starter runtime does not execute yet.
    Unsupported,
}

/// Daily electricity coefficients; operating load is per hectare for field/mine profiles.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ElectricityDemand {
    /// Standby load once per occupied building, independent of production.
    pub idle_kwh_per_day: f32,
    /// Additional full-activity load, scaled by the existing production activity.
    pub operating_kwh_per_day: f32,
    /// Residential demand per linked resident, independent of their ability to pay.
    pub per_resident_kwh_per_day: f32,
}

/// Compiled runtime view of one authored `economy_profile`.
#[derive(Clone, Debug)]
pub(crate) struct EconomyProfileRuntime {
    /// Stable compact runtime id used on live buildings.
    pub runtime_id: u16,
    /// Authored stable profile id.
    pub id: String,
    /// Broad runtime behavior kind used by the starter live economy.
    pub kind: EconomyProfileRuntimeKind,
    /// Optional authored work schedule profile id.
    pub work_schedule_profile: Option<String>,
    /// Optional authored freight timing profile id.
    pub freight_timing_profile: Option<String>,
    /// Fixed baseline unit sale price for the profile's main output.
    pub unit_price_currency: f32,
    /// Authored nominal service or output throughput in units per day.
    pub base_rate_units_per_day: f32,
    /// Fixed minimum daily wage offered by this profile.
    pub wage_min_currency_per_day: f32,
    /// Fixed maximum daily wage offered by this profile.
    pub wage_max_currency_per_day: f32,
    /// Authored worker slots, per staffing reference area for field/extraction profiles.
    pub worker_capacity: u32,
    /// Precompiled field/extraction worker density, independent of output per hectare.
    pub workers_per_hectare: f32,
    /// Whole-site staffing ceiling for area-backed producers, independent of land-scaled costs.
    pub area_worker_capacity_max: Option<u32>,
    /// Smallest field that may be newly committed or resized for this crop profile.
    pub minimum_field_area_m2: f32,
    /// Initial site preparation cost per added hectare, separate from the building footprint.
    pub area_build_cost_per_hectare: f32,
    /// Additional electricity (kWh), water (m³), sewage (m³) per operational hectare/day.
    pub area_utility_units_per_hectare: [f32; 3],
    /// Profile-authored daily standby, activity and residential electricity rates.
    pub electricity: ElectricityDemand,
    /// Maximum electrical output at full staffing; energy capacity is kW times hours.
    pub power_capacity_kw: f32,
    /// Authored target stock horizon in days for the starter live runtime.
    pub stock_target_days: f32,
    /// Output units pre-seeded into inventory when the building is first placed.
    /// Expressed as days of output throughput. Zero disables seeding.
    pub starting_inventory_days: f32,
    /// Authored reorder threshold in days for the starter live runtime.
    pub reorder_threshold_days: f32,
    /// Authored critical threshold in days for emergency freight.
    pub critical_threshold_days: f32,
    /// Household-demand consumption rate used by abstract sink profiles.
    pub consumption_rate_per_resident: f32,
    /// Which utility service this building provides ("power", "water", or "sewage").
    /// `None` for non-utility profiles.
    pub utility_service: Option<String>,
    /// Physical measure of utility throughput and billing; absent for non-utilities.
    pub utility_quantity_unit: Option<QuantityUnit>,
    /// Compiled typed input ports used by the live runtime.
    pub inputs: Vec<RuntimeResourcePort>,
    /// Compiled typed output ports used by the live runtime.
    pub outputs: Vec<RuntimeResourcePort>,
    /// False when the authored profile exists but the live runtime cannot execute it yet.
    pub runtime_supported: bool,
}

impl EconomyProfileRuntime {
    /// Checks the crop's area minimum for previews and commits, before land or cash changes.
    pub(crate) fn validate_field_area(&self, area_m2: f32) -> Result<(), String> {
        if area_m2 < self.minimum_field_area_m2 {
            return Err(format!(
                "field must cover at least {:.1} ha ({:.0} m²)",
                self.minimum_field_area_m2 / 10_000.0,
                self.minimum_field_area_m2
            ));
        }
        Ok(())
    }

    /// Private-building utility demand, including area maintenance independent of production.
    pub(crate) fn utility_demand_units_per_day(&self, hectares: f32) -> [f32; 3] {
        let area = if matches!(
            self.kind,
            EconomyProfileRuntimeKind::FieldProducer | EconomyProfileRuntimeKind::Extractor
        ) {
            crate::simulation::work_area::sanitize_work_area_scale(hectares)
        } else {
            0.0
        };
        [
            self.electricity_demand_kwh_per_day(hectares, 1.0, 0),
            1.0 + area * self.area_utility_units_per_hectare[1],
            1.0 + area * self.area_utility_units_per_hectare[2],
        ]
    }

    /// Physical demand for a building, independent of its cash or electricity supply.
    pub(crate) fn electricity_demand_kwh_per_day(
        &self,
        hectares: f32,
        activity: f32,
        residents: u32,
    ) -> f32 {
        let uses_area =
            crate::simulation::work_area::profile_kind_uses_explicit_work_area(self.kind);
        let area = if uses_area {
            crate::simulation::work_area::sanitize_work_area_scale(hectares)
        } else {
            0.0
        };
        self.electricity.idle_kwh_per_day
            + area * self.area_utility_units_per_hectare[0]
            + self.electricity.operating_kwh_per_day
                * activity.clamp(0.0, 1.0)
                * if uses_area { area } else { 1.0 }
            + self.electricity.per_resident_kwh_per_day * residents as f32
    }

    /// Charges added area only; shrinking or recommitting an unchanged site grants no refund.
    pub(crate) fn area_expansion_cost(&self, previous_m2: f32, new_m2: f32) -> f64 {
        (f64::from(new_m2) - f64::from(previous_m2)).max(0.0) / 10_000.0
            * f64::from(self.area_build_cost_per_hectare)
    }

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

    /// Output available to customers after the recipe's same-resource upkeep.
    pub(crate) fn net_output_units_per_day(&self, port: &RuntimeResourcePort) -> f32 {
        (port.units_per_day
            - self
                .input_port(port.resource_runtime_id)
                .map_or(0.0, |input| input.units_per_day))
        .max(0.0)
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

    /// Input buffers scale with production area and retain two ordinary shipment batches.
    /// A zero-sized producer or disabled port never requests stock.
    pub(crate) fn input_inventory_targets(
        &self,
        port: &RuntimeResourcePort,
        scale: f32,
    ) -> (f32, f32, f32) {
        if scale <= 0.0 || port.units_per_day <= 0.0 {
            return (0.0, 0.0, 0.0);
        }
        let batch = port.shipment_batch_kg;
        let target = (self.inventory_target_units_for(port) * scale).max(2.0 * batch);
        let reorder = if self.reorder_threshold_days > 0.0 {
            (self.inventory_reorder_units_for(port) * scale)
                .max(batch)
                .min(target)
        } else {
            0.0
        };
        let critical = (self.inventory_critical_units_for(port) * scale).min(target);
        (target, reorder, critical)
    }

    /// Funds the initial input buffers at OWA prices without pre-seeding free inventory.
    pub(crate) fn initial_input_import_cost(
        &self,
        catalog: &RuntimeEconomyCatalog,
        scale: f32,
        import_multiplier: f32,
    ) -> f32 {
        self.inputs
            .iter()
            .map(|port| {
                self.input_inventory_targets(port, scale).0
                    * catalog
                        .unit_price_for_resource(port.resource_runtime_id)
                        .expect("compiled input has a price")
                    * import_multiplier
            })
            .sum()
    }

    /// Initial owner equity for a zoned business: seven payroll days and opening input buffers.
    pub(crate) fn zoned_startup_budget(
        &self,
        catalog: &RuntimeEconomyCatalog,
        imports: f32,
    ) -> f32 {
        (self.worker_capacity as f32 * self.average_daily_wage() * 7.0
            + self.initial_input_import_cost(catalog, 1.0, imports))
        .max(500.0)
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
    pub(super) resource_measures: Vec<ResourceMeasure>,
    /// Supplier-before-buyer phases of the authored net-input dependency graph.
    pub(super) funding_phases: Vec<usize>,
    /// Number of dependency phases, including mutually dependent recipes as one phase.
    pub(super) funding_phase_count: usize,
}

impl RuntimeEconomyCatalog {
    /// Stable local-funding phase; mutually dependent recipes share a phase, while
    /// downstream recipes follow all their suppliers. Invalid/non-economic ids use phase zero.
    pub(crate) fn funding_phase(&self, runtime_id: u16) -> usize {
        runtime_id.checked_sub(1)
            .and_then(|index| self.funding_phases.get(index as usize))
            .copied().unwrap_or(0)
    }

    /// Number of authored dependency phases, independent of placed city size.
    pub(crate) fn funding_phase_count(&self) -> usize {
        self.funding_phase_count.max(1)
    }

    /// Returns immutable quantity and batching metadata in O(1) by stable resource id.
    pub(crate) fn resource_measure(&self, resource: ResourceRuntimeId) -> Option<ResourceMeasure> {
        resource
            .checked_sub(1)
            .and_then(|slot| self.resource_measures.get(slot as usize))
            .copied()
    }

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
