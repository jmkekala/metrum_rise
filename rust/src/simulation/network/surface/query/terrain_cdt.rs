//! Terrain-patch road loop extraction and CDT source adaptation.

use super::super::{
    ChunkCacheKind, NodeFootprintBoundaryDirectSource, NodeFootprintBoundarySegmentSource,
    NodeFootprintBoundaryVertexSource, RoadSurfaceBandKind, RoadSurfaceEarthworkFaceSource,
    RoadSurfaceEarthworkSupportPolicy, RoadSurfaceSpanRegionRole, RoadSurfaceSystem,
    RoadSurfaceTerrainClipContourRole, RoadSurfaceTerrainClipExport,
    RoadSurfaceTerrainClipExportError, RoadSurfaceTerrainClipLoop,
    RoadSurfaceTerrainClipLoopTopology, RoadSurfaceVisualNodePieceKind,
    keys::{SurfaceHeightMmKey, SurfaceXzKey},
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::EdgeClass;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::cdt::{
    TerrainCdtEarthworkSupportPolicy, TerrainCdtEdgeClass,
    TerrainCdtNodeFootprintBoundaryDirectSource, TerrainCdtNodeFootprintBoundarySegmentSource,
    TerrainCdtNodeFootprintBoundaryVertexSource, TerrainCdtNodePieceKind, TerrainCdtRoadBandKind,
    TerrainCdtRoadBoundarySource, TerrainCdtRoadLoop, TerrainCdtRoadLoopSourceEdge,
    TerrainCdtSpanRegionRole, TerrainCdtVertex,
};
use godot::prelude::Vector3;
use std::collections::{BTreeMap, HashSet};

mod loops;
mod mapping;
mod patches;
mod stable_ids;
