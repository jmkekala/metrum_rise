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
    NodeRailBuildProfile, NodeRailContourSet, NodeRailGenerationError, NodeRailHeightCarrierPaths,
    RoadSurfaceBandKind, RoadSurfaceSystem,
};
use std::collections::BTreeMap;
use std::time::Instant;

fn elapsed_profile_ms(start: Option<Instant>) -> f64 {
    start
        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

impl RoadSurfaceSystem {
    #[cfg(test)]
    pub(in crate::simulation::network::surface) fn build_node_rail_contours_from_input(
        input: &NodeArrangementInput,
    ) -> Result<NodeRailContourSet, NodeRailGenerationError> {
        NodeRailContourSet::from_input(input)
    }

    pub(in crate::simulation::network::surface) fn build_node_rail_contours_from_input_with_profile(
        input: &NodeArrangementInput,
        profile_enabled: bool,
    ) -> Result<(NodeRailContourSet, NodeRailBuildProfile), NodeRailGenerationError> {
        NodeRailContourSet::from_input_with_profile(input, profile_enabled)
    }
}

impl NodeRailContourSet {
    #[cfg(test)]
    pub(crate) fn from_input(
        input: &NodeArrangementInput,
    ) -> Result<Self, NodeRailGenerationError> {
        Self::from_input_with_profile(input, false).map(|(rails, _)| rails)
    }

    pub(crate) fn from_input_with_profile(
        input: &NodeArrangementInput,
        profile_enabled: bool,
    ) -> Result<(Self, NodeRailBuildProfile), NodeRailGenerationError> {
        let total_start = profile_enabled.then(Instant::now);
        let mut profile = NodeRailBuildProfile {
            mouths: input.mouths.len(),
            ..NodeRailBuildProfile::default()
        };
        if input.mouths.is_empty() {
            return Err(NodeRailGenerationError::EmptyInput {
                node_id: input.node_id,
            });
        }

        let terminal_caps_start = profile_enabled.then(Instant::now);
        let terminal_cap_bands_by_mouth = terminal_cap_bands_by_mouth(input)
            .map_err(|error| NodeRailGenerationError::TerminalCapGeneration { error })?;
        profile.terminal_caps_ms = elapsed_profile_ms(terminal_caps_start);
        let side_joins_start = profile_enabled.then(Instant::now);
        let side_join_bands_by_mouth = side_join_bands_by_mouth(input)
            .map_err(|error| NodeRailGenerationError::SideJoinGeneration { error })?;
        profile.side_joins_ms = elapsed_profile_ms(side_joins_start);
        let owners_start = profile_enabled.then(Instant::now);
        let owners_by_mouth = owners_by_mouth(
            input,
            &terminal_cap_bands_by_mouth,
            &side_join_bands_by_mouth,
        );
        profile.owners_ms = elapsed_profile_ms(owners_start);
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
            let base_start = profile_enabled.then(Instant::now);
            push_full_roadbed_contour(mouth, &mut contours, &mut constraints)?;
            push_raw_carriageway_corridor_contour(
                input.piece_kind,
                mouth,
                &mut contours,
                &mut constraints,
            )?;
            profile.mouth_base_contours_ms += elapsed_profile_ms(base_start);

            let band_start = profile_enabled.then(Instant::now);
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
            profile.mouth_band_contours_ms += elapsed_profile_ms(band_start);
            let cap_carrier_start = profile_enabled.then(Instant::now);
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
            profile.cap_height_carriers_ms += elapsed_profile_ms(cap_carrier_start);
            let terminal_cap_contours_start = profile_enabled.then(Instant::now);
            push_terminal_cap_band_contours(
                input.piece_kind,
                mouth,
                terminal_cap_bands,
                mouth_owners,
                &mouth_owners.terminal_cap_band_owners,
                &mut contours,
                &mut constraints,
            )?;
            profile.terminal_cap_contours_ms += elapsed_profile_ms(terminal_cap_contours_start);
            let side_join_contours_start = profile_enabled.then(Instant::now);
            push_side_join_band_contours(
                input.piece_kind,
                mouth,
                side_join_bands,
                mouth_owners,
                &mouth_owners.side_join_band_owners,
                &mut contours,
                &mut constraints,
            )?;
            profile.side_join_contours_ms += elapsed_profile_ms(side_join_contours_start);
            let boundary_constraints_start = profile_enabled.then(Instant::now);
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
            profile.boundary_constraints_ms += elapsed_profile_ms(boundary_constraints_start);

            let span_handoff_start = profile_enabled.then(Instant::now);
            for profile_rail in &mouth.mouth_rails {
                let owner = mouth_owners.band_owners[profile_rail.band_index];
                push_span_handoff_constraint(mouth, profile_rail, owner, &mut constraints)?;
            }
            profile.span_handoff_ms += elapsed_profile_ms(span_handoff_start);
        }
        let source_constraint_count = constraints.len();
        profile.source_constraints = source_constraint_count;
        let contact_noding_first_start = profile_enabled.then(Instant::now);
        node_generated_contact_contours(&mut contours, &mut constraints)?;
        profile.contact_noding_first_ms = elapsed_profile_ms(contact_noding_first_start);
        let raised_step_contacts_first_start = profile_enabled.then(Instant::now);
        profile.contact_constraints_emitted += append_source_authorized_raised_step_point_contacts(
            input.piece_kind,
            &contours,
            source_constraint_count,
            &mut constraints,
        );
        profile.raised_step_contacts_first_ms =
            elapsed_profile_ms(raised_step_contacts_first_start);
        let material_contacts_start = profile_enabled.then(Instant::now);
        let material_contact_profile =
            append_generated_material_point_contact_constraints(&contours, &mut constraints);
        profile.contact_pair_tests += material_contact_profile.pair_tests;
        profile.contact_pair_aabb_rejected += material_contact_profile.aabb_rejected;
        profile.contact_pair_kind_rejected += material_contact_profile.kind_rejected;
        profile.contact_pair_processed += material_contact_profile.processed_pairs;
        profile.contact_overlay_calls += material_contact_profile.overlay_calls;
        profile.contact_constraints_emitted += material_contact_profile.emitted_constraints;
        profile.material_contacts_ms = elapsed_profile_ms(material_contacts_start);
        let raised_step_contacts_second_start = profile_enabled.then(Instant::now);
        profile.contact_constraints_emitted += append_source_authorized_raised_step_point_contacts(
            input.piece_kind,
            &contours,
            source_constraint_count,
            &mut constraints,
        );
        profile.raised_step_contacts_second_ms =
            elapsed_profile_ms(raised_step_contacts_second_start);
        let contact_noding_second_start = profile_enabled.then(Instant::now);
        node_generated_contact_contours(&mut contours, &mut constraints)?;
        profile.contact_noding_second_ms = elapsed_profile_ms(contact_noding_second_start);
        let same_band_contacts_start = profile_enabled.then(Instant::now);
        let same_band_contact_profile = append_generated_same_band_contact_constraints(
            input.piece_kind,
            &contours,
            source_constraint_count,
            &mut constraints,
        );
        profile.contact_pair_tests += same_band_contact_profile.pair_tests;
        profile.contact_pair_aabb_rejected += same_band_contact_profile.aabb_rejected;
        profile.contact_pair_kind_rejected += same_band_contact_profile.kind_rejected;
        profile.contact_pair_processed += same_band_contact_profile.processed_pairs;
        profile.contact_overlay_calls += same_band_contact_profile.overlay_calls;
        profile.contact_constraints_emitted += same_band_contact_profile.emitted_constraints;
        profile.same_band_contacts_ms = elapsed_profile_ms(same_band_contacts_start);
        let contact_noding_third_start = profile_enabled.then(Instant::now);
        node_generated_contact_contours(&mut contours, &mut constraints)?;
        profile.contact_noding_third_ms = elapsed_profile_ms(contact_noding_third_start);
        let validation_source_constraints_start = profile_enabled.then(Instant::now);
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
        profile.validation_source_constraints_ms =
            elapsed_profile_ms(validation_source_constraints_start);
        let retain_constraints_start = profile_enabled.then(Instant::now);
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
        profile.retain_constraints_ms = elapsed_profile_ms(retain_constraints_start);
        let validate_endpoints_start = profile_enabled.then(Instant::now);
        validate_generated_contact_constraint_endpoints_from_sources(
            &contours,
            &validation_constraints,
            source_constraint_count,
        )?;
        profile.validate_endpoints_ms = elapsed_profile_ms(validate_endpoints_start);
        let source_carriers_start = profile_enabled.then(Instant::now);
        let source_carriers = NodeSourceCarrierRegistry::from_rail_parts(
            &contours,
            &constraints,
            &height_carrier_paths_by_source,
            &height_carrier_points_by_source,
        );
        profile.source_carriers_ms = elapsed_profile_ms(source_carriers_start);
        profile.contours = contours.len();
        profile.constraints = constraints.len();
        profile.validation_constraints = validation_constraints.len();
        profile.height_carrier_sources = height_carrier_points_by_source.len();
        profile.height_carrier_points =
            height_carrier_points_by_source.values().map(Vec::len).sum();
        profile.total_ms = elapsed_profile_ms(total_start);
        Ok((
            Self {
                node_id: input.node_id,
                piece_kind: input.piece_kind,
                contours,
                constraints,
                height_carrier_paths_by_source,
                height_carrier_points_by_source,
                source_carriers,
            },
            profile,
        ))
    }
}
