// SPDX-License-Identifier: GPL-2.0-only

//! Source-constraint noding for generated rail contacts.

use super::super::source_authority::{
    GeneratedSameBandContactConstraint, generated_contact_kind_from_constraint,
    generated_same_band_contact_constraint,
};
use super::super::{
    GeneratedContourDirectedEdge, NodeBandOwner, NodeGeneratedContour,
    NodeGeneratedContourClaimPriority, NodeRailConstraint, NodeRailConstraintKind,
    NodeRailPointKey, generated_constraint_directed_edges, generated_contour_directed_edges,
    generated_contour_keys, generated_point_key_lies_on_segment,
    quantized_proper_segment_intersection,
};
use super::ContactInsertionsByIndex;
use super::insertion::insert_keys_on_generated_source_constraints;
use std::collections::BTreeMap;

struct PreparedContactNodingContour<'a> {
    contour: &'a NodeGeneratedContour,
    keys: Vec<NodeRailPointKey>,
    edges: Vec<GeneratedContourDirectedEdge>,
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ContactSourceConstraintSelector {
    kind: NodeRailConstraintKind,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    lower_owner: NodeBandOwner,
    upper_owner: NodeBandOwner,
}

struct PreparedContactSourceConstraint {
    constraint_index: usize,
    edges: Vec<GeneratedContourDirectedEdge>,
}

type ContactSourceContourSelector = (usize, usize, NodeBandOwner);

