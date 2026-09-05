//! Rail contour construction from validated node arrangement input.

use super::super::input::NodeArrangementInput;
use super::super::joins::{NodeInputSideJoinBand, side_join_plan};
use super::super::ownership::NodeSourceCarrierRegistry;
use super::super::terminal::{NodeTerminalCapBand, terminal_cap_bands_by_mouth};
use super::bands::{push_band_contour, push_full_roadbed_contour};
use super::caps_and_joins::{push_side_join_band_contours, push_terminal_cap_band_contours};
use super::contacts::{
    NodeContactNodingReuseStats, NodeRetainedContactReuseStats, SourceAuthorizedContactReuseStats,
    append_generated_material_point_contact_constraints,
    append_generated_same_band_contact_constraints_with_source_reuse,
    append_source_authorized_raised_step_point_contacts_with_reuse,
    node_generated_contact_contours_with_pair_reuse, node_generated_contact_contours_with_reuse,
    node_generated_contact_source_constraints,
    node_generated_contact_sources_from_contour_backed_contacts,
    retain_source_authorized_generated_contact_constraint_sets_with_reuse,
    synchronize_shared_height_contact_vertices,
    validate_generated_contact_constraint_endpoints_with_authority,
};
use super::contours::{push_boundary_constraint, push_span_handoff_constraint};
use super::owners::{boundary_owners, owners_by_mouth};
use super::reuse::NodeRailIncrementalCache;
use super::source_points::{
    interval_height_carrier_paths, interval_height_carrier_points, push_band_height_carrier_points,
};
use super::{
    NodeGeneratedSideJoinGap, NodeRailBuildProfile, NodeRailContourSet, NodeRailGenerationError,
    NodeRailHeightCarrierPaths, RoadSurfaceBandKind, RoadSurfaceSystem,
};
use std::collections::BTreeMap;
use std::time::Instant;

