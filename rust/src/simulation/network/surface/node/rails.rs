//! Library-backed rail and contour generation for canonical node arrangements.

use super::arrangement::NodeBandOwner;
use super::backend::{RoadPolyline, RoadVec2, RoadVec3};
use super::input::NodeArrangementInput;
use super::joins::{NodeInputSideJoinBand, side_join_bands_by_mouth};
use super::keys::SURFACE_POLYLINE_POINT_EQUAL_EPS_M;
use super::terminal::{
    NodeTerminalCapBand, TerminalCapGenerationError, terminal_cap_bands_by_mouth,
};
use super::{RoadSurfaceBandKind, RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind};
use std::collections::BTreeMap;

mod bands;
mod caps_and_joins;
mod constraints;
mod contacts;
mod contours;
mod geometry;
mod owners;
mod source_points;
mod topology;

use bands::{push_band_contour, push_full_roadbed_contour, push_raw_carriageway_corridor_contour};
use caps_and_joins::{push_side_join_band_contours, push_terminal_cap_band_contours};
use contacts::{
    append_generated_material_point_contact_constraints,
    append_generated_same_band_contact_constraints,
    append_source_authorized_raised_step_point_contacts, node_generated_contact_contours,
    node_generated_contact_source_constraints,
    node_generated_contact_sources_from_contour_backed_contacts,
    retain_source_authorized_generated_contact_constraints,
    validate_generated_contact_constraint_endpoints_from_sources,
};
use contours::{push_boundary_constraint, push_span_handoff_constraint};
use owners::{boundary_owners, owners_by_mouth};
use source_points::{interval_height_carrier_points, push_band_height_carrier_points};

const RAIL_CONTOUR_POINT_EQUAL_EPS_M: f64 = SURFACE_POLYLINE_POINT_EQUAL_EPS_M;

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
    pub(crate) height_carrier_points_by_source:
        BTreeMap<(RoadSurfaceBandKind, usize, usize), Vec<RoadVec2>>,
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
    NonCanonicalGeneratedContactEndpoint {
        kind: NodeRailConstraintKind,
        mouth_order_index: usize,
        band_index: Option<usize>,
        owner: Option<NodeBandOwner>,
        opposite_owner: Option<NodeBandOwner>,
        point_x_key: i64,
        point_z_key: i64,
    },
    TerminalCapGeneration {
        error: TerminalCapGenerationError,
    },
}

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn build_node_rail_contours_from_input(
        input: &NodeArrangementInput,
    ) -> Result<NodeRailContourSet, NodeRailGenerationError> {
        NodeRailContourSet::from_input(input)
    }
}

