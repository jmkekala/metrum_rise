//! Earthwork data contracts, keys, and deterministic ordering helpers.

use super::super::{
    NodeFootprintBoundarySegmentSource, RoadSurfaceBandKind, RoadSurfaceSpanBandOwner,
    RoadSurfaceSpanRegionRole, RoadSurfaceVisualNodePieceKind,
    backend::RoadVec3,
    band_semantics::band_kind_sort_key,
    edge::edge_class_sort_key,
    keys::{SurfaceXzKey, SurfaceXzSegmentKey},
};
use crate::simulation::network::types::EdgeClass;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoadSurfaceEarthworkFaceKind {
    Slope,
    RetainingWall,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoadSurfaceEarthworkGeometryError {
    OpenBoundaryChain {
        segment_count: usize,
    },
    DegenerateBoundaryLoop {
        point_count: usize,
    },
    DegenerateOutwardDirection {
        point_count: usize,
        point_index: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoadSurfaceEarthworkSupportPolicy {
    StandardFullGroundedSpan,
    BridgeEndpointAbutments,
    TunnelVisiblePortals,
}

impl RoadSurfaceEarthworkSupportPolicy {
    pub(crate) fn from_edge_class(edge_class: EdgeClass) -> Self {
        match edge_class {
            EdgeClass::Standard => Self::StandardFullGroundedSpan,
            EdgeClass::Bridge => Self::BridgeEndpointAbutments,
            EdgeClass::Tunnel => Self::TunnelVisiblePortals,
        }
    }

    pub(crate) fn debug_name(self) -> &'static str {
        match self {
            Self::StandardFullGroundedSpan => "standard_full_grounded_span",
            Self::BridgeEndpointAbutments => "bridge_endpoint_abutments",
            Self::TunnelVisiblePortals => "tunnel_visible_portals",
        }
    }

    pub(crate) fn sort_key(self) -> u8 {
        match self {
            Self::StandardFullGroundedSpan => 0,
            Self::BridgeEndpointAbutments => 1,
            Self::TunnelVisiblePortals => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RoadSurfaceEarthworkFaceSource {
    SpanSupportBoundary {
        edge_idx: usize,
        edge_class: EdgeClass,
        support_policy: RoadSurfaceEarthworkSupportPolicy,
        owner: RoadSurfaceSpanBandOwner,
        role: RoadSurfaceSpanRegionRole,
        start_section_index: usize,
        end_section_index: usize,
        start_s_m: f32,
        end_s_m: f32,
    },
    NodeFootprintBoundary {
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        owner_kind: RoadSurfaceBandKind,
        owner_index: usize,
        boundary_source: Option<NodeFootprintBoundarySegmentSource>,
    },
    NodeSameMaterialBoundaryHandoff {
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        owner_kind: RoadSurfaceBandKind,
        owner_index_a: usize,
        owner_index_b: usize,
        boundary_source: Option<NodeFootprintBoundarySegmentSource>,
    },
}

impl RoadSurfaceEarthworkFaceSource {
    pub(crate) fn source_ordering(self, other: Self) -> std::cmp::Ordering {
        match (self, other) {
            (
                Self::SpanSupportBoundary {
                    edge_idx: edge_idx_a,
                    edge_class: edge_class_a,
                    support_policy: support_policy_a,
                    owner: owner_a,
                    role: role_a,
                    start_section_index: start_section_index_a,
                    end_section_index: end_section_index_a,
                    start_s_m: start_s_m_a,
                    end_s_m: end_s_m_a,
                },
                Self::SpanSupportBoundary {
                    edge_idx: edge_idx_b,
                    edge_class: edge_class_b,
                    support_policy: support_policy_b,
                    owner: owner_b,
                    role: role_b,
                    start_section_index: start_section_index_b,
                    end_section_index: end_section_index_b,
                    start_s_m: start_s_m_b,
                    end_s_m: end_s_m_b,
                },
            ) => edge_idx_a
                .cmp(&edge_idx_b)
                .then(edge_class_sort_key(edge_class_a).cmp(&edge_class_sort_key(edge_class_b)))
                .then(
                    support_policy_a
                        .sort_key()
                        .cmp(&support_policy_b.sort_key()),
                )
                .then(owner_a.sort_key().cmp(&owner_b.sort_key()))
                .then(role_a.sort_key().cmp(&role_b.sort_key()))
                .then(start_section_index_a.cmp(&start_section_index_b))
                .then(end_section_index_a.cmp(&end_section_index_b))
                .then(start_s_m_a.total_cmp(&start_s_m_b))
                .then(end_s_m_a.total_cmp(&end_s_m_b)),
            (
                Self::NodeFootprintBoundary {
                    node_id: node_id_a,
                    kind: kind_a,
                    owner_kind: owner_kind_a,
                    owner_index: owner_index_a,
                    boundary_source: boundary_source_a,
                },
                Self::NodeFootprintBoundary {
                    node_id: node_id_b,
                    kind: kind_b,
                    owner_kind: owner_kind_b,
                    owner_index: owner_index_b,
                    boundary_source: boundary_source_b,
                },
            ) => node_id_a
                .cmp(&node_id_b)
                .then(kind_a.sort_key().cmp(&kind_b.sort_key()))
                .then(band_kind_sort_key(owner_kind_a).cmp(&band_kind_sort_key(owner_kind_b)))
                .then(owner_index_a.cmp(&owner_index_b))
                .then(boundary_source_a.cmp(&boundary_source_b)),
            (
                Self::NodeSameMaterialBoundaryHandoff {
                    node_id: node_id_a,
                    kind: kind_a,
                    owner_kind: owner_kind_a,
                    owner_index_a: owner_index_a_a,
                    owner_index_b: owner_index_b_a,
                    boundary_source: boundary_source_a,
                },
                Self::NodeSameMaterialBoundaryHandoff {
                    node_id: node_id_b,
                    kind: kind_b,
                    owner_kind: owner_kind_b,
                    owner_index_a: owner_index_a_b,
                    owner_index_b: owner_index_b_b,
                    boundary_source: boundary_source_b,
                },
            ) => node_id_a
                .cmp(&node_id_b)
                .then(kind_a.sort_key().cmp(&kind_b.sort_key()))
                .then(band_kind_sort_key(owner_kind_a).cmp(&band_kind_sort_key(owner_kind_b)))
                .then(owner_index_a_a.cmp(&owner_index_a_b))
                .then(owner_index_b_a.cmp(&owner_index_b_b))
                .then(boundary_source_a.cmp(&boundary_source_b)),
            (Self::SpanSupportBoundary { .. }, Self::NodeFootprintBoundary { .. }) => {
                std::cmp::Ordering::Less
            }
            (Self::NodeFootprintBoundary { .. }, Self::SpanSupportBoundary { .. }) => {
                std::cmp::Ordering::Greater
            }
            (Self::SpanSupportBoundary { .. }, Self::NodeSameMaterialBoundaryHandoff { .. }) => {
                std::cmp::Ordering::Less
            }
            (Self::NodeSameMaterialBoundaryHandoff { .. }, Self::SpanSupportBoundary { .. }) => {
                std::cmp::Ordering::Greater
            }
            (Self::NodeFootprintBoundary { .. }, Self::NodeSameMaterialBoundaryHandoff { .. }) => {
                std::cmp::Ordering::Less
            }
            (Self::NodeSameMaterialBoundaryHandoff { .. }, Self::NodeFootprintBoundary { .. }) => {
                std::cmp::Ordering::Greater
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RoadSurfaceEarthworkBoundarySegment {
    pub(crate) inner_start: RoadVec3,
    pub(crate) inner_end: RoadVec3,
    pub(crate) source: RoadSurfaceEarthworkFaceSource,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoadSurfaceEarthworkRenderFace {
    pub(crate) kind: RoadSurfaceEarthworkFaceKind,
    pub(crate) source: RoadSurfaceEarthworkFaceSource,
    pub(crate) inner_start: RoadVec3,
    pub(crate) inner_end: RoadVec3,
    pub(crate) polygon: super::super::RoadSurfaceVisualPolygon,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct EarthworkBoundaryPointKey {
    pub(super) x_key: i64,
    pub(super) z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct EarthworkBoundaryEdgeKey {
    pub(super) start: EarthworkBoundaryPointKey,
    pub(super) end: EarthworkBoundaryPointKey,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct IndexedEarthworkBoundarySegment {
    pub(super) segment_index: usize,
    pub(super) segment: RoadSurfaceEarthworkBoundarySegment,
    pub(super) start_key: EarthworkBoundaryPointKey,
    pub(super) end_key: EarthworkBoundaryPointKey,
}

impl EarthworkBoundaryPointKey {
    pub(super) fn from_point(point: RoadVec3) -> Self {
        Self::from_surface_key(SurfaceXzKey::from_world_xz(point))
    }

    fn from_surface_key(key: SurfaceXzKey) -> Self {
        Self {
            x_key: key.x_key(),
            z_key: key.z_key(),
        }
    }

    fn surface_key(self) -> SurfaceXzKey {
        SurfaceXzKey::from_raw_keys(self.x_key, self.z_key)
    }
}

impl EarthworkBoundaryEdgeKey {
    pub(super) fn normalized(
        start: EarthworkBoundaryPointKey,
        end: EarthworkBoundaryPointKey,
    ) -> Option<Self> {
        let segment = SurfaceXzSegmentKey::non_degenerate(start.surface_key(), end.surface_key())?;
        Some(Self {
            start: EarthworkBoundaryPointKey::from_surface_key(segment.start()),
            end: EarthworkBoundaryPointKey::from_surface_key(segment.end()),
        })
    }
}
