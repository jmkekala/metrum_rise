// SPDX-License-Identifier: GPL-3.0-or-later

//! Async terrain and water payload preparation helpers for `SimulationNode`.

mod node_jobs;
mod state;

pub(in crate::nodes::simulation_node) use state::{
    TerrainPatchPayload, TerrainPatchPayloadAsyncState, TerrainPatchPayloadData,
    TerrainPatchPayloadRequest, TerrainPatchPayloadRequestState, TerrainPayloadWorkerPerf,
    WaterPatchMeshAsyncState, WaterPatchPayload, WaterPatchPayloadAsyncState,
    WaterPatchPayloadRequest,
};

#[cfg(test)]
pub(in crate::nodes::simulation_node) use state::{
    PayloadDeferral, TerrainPatchPayloadKey, WaterPatchPayloadKey, WaterPatchPayloadRequestState,
};
