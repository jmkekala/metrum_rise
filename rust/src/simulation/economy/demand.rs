// SPDX-License-Identifier: GPL-2.0-only

//! Demand-driven daily growth pass built from authored baseline tuning.

mod actions;
mod admission;
mod building_actions;
mod config;
mod credits;
mod diagnostics;
mod display;
mod execution;
mod queued;
mod setup;
mod snapshot;
mod spawn_need;
mod supply;
mod system;
#[cfg(test)]
mod tests;
mod types;
mod viability;
mod workforce;

#[cfg(test)]
pub(crate) use actions::demand_building_action_key;
pub(crate) use actions::{
    DemandBuildingActionKey, DemandBuildingActionPlan, DemandLevelChangeAction, DemandSpawnAction,
    DemandSpawnCandidate, DemandSpawnCandidatesByUse, DemandUseActionPlan, PlanningSlot,
    SpawnSiteAvailability, private_zone_index,
};
pub(crate) use diagnostics::{
    BuildingActionDiagnosticsByUse, HouseholdAdmissionDiagnostics, HouseholdRemovalDiagnostics,
};
pub(crate) use execution::{
    ConstructionPermit, DemandLevelChangeGuard, LevelChangeRefusal, QueuedLevelChange, SpawnPermit,
    StartRefusal,
};
#[cfg(test)]
pub(crate) use execution::{door_digest, level_change_permit};
pub(crate) use snapshot::SavedDemandSnapshot;
#[cfg(test)]
pub(crate) use supply::ProjectProof;
pub use system::DemandSystem;
pub(crate) use system::PlanningPreparation;
#[cfg(test)]
pub(crate) use system::VERIFY_LIVE_PASSES;
pub(crate) use types::UseTuningF32;

#[cfg(test)]
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
#[cfg(test)]
use crate::simulation::network::types::NodeType;
#[cfg(test)]
use crate::simulation::zoning::ZoneType;
#[cfg(test)]
use config::load_builtin_demand_config;
#[cfg(test)]
use snapshot::{DailyDemandSnapshot, ResidentialOccupantSnapshot};
#[cfg(test)]
use spawn_need::{residential_missing_household_slots, select_residential_spawns};
