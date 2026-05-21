//! Source-backed earthwork boundary segment export from footprint loops.

use super::super::band_semantics::{raised_step_band_rank, raised_step_kinds_can_contact};
use super::sources::{
    node_footprint_boundary_vertex_source_at_point,
    node_footprint_boundary_vertex_source_for_edge_point,
};
use super::*;
use crate::simulation::network::surface::{
    RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkFaceSource,
};

mod loops;
mod merge;
mod sources;
mod splits;

pub(in crate::simulation::network::surface) use loops::node_earthwork_boundary_segments_from_footprint_loops;
pub(in crate::simulation::network::surface::node) use loops::same_winding_boundary_point_loops_from_loop;

#[cfg(test)]
pub(super) use splits::{
    insert_node_footprint_boundary_split_point, push_sourced_node_earthwork_boundary_segments,
};

#[derive(Clone, Copy, Debug)]
pub(super) struct NodeFootprintBoundarySplitPoint {
    pub(super) point_key: ArrangementBoundaryPointKey,
    pub(super) source: Option<NodeFootprintBoundaryDirectVertex>,
}

#[derive(Clone, Copy, Debug)]
struct NodeEarthworkBoundarySourceCandidate {
    face_source: RoadSurfaceEarthworkFaceSource,
    height_field_id: Option<arrangement::NodeBandHeightFieldId>,
}

impl NodeEarthworkBoundarySourceCandidate {
    fn from_face_source(face_source: RoadSurfaceEarthworkFaceSource) -> Self {
        Self {
            face_source,
            height_field_id: None,
        }
    }
}
