// SPDX-License-Identifier: GPL-2.0-only

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

impl RoadSurfaceVisualNodeCompileInput {
    /// Compares exact solved geometry while ignoring graph-local edge identifiers.
    pub(in crate::simulation::network::surface) fn topology_eq_ignoring_edge_identity(
        &self,
        other: &Self,
    ) -> bool {
        self.kind == other.kind
            && self.mouth_edge_classes == other.mouth_edge_classes
            && self.mouths.len() == other.mouths.len()
            && self.mouths.iter().zip(&other.mouths).all(|(a, b)| {
                a.profile == b.profile
                    && a.endpoint_profile == b.endpoint_profile
                    && a.boundary_paths_world == b.boundary_paths_world
                    && a.band_start_paths_world == b.band_start_paths_world
                    && a.band_end_paths_world == b.band_end_paths_world
                    && a.uses_explicit_band_domain_paths == b.uses_explicit_band_domain_paths
                    && a.direction_angle_ccw == b.direction_angle_ccw
                    && a.direction_xz == b.direction_xz
                    && a.side == b.side
            })
    }
}
