// SPDX-License-Identifier: GPL-2.0-only

//! Background simulation thread, `SimCore` state bundle, and `RenderSnapshot`.
//!
//! `SimCore` owns all simulation state. The background thread continuously ticks
//! it at ~60 Hz, writes a `RenderSnapshot` after every tick, and never touches
//! Godot objects. The Godot main thread reads render snapshots and locks the core
//! briefly for mutations. Interpolated off-lane vehicle poses also use a nonblocking
//! read of local surface indices; a busy core falls back to supported snapshot poses.

mod budget;
mod road_edit_plan;
mod road_preview;
mod road_terrain_plan;
mod snapshot;
mod state;
mod terrain_payloads;
mod thread;
mod water_preview;

pub(crate) use budget::CityServicePolicy;
pub use budget::CityTreasury;
pub(crate) use road_edit_plan::RoadEditPlan;
pub(crate) use road_terrain_plan::RoadTerrainPlan;
pub use snapshot::RenderSnapshot;
pub use state::SimCore;
pub(crate) use thread::SimCommand;

pub(crate) use budget::{
    DailyBudgetLedgerEntry, ROAD_BUILD_COST_PER_METER, SERVICE_BUILD_COST_PER_LOT_CELL,
    SERVICE_POLICY_ELECTRICITY,
};
pub(crate) use road_preview::{
    RoadPreviewRequest, RoadPreviewSender, RoadPreviewSnapshot, RoadPreviewWorkerContext,
    RoadToolQuerySnapshot, road_preview_channel, road_tool_snapshots_from_core,
    run_road_preview_worker,
};
pub(crate) use snapshot::{
    BuildingRemovalUndo, SimulationRuntimeSnapshot, SimulationSnapshot, WaterRuntimeSnapshot,
    access_phase_target,
};
pub(crate) use state::PendingDemandSpawnAction;
pub(crate) use terrain_payloads::{
    CachedRefinedTerrainCdtWindow, CachedRefinedTerrainMeshBuffers, CachedRefinedTerrainPatch,
    CachedTerrainCdtRoadInput, ROAD_LOCKED_TERRAIN_RENDER_STEP_M, RefinedTerrainAssemblyScope,
    RefinedTerrainCdtWindowBuildInput, RefinedTerrainCdtWindowKey, RefinedTerrainPatchBuildInput,
    RefinedTerrainPatchCacheKey,
};
pub(crate) use thread::run_sim_thread;
pub(crate) use water_preview::{
    AuthoredWaterPatchFillDebug, WorldLakeFillPreview, WorldLakeFillPreviewStatus,
    WorldWaterFillKind,
};

#[cfg(test)]
use snapshot::pedestrian_lane_surface_height;
#[cfg(test)]
use state::{
    absolute_operational_minute, demand_plan_has_non_spawn_actions, demand_plan_without_spawns,
};

#[cfg(test)]
mod tests;
