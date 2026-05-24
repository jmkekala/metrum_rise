//! Canonical contact-point insertion for generated rail contacts.

mod candidates;
mod insertion;
mod source_constraints;

use super::{
    GeneratedContourDirectedEdge, NodeGeneratedContour, NodeRailConstraint,
    NodeRailGenerationError, NodeRailPointKey,
};
use std::collections::{BTreeMap, BTreeSet};

type ContactNodingCandidate = (usize, GeneratedContourDirectedEdge, NodeRailPointKey);
type ContactEdgeInsertions = BTreeMap<GeneratedContourDirectedEdge, BTreeSet<NodeRailPointKey>>;
type ContactInsertionsByIndex = BTreeMap<usize, ContactEdgeInsertions>;

pub(in crate::simulation::network::surface::node::rails) use source_constraints::{
    node_generated_contact_source_constraints,
    node_generated_contact_sources_from_contour_backed_contacts,
};

pub(in crate::simulation::network::surface::node::rails) fn node_generated_contact_contours(
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
) -> Result<(), NodeRailGenerationError> {
    let max_passes = contours.len().saturating_mul(contours.len()).max(1) * 4;
    let mut previous_candidates = None;
    let mut attempted_insertions = BTreeSet::<(usize, NodeRailPointKey)>::new();
    for _ in 0..max_passes {
        let mut candidates =
            candidates::generated_contact_contour_noding_candidates(contours, constraints);
        candidates.retain(|(contour_index, _, insert_key)| {
            !attempted_insertions.contains(&(*contour_index, *insert_key))
        });
        if candidates.is_empty() {
            return Ok(());
        };
        if previous_candidates.as_ref() == Some(&candidates) {
            return Ok(());
        }
        attempted_insertions.extend(
            candidates
                .iter()
                .map(|(contour_index, _, insert_key)| (*contour_index, *insert_key)),
        );
        if !insertion::insert_contact_noding_candidates(contours, constraints, &candidates)? {
            return Ok(());
        }
        previous_candidates = Some(candidates);
    }
    Ok(())
}
