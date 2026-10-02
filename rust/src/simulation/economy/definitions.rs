// SPDX-License-Identifier: GPL-2.0-only

//! Authored economy definitions compiled into the runtime catalog.
//!
//! The runtime household/building simulation owns live economic state. This module loads the
//! hand-authored `economy/profiles.toml`, validates it, and compiles the runtime catalog and
//! tuning used by asset bindings and simulation.

mod io;
mod runtime;
mod runtime_compile;
mod runtime_loader;
mod schema;
mod serde_helpers;
#[cfg(test)]
mod tests;
mod validation;

pub(crate) use runtime::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, FreightTimingProfile, HouseholdRuntimeTuning,
    LogisticsRuntimeTuning, MinuteWindow, OperationalClockRuntimeTuning, ResourceRuntimeId,
    RuntimeEconomyCatalog, RuntimeEconomyTuning, RuntimeResourcePort, WorkTimingProfile,
};
pub(crate) use runtime_loader::{load_runtime_economy_catalog, load_runtime_economy_tuning};
pub(crate) use serde_helpers::deserialize_unsigned_from_number;

/// Required demand-sink profile that owns household supply consumption and stock targets.
pub(crate) const HOUSEHOLD_DEMAND_PROFILE_ID: &str = "basic_household_demand";
/// Required physical resource used by household shopping and consumption.
pub(crate) const HOUSEHOLD_SUPPLY_RESOURCE_ID: &str = "household_supplies";
