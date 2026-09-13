// SPDX-License-Identifier: GPL-2.0-only

//! Authored economy definitions used by the developer-facing economy editor.
//!
//! The runtime household/building simulation still owns live economic state, but
//! this module defines the authoritative TOML-backed profile/controller/scenario
//! data used by the economy editor, asset bindings, and compiled runtime catalog.

mod api;
mod io;
mod runtime;
mod runtime_compile;
mod runtime_loader;
mod sandbox;
mod scenario_graph;
mod schema;
mod serde_helpers;
#[cfg(test)]
mod tests;
mod validation;

pub use api::{export_project_json, load_project_json, run_sandbox_json};
pub(crate) use runtime::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, FreightTimingProfile, HouseholdRuntimeTuning,
    LogisticsRuntimeTuning, MinuteWindow, OperationalClockRuntimeTuning, ResourceRuntimeId,
    RuntimeEconomyCatalog, RuntimeEconomyTuning, RuntimeResourcePort, WorkTimingProfile,
};
pub(crate) use runtime_loader::{load_runtime_economy_catalog, load_runtime_economy_tuning};

/// Required demand-sink profile that owns household supply consumption and stock targets.
pub(crate) const HOUSEHOLD_DEMAND_PROFILE_ID: &str = "basic_household_demand";
/// Required physical resource used by household shopping and consumption.
pub(crate) const HOUSEHOLD_SUPPLY_RESOURCE_ID: &str = "household_supplies";
