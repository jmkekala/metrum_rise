// SPDX-License-Identifier: GPL-2.0-only

//! Canonical contact-point insertion for generated rail contacts.

mod candidates;
mod insertion;
mod source_constraints;

use super::{
    GeneratedContourDirectedEdge, NodeGeneratedContour, NodeRailConstraint,
    NodeRailGenerationError, NodeRailPointKey, generated_contour_keys, road_point_from_key,
    set_generated_contour_from_keys,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

pub(in crate::simulation::network::surface::node::rails) use candidates::{
    NodeContactNodingPairCache, NodeContactNodingReuseStats,
};

type ContactNodingCandidate = (usize, GeneratedContourDirectedEdge, NodeRailPointKey);
type ContactEdgeInsertions = BTreeMap<GeneratedContourDirectedEdge, BTreeSet<NodeRailPointKey>>;
type ContactInsertionsByIndex = BTreeMap<usize, ContactEdgeInsertions>;

pub(in crate::simulation::network::surface::node::rails) use source_constraints::{
    node_generated_contact_source_constraints,
    node_generated_contact_sources_from_contour_backed_contacts,
};

/// Nodes generated contacts while retaining exact pair-local candidates for later passes.
pub(in crate::simulation::network::surface::node::rails) fn node_generated_contact_contours_with_reuse(
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
    previous_cache: Option<&NodeContactNodingPairCache>,
    current_cache: &mut NodeContactNodingPairCache,
) -> Result<NodeContactNodingReuseStats, NodeRailGenerationError> {
    node_generated_contact_contours_with_reuse_mode(
        contours,
        constraints,
        previous_cache,
        current_cache,
        true,
    )
}

/// Nodes contacts with pair-local reuse but defers final-component cache construction.
pub(in crate::simulation::network::surface::node::rails) fn node_generated_contact_contours_with_pair_reuse(
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
    previous_cache: Option<&NodeContactNodingPairCache>,
    current_cache: &mut NodeContactNodingPairCache,
) -> Result<NodeContactNodingReuseStats, NodeRailGenerationError> {
    node_generated_contact_contours_with_reuse_mode(
        contours,
        constraints,
        previous_cache,
        current_cache,
        false,
    )
}

fn node_generated_contact_contours_with_reuse_mode(
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
    previous_cache: Option<&NodeContactNodingPairCache>,
    current_cache: &mut NodeContactNodingPairCache,
    reuse_final_components: bool,
) -> Result<NodeContactNodingReuseStats, NodeRailGenerationError> {
    let profile_enabled = crate::debug::category_enabled("road");
    let total_start = profile_enabled.then(Instant::now);
    current_cache.begin_noding_call();
    let preparation_start = profile_enabled.then(Instant::now);
    let initial_preparation = candidates::prepare_contact_noding_pass(contours, constraints);
    let preparation_ms = preparation_start
        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    let component_start = profile_enabled.then(Instant::now);
    let component_plans = reuse_final_components
        .then(|| candidates::generated_contact_noding_component_plans(&initial_preparation))
        .unwrap_or_default();
    let component_ms = component_start
        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    let mut stats = NodeContactNodingReuseStats::default();
    let mut all_components_reused = true;
    for plan in &component_plans {
        let cached = current_cache
            .component_entries
            .get(&plan.key)
            .cloned()
            .or_else(|| {
                previous_cache
                    .and_then(|cache| cache.component_entries.get(&plan.key))
                    .cloned()
            });
        let Some(cached) =
            cached.filter(|output| output.contour_keys.len() == plan.contour_indices.len())
        else {
            stats.component_cache_misses += 1;
            all_components_reused = false;
            continue;
        };
        stats.component_cache_hits += 1;
        replay_contact_noding_component(contours, constraints, plan, &cached)?;
        current_cache.promote_component_pair_entries(&cached);
        current_cache
            .component_entries
            .insert(plan.key.clone(), cached);
    }
    if reuse_final_components && all_components_reused {
        return Ok(stats);
    }

    let max_passes = contours.len().saturating_mul(contours.len()).max(1) * 4;
    let mut previous_candidates = None;
    let mut preparation = initial_preparation;
    let mut attempted_insertions = BTreeSet::<(usize, NodeRailPointKey)>::new();
    let mut candidate_ms = 0.0;
    let mut insertion_ms = 0.0;
    let mut refresh_ms = 0.0;
    let mut pass_count = 0;
    for _ in 0..max_passes {
        pass_count += 1;
        let candidate_start = profile_enabled.then(Instant::now);
        let (mut candidates, pass_stats) =
            candidates::generated_contact_contour_noding_candidates_from_preparation_with_reuse(
                &preparation,
                previous_cache,
                current_cache,
            );
        candidate_ms += candidate_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        stats.merge(pass_stats);
        candidates.retain(|(contour_index, _, insert_key)| {
            !attempted_insertions.contains(&(*contour_index, *insert_key))
        });
        if candidates.is_empty() {
            break;
        };
        if previous_candidates.as_ref() == Some(&candidates) {
            break;
        }
        attempted_insertions.extend(
            candidates
                .iter()
                .map(|(contour_index, _, insert_key)| (*contour_index, *insert_key)),
        );
        let insertion_start = profile_enabled.then(Instant::now);
        if !insertion::insert_contact_noding_candidates(contours, constraints, &candidates)? {
            break;
        }
        insertion_ms += insertion_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let refresh_start = profile_enabled.then(Instant::now);
        candidates::refresh_contact_noding_pass(&mut preparation, contours, &candidates);
        refresh_ms += refresh_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        previous_candidates = Some(candidates);
    }
    if reuse_final_components {
        retain_contact_noding_component_outputs(
            contours,
            constraints,
            &component_plans,
            current_cache,
        );
    }
    if let Some(total_start) = total_start {
        let total_ms = total_start.elapsed().as_secs_f64() * 1000.0;
        if total_ms >= 0.5 {
            crate::debug_log!(
                "road",
                "contact_noding_detail contours={} constraints={} final_components={} passes={} pair_hits={} pair_misses={} preparation_ms={:.3} component_ms={:.3} candidate_ms={:.3} insertion_ms={:.3} refresh_ms={:.3} total_ms={:.3}",
                contours.len(),
                constraints.len(),
                reuse_final_components,
                pass_count,
                stats.pair_cache_hits,
                stats.pair_cache_misses,
                preparation_ms,
                component_ms,
                candidate_ms,
                insertion_ms,
                refresh_ms,
                total_ms,
            );
        }
    }
    Ok(stats)
}

fn replay_contact_noding_component(
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
    plan: &candidates::ContactNodingComponentPlan,
    output: &candidates::ContactNodingComponentOutput,
) -> Result<(), NodeRailGenerationError> {
    for (&contour_index, contour_keys) in
        plan.contour_indices.iter().zip(output.contour_keys.iter())
    {
        let Some(contour) = contours.get_mut(contour_index) else {
            continue;
        };
        if generated_contour_keys(contour).as_slice() == contour_keys.as_ref() {
            continue;
        }
        set_generated_contour_from_keys(contour, constraints, contour_keys.to_vec())?;
    }
    for constraint_output in output.band_constraints.iter() {
        let points_xz = constraint_output
            .ordered_points_xz
            .iter()
            .copied()
            .map(road_point_from_key)
            .collect::<Vec<_>>();
        for constraint in constraints
            .iter_mut()
            .filter(|constraint| constraint_output.selector.matches_constraint(constraint))
        {
            constraint.points_xz.clone_from(&points_xz);
        }
    }
    Ok(())
}

fn retain_contact_noding_component_outputs(
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
    plans: &[candidates::ContactNodingComponentPlan],
    current_cache: &mut NodeContactNodingPairCache,
) {
    for plan in plans {
        let outputs = plan
            .contour_indices
            .iter()
            .filter_map(|index| contours.get(*index))
            .map(|contour| Arc::from(generated_contour_keys(contour)))
            .collect::<Vec<_>>();
        if outputs.len() == plan.contour_indices.len() {
            let band_constraints = plan
                .affected_band_constraint_selectors(contours)
                .into_iter()
                .filter_map(|selector| {
                    constraints
                        .iter()
                        .find(|constraint| selector.matches_constraint(constraint))
                        .map(|constraint| candidates::ContactNodingBandConstraintOutput {
                            selector,
                            ordered_points_xz: constraint
                                .points_xz
                                .iter()
                                .copied()
                                .map(super::road_point_key)
                                .collect(),
                        })
                })
                .collect::<Vec<_>>();
            let output = current_cache.component_output(
                &plan.band_constraint_selectors,
                Arc::from(outputs),
                Arc::from(band_constraints),
            );
            current_cache
                .component_entries
                .insert(plan.key.clone(), output);
        }
    }
}
