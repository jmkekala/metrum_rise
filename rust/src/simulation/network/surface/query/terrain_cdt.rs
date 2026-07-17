//! Terrain-patch road loop extraction and CDT source adaptation.

use super::super::{
    NodeFootprintBoundaryDirectSource, NodeFootprintBoundarySegmentSource,
    NodeFootprintBoundaryVertexSource, RoadSurfaceBandKind, RoadSurfaceEarthworkFaceSource,
    RoadSurfaceEarthworkSupportPolicy, RoadSurfaceSpanRegionRole, RoadSurfaceSystem,
    RoadSurfaceTerrainClipContourRole, RoadSurfaceTerrainClipExport,
    RoadSurfaceTerrainClipExportError, RoadSurfaceTerrainClipLoop,
    RoadSurfaceTerrainClipLoopTopology, RoadSurfaceVisualNodePieceKind,
    backend::RoadVec3,
    earthwork::EARTHWORK_MAX_MARGIN_M,
    keys::{SurfaceHeightMmKey, SurfaceXzKey},
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::EdgeClass;
use crate::simulation::terrain::cdt::{
    MAX_TERRAIN_TIE_IN_SLOPE_RATIO, TerrainCdtEarthworkSupportPolicy, TerrainCdtEdgeClass,
    TerrainCdtNodeFootprintBoundaryDirectSource, TerrainCdtNodeFootprintBoundarySegmentSource,
    TerrainCdtNodeFootprintBoundaryVertexSource, TerrainCdtNodePieceKind, TerrainCdtRoadBandKind,
    TerrainCdtRoadBoundarySource, TerrainCdtRoadLoop, TerrainCdtRoadLoopSourceEdge,
    TerrainCdtSpanRegionRole, TerrainCdtTieInGuideConstraint, TerrainCdtTieInGuideSample,
    TerrainCdtVertex,
};
use crate::simulation::terrain::{TerrainSystem, terrain_cdt_local_sample_margin_m};
use std::collections::BTreeMap;
#[cfg(test)]
use std::collections::HashSet;

mod grading;
mod loops;
mod mapping;
mod patches;
mod stable_ids;