pub(in crate::simulation::network::surface::node::rails) fn node_generated_contact_source_constraints(
    contours: &[NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
    source_constraint_count: usize,
) {
    let source_constraint_count = source_constraint_count.min(constraints.len());
    if source_constraint_count == 0 {
        return;
    }
    let insertions = generated_contact_source_constraint_noding_candidates(
        contours,
        &constraints[..source_constraint_count],
    );
    insert_keys_on_generated_source_constraints(
        &mut constraints[..source_constraint_count],
        insertions,
    );
}

fn generated_contact_source_constraint_noding_candidates(
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
) -> ContactInsertionsByIndex {
    let prepared_contours = contours
        .iter()
        .map(PreparedContactNodingContour::new)
        .collect::<Vec<_>>();
    let mut candidates = ContactInsertionsByIndex::new();
    for constraint in constraints {
        if generated_contact_kind_from_constraint(constraint.kind).is_none()
            || constraint.owner.is_none()
            || constraint.opposite_owner.is_none()
        {
            continue;
        }
        let source_edges = generated_constraint_directed_edges(constraint);
        let source_bounds = directed_edges_bounds(&source_edges);
        for prepared in &prepared_contours {
            if !generated_contact_source_constraint_can_node_with_contour(
                constraint,
                prepared.contour,
            ) || prepared.bounds_disjoint(source_bounds)
            {
                continue;
            }
            for source_edge in source_edges.iter().copied() {
                if prepared.bounds_disjoint_edge(source_edge) {
                    continue;
                }
                for &point in &prepared.keys {
                    if generated_point_key_lies_on_segment(
                        point,
                        source_edge.start,
                        source_edge.end,
                    ) {
                        candidates
                            .entry(constraint.constraint_index)
                            .or_default()
                            .entry(source_edge)
                            .or_default()
                            .insert(point);
                    }
                }
                for &contour_edge in &prepared.edges {
                    if let Some(point) = quantized_proper_segment_intersection(
                        source_edge.start,
                        source_edge.end,
                        contour_edge.start,
                        contour_edge.end,
                    ) {
                        candidates
                            .entry(constraint.constraint_index)
                            .or_default()
                            .entry(source_edge)
                            .or_default()
                            .insert(point);
                    }
                }
            }
        }
    }
    candidates
}

impl<'a> PreparedContactNodingContour<'a> {
    fn new(contour: &'a NodeGeneratedContour) -> Self {
        let keys = generated_contour_keys(contour);
        let (mut min_x, mut min_z) = (i64::MAX, i64::MAX);
        let (mut max_x, mut max_z) = (i64::MIN, i64::MIN);
        for &(x, z) in &keys {
            min_x = min_x.min(x);
            min_z = min_z.min(z);
            max_x = max_x.max(x);
            max_z = max_z.max(z);
        }
        if keys.is_empty() {
            min_x = 1;
            min_z = 1;
            max_x = 0;
            max_z = 0;
        }
        Self {
            contour,
            edges: generated_contour_directed_edges(contour),
            keys,
            min_x,
            min_z,
            max_x,
            max_z,
        }
    }

    fn bounds_disjoint(&self, bounds: (i64, i64, i64, i64)) -> bool {
        self.max_x < bounds.0
            || bounds.2 < self.min_x
            || self.max_z < bounds.1
            || bounds.3 < self.min_z
    }

    fn bounds_disjoint_edge(&self, edge: GeneratedContourDirectedEdge) -> bool {
        self.max_x < edge.start.0.min(edge.end.0)
            || edge.start.0.max(edge.end.0) < self.min_x
            || self.max_z < edge.start.1.min(edge.end.1)
            || edge.start.1.max(edge.end.1) < self.min_z
    }
}

fn directed_edges_bounds(edges: &[GeneratedContourDirectedEdge]) -> (i64, i64, i64, i64) {
    let (mut min_x, mut min_z) = (i64::MAX, i64::MAX);
    let (mut max_x, mut max_z) = (i64::MIN, i64::MIN);
    for edge in edges {
        min_x = min_x.min(edge.start.0).min(edge.end.0);
        min_z = min_z.min(edge.start.1).min(edge.end.1);
        max_x = max_x.max(edge.start.0).max(edge.end.0);
        max_z = max_z.max(edge.start.1).max(edge.end.1);
    }
    if edges.is_empty() {
        (1, 1, 0, 0)
    } else {
        (min_x, min_z, max_x, max_z)
    }
}

fn generated_contact_source_constraint_can_node_with_contour(
    constraint: &NodeRailConstraint,
    contour: &NodeGeneratedContour,
) -> bool {
    let Some(contour_owner) = contour.owner else {
        return false;
    };
    let (Some(owner), Some(opposite_owner)) = (constraint.owner, constraint.opposite_owner) else {
        return false;
    };
    contour_owner == owner
        || contour_owner == opposite_owner
        || contour_owner.kind() == owner.kind()
        || contour_owner.kind() == opposite_owner.kind()
}

pub(in crate::simulation::network::surface::node::rails) fn node_generated_contact_sources_from_contour_backed_contacts(
    contours: &[NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
    generated_constraint_start_index: usize,
) {
    let generated_constraint_start_index = generated_constraint_start_index.min(constraints.len());
    let source_contour_edges = prepared_contact_source_contour_edges(contours);
    let source_constraints =
        prepared_contact_source_constraints(&constraints[..generated_constraint_start_index]);
    let mut insertions = ContactInsertionsByIndex::new();
    for constraint in constraints.iter().skip(generated_constraint_start_index) {
        let Some(contact) = generated_same_band_contact_constraint(constraint) else {
            continue;
        };
        for point in [contact.start, contact.end] {
            if !generated_contact_point_has_source_contour_authority(
                &source_contour_edges,
                contact,
                point,
            ) {
                continue;
            }
            let Some(selector) = ContactSourceConstraintSelector::from_contact(contact) else {
                continue;
            };
            if let Some(matching_constraints) = source_constraints.get(&selector) {
                for source_constraint in matching_constraints {
                    for &edge in &source_constraint.edges {
                        if generated_point_key_lies_on_segment(point, edge.start, edge.end) {
                            insertions
                                .entry(source_constraint.constraint_index)
                                .or_default()
                                .entry(edge)
                                .or_default()
                                .insert(point);
                        }
                    }
                }
            }
        }
    }
    insert_keys_on_generated_source_constraints(
        &mut constraints[..generated_constraint_start_index],
        insertions,
    );
}

fn generated_contact_point_has_source_contour_authority(
    source_contour_edges: &BTreeMap<
        ContactSourceContourSelector,
        Vec<GeneratedContourDirectedEdge>,
    >,
    contact: GeneratedSameBandContactConstraint,
    point: NodeRailPointKey,
) -> bool {
    let Some(source_band_index) = contact.source_band_index else {
        return false;
    };
    [contact.owner, contact.opposite_owner]
        .into_iter()
        .filter_map(|owner| {
            source_contour_edges.get(&(contact.source_mouth_order_index, source_band_index, owner))
        })
        .flatten()
        .any(|edge| generated_point_key_lies_on_segment(point, edge.start, edge.end))
}

impl ContactSourceConstraintSelector {
    fn new(
        kind: NodeRailConstraintKind,
        source_mouth_order_index: usize,
        source_band_index: Option<usize>,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
    ) -> Self {
        let (lower_owner, upper_owner) = if owner <= opposite_owner {
            (owner, opposite_owner)
        } else {
            (opposite_owner, owner)
        };
        Self {
            kind,
            source_mouth_order_index,
            source_band_index,
            lower_owner,
            upper_owner,
        }
    }

    fn from_constraint(constraint: &NodeRailConstraint) -> Option<Self> {
        Some(Self::new(
            constraint.kind,
            constraint.source_mouth_order_index,
            constraint.source_band_index,
            constraint.owner?,
            constraint.opposite_owner?,
        ))
    }

    fn from_contact(contact: GeneratedSameBandContactConstraint) -> Option<Self> {
        Some(Self::new(
            contact.kind,
            contact.source_mouth_order_index,
            contact.source_band_index,
            contact.owner,
            contact.opposite_owner,
        ))
    }
}

fn prepared_contact_source_contour_edges(
    contours: &[NodeGeneratedContour],
) -> BTreeMap<ContactSourceContourSelector, Vec<GeneratedContourDirectedEdge>> {
    let mut edges_by_source = BTreeMap::new();
    for contour in contours {
        let (Some(owner), Some(source_band_index)) = (contour.owner, contour.source_band_index)
        else {
            continue;
        };
        if contour.claim_priority != NodeGeneratedContourClaimPriority::MouthBand {
            continue;
        }
        edges_by_source
            .entry((contour.source_mouth_order_index, source_band_index, owner))
            .or_insert_with(Vec::new)
            .extend(generated_contour_directed_edges(contour));
    }
    edges_by_source
}

fn prepared_contact_source_constraints(
    constraints: &[NodeRailConstraint],
) -> BTreeMap<ContactSourceConstraintSelector, Vec<PreparedContactSourceConstraint>> {
    let mut constraints_by_selector = BTreeMap::new();
    for constraint in constraints {
        let Some(selector) = ContactSourceConstraintSelector::from_constraint(constraint) else {
            continue;
        };
        constraints_by_selector
            .entry(selector)
            .or_insert_with(Vec::new)
            .push(PreparedContactSourceConstraint {
                constraint_index: constraint.constraint_index,
                edges: generated_constraint_directed_edges(constraint),
            });
    }
    constraints_by_selector
}
