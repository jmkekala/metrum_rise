//! Source-preserving terrain-clip boundary export for owned road pieces.

mod model;
mod output;
mod union;

pub(crate) use model::{
    RoadSurfaceTerrainClipEdgeKind, RoadSurfaceTerrainClipExportError, RoadSurfaceTerrainClipLoop,
    RoadSurfaceTerrainClipSourceEdge, terrain_clip_edge_kind_for_band,
};
