// SPDX-License-Identifier: GPL-2.0-only

//! JSON-serializable sandbox result contracts.

use serde::Serialize;

#[derive(Serialize)]
pub(in crate::simulation::economy::definitions) struct SandboxResult {
    pub(super) scenario_id: String,
    pub(super) display_name: String,
    pub(super) duration_days: u32,
    pub(super) daily_household_demand_units: f32,
    pub(super) final_household_stock_days: f32,
    pub(super) lowest_household_stock_days: f32,
    pub(super) total_delivered_units: f32,
    pub(super) total_unmet_units: f32,
    pub(super) average_household_cost_per_day: f32,
    pub(super) bottlenecks: Vec<String>,
    pub(super) daily: Vec<DailySandboxMetric>,
}

#[derive(Serialize)]
pub(in crate::simulation::economy::definitions) struct DailySandboxMetric {
    pub(super) day: u32,
    pub(super) household_stock_days: f32,
    pub(super) delivered_units: f32,
    pub(super) unmet_units: f32,
    pub(super) average_household_cost: f32,
}
