//! Terrain CDT variant export helpers.

mod buffers;
mod diagnostics;
mod input;
mod road_clip;
mod sidecars;
mod status;
mod types;

pub(in crate::nodes::simulation_node) use input::{TERRAIN_CDT_TILE_NEIGHBORS, TerrainCdtTileId};
#[cfg(test)]
pub(in crate::nodes::simulation_node) use types::{
    TerrainCdtSourceExport, TerrainCdtTriangleBufferExport,
};
