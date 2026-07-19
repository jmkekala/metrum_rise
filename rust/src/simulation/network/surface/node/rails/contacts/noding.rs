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
    current_cache.begin_noding_call();
    let component_plans =
        candidates::generated_contact_noding_component_plans(contours, constraints);
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
    if all_components_reused {
        return Ok(stats);
    }

    let max_passes = contours.len().saturating_mul(contours.len()).max(1) * 4;
    let mut previous_candidates = None;
    let mut attempted_insertions = BTreeSet::<(usize, NodeRailPointKey)>::new();
    for _ in 0..max_passes {
        let (mut candidates, pass_stats) =
            candidates::generated_contact_contour_noding_candidates_with_reuse(
                contours,
                constraints,
                previous_cache,
                current_cache,
            );
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
        if !insertion::insert_contact_noding_candidates(contours, constraints, &candidates)? {
            break;
        }
        previous_candidates = Some(candidates);
    }
    retain_contact_noding_component_outputs(contours, constraints, &component_plans, current_cache);
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
