//! Demand-driven daily growth pass built from authored baseline tuning.

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::households::{HouseholdSystem, household_reserve_days};
use crate::simulation::grid::zoning::{ZoneType, ZoningSystem};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::{NodeType, TransitFlags, TransitType};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

const GROWTH_PROFILES_FILE: &str = "demand/growth_profiles.toml";
const EPSILON: f32 = 0.0001;
const SHIPPED_PROFILE_ORDER: [(&str, DemandChannel); 9] = [
    ("residential_low_default", DemandChannel::ResidentialGrowth),
    (
        "residential_medium_default",
        DemandChannel::ResidentialGrowth,
    ),
    ("residential_high_default", DemandChannel::ResidentialGrowth),
    ("commercial_low_default", DemandChannel::CommercialGrowth),
    ("commercial_medium_default", DemandChannel::CommercialGrowth),
    ("commercial_high_default", DemandChannel::CommercialGrowth),
    ("industrial_low_default", DemandChannel::IndustrialGrowth),
    ("industrial_medium_default", DemandChannel::IndustrialGrowth),
    ("industrial_high_default", DemandChannel::IndustrialGrowth),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum DemandUse {
    Residential,
    Commercial,
    Industrial,
}

impl DemandUse {
    fn zone_type(self) -> ZoneType {
        match self {
            Self::Residential => ZoneType::Residential,
            Self::Commercial => ZoneType::Commercial,
            Self::Industrial => ZoneType::Industrial,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum DemandChannel {
    ResidentialGrowth,
    CommercialGrowth,
    IndustrialGrowth,
}

impl DemandChannel {
    fn from_str_name(value: &str) -> Option<Self> {
        match value.trim() {
            "ResidentialGrowth" => Some(Self::ResidentialGrowth),
            "CommercialGrowth" => Some(Self::CommercialGrowth),
            "IndustrialGrowth" => Some(Self::IndustrialGrowth),
            _ => None,
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
struct GrowthProfileRuntime {
    id: String,
    demand_channel: DemandChannel,
    cadence_days: u32,
    base_pressure_weight: f32,
    local_modifier_scale: f32,
    spawn_threshold: f32,
    despawn_threshold: f32,
    upgrade_threshold: f32,
    downgrade_threshold: f32,
    hysteresis_margin: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct UseTuningF32 {
    residential: f32,
    commercial: f32,
    industrial: f32,
}

impl UseTuningF32 {
    fn get(&self, use_kind: DemandUse) -> f32 {
        match use_kind {
            DemandUse::Residential => self.residential,
            DemandUse::Commercial => self.commercial,
            DemandUse::Industrial => self.industrial,
        }
    }

    fn get_mut(&mut self, use_kind: DemandUse) -> &mut f32 {
        match use_kind {
            DemandUse::Residential => &mut self.residential,
            DemandUse::Commercial => &mut self.commercial,
            DemandUse::Industrial => &mut self.industrial,
        }
    }

    pub(crate) fn as_array(self) -> [f32; 3] {
        [self.residential, self.commercial, self.industrial]
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct DemandBuildingActionKey {
    pub(crate) edge_idx: usize,
    pub(crate) side: i8,
    pub(crate) cell_x: usize,
    pub(crate) width_cells: u16,
    pub(crate) depth_cells: u16,
    pub(crate) level: u8,
    pub(crate) asset_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DemandLevelChangeAction {
    pub(crate) building: DemandBuildingActionKey,
    pub(crate) target_asset_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DemandSpawnAction {
    pub(crate) edge_idx: usize,
    pub(crate) side: i8,
    pub(crate) cell_x: usize,
    pub(crate) asset_id: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct DemandUseActionPlan {
    pub(crate) despawns: Vec<DemandBuildingActionKey>,
    pub(crate) downgrades: Vec<DemandLevelChangeAction>,
    pub(crate) upgrades: Vec<DemandLevelChangeAction>,
    pub(crate) spawns: Vec<DemandSpawnAction>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct DemandBuildingActionPlan {
    pub(crate) residential: DemandUseActionPlan,
    pub(crate) commercial: DemandUseActionPlan,
    pub(crate) industrial: DemandUseActionPlan,
}

impl DemandBuildingActionPlan {
    fn use_plan_mut(&mut self, use_kind: DemandUse) -> &mut DemandUseActionPlan {
        match use_kind {
            DemandUse::Residential => &mut self.residential,
            DemandUse::Commercial => &mut self.commercial,
            DemandUse::Industrial => &mut self.industrial,
        }
    }
}

#[derive(Clone, Debug)]
struct SignalNormalizationConfig {
    resident_presence_saturation_residents: u32,
    household_affordability_target_reserve_days: f32,
    household_stock_stability_target_days: f32,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
struct StartupSupportConfig {
    target_housed_residents: u32,
    target_private_building_count: u32,
    target_filled_job_slots: u32,
    household_bonus: f32,
    growth_floor_by_use: UseTuningF32,
    spawn_bonus_by_use: UseTuningF32,
}

#[derive(Clone, Debug)]
struct HouseholdActionConfig {
    base_inflow: f32,
    admission_threshold: f32,
    removal_threshold: f32,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
struct ActionBudgetConfig {
    max_households_per_day: u32,
    spawn_batch_fraction_by_use: UseTuningF32,
    upgrade_batch_fraction_by_use: UseTuningF32,
    downgrade_batch_fraction_by_use: UseTuningF32,
    despawn_batch_fraction_by_use: UseTuningF32,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
struct DemandConfig {
    signal_normalization: SignalNormalizationConfig,
    startup_support: StartupSupportConfig,
    household_action: HouseholdActionConfig,
    action_budget: ActionBudgetConfig,
    profiles: Vec<GrowthProfileRuntime>,
}

impl DemandConfig {
    fn profile_for_zone_density(
        &self,
        zone_type: ZoneType,
        density: &str,
    ) -> Option<&GrowthProfileRuntime> {
        let idx = match (zone_type, density) {
            (ZoneType::Residential, "low") => 0,
            (ZoneType::Residential, "medium") => 1,
            (ZoneType::Residential, "high") => 2,
            (ZoneType::Commercial, "low") => 3,
            (ZoneType::Commercial, "medium") => 4,
            (ZoneType::Commercial, "high") => 5,
            (ZoneType::Industrial, "low") => 6,
            (ZoneType::Industrial, "medium") => 7,
            (ZoneType::Industrial, "high") => 8,
            _ => return None,
        };
        self.profiles.get(idx)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredGrowthProfilesFile {
    signal_normalization: AuthoredSignalNormalization,
    startup_support: AuthoredStartupSupport,
    household_action: AuthoredHouseholdAction,
    action_budget: AuthoredActionBudget,
    profiles: Vec<AuthoredGrowthProfile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredSignalNormalization {
    resident_presence_saturation_residents: u32,
    household_affordability_target_reserve_days: f32,
    household_stock_stability_target_days: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredStartupSupport {
    target_housed_residents: u32,
    target_private_building_count: u32,
    target_filled_job_slots: u32,
    household_bonus: f32,
    growth_floor_by_use: AuthoredUseTuningF32,
    spawn_bonus_by_use: AuthoredUseTuningF32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredHouseholdAction {
    base_inflow: f32,
    admission_threshold: f32,
    removal_threshold: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredActionBudget {
    max_households_per_day: u32,
    spawn_batch_fraction_by_use: AuthoredUseTuningF32,
    upgrade_batch_fraction_by_use: AuthoredUseTuningF32,
    downgrade_batch_fraction_by_use: AuthoredUseTuningF32,
    despawn_batch_fraction_by_use: AuthoredUseTuningF32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredUseTuningF32 {
    residential: f32,
    commercial: f32,
    industrial: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredGrowthProfile {
    id: String,
    demand_channel: String,
    cadence_days: u32,
    base_pressure_weight: f32,
    local_modifier_scale: f32,
    spawn_threshold: f32,
    despawn_threshold: f32,
    upgrade_threshold: f32,
    downgrade_threshold: f32,
    hysteresis_margin: f32,
}

static BUILTIN_CONFIG: OnceLock<Result<DemandConfig, String>> = OnceLock::new();

/// Demand-owned daily growth state derived from the settled economy snapshot.
pub struct DemandSystem {
    config: Arc<DemandConfig>,
    pub(crate) residential: f32,
    pub(crate) commercial: f32,
    pub(crate) industrial: f32,
    pub(crate) households_to_admit_today: u32,
    pub(crate) households_to_remove_today: u32,
    pub(crate) startup_support_factor: f32,
    pub(crate) admission_action_credit: f32,
    pub(crate) removal_action_credit: f32,
    pub(crate) spawn_action_credit: UseTuningF32,
    pub(crate) upgrade_action_credit: UseTuningF32,
    pub(crate) downgrade_action_credit: UseTuningF32,
    pub(crate) despawn_action_credit: UseTuningF32,
    pub(crate) building_actions: DemandBuildingActionPlan,
}

impl DemandSystem {
    /// Creates a new demand system using the shipped demand tuning file.
    pub fn new() -> Self {
        let config = load_builtin_demand_config()
            .unwrap_or_else(|err| panic!("could not load built-in demand tuning: {err}"));
        Self {
            config,
            residential: 0.0,
            commercial: 0.0,
            industrial: 0.0,
            households_to_admit_today: 0,
            households_to_remove_today: 0,
            startup_support_factor: 0.0,
            admission_action_credit: 0.0,
            removal_action_credit: 0.0,
            spawn_action_credit: UseTuningF32::default(),
            upgrade_action_credit: UseTuningF32::default(),
            downgrade_action_credit: UseTuningF32::default(),
            despawn_action_credit: UseTuningF32::default(),
            building_actions: DemandBuildingActionPlan::default(),
        }
    }

    pub(crate) fn with_persisted_state(
        residential: f32,
        commercial: f32,
        industrial: f32,
        households_to_admit_today: u32,
        households_to_remove_today: u32,
        startup_support_factor: f32,
        admission_action_credit: f32,
        removal_action_credit: f32,
        spawn_action_credit: [f32; 3],
        upgrade_action_credit: [f32; 3],
        downgrade_action_credit: [f32; 3],
        despawn_action_credit: [f32; 3],
    ) -> Self {
        let mut system = Self::new();
        system.residential = residential;
        system.commercial = commercial;
        system.industrial = industrial;
        system.households_to_admit_today = households_to_admit_today;
        system.households_to_remove_today = households_to_remove_today;
        system.startup_support_factor = startup_support_factor;
        system.admission_action_credit = admission_action_credit;
        system.removal_action_credit = removal_action_credit;
        system.spawn_action_credit = UseTuningF32 {
            residential: spawn_action_credit[0],
            commercial: spawn_action_credit[1],
            industrial: spawn_action_credit[2],
        };
        system.upgrade_action_credit = UseTuningF32 {
            residential: upgrade_action_credit[0],
            commercial: upgrade_action_credit[1],
            industrial: upgrade_action_credit[2],
        };
        system.downgrade_action_credit = UseTuningF32 {
            residential: downgrade_action_credit[0],
            commercial: downgrade_action_credit[1],
            industrial: downgrade_action_credit[2],
        };
        system.despawn_action_credit = UseTuningF32 {
            residential: despawn_action_credit[0],
            commercial: despawn_action_credit[1],
            industrial: despawn_action_credit[2],
        };
        system
    }

    /// Rebuilds the daily city-growth outputs from the post-settlement snapshot.
    pub(crate) fn run_daily_pass(
        &mut self,
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        graph: &RegionGraph,
        zoning: &ZoningSystem,
    ) {
        self.building_actions = DemandBuildingActionPlan::default();
        let snapshot =
            DailyDemandSnapshot::from_runtime(allocator, households, graph, &self.config);

        let housing_shortage = 1.0 - snapshot.housing_availability;
        let goods_shortage = 1.0 - snapshot.household_stock_stability;
        let service_gate =
            snapshot.utility_service_stability * snapshot.external_connection_available;

        let base_residential = clamp01(
            housing_shortage
                * snapshot.job_availability
                * snapshot.household_affordability
                * service_gate,
        );
        let base_commercial = clamp01(
            snapshot.resident_presence
                * goods_shortage
                * snapshot.household_affordability
                * service_gate,
        );
        let base_industrial = clamp01(snapshot.resident_presence * goods_shortage * service_gate);

        let startup_progress = (ratio_u32(
            snapshot.housed_resident_count,
            self.config.startup_support.target_housed_residents,
        ) + ratio_u32(
            snapshot.existing_private_building_count,
            self.config.startup_support.target_private_building_count,
        ) + ratio_u32(
            snapshot.filled_job_slots,
            self.config.startup_support.target_filled_job_slots,
        )) / 3.0;

        self.startup_support_factor =
            clamp01(snapshot.external_connection_available * (1.0 - startup_progress));

        self.residential = base_residential.max(
            self.startup_support_factor
                * self
                    .config
                    .startup_support
                    .growth_floor_by_use
                    .get(DemandUse::Residential),
        );
        self.commercial = base_commercial.max(
            self.startup_support_factor
                * self
                    .config
                    .startup_support
                    .growth_floor_by_use
                    .get(DemandUse::Commercial),
        );
        self.industrial = base_industrial.max(
            self.startup_support_factor
                * self
                    .config
                    .startup_support
                    .growth_floor_by_use
                    .get(DemandUse::Industrial),
        );

        let city_stability_factor = snapshot
            .household_stock_stability
            .min(snapshot.utility_service_stability);
        let admission_pressure = clamp01(
            self.config.household_action.base_inflow
                * snapshot.external_connection_available
                * (1.0 + self.startup_support_factor * self.config.startup_support.household_bonus)
                * snapshot.housing_availability
                * snapshot.job_availability
                * city_stability_factor,
        );
        self.households_to_admit_today = advance_household_action_credit(
            &mut self.admission_action_credit,
            admission_pressure,
            self.config.household_action.admission_threshold,
            self.config.action_budget.max_households_per_day,
            snapshot.vacant_household_slots,
        );

        let job_failure = 1.0 - snapshot.job_availability;
        let city_instability = 1.0 - city_stability_factor;
        let removal_pressure = clamp01(
            snapshot.unhoused_household_ratio * 0.50 + job_failure * 0.25 + city_instability * 0.25,
        );
        self.households_to_remove_today = advance_household_action_credit(
            &mut self.removal_action_credit,
            removal_pressure,
            self.config.household_action.removal_threshold,
            self.config.action_budget.max_households_per_day,
            snapshot.total_household_count,
        );

        for use_kind in [
            DemandUse::Residential,
            DemandUse::Commercial,
            DemandUse::Industrial,
        ] {
            let zone_type = use_kind.zone_type();
            let growth_pressure = self.pressure_for_use(use_kind);
            let spawn_candidates =
                allocator.collect_demand_spawn_candidates(zone_type, zoning, graph);
            let existing_candidates =
                self.collect_existing_building_candidates(allocator, zone_type, growth_pressure);

            let normalized_spawn_pressure = spawn_candidates
                .iter()
                .filter_map(|candidate| {
                    self.config
                        .profile_for_zone_density(zone_type, &candidate.density)
                        .map(|profile| {
                            normalized_positive_pressure(growth_pressure, profile.spawn_threshold)
                        })
                })
                .sum::<f32>();
            let spawn_budget_units = normalized_spawn_pressure
                * self
                    .config
                    .action_budget
                    .spawn_batch_fraction_by_use
                    .get(use_kind)
                + self.startup_support_factor
                    * self.config.startup_support.spawn_bonus_by_use.get(use_kind);
            let spawns_today = advance_building_action_credit(
                self.spawn_action_credit.get_mut(use_kind),
                spawn_budget_units,
                spawn_candidates.len(),
            );
            let selected_spawns: Vec<_> = spawn_candidates
                .into_iter()
                .take(spawns_today)
                .map(|candidate| candidate.action)
                .collect();

            let normalized_upgrade_pressure = existing_candidates
                .upgrades
                .iter()
                .map(|candidate| candidate.normalized_action_pressure)
                .sum::<f32>();
            let upgrade_budget_units = normalized_upgrade_pressure
                * self
                    .config
                    .action_budget
                    .upgrade_batch_fraction_by_use
                    .get(use_kind);
            let upgrades_today = advance_building_action_credit(
                self.upgrade_action_credit.get_mut(use_kind),
                upgrade_budget_units,
                existing_candidates.upgrades.len(),
            );
            let selected_upgrades: Vec<_> = existing_candidates
                .upgrades
                .iter()
                .take(upgrades_today)
                .map(|candidate| candidate.action.clone())
                .collect();

            let normalized_downgrade_pressure = existing_candidates
                .downgrades
                .iter()
                .map(|candidate| candidate.normalized_action_pressure)
                .sum::<f32>();
            let downgrade_budget_units = normalized_downgrade_pressure
                * self
                    .config
                    .action_budget
                    .downgrade_batch_fraction_by_use
                    .get(use_kind);
            let downgrades_today = advance_building_action_credit(
                self.downgrade_action_credit.get_mut(use_kind),
                downgrade_budget_units,
                existing_candidates.downgrades.len(),
            );
            let selected_downgrades: Vec<_> = existing_candidates
                .downgrades
                .iter()
                .take(downgrades_today)
                .map(|candidate| candidate.action.clone())
                .collect();

            let normalized_despawn_pressure = existing_candidates
                .despawns
                .iter()
                .map(|candidate| candidate.normalized_action_pressure)
                .sum::<f32>();
            let despawn_budget_units = normalized_despawn_pressure
                * self
                    .config
                    .action_budget
                    .despawn_batch_fraction_by_use
                    .get(use_kind);
            let despawns_today = advance_building_action_credit(
                self.despawn_action_credit.get_mut(use_kind),
                despawn_budget_units,
                existing_candidates.despawns.len(),
            );
            let selected_despawns: Vec<_> = existing_candidates
                .despawns
                .iter()
                .take(despawns_today)
                .map(|candidate| candidate.action.clone())
                .collect();

            let plan = self.building_actions.use_plan_mut(use_kind);
            plan.spawns.extend(selected_spawns);
            plan.upgrades.extend(selected_upgrades);
            plan.downgrades.extend(selected_downgrades);
            plan.despawns.extend(selected_despawns);
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DemandSpawnCandidate {
    pub(crate) action: DemandSpawnAction,
    pub(crate) density: String,
}

#[derive(Clone, Debug)]
struct WeightedLevelChangeCandidate {
    action: DemandLevelChangeAction,
    normalized_action_pressure: f32,
}

#[derive(Clone, Debug)]
struct WeightedDespawnCandidate {
    action: DemandBuildingActionKey,
    normalized_action_pressure: f32,
}

#[derive(Clone, Debug, Default)]
struct ExistingBuildingCandidates {
    despawns: Vec<WeightedDespawnCandidate>,
    downgrades: Vec<WeightedLevelChangeCandidate>,
    upgrades: Vec<WeightedLevelChangeCandidate>,
}

impl DemandSystem {
    fn pressure_for_use(&self, use_kind: DemandUse) -> f32 {
        match use_kind {
            DemandUse::Residential => self.residential,
            DemandUse::Commercial => self.commercial,
            DemandUse::Industrial => self.industrial,
        }
    }

    fn collect_existing_building_candidates(
        &self,
        allocator: &BuildingAllocator,
        zone_type: ZoneType,
        growth_pressure: f32,
    ) -> ExistingBuildingCandidates {
        let mut building_indices: Vec<usize> = allocator
            .buildings
            .iter()
            .enumerate()
            .filter_map(|(idx, building)| (building.zone_type == zone_type).then_some(idx))
            .collect();
        building_indices.sort_by(|&a, &b| {
            let left = &allocator.buildings[a];
            let right = &allocator.buildings[b];
            attachment_sort_key(left).cmp(&attachment_sort_key(right))
        });

        let mut candidates = ExistingBuildingCandidates::default();
        for building_idx in building_indices {
            let building = &allocator.buildings[building_idx];
            if building.broken || building.pending_redevelopment {
                continue;
            }
            let Some(entry) = allocator.registry.get(&building.asset_id) else {
                continue;
            };
            let Some(asset_building) = entry.manifest.building.as_ref() else {
                continue;
            };
            if !asset_building.is_zoned_private() {
                continue;
            }
            let Some(density) = asset_building.density_key() else {
                continue;
            };
            let Some(profile) = self.config.profile_for_zone_density(zone_type, density) else {
                continue;
            };

            if building.occupancy == 0
                && building.worker_count == 0
                && normalized_negative_pressure(growth_pressure, profile.despawn_threshold) > 0.0
            {
                candidates.despawns.push(WeightedDespawnCandidate {
                    action: demand_building_action_key(building),
                    normalized_action_pressure: normalized_negative_pressure(
                        growth_pressure,
                        profile.despawn_threshold,
                    ),
                });
                continue;
            }

            if building.occupancy == 0
                && building.worker_count == 0
                && normalized_negative_pressure(growth_pressure, profile.downgrade_threshold) > 0.0
                && let Some(target_asset_id) = allocator.registry.prev_level(&building.asset_id)
                && level_change_is_compatible(allocator, building_idx, target_asset_id)
            {
                candidates.downgrades.push(WeightedLevelChangeCandidate {
                    action: DemandLevelChangeAction {
                        building: demand_building_action_key(building),
                        target_asset_id: target_asset_id.to_owned(),
                    },
                    normalized_action_pressure: normalized_negative_pressure(
                        growth_pressure,
                        profile.downgrade_threshold,
                    ),
                });
                continue;
            }

            let capacity_saturated = match zone_type {
                ZoneType::Residential => {
                    let resident_capacity = allocator.resident_capacity(building_idx);
                    resident_capacity > 0 && building.occupancy >= resident_capacity
                }
                ZoneType::Commercial | ZoneType::Industrial => {
                    let worker_capacity = allocator.worker_capacity(building_idx);
                    worker_capacity > 0
                        && building.worker_count >= worker_capacity
                        && building.utility_service_available
                }
                _ => false,
            };
            if capacity_saturated
                && normalized_positive_pressure(growth_pressure, profile.upgrade_threshold) > 0.0
                && let Some(target_asset_id) = allocator.registry.next_level(&building.asset_id)
                && level_change_is_compatible(allocator, building_idx, target_asset_id)
            {
                candidates.upgrades.push(WeightedLevelChangeCandidate {
                    action: DemandLevelChangeAction {
                        building: demand_building_action_key(building),
                        target_asset_id: target_asset_id.to_owned(),
                    },
                    normalized_action_pressure: normalized_positive_pressure(
                        growth_pressure,
                        profile.upgrade_threshold,
                    ),
                });
            }
        }

        candidates
    }
}

fn attachment_sort_key(
    building: &crate::simulation::buildings::allocator::Building,
) -> (usize, u8, usize, u16, u16, u8, &str) {
    (
        building.edge_idx,
        if building.side > 0 { 0 } else { 1 },
        building.cell_x,
        building.width_cells,
        building.depth_cells,
        building.level,
        building.asset_id.as_str(),
    )
}

fn demand_building_action_key(
    building: &crate::simulation::buildings::allocator::Building,
) -> DemandBuildingActionKey {
    DemandBuildingActionKey {
        edge_idx: building.edge_idx,
        side: building.side,
        cell_x: building.cell_x,
        width_cells: building.width_cells,
        depth_cells: building.depth_cells,
        level: building.level,
        asset_id: building.asset_id.clone(),
    }
}

fn level_change_is_compatible(
    allocator: &BuildingAllocator,
    building_idx: usize,
    target_asset_id: &str,
) -> bool {
    let Some(building) = allocator.buildings.get(building_idx) else {
        return false;
    };
    let Some(target_entry) = allocator.registry.get(target_asset_id) else {
        return false;
    };
    let Some(target_building) = target_entry.manifest.building.as_ref() else {
        return false;
    };
    if !target_building.is_zoned_private() {
        return false;
    }
    if target_building.lot_width_cells != building.width_cells
        || target_building.lot_depth_cells != building.depth_cells
    {
        return false;
    }
    allocator.registry.resident_capacity(target_asset_id) >= building.occupancy
        && allocator.registry.worker_capacity(target_asset_id) >= building.worker_count
}

fn load_builtin_demand_config() -> Result<Arc<DemandConfig>, String> {
    match BUILTIN_CONFIG.get_or_init(load_config_from_disk) {
        Ok(config) => Ok(Arc::new(config.clone())),
        Err(err) => Err(err.clone()),
    }
}

fn load_config_from_disk() -> Result<DemandConfig, String> {
    let path = repo_relative_path(GROWTH_PROFILES_FILE);
    let content = std::fs::read_to_string(&path)
        .map_err(|err| format!("could not read '{}': {err}", path.display()))?;
    let authored: AuthoredGrowthProfilesFile = toml::from_str(&content)
        .map_err(|err| format!("could not parse '{}': {err}", path.display()))?;
    compile_config(authored)
}

fn repo_relative_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(relative)
}

fn compile_config(authored: AuthoredGrowthProfilesFile) -> Result<DemandConfig, String> {
    validate_positive_u32(
        authored
            .signal_normalization
            .resident_presence_saturation_residents,
        "signal_normalization.resident_presence_saturation_residents",
    )?;
    validate_positive_f32(
        authored
            .signal_normalization
            .household_affordability_target_reserve_days,
        "signal_normalization.household_affordability_target_reserve_days",
    )?;
    validate_positive_f32(
        authored
            .signal_normalization
            .household_stock_stability_target_days,
        "signal_normalization.household_stock_stability_target_days",
    )?;

    validate_positive_u32(
        authored.startup_support.target_housed_residents,
        "startup_support.target_housed_residents",
    )?;
    validate_positive_u32(
        authored.startup_support.target_private_building_count,
        "startup_support.target_private_building_count",
    )?;
    validate_positive_u32(
        authored.startup_support.target_filled_job_slots,
        "startup_support.target_filled_job_slots",
    )?;
    validate_range_f32(
        authored.startup_support.household_bonus,
        0.0,
        f32::INFINITY,
        "startup_support.household_bonus",
    )?;
    let growth_floor_by_use = validate_use_tuning(
        authored.startup_support.growth_floor_by_use,
        0.0,
        1.0,
        "startup_support.growth_floor_by_use",
    )?;
    let spawn_bonus_by_use = validate_use_tuning(
        authored.startup_support.spawn_bonus_by_use,
        0.0,
        f32::INFINITY,
        "startup_support.spawn_bonus_by_use",
    )?;

    validate_range_f32(
        authored.household_action.base_inflow,
        0.0,
        1.0,
        "household_action.base_inflow",
    )?;
    validate_range_f32(
        authored.household_action.admission_threshold,
        0.0,
        1.0,
        "household_action.admission_threshold",
    )?;
    validate_range_f32(
        authored.household_action.removal_threshold,
        0.0,
        1.0,
        "household_action.removal_threshold",
    )?;

    let spawn_batch_fraction_by_use = validate_use_tuning(
        authored.action_budget.spawn_batch_fraction_by_use,
        0.0,
        1.0,
        "action_budget.spawn_batch_fraction_by_use",
    )?;
    let upgrade_batch_fraction_by_use = validate_use_tuning(
        authored.action_budget.upgrade_batch_fraction_by_use,
        0.0,
        1.0,
        "action_budget.upgrade_batch_fraction_by_use",
    )?;
    let downgrade_batch_fraction_by_use = validate_use_tuning(
        authored.action_budget.downgrade_batch_fraction_by_use,
        0.0,
        1.0,
        "action_budget.downgrade_batch_fraction_by_use",
    )?;
    let despawn_batch_fraction_by_use = validate_use_tuning(
        authored.action_budget.despawn_batch_fraction_by_use,
        0.0,
        1.0,
        "action_budget.despawn_batch_fraction_by_use",
    )?;

    let mut by_id = std::collections::HashMap::new();
    for profile in authored.profiles {
        if by_id.contains_key(&profile.id) {
            return Err(format!("duplicate GrowthProfile id '{}'", profile.id));
        }
        let Some(demand_channel) = DemandChannel::from_str_name(&profile.demand_channel) else {
            return Err(format!(
                "unknown demand_channel '{}' for GrowthProfile '{}'",
                profile.demand_channel, profile.id
            ));
        };
        validate_positive_u32(
            profile.cadence_days,
            &format!("profiles.{}.cadence_days", profile.id),
        )?;
        validate_range_f32(
            profile.base_pressure_weight,
            0.0,
            1.0,
            &format!("profiles.{}.base_pressure_weight", profile.id),
        )?;
        validate_range_f32(
            profile.local_modifier_scale,
            0.0,
            1.0,
            &format!("profiles.{}.local_modifier_scale", profile.id),
        )?;
        validate_range_f32(
            profile.spawn_threshold,
            0.0,
            1.0,
            &format!("profiles.{}.spawn_threshold", profile.id),
        )?;
        validate_range_f32(
            profile.despawn_threshold,
            0.0,
            1.0,
            &format!("profiles.{}.despawn_threshold", profile.id),
        )?;
        validate_range_f32(
            profile.upgrade_threshold,
            0.0,
            1.0,
            &format!("profiles.{}.upgrade_threshold", profile.id),
        )?;
        validate_range_f32(
            profile.downgrade_threshold,
            0.0,
            1.0,
            &format!("profiles.{}.downgrade_threshold", profile.id),
        )?;
        validate_range_f32(
            profile.hysteresis_margin,
            0.0,
            1.0,
            &format!("profiles.{}.hysteresis_margin", profile.id),
        )?;
        if profile.upgrade_threshold < profile.downgrade_threshold {
            return Err(format!(
                "profiles.{}.upgrade_threshold must be >= downgrade_threshold",
                profile.id
            ));
        }
        if profile.local_modifier_scale != 0.0 {
            return Err(format!(
                "shipped base-game profile '{}' must set local_modifier_scale = 0.0",
                profile.id
            ));
        }
        by_id.insert(
            profile.id.clone(),
            GrowthProfileRuntime {
                id: profile.id,
                demand_channel,
                cadence_days: profile.cadence_days,
                base_pressure_weight: profile.base_pressure_weight,
                local_modifier_scale: profile.local_modifier_scale,
                spawn_threshold: profile.spawn_threshold,
                despawn_threshold: profile.despawn_threshold,
                upgrade_threshold: profile.upgrade_threshold,
                downgrade_threshold: profile.downgrade_threshold,
                hysteresis_margin: profile.hysteresis_margin,
            },
        );
    }

    if by_id.len() != SHIPPED_PROFILE_ORDER.len() {
        return Err(format!(
            "expected {} shipped GrowthProfiles, found {}",
            SHIPPED_PROFILE_ORDER.len(),
            by_id.len()
        ));
    }

    let mut profiles = Vec::with_capacity(SHIPPED_PROFILE_ORDER.len());
    for (id, expected_channel) in SHIPPED_PROFILE_ORDER {
        let Some(profile) = by_id.remove(id) else {
            return Err(format!("missing shipped GrowthProfile '{}'", id));
        };
        if profile.demand_channel != expected_channel {
            return Err(format!(
                "GrowthProfile '{}' must use demand_channel {:?}",
                id, expected_channel
            ));
        }
        profiles.push(profile);
    }
    if let Some(extra_id) = by_id.keys().next() {
        return Err(format!(
            "unexpected extra shipped GrowthProfile '{}'",
            extra_id
        ));
    }

    Ok(DemandConfig {
        signal_normalization: SignalNormalizationConfig {
            resident_presence_saturation_residents: authored
                .signal_normalization
                .resident_presence_saturation_residents,
            household_affordability_target_reserve_days: authored
                .signal_normalization
                .household_affordability_target_reserve_days,
            household_stock_stability_target_days: authored
                .signal_normalization
                .household_stock_stability_target_days,
        },
        startup_support: StartupSupportConfig {
            target_housed_residents: authored.startup_support.target_housed_residents,
            target_private_building_count: authored.startup_support.target_private_building_count,
            target_filled_job_slots: authored.startup_support.target_filled_job_slots,
            household_bonus: authored.startup_support.household_bonus,
            growth_floor_by_use,
            spawn_bonus_by_use,
        },
        household_action: HouseholdActionConfig {
            base_inflow: authored.household_action.base_inflow,
            admission_threshold: authored.household_action.admission_threshold,
            removal_threshold: authored.household_action.removal_threshold,
        },
        action_budget: ActionBudgetConfig {
            max_households_per_day: authored.action_budget.max_households_per_day,
            spawn_batch_fraction_by_use,
            upgrade_batch_fraction_by_use,
            downgrade_batch_fraction_by_use,
            despawn_batch_fraction_by_use,
        },
        profiles,
    })
}

fn validate_use_tuning(
    authored: AuthoredUseTuningF32,
    min_value: f32,
    max_value: f32,
    label: &str,
) -> Result<UseTuningF32, String> {
    validate_range_f32(
        authored.residential,
        min_value,
        max_value,
        &format!("{label}.residential"),
    )?;
    validate_range_f32(
        authored.commercial,
        min_value,
        max_value,
        &format!("{label}.commercial"),
    )?;
    validate_range_f32(
        authored.industrial,
        min_value,
        max_value,
        &format!("{label}.industrial"),
    )?;
    Ok(UseTuningF32 {
        residential: authored.residential,
        commercial: authored.commercial,
        industrial: authored.industrial,
    })
}

fn validate_positive_u32(value: u32, label: &str) -> Result<(), String> {
    if value == 0 {
        Err(format!("{label} must be > 0"))
    } else {
        Ok(())
    }
}

fn validate_positive_f32(value: f32, label: &str) -> Result<(), String> {
    validate_range_f32(value, EPSILON, f32::INFINITY, label)
}

fn validate_range_f32(
    value: f32,
    min_value: f32,
    max_value: f32,
    label: &str,
) -> Result<(), String> {
    if !value.is_finite() || value < min_value || value > max_value {
        Err(format!(
            "{label} must be finite and in [{}..={}]",
            min_value, max_value
        ))
    } else {
        Ok(())
    }
}

fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

fn ratio_u32(value: u32, target: u32) -> f32 {
    if target == 0 {
        0.0
    } else {
        clamp01(value as f32 / target as f32)
    }
}

fn advance_household_action_credit(
    credit: &mut f32,
    pressure: f32,
    threshold: f32,
    max_households_per_day: u32,
    max_actionable_households: u32,
) -> u32 {
    let normalized_action_pressure = if threshold >= 1.0 {
        0.0
    } else {
        clamp01((pressure - threshold) / (1.0 - threshold))
    };
    *credit += normalized_action_pressure * max_households_per_day as f32;
    let households_to_act = (*credit).floor().max(0.0) as u32;
    *credit -= households_to_act as f32;
    households_to_act
        .min(max_households_per_day)
        .min(max_actionable_households)
}

fn advance_building_action_credit(
    credit: &mut f32,
    budget_units: f32,
    max_actionable_buildings: usize,
) -> usize {
    *credit += budget_units.max(0.0);
    let buildings_to_act = (*credit).floor().max(0.0) as usize;
    *credit -= buildings_to_act as f32;
    buildings_to_act.min(max_actionable_buildings)
}

fn normalized_positive_pressure(pressure: f32, threshold: f32) -> f32 {
    if threshold >= 1.0 {
        0.0
    } else {
        clamp01((pressure - threshold) / (1.0 - threshold))
    }
}

fn normalized_negative_pressure(pressure: f32, threshold: f32) -> f32 {
    if threshold <= 0.0 {
        0.0
    } else {
        clamp01((threshold - pressure) / threshold.max(EPSILON))
    }
}

struct DailyDemandSnapshot {
    vacant_household_slots: u32,
    housed_resident_count: u32,
    total_household_count: u32,
    unhoused_household_ratio: f32,
    existing_private_building_count: u32,
    filled_job_slots: u32,
    housing_availability: f32,
    resident_presence: f32,
    job_availability: f32,
    household_affordability: f32,
    household_stock_stability: f32,
    utility_service_stability: f32,
    external_connection_available: f32,
}

impl DailyDemandSnapshot {
    fn from_runtime(
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        graph: &RegionGraph,
        config: &DemandConfig,
    ) -> Self {
        let mut total_household_slots = 0_u32;
        let mut occupied_household_slots = 0_u32;
        let mut occupied_reachable_job_slots = 0_u32;
        let mut open_reachable_job_slots = 0_u32;
        let mut filled_job_slots = 0_u32;
        let mut existing_private_building_count = 0_u32;
        let mut utility_service_consumer_count = 0_u32;
        let mut utility_service_satisfied_count = 0_u32;

        for (idx, building) in allocator.buildings.iter().enumerate() {
            if building.broken {
                continue;
            }

            let is_private_building = allocator
                .registry
                .get(&building.asset_id)
                .and_then(|entry| entry.manifest.building.as_ref())
                .map(|authored| authored.is_zoned_private())
                .unwrap_or(!matches!(building.zone_type, ZoneType::None));
            if is_private_building {
                existing_private_building_count = existing_private_building_count.saturating_add(1);
            }

            if matches!(building.zone_type, ZoneType::Residential | ZoneType::Mixed) {
                let resident_capacity = allocator.resident_capacity(idx);
                total_household_slots = total_household_slots.saturating_add(resident_capacity);
                occupied_household_slots = occupied_household_slots
                    .saturating_add(building.occupancy.min(resident_capacity));
            }

            if matches!(
                building.zone_type,
                ZoneType::Commercial | ZoneType::Industrial | ZoneType::Office | ZoneType::Mixed
            ) {
                let worker_capacity = allocator.worker_capacity(idx);
                let occupied = building.worker_count.min(worker_capacity);
                occupied_reachable_job_slots =
                    occupied_reachable_job_slots.saturating_add(occupied);
                open_reachable_job_slots = open_reachable_job_slots
                    .saturating_add(worker_capacity.saturating_sub(occupied));
                filled_job_slots = filled_job_slots.saturating_add(occupied);
                utility_service_consumer_count = utility_service_consumer_count.saturating_add(1);
                if building.utility_service_available {
                    utility_service_satisfied_count =
                        utility_service_satisfied_count.saturating_add(1);
                }
            }
        }

        let vacant_household_slots = total_household_slots.saturating_sub(occupied_household_slots);
        let total_reachable_job_slots =
            occupied_reachable_job_slots.saturating_add(open_reachable_job_slots);

        let mut housed_resident_count = 0_u32;
        let mut housed_household_count = 0_u32;
        let mut unhoused_household_count = 0_u32;
        let mut household_affordability_sum = 0.0;
        let mut household_stock_stability_sum = 0.0;

        for household in &households.households {
            if household.member_count == 0 {
                continue;
            }
            let is_housed = household.home_building_id < allocator.buildings.len()
                && !allocator.buildings[household.home_building_id].broken;
            if is_housed {
                housed_household_count = housed_household_count.saturating_add(1);
                housed_resident_count =
                    housed_resident_count.saturating_add(household.member_count as u32);
                household_affordability_sum += clamp01(
                    household_reserve_days(household)
                        / config
                            .signal_normalization
                            .household_affordability_target_reserve_days,
                );
                household_stock_stability_sum += clamp01(
                    household.stock_days
                        / config
                            .signal_normalization
                            .household_stock_stability_target_days,
                );
            } else {
                unhoused_household_count = unhoused_household_count.saturating_add(1);
            }
        }

        let total_household_count = housed_household_count.saturating_add(unhoused_household_count);
        let housing_availability = if total_household_slots == 0 {
            0.0
        } else {
            clamp01(vacant_household_slots as f32 / total_household_slots as f32)
        };
        let resident_presence = clamp01(
            housed_resident_count as f32
                / config
                    .signal_normalization
                    .resident_presence_saturation_residents as f32,
        );
        let job_availability = if total_reachable_job_slots == 0 {
            0.0
        } else {
            clamp01(open_reachable_job_slots as f32 / total_reachable_job_slots as f32)
        };
        let household_affordability = if housed_household_count == 0 {
            1.0
        } else {
            clamp01(household_affordability_sum / housed_household_count as f32)
        };
        let household_stock_stability = if housed_household_count == 0 {
            1.0
        } else {
            clamp01(household_stock_stability_sum / housed_household_count as f32)
        };
        let utility_service_stability = if utility_service_consumer_count == 0 {
            1.0
        } else {
            clamp01(utility_service_satisfied_count as f32 / utility_service_consumer_count as f32)
        };
        let connected_border_count = graph
            .nodes()
            .iter()
            .enumerate()
            .filter(|(idx, node)| {
                node.node_type == NodeType::Border
                    && graph.node_adjacency(*idx as u32).iter().any(|&edge_idx| {
                        let edge = graph.edge(edge_idx);
                        !edge.deleted
                            && edge.primary_type == TransitType::Road
                            && (edge.allowed_types & TransitFlags::CAR) != 0
                    })
            })
            .count() as u32;
        let external_connection_available = if connected_border_count > 0 { 1.0 } else { 0.0 };
        let unhoused_household_ratio = if total_household_count == 0 {
            0.0
        } else {
            clamp01(unhoused_household_count as f32 / total_household_count as f32)
        };

        Self {
            vacant_household_slots,
            housed_resident_count,
            total_household_count,
            unhoused_household_ratio,
            existing_private_building_count,
            filled_job_slots,
            housing_availability,
            resident_presence,
            job_availability,
            household_affordability,
            household_stock_stability,
            utility_service_stability,
            external_connection_available,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::AssetManifest;
    use crate::assets::asset::{
        Anchor, AnchorType, BuildingData, LodEntry, PlacementMode, ZoneClass,
    };
    use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
    use crate::simulation::core::config::MapConfig;
    use crate::simulation::economy::households::{
        Household, HouseholdSystem, REPLENISHMENT_STABLE,
    };
    use crate::simulation::grid::zoning::ZoningSystem;
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::types::{
        EdgeClass, TransitFlags, TransitType, VehicleFrontageAccess,
    };
    use godot::prelude::{Vector2, Vector3};

    fn register_test_asset(
        allocator: &mut BuildingAllocator,
        asset_id: &str,
        zone_type: ZoneType,
    ) -> String {
        let (zone_class, residents_capacity, worker_capacity) = match zone_type {
            ZoneType::Residential => (ZoneClass::Residential, Some(6), None),
            ZoneType::Commercial => (ZoneClass::Commercial, None, Some(4)),
            ZoneType::Industrial => (ZoneClass::Industrial, None, Some(4)),
            ZoneType::Office => (ZoneClass::Office, None, Some(4)),
            ZoneType::Mixed => (ZoneClass::Mixed, Some(4), Some(2)),
            ZoneType::None => panic!("test assets must use a real zone type"),
        };
        let manifest = AssetManifest {
            asset_id: asset_id.to_owned(),
            display_name: "Test".to_owned(),
            asset_set: None,
            tags: vec![],
            thumbnail: None,
            lods: vec![LodEntry {
                file: "lod0.glb".to_owned(),
                distance_min_m: 0.0,
                distance_max_m: None,
            }],
            anchors: vec![Anchor {
                anchor_type: AnchorType::Entrance,
                name: "main".to_owned(),
                position: [0.0, 0.0, 0.5],
                forward: [0.0, 0.0, 1.0],
            }],
            building: Some(BuildingData {
                placement_mode: PlacementMode::ZonedPrivate,
                zone_type: Some(zone_class),
                density: Some("low".to_owned()),
                lot_width_cells: 2,
                lot_depth_cells: 2,
                min_zone_width_cells: None,
                min_zone_depth_cells: None,
                level: 1,
                residents_capacity,
                worker_capacity,
                service_class: None,
                economy_profile: None,
                preview_scale: Some(1.0),
            }),
            prop: None,
            vehicle: None,
            character: None,
            pivot_offset: None,
        };
        allocator.registry.register("test", manifest, String::new());
        format!("test:{asset_id}")
    }

    fn building(
        zone_type: ZoneType,
        stock: f32,
        occupancy: u32,
        worker_count: u32,
        asset_id: String,
    ) -> Building {
        Building {
            center_x: 0.0,
            center_y: 0.0,
            width_cells: 2,
            depth_cells: 2,
            zone_type,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.5,
            side_offset: 1.0,
            abandoned_timer: 0,
            edge_idx: 0,
            side: 1,
            cell_x: 0,
            cell_y: 0,
            occupancy,
            worker_count,
            asset_id,
            level: 1,
            broken: false,
            stock,
            revenue: 0.0,
            operating_budget: 500.0,
            utility_service_available: true,
            shipment_cooldown_days: 0,
            pending_redevelopment: false,
            rezone_grace_days_remaining: 0,
        }
    }

    fn housed_household(
        home_building_id: usize,
        member_count: u16,
        budget: f32,
        stock_days: f32,
    ) -> Household {
        Household {
            home_building_id,
            budget,
            stock: stock_days * member_count as f32,
            member_count,
            consumption_rate: 1.0,
            stock_days,
            replenishment_state: REPLENISHMENT_STABLE,
            cooldown_days: 0,
            reserved_store_building_id: usize::MAX,
            reserved_amount: 0.0,
            reserved_total_cost: 0.0,
            pickup_eta_days: 0,
        }
    }

    fn graph_with_connected_border() -> RegionGraph {
        let mut graph = RegionGraph::new();
        let border = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Border);
        let junction = graph.add_node(Vector3::new(50.0, 0.0, 0.0), NodeType::Junction);
        graph.add_edge(Edge {
            start_node: border,
            end_node: junction,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 50.0,
            physical_length: 50.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(50.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(50.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        });
        graph
    }

    fn empty_zoning() -> ZoningSystem {
        ZoningSystem::new(&MapConfig::default())
    }

    #[test]
    fn daily_pass_raises_commercial_and_industrial_pressure_on_shortages() {
        let mut allocator = BuildingAllocator::new();
        let industrial_asset =
            register_test_asset(&mut allocator, "industrial", ZoneType::Industrial);
        let commercial_asset =
            register_test_asset(&mut allocator, "commercial", ZoneType::Commercial);
        let residential_asset =
            register_test_asset(&mut allocator, "residential", ZoneType::Residential);
        allocator
            .buildings
            .push(building(ZoneType::Industrial, 20.0, 0, 1, industrial_asset));
        allocator
            .buildings
            .push(building(ZoneType::Commercial, 80.0, 0, 1, commercial_asset));
        allocator.buildings.push(building(
            ZoneType::Residential,
            0.0,
            0,
            0,
            residential_asset,
        ));

        let mut households = HouseholdSystem::new();
        households
            .households
            .push(housed_household(2, 2, 120.0, 0.25));

        let graph = graph_with_connected_border();
        let zoning = empty_zoning();
        let mut demand = DemandSystem::new();
        demand.run_daily_pass(&allocator, &households, &graph, &zoning);

        assert!(demand.commercial > 0.60);
        assert!(demand.industrial > 0.60);
    }

    #[test]
    fn daily_pass_raises_residential_pressure_when_jobs_outrun_housing() {
        let mut allocator = BuildingAllocator::new();
        let industrial_asset =
            register_test_asset(&mut allocator, "industrial", ZoneType::Industrial);
        let commercial_asset =
            register_test_asset(&mut allocator, "commercial", ZoneType::Commercial);
        let residential_asset =
            register_test_asset(&mut allocator, "residential", ZoneType::Residential);
        allocator.buildings.push(building(
            ZoneType::Industrial,
            300.0,
            0,
            1,
            industrial_asset,
        ));
        allocator.buildings.push(building(
            ZoneType::Commercial,
            500.0,
            0,
            1,
            commercial_asset,
        ));
        allocator.buildings.push(building(
            ZoneType::Residential,
            0.0,
            5,
            0,
            residential_asset,
        ));

        let mut households = HouseholdSystem::new();
        for _ in 0..5 {
            households
                .households
                .push(housed_household(2, 1, 120.0, 3.0));
        }

        let graph = graph_with_connected_border();
        let zoning = empty_zoning();
        let mut demand = DemandSystem::new();
        demand.run_daily_pass(&allocator, &households, &graph, &zoning);

        assert!(demand.residential > 0.50);
    }

    #[test]
    fn daily_pass_blocks_growth_without_external_connection() {
        let allocator = BuildingAllocator::new();
        let households = HouseholdSystem::new();
        let graph = RegionGraph::new();
        let zoning = empty_zoning();
        let mut demand = DemandSystem::new();

        demand.run_daily_pass(&allocator, &households, &graph, &zoning);

        assert_eq!(demand.residential, 0.0);
        assert_eq!(demand.commercial, 0.0);
        assert_eq!(demand.industrial, 0.0);
        assert_eq!(demand.households_to_admit_today, 0);
    }

    #[test]
    fn daily_pass_produces_startup_household_admission_when_capacity_jobs_and_border_exist() {
        let mut allocator = BuildingAllocator::new();
        let residential_asset =
            register_test_asset(&mut allocator, "residential", ZoneType::Residential);
        let commercial_asset =
            register_test_asset(&mut allocator, "commercial", ZoneType::Commercial);
        allocator.buildings.push(building(
            ZoneType::Residential,
            0.0,
            0,
            0,
            residential_asset,
        ));
        allocator.buildings.push(building(
            ZoneType::Commercial,
            500.0,
            0,
            0,
            commercial_asset,
        ));

        let households = HouseholdSystem::new();
        let graph = graph_with_connected_border();
        let zoning = empty_zoning();
        let mut demand = DemandSystem::new();

        demand.run_daily_pass(&allocator, &households, &graph, &zoning);

        assert!(demand.households_to_admit_today > 0);
        assert!(demand.startup_support_factor > 0.0);
    }

    #[test]
    fn snapshot_uses_settled_building_utility_availability() {
        let mut allocator = BuildingAllocator::new();
        let commercial_asset =
            register_test_asset(&mut allocator, "commercial", ZoneType::Commercial);
        let industrial_asset =
            register_test_asset(&mut allocator, "industrial", ZoneType::Industrial);
        allocator
            .buildings
            .push(building(ZoneType::Commercial, 40.0, 0, 1, commercial_asset));
        allocator
            .buildings
            .push(building(ZoneType::Industrial, 40.0, 0, 1, industrial_asset));
        allocator.buildings[1].utility_service_available = false;

        let households = HouseholdSystem::new();
        let graph = graph_with_connected_border();
        let config = load_builtin_demand_config().expect("built-in demand config must load");

        let snapshot = DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config);

        assert!((snapshot.utility_service_stability - 0.5).abs() < f32::EPSILON);
    }
}
