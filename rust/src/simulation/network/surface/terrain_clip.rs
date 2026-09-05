// SPDX-License-Identifier: GPL-2.0-only

//! Source-preserving terrain-clip boundary export for owned road pieces.

mod dust;
mod geometry;
mod heights;
mod model;
mod output;
mod recovery;
mod source_edges;
mod union;

pub(crate) use model::{
    RoadSurfaceTerrainClipContourRole, RoadSurfaceTerrainClipEdgeKind,
    RoadSurfaceTerrainClipExport, RoadSurfaceTerrainClipExportError, RoadSurfaceTerrainClipLoop,
    RoadSurfaceTerrainClipLoopTopology, RoadSurfaceTerrainClipSourceEdge,
    terrain_clip_edge_kind_for_band,
};
