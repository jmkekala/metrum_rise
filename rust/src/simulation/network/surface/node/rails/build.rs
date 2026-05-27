//! Rail contour construction from validated node arrangement input.

use super::super::input::NodeArrangementInput;
use super::super::joins::{NodeInputSideJoinBand, side_join_bands_by_mouth};
use super::super::ownership::NodeSourceCarrierRegistry;
use super::super::terminal::{NodeTerminalCapBand, terminal_cap_bands_by_mouth};
use super::bands::{
    push_band_contour, push_full_roadbed_contour, push_raw_carriageway_corridor_contour,
};
use super::caps_and_joins::{push_side_join_band_contours, push_terminal_cap_band_contours};
use super::contacts::{
    append_generated_material_point_contact_constraints,
    append_generated_same_band_contact_constraints,
    append_source_authorized_raised_step_point_contacts, node_generated_contact_contours,
    node_generated_contact_source_constraints,
    node_generated_contact_sources_from_contour_backed_contacts,
    retain_source_authorized_generated_contact_constraints,
    validate_generated_contact_constraint_endpoints_from_sources,
};
use super::contours::{push_boundary_constraint, push_span_handoff_constraint};
use super::owners::{boundary_owners, owners_by_mouth};
use super::source_points::{
    interval_height_carrier_paths, interval_height_carrier_points, push_band_height_carrier_points,
};
use super::{
    NodeRailContourSet, NodeRailGenerationError, NodeRailHeightCarrierPaths, RoadSurfaceBandKind,
    RoadSurfaceSystem,
};
use std::collections::BTreeMap;

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
        let side_join_bands_by_mouth = side_join_bands_by_mouth(input)
            .map_err(|error| NodeRailGenerationError::SideJoinGeneration { error })?;
        let owners_by_mouth = owners_by_mouth(
            input,
            &terminal_cap_bands_by_mouth,
            &side_join_bands_by_mouth,
        );
        let mut contours = Vec::new();
        let mut constraints = Vec::new();
        let mut height_carrier_paths_by_source =
            BTreeMap::<(RoadSurfaceBandKind, usize, usize), Vec<NodeRailHeightCarrierPaths>>::new();
        let mut height_carrier_points_by_source =
            BTreeMap::<(RoadSurfaceBandKind, usize, usize), Vec<_>>::new();

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
                let height_carrier_paths = interval_height_carrier_paths(interval);
                height_carrier_paths_by_source
                    .entry((interval.band_kind, mouth.order_index, interval.band_index))
                    .or_default()
                    .push(height_carrier_paths.clone());
                push_band_height_carrier_points(
                    &mut height_carrier_points_by_source,
                    mouth.order_index,
                    interval.band_index,
                    interval.band_kind,
                    interval_height_carrier_points(interval, &height_carrier_paths),
                )?;
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
                height_carrier_paths_by_source
                    .entry((
                        cap_band.band_kind,
                        mouth.order_index,
                        cap_band.source_band_index,
                    ))
                    .or_default()
                    .push(NodeRailHeightCarrierPaths {
                        start_path_world: cap_band.inner_path_world.clone(),
                        end_path_world: cap_band.outer_path_world.clone(),
                    });
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
                )?;
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
            source_constraint_count,
            &mut constraints,
        );
        append_generated_material_point_contact_constraints(&contours, &mut constraints);
        append_source_authorized_raised_step_point_contacts(
            input.piece_kind,
            &contours,
            source_constraint_count,
            &mut constraints,
        );
        node_generated_contact_contours(&mut contours, &mut constraints)?;
        append_generated_same_band_contact_constraints(
            input.piece_kind,
            &contours,
            source_constraint_count,
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
        let source_carriers = NodeSourceCarrierRegistry::from_rail_parts(
            &contours,
            &constraints,
            &height_carrier_paths_by_source,
            &height_carrier_points_by_source,
        );
        Ok(Self {
            node_id: input.node_id,
            piece_kind: input.piece_kind,
            contours,
            constraints,
            height_carrier_paths_by_source,
            height_carrier_points_by_source,
            source_carriers,
        })
    }
}
