//! Incident edge and mouth-profile data shared by span, node, and rail compilation.

use super::{
    RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind,
    backend::{RoadVec2, RoadVec3},
};
use crate::simulation::network::types::EdgeClass;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Ord, PartialOrd)]
pub(crate) enum IncidentEdgeSide {
    Start,
    End,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompiledNodeKind {
    Terminal,
    PassThrough,
    Bend,
    JunctionN,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct IncidentSurfaceEdge {
    pub(crate) edge_idx: usize,
    pub(crate) side: IncidentEdgeSide,
    pub(crate) direction_xz: RoadVec2,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct IncidentMouthBand {
    pub(crate) kind: RoadSurfaceBandKind,
    pub(crate) start_point_world: RoadVec3,
    pub(crate) end_point_world: RoadVec3,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct IncidentMouthProfile {
    pub(crate) inward_direction_xz: RoadVec2,
    pub(crate) boundary_points_world: Vec<RoadVec3>,
    pub(crate) bands: Vec<IncidentMouthBand>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OrderedIncidentPieceMouth {
    pub(crate) profile: IncidentMouthProfile,
    pub(crate) endpoint_profile: IncidentMouthProfile,
    pub(crate) boundary_paths_world: Vec<Vec<RoadVec3>>,
    pub(crate) band_start_paths_world: Vec<Vec<RoadVec3>>,
    pub(crate) band_end_paths_world: Vec<Vec<RoadVec3>>,
    pub(crate) uses_explicit_band_domain_paths: bool,
    pub(crate) direction_angle_ccw: f32,
    pub(crate) direction_xz: RoadVec2,
    pub(crate) edge_idx: usize,
    pub(crate) side: IncidentEdgeSide,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoadSurfaceVisualNodeCompileInput {
    pub(crate) kind: RoadSurfaceVisualNodePieceKind,
    pub(crate) mouths: Vec<OrderedIncidentPieceMouth>,
    pub(crate) mouth_edge_classes: Vec<EdgeClass>,
}
