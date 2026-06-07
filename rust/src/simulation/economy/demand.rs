//! Demand-driven daily growth pass built from authored baseline tuning.

mod actions;
mod building_actions;
mod config;
mod credits;
mod diagnostics;
mod pressure;
mod snapshot;
mod spawn_need;
mod system;
#[cfg(test)]
mod tests;
mod types;
mod viability;

pub(crate) use actions::{
    DemandBuildingActionKey, DemandBuildingActionPlan, DemandLevelChangeAction, DemandSpawnAction,
    DemandSpawnCandidate, DemandSpawnCandidatesByUse, demand_building_action_key,
};
pub use system::DemandSystem;
pub(crate) use types::{UseTuningBool, UseTuningF32};

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
use spawn_need::{
    commercial_spawn_need_buildings, industrial_spawn_need_buildings,
    residential_spawn_need_buildings,
};
