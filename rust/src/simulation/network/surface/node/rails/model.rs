// SPDX-License-Identifier: GPL-2.0-only

//! Generated rail contour model and stage-local error types.

use super::super::IncidentEdgeSide;
use super::super::arrangement::NodeBandOwner;
use super::super::backend::{RoadPolyline, RoadVec2, RoadVec3};
use super::super::joins::{
    NodeInputSideJoinGapRole, NodeInputSideJoinGapSummary, SideJoinGenerationError,
};
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
    pub(crate) corner_trims: Vec<NodeGeneratedCornerTrim>,
    pub(crate) side_join_gaps: Vec<NodeGeneratedSideJoinGap>,
    pub(crate) constraints: Vec<NodeRailConstraint>,
    pub(crate) height_carrier_paths_by_source:
        BTreeMap<(RoadSurfaceBandKind, usize, usize), Vec<NodeRailHeightCarrierPaths>>,
    pub(crate) height_carrier_points_by_source:
        BTreeMap<(RoadSurfaceBandKind, usize, usize), Vec<RoadVec3>>,
    pub(crate) source_carriers: NodeSourceCarrierRegistry,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct NodeRailBuildProfile {
    pub(crate) total_ms: f64,
    pub(crate) terminal_caps_ms: f64,
    pub(crate) side_joins_ms: f64,
    pub(crate) owners_ms: f64,
    pub(crate) mouth_base_contours_ms: f64,
    pub(crate) mouth_band_contours_ms: f64,
    pub(crate) cap_height_carriers_ms: f64,
    pub(crate) terminal_cap_contours_ms: f64,
    pub(crate) side_join_contours_ms: f64,
    pub(crate) boundary_constraints_ms: f64,
    pub(crate) span_handoff_ms: f64,
    pub(crate) contact_noding_first_ms: f64,
    pub(crate) raised_step_contacts_first_ms: f64,
    pub(crate) material_contacts_ms: f64,
    pub(crate) raised_step_contacts_second_ms: f64,
    pub(crate) contact_noding_second_ms: f64,
    pub(crate) same_band_contacts_ms: f64,
    pub(crate) contact_noding_third_ms: f64,
    pub(crate) validation_source_constraints_ms: f64,
    pub(crate) retain_constraints_ms: f64,
    pub(crate) validate_endpoints_ms: f64,
    pub(crate) source_carriers_ms: f64,
    pub(crate) mouths: usize,
    pub(crate) contours: usize,
    pub(crate) constraints: usize,
    pub(crate) source_constraints: usize,
    pub(crate) validation_constraints: usize,
    pub(crate) height_carrier_sources: usize,
    pub(crate) height_carrier_points: usize,
    pub(crate) contact_pair_tests: usize,
    pub(crate) contact_pair_aabb_rejected: usize,
    pub(crate) contact_pair_kind_rejected: usize,
    pub(crate) contact_pair_processed: usize,
    pub(crate) contact_overlay_calls: usize,
    pub(crate) contact_constraints_emitted: usize,
    pub(crate) contact_candidate_pairs: usize,
    pub(crate) contact_same_material_candidate_pairs: usize,
    pub(crate) contact_raised_step_candidate_pairs: usize,
    pub(crate) contact_authority_rejected: usize,
    pub(crate) contact_same_authority_skipped: usize,
    pub(crate) same_material_overlay_calls: usize,
    pub(crate) same_material_pair_cache_hits: usize,
    pub(crate) raised_step_pair_cache_previous_hits: usize,
    pub(crate) raised_step_pair_cache_misses: usize,
    pub(crate) source_target_group_cache_hits: usize,
    pub(crate) source_contact_cache_hits: usize,
    pub(crate) source_contact_cache_misses: usize,
    pub(crate) source_pair_cache_hits: usize,
    pub(crate) source_pair_cache_misses: usize,
    pub(crate) contact_noding_pair_cache_hits: usize,
    pub(crate) contact_noding_pair_cache_misses: usize,
    pub(crate) contact_noding_component_cache_hits: usize,
    pub(crate) contact_noding_component_cache_misses: usize,
    pub(crate) retained_authority_cache_hits: usize,
    pub(crate) retained_authority_current_hits: usize,
    pub(crate) retained_authority_previous_hits: usize,
    pub(crate) retained_authority_cache_misses: usize,
    pub(crate) retained_decision_cache_hits: usize,
    pub(crate) retained_decision_current_hits: usize,
    pub(crate) retained_decision_previous_hits: usize,
    pub(crate) retained_decision_cache_misses: usize,
    pub(crate) same_material_height_split_candidates: usize,
    pub(crate) same_material_height_split_appended: usize,
    pub(crate) same_material_height_split_duplicates: usize,
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

#[derive(Clone, Debug)]
pub(crate) struct NodeGeneratedCornerTrim {
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: usize,
    pub(crate) source_band_kind: RoadSurfaceBandKind,
    pub(crate) source_owner: NodeBandOwner,
    pub(crate) points_xz: Vec<RoadVec2>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeGeneratedSideJoinGap {
    pub(crate) from_mouth_order_index: usize,
    pub(crate) to_mouth_order_index: usize,
    pub(crate) from_edge_idx: usize,
    pub(crate) to_edge_idx: usize,
    pub(crate) from_side: IncidentEdgeSide,
    pub(crate) to_side: IncidentEdgeSide,
    pub(crate) angle_rad: f64,
    pub(crate) role: NodeInputSideJoinGapRole,
    pub(crate) emitted_band_kinds: Vec<RoadSurfaceBandKind>,
    pub(crate) suppressed_band_kinds: Vec<RoadSurfaceBandKind>,
}

impl NodeGeneratedSideJoinGap {
    pub(crate) fn from_side_join_gap_summaries(
        gap_summaries: &[NodeInputSideJoinGapSummary],
    ) -> Vec<Self> {
        gap_summaries.iter().map(Self::from_gap_summary).collect()
    }

    fn from_gap_summary(summary: &NodeInputSideJoinGapSummary) -> Self {
        let gap = summary.gap;
        Self {
            from_mouth_order_index: gap.from_mouth_order_index,
            to_mouth_order_index: gap.to_mouth_order_index,
            from_edge_idx: gap.from_edge_idx,
            to_edge_idx: gap.to_edge_idx,
            from_side: gap.from_side,
            to_side: gap.to_side,
            angle_rad: gap.angle_rad,
            role: gap.role,
            emitted_band_kinds: summary.emitted_band_kinds.clone(),
            suppressed_band_kinds: summary.suppressed_band_kinds.clone(),
        }
    }
}

impl NodeGeneratedContour {
    pub(crate) fn contributes_to_footprint(&self) -> bool {
        if self.kind != NodeGeneratedContourKind::FullRoadbed {
            return false;
        }
        matches!(
            self.purpose,
            NodeGeneratedContourPurpose::FullRoadbedCorridor
                | NodeGeneratedContourPurpose::TerminalCap
                | NodeGeneratedContourPurpose::BendSideJoin
        ) || (self.purpose == NodeGeneratedContourPurpose::JunctionSideJoin
            && self.claim_priority == NodeGeneratedContourClaimPriority::Footprint)
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
                | NodeGeneratedContourPurpose::CarriagewayOwnerCarrier
                | NodeGeneratedContourPurpose::BendSideJoin
                | NodeGeneratedContourPurpose::JunctionSideJoin
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
                    | NodeGeneratedContourPurpose::JunctionSideJoin
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
