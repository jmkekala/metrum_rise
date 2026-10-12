// SPDX-License-Identifier: GPL-2.0-only

//! Economic simulation foundations: explicit households, agent movement, and
//! live economy-pressure counters.
//!
//! [`agents::AgentSystem`] owns movement and transit state in Structure-of-Arrays
//! form. [`households::HouseholdSystem`] owns the first-pass building-centric
//! household economy loop with explicit household budgets, replenishment, and
//! work/home planning. [`logistics::ShipmentSystem`] owns batched building-level
//! freight reservations and delayed deliveries between suppliers, stores, and
//! `OWA` border terminals.
//!
//! [`demand::DemandSystem`] owns growth pressure, household admission and removal,
//! and private building selection from eligible zoned land and authored assets.

pub(crate) mod accessibility;
pub mod agents;
pub(crate) mod customers;
pub mod definitions;
pub mod demand;
pub mod electricity;
pub(crate) mod fiscal;
pub(crate) mod hiring;
pub mod households;
pub mod logistics;
pub(crate) mod operation;
mod reduction;
mod resource_totals;
