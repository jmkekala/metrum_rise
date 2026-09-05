// SPDX-License-Identifier: GPL-2.0-only

//! Authored economy definitions used by the developer-facing economy editor.
//!
//! The runtime household/building simulation still owns live economic state, but
//! this module defines the authoritative TOML-backed profile/controller/scenario
//! data used to validate and tune the first-pass economy chains. The same data
//! will later feed the asset editor and a fuller compiled runtime representation.

mod api;
mod index;
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