fn elapsed_profile_ms(start: Option<Instant>) -> f64 {
    start
        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

fn include_source_authorized_reuse_profile(
    profile: &mut NodeRailBuildProfile,
    stats: SourceAuthorizedContactReuseStats,
) {
    profile.source_target_group_cache_hits += stats.target_group_cache_hits;
    profile.source_contact_cache_hits += stats.source_cache_hits;
    profile.source_contact_cache_misses += stats.source_cache_misses;
    profile.source_pair_cache_hits += stats.source_pair_cache_hits;
    profile.source_pair_cache_misses += stats.source_pair_cache_misses;
}

fn include_contact_noding_reuse_profile(
    profile: &mut NodeRailBuildProfile,
    stats: NodeContactNodingReuseStats,
) {
    profile.contact_noding_pair_cache_hits += stats.pair_cache_hits;
    profile.contact_noding_pair_cache_misses += stats.pair_cache_misses;
    profile.contact_noding_component_cache_hits += stats.component_cache_hits;
    profile.contact_noding_component_cache_misses += stats.component_cache_misses;
}

fn include_retained_contact_reuse_profile(
    profile: &mut NodeRailBuildProfile,
    stats: NodeRetainedContactReuseStats,
) {
    profile.retained_authority_cache_hits += stats.authority_cache_hits;
    profile.retained_authority_current_hits += stats.authority_current_hits;
    profile.retained_authority_previous_hits += stats.authority_previous_hits;
    profile.retained_authority_cache_misses += stats.authority_cache_misses;
    profile.retained_decision_cache_hits += stats.decision_cache_hits;
    profile.retained_decision_current_hits += stats.decision_current_hits;
    profile.retained_decision_previous_hits += stats.decision_previous_hits;
    profile.retained_decision_cache_misses += stats.decision_cache_misses;
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
        let (base, source_constraint_count, profile) =
            Self::base_from_input_with_profile(input, profile_enabled)?;
        Self::finish_base_with_profile(
            base,
            source_constraint_count,
            profile,
            profile_enabled,
            None,
        )
        .map(|(rails, profile, _)| (rails, profile))
    }

    pub(super) fn base_from_input_with_profile(
        input: &NodeArrangementInput,
        profile_enabled: bool,
    ) -> Result<(Self, usize, NodeRailBuildProfile), NodeRailGenerationError> {
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
        let side_join_plan = side_join_plan(input)
            .map_err(|error| NodeRailGenerationError::SideJoinGeneration { error })?;
        let side_join_gaps =
            NodeGeneratedSideJoinGap::from_side_join_gap_summaries(&side_join_plan.gap_summaries);
        profile.side_joins_ms = elapsed_profile_ms(side_joins_start);
        let owners_start = profile_enabled.then(Instant::now);
        let owners_by_mouth = owners_by_mouth(
            input,
            &terminal_cap_bands_by_mouth,
            &side_join_plan.bands_by_mouth,
        );
        profile.owners_ms = elapsed_profile_ms(owners_start);
        let mut contours = Vec::new();
        let mut corner_trims = Vec::new();
        let mut constraints = Vec::new();
        let mut height_carrier_paths_by_source =
            BTreeMap::<(RoadSurfaceBandKind, usize, usize), Vec<NodeRailHeightCarrierPaths>>::new();
        let mut height_carrier_points_by_source =
            BTreeMap::<(RoadSurfaceBandKind, usize, usize), Vec<_>>::new();

        for (mouth_index, (mouth, mouth_owners)) in
            input.mouths.iter().zip(&owners_by_mouth).enumerate()
        {
            let side_join_bands = side_join_plan
                .bands_by_mouth
                .get(mouth_index)
                .map_or(&[] as &[NodeInputSideJoinBand], Vec::as_slice);
            let terminal_cap_bands = terminal_cap_bands_by_mouth
                .get(mouth_index)
                .map_or(&[] as &[NodeTerminalCapBand], Vec::as_slice);
            let base_start = profile_enabled.then(Instant::now);
            push_full_roadbed_contour(mouth, &mut contours, &mut constraints)?;
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
                &mut corner_trims,
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
        profile.total_ms = elapsed_profile_ms(total_start);
        Ok((
            Self {
                node_id: input.node_id,
                piece_kind: input.piece_kind,
                contours,
                corner_trims,
                side_join_gaps,
                constraints,
                height_carrier_paths_by_source,
                height_carrier_points_by_source,
                source_carriers: NodeSourceCarrierRegistry::default(),
            },
            source_constraint_count,
            profile,
        ))
    }

    pub(super) fn finish_base_with_profile(
        base: Self,
        source_constraint_count: usize,
        mut profile: NodeRailBuildProfile,
        profile_enabled: bool,
        previous_incremental: Option<&NodeRailIncrementalCache>,
    ) -> Result<(Self, NodeRailBuildProfile, NodeRailIncrementalCache), NodeRailGenerationError>
    {
        let total_start = profile_enabled.then(Instant::now);
        let Self {
            node_id,
            piece_kind,
            mut contours,
            corner_trims,
            side_join_gaps,
            mut constraints,
            height_carrier_paths_by_source,
            height_carrier_points_by_source,
            source_carriers: _,
        } = base;
        let mut source_authorized_contacts = Default::default();
        let mut contact_noding_pairs = Default::default();
        let mut retained_contacts = Default::default();
        let contact_noding_first_start = profile_enabled.then(Instant::now);
        let noding_profile = node_generated_contact_contours_with_pair_reuse(
            &mut contours,
            &mut constraints,
            previous_incremental.map(|previous| &previous.contact_noding_pairs),
            &mut contact_noding_pairs,
        )?;
        include_contact_noding_reuse_profile(&mut profile, noding_profile);
        profile.contact_noding_first_ms = elapsed_profile_ms(contact_noding_first_start);
        let raised_step_contacts_first_start = profile_enabled.then(Instant::now);
        let (emitted, source_reuse) =
            append_source_authorized_raised_step_point_contacts_with_reuse(
                piece_kind,
                &contours,
                source_constraint_count,
                &mut constraints,
                previous_incremental.map(|previous| &previous.source_authorized_contacts),
                &mut source_authorized_contacts,
            );
        profile.contact_constraints_emitted += emitted;
        include_source_authorized_reuse_profile(&mut profile, source_reuse);
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
        profile.contact_candidate_pairs += material_contact_profile.candidate_pairs;
        profile.contact_same_material_candidate_pairs +=
            material_contact_profile.same_material_candidate_pairs;
        profile.contact_raised_step_candidate_pairs +=
            material_contact_profile.raised_step_candidate_pairs;
        profile.contact_authority_rejected += material_contact_profile.authority_rejected;
        profile.contact_same_authority_skipped += material_contact_profile.same_authority_skipped;
        profile.same_material_overlay_calls += material_contact_profile.same_material_overlay_calls;
        profile.same_material_height_split_candidates +=
            material_contact_profile.same_material_height_split_candidates;
        profile.same_material_height_split_appended +=
            material_contact_profile.same_material_height_split_appended;
        profile.same_material_height_split_duplicates +=
            material_contact_profile.same_material_height_split_duplicates;
        profile.material_contacts_ms = elapsed_profile_ms(material_contacts_start);
        let raised_step_contacts_second_start = profile_enabled.then(Instant::now);
        let (emitted, source_reuse) =
            append_source_authorized_raised_step_point_contacts_with_reuse(
                piece_kind,
                &contours,
                source_constraint_count,
                &mut constraints,
                previous_incremental.map(|previous| &previous.source_authorized_contacts),
                &mut source_authorized_contacts,
            );
        profile.contact_constraints_emitted += emitted;
        include_source_authorized_reuse_profile(&mut profile, source_reuse);
        profile.raised_step_contacts_second_ms =
            elapsed_profile_ms(raised_step_contacts_second_start);
        let contact_noding_second_start = profile_enabled.then(Instant::now);
        let noding_profile = node_generated_contact_contours_with_pair_reuse(
            &mut contours,
            &mut constraints,
            previous_incremental.map(|previous| &previous.contact_noding_pairs),
            &mut contact_noding_pairs,
        )?;
        include_contact_noding_reuse_profile(&mut profile, noding_profile);
        profile.contact_noding_second_ms = elapsed_profile_ms(contact_noding_second_start);
        let same_band_contacts_start = profile_enabled.then(Instant::now);
        let (same_band_contact_profile, same_material_pair_cache) =
            append_generated_same_band_contact_constraints_with_source_reuse(
                piece_kind,
                &contours,
                source_constraint_count,
                &mut constraints,
                previous_incremental.map(|previous| &previous.same_material_contact_pairs),
                previous_incremental.map(|previous| &previous.source_authorized_contacts),
                &mut source_authorized_contacts,
            );
        profile.contact_pair_tests += same_band_contact_profile.pair_tests;
        profile.contact_pair_aabb_rejected += same_band_contact_profile.aabb_rejected;
        profile.contact_pair_kind_rejected += same_band_contact_profile.kind_rejected;
        profile.contact_pair_processed += same_band_contact_profile.processed_pairs;
        profile.contact_overlay_calls += same_band_contact_profile.overlay_calls;
        profile.contact_constraints_emitted += same_band_contact_profile.emitted_constraints;
        profile.contact_candidate_pairs += same_band_contact_profile.candidate_pairs;
        profile.contact_same_material_candidate_pairs +=
            same_band_contact_profile.same_material_candidate_pairs;
        profile.contact_raised_step_candidate_pairs +=
            same_band_contact_profile.raised_step_candidate_pairs;
        profile.contact_authority_rejected += same_band_contact_profile.authority_rejected;
        profile.contact_same_authority_skipped += same_band_contact_profile.same_authority_skipped;
        profile.same_material_overlay_calls +=
            same_band_contact_profile.same_material_overlay_calls;
        profile.same_material_pair_cache_hits +=
            same_band_contact_profile.same_material_pair_cache_hits;
        profile.raised_step_pair_cache_previous_hits +=
            same_band_contact_profile.raised_step_pair_cache_previous_hits;
        profile.raised_step_pair_cache_misses +=
            same_band_contact_profile.raised_step_pair_cache_misses;
        profile.source_target_group_cache_hits +=
            same_band_contact_profile.source_target_group_cache_hits;
        profile.source_contact_cache_hits += same_band_contact_profile.source_contact_cache_hits;
        profile.source_contact_cache_misses +=
            same_band_contact_profile.source_contact_cache_misses;
        profile.source_pair_cache_hits += same_band_contact_profile.source_pair_cache_hits;
        profile.source_pair_cache_misses += same_band_contact_profile.source_pair_cache_misses;
        profile.same_material_height_split_candidates +=
            same_band_contact_profile.same_material_height_split_candidates;
        profile.same_material_height_split_appended +=
            same_band_contact_profile.same_material_height_split_appended;
        profile.same_material_height_split_duplicates +=
            same_band_contact_profile.same_material_height_split_duplicates;
        profile.same_band_contacts_ms = elapsed_profile_ms(same_band_contacts_start);
        let contact_noding_third_start = profile_enabled.then(Instant::now);
        let noding_profile = node_generated_contact_contours_with_reuse(
            &mut contours,
            &mut constraints,
            previous_incremental.map(|previous| &previous.contact_noding_pairs),
            &mut contact_noding_pairs,
        )?;
        include_contact_noding_reuse_profile(&mut profile, noding_profile);
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
        profile.validation_source_constraints_ms =
            elapsed_profile_ms(validation_source_constraints_start);
        let retain_constraints_start = profile_enabled.then(Instant::now);
        let (retention_authority, retention_reuse) =
            retain_source_authorized_generated_contact_constraint_sets_with_reuse(
                &contours,
                &mut constraints,
                &mut validation_constraints,
                source_constraint_count,
                previous_incremental.map(|previous| &previous.retained_contacts),
                &mut retained_contacts,
            );
        include_retained_contact_reuse_profile(&mut profile, retention_reuse);
        profile.retain_constraints_ms = elapsed_profile_ms(retain_constraints_start);
        let validate_endpoints_start = profile_enabled.then(Instant::now);
        validate_generated_contact_constraint_endpoints_with_authority(
            &validation_constraints,
            source_constraint_count,
            &retention_authority,
        )?;
        profile.validate_endpoints_ms = elapsed_profile_ms(validate_endpoints_start);
        synchronize_shared_height_contact_vertices(&mut contours, &constraints);
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
        profile.total_ms += elapsed_profile_ms(total_start);
        Ok((
            Self {
                node_id,
                piece_kind,
                contours,
                corner_trims,
                side_join_gaps,
                constraints,
                height_carrier_paths_by_source,
                height_carrier_points_by_source,
                source_carriers,
            },
            profile,
            NodeRailIncrementalCache {
                same_material_contact_pairs: same_material_pair_cache,
                source_authorized_contacts,
                contact_noding_pairs,
                retained_contacts,
            },
        ))
    }
}
