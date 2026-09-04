//! Background simulation thread, `SimCore` state bundle, and `RenderSnapshot`.
//!
//! `SimCore` owns all simulation state. The background thread continuously ticks
//! it at ~60 Hz, writes a `RenderSnapshot` after every tick, and never touches
//! Godot objects. The Godot main thread reads only from the snapshot for rendering
//! and locks the `Arc<Mutex<SimCore>>` briefly for mutations (road edits, etc.).

mod budget;
mod road_preview;
mod snapshot;
mod state;
mod terrain_payloads;
mod thread;
mod water_preview;

pub(crate) use budget::CityServicePolicy;
pub use budget::CityTreasury;
pub use snapshot::RenderSnapshot;
pub use state::SimCore;
pub(crate) use thread::SimCommand;

pub(crate) use budget::{
    DailyBudgetLedgerEntry, ROAD_BUILD_COST_PER_METER, SERVICE_BUILD_COST_PER_LOT_CELL,
    SERVICE_POLICY_ELECTRICITY,
};
pub(crate) use road_preview::{
    RoadPreviewRequest, RoadPreviewSnapshot, RoadPreviewValidationCertificate,
    RoadPreviewWorkerContext, RoadToolQuerySnapshot, road_tool_snapshots_from_core,
    run_road_preview_worker,
};
pub(crate) use snapshot::{
    BuildingRemovalUndo, SimulationRuntimeSnapshot, SimulationSnapshot, WaterRuntimeSnapshot,
};
pub(crate) use state::PendingDemandSpawnAction;
pub(crate) use terrain_payloads::{
    CachedRefinedTerrainCdtWindow, CachedRefinedTerrainMeshBuffers, CachedRefinedTerrainPatch,
    ROAD_LOCKED_TERRAIN_RENDER_STEP_M, RefinedTerrainAssemblyScope,
    RefinedTerrainCdtWindowBuildInput, RefinedTerrainCdtWindowKey, RefinedTerrainPatchBuildInput,
    RefinedTerrainPatchCacheKey,
};
pub(crate) use thread::run_sim_thread;
pub(crate) use water_preview::{
    AuthoredWaterPatchFillDebug, WorldLakeFillPreview, WorldLakeFillPreviewStatus,
    WorldWaterFillKind,
};

#[cfg(test)]
use snapshot::{
    pedestrian_access_surface_height_from_samples, pedestrian_lane_surface_height,
    pedestrian_needs_access_surface,
};
#[cfg(test)]
use state::{
    absolute_operational_minute, demand_plan_has_non_spawn_actions, demand_plan_without_spawns,
};

#[cfg(test)]
mod tests;