impl NodeRailContourSet {
    pub(crate) fn from_input(
        input: &NodeArrangementInput,
    ) -> Result<Self, NodeRailGenerationError> {
        if input.mouths.is_empty() {
            return Err(NodeRailGenerationError::EmptyInput {
                node_id: input.node_id,
            });
        }

        let terminal_cap_bands_by_mouth = terminal_cap_bands_by_mouth(input)
            .map_err(|error| NodeRailGenerationError::TerminalCapGeneration { error })?;
        let side_join_bands_by_mouth = side_join_bands_by_mouth(input);
        let owners_by_mouth = owners_by_mouth(
            input,
            &terminal_cap_bands_by_mouth,
            &side_join_bands_by_mouth,
        );
        let mut contours = Vec::new();
        let mut constraints = Vec::new();
        let mut height_carrier_points_by_source =
            BTreeMap::<(RoadSurfaceBandKind, usize, usize), Vec<RoadVec2>>::new();

        for (mouth_index, (mouth, mouth_owners)) in
            input.mouths.iter().zip(&owners_by_mouth).enumerate()
        {
            let side_join_bands = side_join_bands_by_mouth
                .get(mouth_index)
                .map_or(&[] as &[NodeInputSideJoinBand], Vec::as_slice);
            let terminal_cap_bands = terminal_cap_bands_by_mouth
                .get(mouth_index)
                .map_or(&[] as &[NodeTerminalCapBand], Vec::as_slice);
            push_full_roadbed_contour(mouth, &mut contours, &mut constraints)?;
            push_raw_carriageway_corridor_contour(
                input.piece_kind,
                mouth,
                &mut contours,
                &mut constraints,
            )?;

            for (band_index, interval) in mouth.band_intervals.iter().enumerate() {
                push_band_height_carrier_points(
                    &mut height_carrier_points_by_source,
                    mouth.order_index,
                    interval.band_index,
                    interval.band_kind,
                    interval_height_carrier_points(interval),
                );
                let owner = mouth_owners.band_owners[band_index];
                push_band_contour(
                    input.piece_kind,
                    mouth,
                    interval,
                    owner,
                    &mut contours,
                    &mut constraints,
                )?;
            }
            for cap_band in terminal_cap_bands {
                push_band_height_carrier_points(
                    &mut height_carrier_points_by_source,
                    mouth.order_index,
                    cap_band.source_band_index,
                    cap_band.band_kind,
                    cap_band
                        .contour_world
                        .iter()
                        .chain(&cap_band.inner_path_world)
                        .chain(&cap_band.outer_path_world)
                        .copied(),
                );
            }
            for side_join_band in side_join_bands {
                push_band_height_carrier_points(
                    &mut height_carrier_points_by_source,
                    mouth.order_index,
                    side_join_band.source_band_index,
                    side_join_band.band_kind,
                    side_join_band.contour_world.iter().copied(),
                );
            }

            push_terminal_cap_band_contours(
                input.piece_kind,
                mouth,
                terminal_cap_bands,
                mouth_owners,
                &mouth_owners.terminal_cap_band_owners,
                &mut contours,
                &mut constraints,
            )?;
            push_side_join_band_contours(
                input.piece_kind,
                mouth,
                side_join_bands,
                mouth_owners,
                &mouth_owners.side_join_band_owners,
                &mut contours,
                &mut constraints,
            )?;
            for boundary_rail in &mouth.boundary_rails {
                let (owner, opposite_owner) =
                    boundary_owners(boundary_rail.boundary_index, &mouth_owners.band_owners);
                push_boundary_constraint(
                    mouth,
                    boundary_rail.boundary_index,
                    boundary_rail.role,
                    owner,
                    opposite_owner,
                    &mut constraints,
                )?;
            }

            for profile_rail in &mouth.mouth_rails {
                let owner = mouth_owners.band_owners[profile_rail.band_index];
                push_span_handoff_constraint(mouth, profile_rail, owner, &mut constraints)?;
            }
        }
        let source_constraint_count = constraints.len();
        node_generated_contact_contours(&mut contours, &mut constraints)?;
        append_source_authorized_raised_step_point_contacts(
            input.piece_kind,
            &contours,
            &mut constraints,
        );
        append_generated_material_point_contact_constraints(&contours, &mut constraints);
        append_source_authorized_raised_step_point_contacts(
            input.piece_kind,
            &contours,
            &mut constraints,
        );
        node_generated_contact_contours(&mut contours, &mut constraints)?;
        append_generated_same_band_contact_constraints(
            input.piece_kind,
            &contours,
            &mut constraints,
        );
        node_generated_contact_contours(&mut contours, &mut constraints)?;
        let mut validation_constraints = constraints.clone();
        node_generated_contact_source_constraints(
            &contours,
            &mut validation_constraints,
            source_constraint_count,
        );
        node_generated_contact_sources_from_contour_backed_contacts(
            &contours,
            &mut validation_constraints,
            source_constraint_count,
        );
        let authority_constraints = validation_constraints.clone();
        retain_source_authorized_generated_contact_constraints(
            &contours,
            &authority_constraints,
            &mut constraints,
            source_constraint_count,
        );
        retain_source_authorized_generated_contact_constraints(
            &contours,
            &authority_constraints,
            &mut validation_constraints,
            source_constraint_count,
        );
        validate_generated_contact_constraint_endpoints_from_sources(
            &contours,
            &validation_constraints,
            source_constraint_count,
        )?;
        Ok(Self {
            node_id: input.node_id,
            piece_kind: input.piece_kind,
            contours,
            constraints,
            height_carrier_points_by_source,
        })
    }
}

#[cfg(test)]
mod tests;
