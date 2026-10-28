// SPDX-License-Identifier: GPL-2.0-only

//! Background simulation thread, `SimCore` state bundle, and `RenderSnapshot`.
//!
//! `SimCore` owns all simulation state. The background thread continuously ticks
//! it at ~60 Hz, writes a `RenderSnapshot` after every tick, and never touches
//! Godot objects. The Godot main thread reads render snapshots and locks the core
//! briefly for mutations. Interpolated off-lane vehicle poses also use a nonblocking
//! read of local surface indices; a busy core falls back to supported snapshot poses.

mod budget;
mod demand_requests;
mod entity_days;
mod entity_phases;
mod frame;
mod graded_edits;
mod graded_stage;
mod household_requests;
mod populated_city;
mod road_commit;
mod road_edit_plan;
mod road_preview;
mod road_rejection;
mod road_terrain_inputs;
mod road_tool_context;
mod scenario;
mod schedule;
mod snapshot;
mod state;
mod thread;
mod trips;
mod water_preview;

pub(crate) use budget::CityServicePolicy;
pub use budget::CityTreasury;
pub use populated_city::{EconomyHourTimes, PopulatedCity, PopulatedCityBuildTimes};
pub(crate) use graded_edits::{GradedEdit, GradedEditJournal, GradedEditSelection, GradedInstallTiles};
pub(crate) use graded_stage::{GradedPose, GradedPreview, GradedUndo};
pub(crate) use road_commit::RoadCommitAck;
pub(crate) use road_edit_plan::{ReservedOverlap, RoadEditPlan};
pub(crate) use road_rejection::{RejectedBy, RoadAttempt, RoadRejection};
pub use scenario::{
    GrowthBusinessCashFlow, GrowthBusinessOutcome, GrowthDayRecord, GrowthReport, GrowthScenario,
};
pub(crate) use schedule::SimEvent;
pub use snapshot::RenderSnapshot;
pub use state::SimCore;
pub(crate) use thread::SimCommand;

pub(crate) use budget::{
    DailyBudgetLedgerEntry, ROAD_BUILD_COST_PER_METER, RoadLengthRoll,
    SERVICE_BUILD_COST_PER_LOT_CELL, SERVICE_POLICY_ELECTRICITY,
};
pub(crate) use road_preview::{
    RoadCorridorClearance, RoadPreviewRequest, RoadToolQuerySnapshot,
};
pub(crate) use road_terrain_inputs::RoadTerrainInputsLog;
pub(crate) use road_tool_context::{RoadToolPublisher, RoadToolShared};
pub(crate) use snapshot::{
    BuildingRemovalUndo, SimulationRuntimeSnapshot, SimulationSnapshot, VegetationEditUndo,
    WaterRuntimeSnapshot, access_phase_target,
};
pub(crate) use state::PendingDemandSpawnAction;
pub(crate) use thread::run_sim_thread;
pub(crate) use water_preview::{
    AuthoredWaterPatchFillDebug, WorldLakeFillPreview, WorldLakeFillPreviewStatus,
    WorldWaterFillKind,
};

#[cfg(test)]
pub(crate) use road_commit::RoadCommitRequest;
#[cfg(test)]
pub(crate) use road_preview::road_tool_snapshot_from_core;
#[cfg(test)]
use snapshot::pedestrian_lane_surface_height;
#[cfg(test)]
use state::{
    absolute_operational_minute, demand_plan_has_non_spawn_actions, demand_plan_without_spawns,
};

#[cfg(test)]
mod tests;
