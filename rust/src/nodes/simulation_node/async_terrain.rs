//! Async terrain and water payload preparation helpers for `SimulationNode`.

mod node_jobs;
mod refined_cache;
mod state;

pub(in crate::nodes::simulation_node) use state::{
    TerrainPatchPayload, TerrainPatchPayloadAsyncState, TerrainPatchPayloadData,
    TerrainPatchPayloadRequest, TerrainPatchPayloadRequestState, WaterPatchMeshAsyncState,
    WaterPatchPayload, WaterPatchPayloadAsyncState, WaterPatchPayloadRequest,
};

#[cfg(test)]
pub(in crate::nodes::simulation_node) use state::{TerrainPatchPayloadKey, WaterPatchPayloadKey};
