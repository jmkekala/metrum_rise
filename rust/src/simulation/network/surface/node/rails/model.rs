//! Generated rail contour model and stage-local error types.

use super::super::arrangement::NodeBandOwner;
use super::super::backend::{RoadPolyline, RoadVec2, RoadVec3};
use super::super::joins::SideJoinGenerationError;
use super::super::ownership::{
    NodeBooleanOwnership, NodeSourceCarrierRegistry, NodeSourceCarrierSegmentId,
};
use super::super::terminal::TerminalCapGenerationError;
use super::source_points::push_owned_region_height_carrier_points;
use super::{RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum NodeGeneratedContourKind {
    FullRoadbed,
    Band { kind: RoadSurfaceBandKind },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum NodeGeneratedContourPurpose {
    FullRoadbedCorridor,
    CarriagewayCorridor,
    CarriagewayOwnerCarrier,
    NonRoadBand,
    TerminalCap,
    BendSideJoin,
    JunctionSideJoin,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum NodeRailConstraintKind {
    FullRoadbedContour,
    BandContour {
        kind: RoadSurfaceBandKind,
    },
    SpanHandoff {
        kind: RoadSurfaceBandKind,
    },
    FootprintSeam {
        adjacent_kind: RoadSurfaceBandKind,
    },
    AsphaltBoundary {
        adjacent_kind: RoadSurfaceBandKind,
    },
    RaisedStepContact,
    BandBoundary {
        left_kind: RoadSurfaceBandKind,
        right_kind: RoadSurfaceBandKind,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct NodeRailContourSet {
    pub(crate) node_id: u32,
    pub(crate) piece_kind: RoadSurfaceVisualNodePieceKind,
    pub(crate) contours: Vec<NodeGeneratedContour>,
    pub(crate) constraints: Vec<NodeRailConstraint>,
    pub(crate) height_carrier_paths_by_source:
        BTreeMap<(RoadSurfaceBandKind, usize, usize), Vec<NodeRailHeightCarrierPaths>>,
    pub(crate) height_carrier_points_by_source:
        BTreeMap<(RoadSurfaceBandKind, usize, usize), Vec<RoadVec3>>,
    pub(crate) source_carriers: NodeSourceCarrierRegistry,
}

#[derive(Clone, Debug)]
pub(crate) struct NodeRailHeightCarrierPaths {
    pub(crate) start_path_world: Vec<RoadVec3>,
    pub(crate) end_path_world: Vec<RoadVec3>,
}

#[derive(Clone, Debug)]
pub(crate) struct NodeGeneratedContour {
    pub(crate) kind: NodeGeneratedContourKind,
    pub(crate) purpose: NodeGeneratedContourPurpose,
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: Option<usize>,
    pub(crate) owner: Option<NodeBandOwner>,
    pub(crate) claim_priority: NodeGeneratedContourClaimPriority,
    pub(crate) points_xz: Vec<RoadVec2>,
    pub(crate) height_points_world: Option<Vec<RoadVec3>>,
    pub(crate) backend_polyline: RoadPolyline,
}

impl NodeGeneratedContour {
    pub(crate) fn contributes_to_footprint(&self) -> bool {
        self.kind == NodeGeneratedContourKind::FullRoadbed
            && matches!(
                self.purpose,
                NodeGeneratedContourPurpose::FullRoadbedCorridor
                    | NodeGeneratedContourPurpose::TerminalCap
                    | NodeGeneratedContourPurpose::BendSideJoin
            )
    }

    pub(crate) fn contributes_to_asphalt(&self) -> bool {
        matches!(
            self.kind,
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Carriageway,
            }
        ) && matches!(
            self.purpose,
            NodeGeneratedContourPurpose::CarriagewayCorridor
                | NodeGeneratedContourPurpose::BendSideJoin
        )
    }

    pub(crate) fn claims_asphalt_owner_region(&self) -> bool {
        matches!(
            self.kind,
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Carriageway,
            }
        ) && self.owner.is_some()
            && matches!(
                self.purpose,
                NodeGeneratedContourPurpose::CarriagewayCorridor
                    | NodeGeneratedContourPurpose::CarriagewayOwnerCarrier
                    | NodeGeneratedContourPurpose::TerminalCap
                    | NodeGeneratedContourPurpose::BendSideJoin
            )
    }

    pub(crate) fn contributes_to_non_road_band(&self) -> bool {
        matches!(
            self.kind,
            NodeGeneratedContourKind::Band { kind }
                if kind != RoadSurfaceBandKind::Carriageway
        ) && matches!(
            self.purpose,
            NodeGeneratedContourPurpose::NonRoadBand
                | NodeGeneratedContourPurpose::TerminalCap
                | NodeGeneratedContourPurpose::BendSideJoin
                | NodeGeneratedContourPurpose::JunctionSideJoin
        )
    }
}

impl NodeRailContourSet {
    pub(in crate::simulation::network::surface::node) fn height_carrier_points_for_ownership(
        &self,
        ownership: Option<&NodeBooleanOwnership>,
    ) -> Result<BTreeMap<(RoadSurfaceBandKind, usize, usize), Vec<RoadVec3>>, NodeRailGenerationError>
    {
        let mut points_by_source = self.height_carrier_points_by_source.clone();
        if let Some(ownership) = ownership {
            push_owned_region_height_carrier_points(&mut points_by_source, self, ownership)?;
        }
        Ok(points_by_source)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum NodeGeneratedContourClaimPriority {
    JoinOrCap,
    SideJoin,
    MouthBand,
    Footprint,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeRailConstraint {
    pub(crate) constraint_index: usize,
    pub(crate) kind: NodeRailConstraintKind,
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: Option<usize>,
    pub(crate) source_boundary_index: Option<usize>,
    pub(crate) owner: Option<NodeBandOwner>,
    pub(crate) opposite_owner: Option<NodeBandOwner>,
    pub(crate) points_xz: Vec<RoadVec2>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeRailGenerationError {
    EmptyInput {
        node_id: u32,
    },
    DegenerateContour {
        kind: NodeGeneratedContourKind,
        mouth_order_index: usize,
        band_index: Option<usize>,
        area_m2: f64,
        vertex_count: usize,
    },
    DegenerateConstraint {
        kind: NodeRailConstraintKind,
        mouth_order_index: usize,
        band_index: Option<usize>,
        boundary_index: Option<usize>,
        path_length_m: f64,
        vertex_count: usize,
    },
    InvalidHeightCarrier {
        kind: NodeGeneratedContourKind,
        mouth_order_index: usize,
        band_index: Option<usize>,
        reason: &'static str,
    },
    ConflictingHeightCarrierPoint {
        kind: RoadSurfaceBandKind,
        mouth_order_index: usize,
        band_index: usize,
        point_x_key: i64,
        point_z_key: i64,
        existing_height_mm: i64,
        incoming_height_mm: i64,
    },
    MissingCarrierProvenanceHeight {
        kind: RoadSurfaceBandKind,
        mouth_order_index: usize,
        band_index: usize,
        point_x_key: i64,
        point_z_key: i64,
        source_segment_id: NodeSourceCarrierSegmentId,
    },
    NonCanonicalGeneratedContactEndpoint {
        kind: NodeRailConstraintKind,
        mouth_order_index: usize,
        band_index: Option<usize>,
        owner: Option<NodeBandOwner>,
        opposite_owner: Option<NodeBandOwner>,
        point_x_key: i64,
        point_z_key: i64,
    },
    SideJoinGeneration {
        error: SideJoinGenerationError,
    },
    TerminalCapGeneration {
        error: TerminalCapGenerationError,
    },
}
