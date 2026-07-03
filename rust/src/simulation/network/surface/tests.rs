//! Unit tests for the road-surface compiler and ownership caches.

use super::arrangement::{
    NodeArrangement, NodeArrangementError, NodeArrangementKey, NodeBandOwner,
};
use super::band_semantics::ordered_raised_step_kinds;
use super::earthwork::EARTHWORK_MAX_MARGIN_M;
use super::edge::CURB_STEP_HEIGHT_M;
use super::height::NodeHeightFieldError;
use super::keys::SurfaceHeightMmKey;
use super::validation::{NodeGeometryDiagnosticKind, NodeValidationReport};
use super::{
    IncidentEdgeSide, IncidentMouthProfile, NodeFootprintBoundaryVertexSource, NodeOverlayContour,
    NodeOverlayShape, NodeOverlayShapes, PreviewRoadSurfaceResult, RoadSurfaceBand,
    RoadSurfaceBandKind, RoadSurfaceEarthworkFaceKind, RoadSurfaceEarthworkFaceSource,
    RoadSurfaceEarthworkRenderFace, RoadSurfaceEarthworkSupportPolicy, RoadSurfaceSection,
    RoadSurfaceSpanRegionRole, RoadSurfaceSystem, RoadSurfaceTerrainClipEdgeKind,
    RoadSurfaceTerrainClipExportError, RoadSurfaceTerrainClipLoop,
    RoadSurfaceTerrainClipSourceEdge, RoadSurfaceVerticalFaceSource, RoadSurfaceVisualNodePiece,
    RoadSurfaceVisualNodePieceKind, RoadSurfaceVisualPolygon, RoadSurfaceVisualSpanPiece,
    SAMPLE_EPSILON_M, SurfaceChunkKey, arrangement, backend, height, input, node, ownership, rails,
    segments, validation,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{
    EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::cdt::{
    TerrainCdtInput, TerrainCdtMesh, TerrainCdtPatch, TerrainCdtRoadBoundarySource,
    TerrainCdtRoadLoop, TerrainCdtRoadLoopSourceEdge, TerrainCdtTieInKind, TerrainCdtVertex,
    build_road_touched_terrain_patch,
};
use godot::prelude::{Vector2, Vector3};
use i_overlay::core::overlay_rule::OverlayRule;
use std::collections::{BTreeMap, BTreeSet, HashSet};

mod bend_terminal;
mod core;
mod earthwork;
mod junction;
mod preview;
mod span;
mod support;
mod terrain_clip;
mod visibility_debug;

use support::*;
