//! Source-authorized generated rail contact materialization.

use super::super::band_semantics::{raised_step_band_rank, raised_step_kinds_can_contact};
use super::*;

fn insert_generated_material_point_constraint(
    constraints: &mut Vec<NodeRailConstraint>,
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    point: NodeRailPointKey,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
) {
    let (owner, opposite_owner) = if kind == NodeRailConstraintKind::RaisedStepContact {
        let Some(pair) = GeneratedRaisedStepOwnerPair::new(owner, opposite_owner) else {
            return;
        };
        (pair.owner, pair.opposite_owner)
    } else {
        (owner, opposite_owner)
    };
    let edge = [road_point_from_key(point), road_point_from_key(point)];
    if constraints.iter().any(|constraint| {
        constraint.kind == kind
            && owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                owner,
                opposite_owner,
            )
            && constraint.points_xz.len() == 2
            && road_point_key(constraint.points_xz[0]) == point
            && road_point_key(constraint.points_xz[1]) == point
    }) {
        return;
    }
    constraints.push(NodeRailConstraint {
        constraint_index: constraints.len(),
        kind,
        source_mouth_order_index,
        source_band_index,
        source_boundary_index: None,
        owner: Some(owner),
        opposite_owner: Some(opposite_owner),
        points_xz: edge.to_vec(),
    });
}

fn generated_role_edge_segments_inside_contour(
    role_edge: GeneratedContourDirectedEdge,
    target: &NodeGeneratedContour,
) -> Vec<GeneratedContourEdgeKey> {
    let mut keys = vec![role_edge.start, role_edge.end];
    for target_edge in generated_contour_directed_edges(target) {
        if let Some(point) = quantized_proper_segment_intersection(
            role_edge.start,
            role_edge.end,
            target_edge.start,
            target_edge.end,
        ) {
            keys.push(point);
        }
        for point in [target_edge.start, target_edge.end] {
            if generated_point_key_lies_on_segment(point, role_edge.start, role_edge.end) {
                keys.push(point);
            }
        }
        for point in [role_edge.start, role_edge.end] {
            if generated_point_key_lies_on_segment(point, target_edge.start, target_edge.end) {
                keys.push(point);
            }
        }
    }
    keys.sort_by_key(|point| {
        generated_segment_parameter_key(role_edge.start, role_edge.end, *point)
    });
    keys.dedup();

    let mut edges = BTreeSet::new();
    for segment in keys.windows(2) {
        let start = segment[0];
        let end = segment[1];
        if start == end {
            continue;
        }
        let point_x2 = i128::from(start.0) + i128::from(end.0);
        let point_z2 = i128::from(start.1) + i128::from(end.1);
        if doubled_point_inside_or_on_generated_contour(point_x2, point_z2, target) {
            edges.insert(GeneratedContourEdgeKey::new(start, end));
        }
    }
    edges.into_iter().collect()
}

pub(super) fn append_source_authorized_raised_step_point_contacts(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    constraints: &mut Vec<NodeRailConstraint>,
) {
    let mut existing = constraints
        .iter()
        .filter_map(generated_same_band_contact_constraint_key)
        .collect::<BTreeSet<_>>();
    let mut contacts = BTreeSet::<GeneratedSameBandContactConstraint>::new();
    collect_source_authorized_raised_step_contacts(
        piece_kind,
        contours,
        constraints,
        &mut contacts,
    );

    for contact in contacts {
        if !existing.insert(contact.key()) {
            continue;
        }
        constraints.push(NodeRailConstraint {
            constraint_index: constraints.len(),
            kind: contact.kind,
            source_mouth_order_index: contact.source_mouth_order_index,
            source_band_index: contact.source_band_index,
            source_boundary_index: None,
            owner: Some(contact.owner),
            opposite_owner: Some(contact.opposite_owner),
            points_xz: vec![
                road_point_from_key(contact.start),
                road_point_from_key(contact.end),
            ],
        });
    }
}

fn collect_source_authorized_raised_step_contacts(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
    contacts: &mut BTreeSet<GeneratedSameBandContactConstraint>,
) {
    let source_authority = RaisedStepSourceAuthority::from_constraints(constraints);
    let target_groups = source_authorized_target_groups(contours);
    for source_constraint in source_authority.constraints() {
        for target_group in &target_groups {
            let target_contacts = source_authorized_raised_step_target_pairs(
                piece_kind,
                contours,
                source_constraint.source,
                target_group.key,
            );
            if target_contacts.is_empty() {
                continue;
            }
            for source_edge in generated_constraint_directed_edges(source_constraint.constraint) {
                let mut source_edges = generated_directed_edge_segments_inside_shape_edges(
                    source_edge,
                    &target_group.shape_edges,
                    &target_group.shapes,
                )
                .into_iter()
                .collect::<BTreeSet<_>>();
                source_edges.extend(generated_shape_boundary_segments_on_source_edge(
                    source_edge,
                    &target_group.shape_edges,
                ));
                for edge in source_edges {
                    for (owner, opposite_owner, include_edge) in &target_contacts {
                        for (start, end) in source_authorized_contact_segments(edge, *include_edge)
                        {
                            contacts.insert(GeneratedSameBandContactConstraint {
                                kind: NodeRailConstraintKind::RaisedStepContact,
                                owner: *owner,
                                opposite_owner: *opposite_owner,
                                start,
                                end,
                                source_mouth_order_index: source_constraint
                                    .source
                                    .source_mouth_order_index,
                                source_band_index: source_constraint.source.source_band_index,
                            });
                        }
                    }
                }
            }
        }
        collect_source_authorized_exact_group_overlap_contacts(
            source_constraint,
            contours,
            &target_groups,
            contacts,
        );
    }

    for (point, sources) in source_authority.sources_by_contact_point() {
        for left_index in 0..sources.len() {
            for right_index in left_index + 1..sources.len() {
                let source = sources[left_index].min(sources[right_index]);
                for left_owner in sources[left_index].owners {
                    for right_owner in sources[right_index].owners {
                        let Some(kind) =
                            generated_raised_step_contact_kind_for_owners(left_owner, right_owner)
                        else {
                            continue;
                        };
                        let Some(pair) = GeneratedRaisedStepOwnerPair::new(left_owner, right_owner)
                        else {
                            continue;
                        };
                        contacts.insert(GeneratedSameBandContactConstraint {
                            kind,
                            owner: pair.owner,
                            opposite_owner: pair.opposite_owner,
                            start: point,
                            end: point,
                            source_mouth_order_index: source.source_mouth_order_index,
                            source_band_index: source.source_band_index,
                        });
                    }
                }
            }
        }
    }
}

impl<'a> RaisedStepSourceAuthority<'a> {
    fn from_constraints(constraints: &'a [NodeRailConstraint]) -> Self {
        Self {
            constraints: constraints
                .iter()
                .filter_map(|constraint| {
                    generated_raised_step_endpoint_source(constraint)
                        .map(|source| RaisedStepSourceConstraint { source, constraint })
                })
                .collect(),
        }
    }

    fn constraints(&self) -> &[RaisedStepSourceConstraint<'a>] {
        &self.constraints
    }

    fn sources_by_contact_point(
        &self,
    ) -> BTreeMap<NodeRailPointKey, Vec<GeneratedRaisedStepEndpointSource>> {
        generated_raised_step_source_contact_points(&self.constraints)
            .into_iter()
            .filter_map(|point| {
                let mut sources = self
                    .constraints
                    .iter()
                    .filter(|source_constraint| {
                        generated_constraint_touches_key(source_constraint.constraint, point)
                    })
                    .map(|source_constraint| source_constraint.source)
                    .collect::<Vec<_>>();
                sources.sort_unstable();
                sources.dedup();
                (!sources.is_empty()).then_some((point, sources))
            })
            .collect()
    }
}

fn collect_source_authorized_exact_group_overlap_contacts(
    source_constraint: &RaisedStepSourceConstraint<'_>,
    contours: &[NodeGeneratedContour],
    target_groups: &[SourceAuthorizedTargetGroup],
    contacts: &mut BTreeSet<GeneratedSameBandContactConstraint>,
) {
    let [left_owner, right_owner] = source_constraint.source.owners;
    let left_groups = source_authorized_exact_target_groups(target_groups, left_owner);
    let right_groups = source_authorized_exact_target_groups(target_groups, right_owner);
    for left_group in &left_groups {
        for right_group in &right_groups {
            for edge in source_authorized_group_edges_inside_group(
                source_constraint.constraint,
                left_group,
                right_group,
                contours,
            )
            .into_iter()
            .chain(source_authorized_group_edges_inside_group(
                source_constraint.constraint,
                right_group,
                left_group,
                contours,
            ))
            .chain(source_authorized_source_edges_inside_group_intersection(
                source_constraint.constraint,
                left_group,
                right_group,
            )) {
                for (start, end) in source_authorized_contact_segments(edge, true) {
                    contacts.insert(GeneratedSameBandContactConstraint {
                        kind: NodeRailConstraintKind::RaisedStepContact,
                        owner: left_owner,
                        opposite_owner: right_owner,
                        start,
                        end,
                        source_mouth_order_index: source_constraint.source.source_mouth_order_index,
                        source_band_index: source_constraint.source.source_band_index,
                    });
                }
            }
        }
    }
}

fn source_authorized_exact_target_groups(
    target_groups: &[SourceAuthorizedTargetGroup],
    owner: NodeBandOwner,
) -> Vec<&SourceAuthorizedTargetGroup> {
    target_groups
        .iter()
        .filter(|group| group.key.owner == owner)
        .collect()
}

fn source_authorized_group_edges_inside_group(
    source_constraint: &NodeRailConstraint,
    edge_group: &SourceAuthorizedTargetGroup,
    containing_group: &SourceAuthorizedTargetGroup,
    contours: &[NodeGeneratedContour],
) -> Vec<GeneratedContourEdgeKey> {
    let mut edges = BTreeSet::new();
    for contour_index in &edge_group.contour_indices {
        let Some(contour) = contours.get(*contour_index) else {
            continue;
        };
        for contour_edge in generated_contour_directed_edges(contour) {
            let mut candidate_edges = generated_directed_edge_segments_inside_shape_edges(
                contour_edge,
                &containing_group.shape_edges,
                &containing_group.shapes,
            )
            .into_iter()
            .collect::<BTreeSet<_>>();
            candidate_edges.extend(generated_shape_boundary_segments_on_source_edge(
                contour_edge,
                &containing_group.shape_edges,
            ));
            for edge in candidate_edges {
                if generated_constraint_contains_key_segment(
                    source_constraint,
                    edge.start,
                    edge.end,
                ) {
                    edges.insert(edge);
                }
            }
        }
    }
    edges.into_iter().collect()
}

fn source_authorized_source_edges_inside_group_intersection(
    source_constraint: &NodeRailConstraint,
    left_group: &SourceAuthorizedTargetGroup,
    right_group: &SourceAuthorizedTargetGroup,
) -> Vec<GeneratedContourEdgeKey> {
    let Some(intersection_shapes) = RoadSurfaceSystem::overlay_binary_shapes(
        &left_group.shapes,
        &right_group.shapes,
        OverlayRule::Intersect,
    ) else {
        return Vec::new();
    };
    let intersection_edges = generated_overlay_shapes_directed_edges(&intersection_shapes);
    let mut edges = BTreeSet::new();
    for source_edge in generated_constraint_directed_edges(source_constraint) {
        edges.extend(generated_directed_edge_segments_inside_shape_edges(
            source_edge,
            &intersection_edges,
            &intersection_shapes,
        ));
        edges.extend(generated_shape_boundary_segments_on_source_edge(
            source_edge,
            &intersection_edges,
        ));
    }
    edges.into_iter().collect()
}

fn source_authorized_raised_step_target_pairs(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    source: GeneratedRaisedStepEndpointSource,
    target: SourceAuthorizedTargetGroupKey,
) -> Vec<(NodeBandOwner, NodeBandOwner, bool)> {
    let target_owner = target.owner;
    if source.owners.contains(&target_owner) {
        if Some(target.claim_priority)
            == source_authorized_target_claim_priority(contours, target_owner)
        {
            let Some(pair) = GeneratedRaisedStepOwnerPair::new(source.owners[0], source.owners[1])
            else {
                return Vec::new();
            };
            return vec![(pair.owner, pair.opposite_owner, true)];
        }
        return Vec::new();
    }

    if piece_kind != RoadSurfaceVisualNodePieceKind::Bend
        || target.claim_priority != NodeGeneratedContourClaimPriority::SideJoin
    {
        return Vec::new();
    }

    let mut pairs = Vec::new();
    for source_owner_index in 0..source.owners.len() {
        let source_owner = source.owners[source_owner_index];
        let replaced_owner = source.owners[1 - source_owner_index];
        if target_owner.kind() != replaced_owner.kind()
            || generated_raised_step_contact_kind_for_owners(source_owner, target_owner).is_none()
        {
            continue;
        }
        let Some(pair) = GeneratedRaisedStepOwnerPair::new(source_owner, target_owner) else {
            continue;
        };
        pairs.push((pair.owner, pair.opposite_owner, false));
    }
    pairs.sort_unstable();
    pairs.dedup();
    pairs
}

fn source_authorized_contact_segments(
    edge: GeneratedContourEdgeKey,
    include_edge: bool,
) -> Vec<(NodeRailPointKey, NodeRailPointKey)> {
    if include_edge {
        vec![(edge.start, edge.end)]
    } else {
        vec![(edge.start, edge.start), (edge.end, edge.end)]
    }
}

fn source_authorized_target_claim_priority(
    contours: &[NodeGeneratedContour],
    owner: NodeBandOwner,
) -> Option<NodeGeneratedContourClaimPriority> {
    if contours.iter().any(|contour| {
        contour.owner == Some(owner)
            && contour.claim_priority == NodeGeneratedContourClaimPriority::MouthBand
    }) {
        return Some(NodeGeneratedContourClaimPriority::MouthBand);
    }
    contours
        .iter()
        .filter(|contour| contour.owner == Some(owner))
        .map(|contour| contour.claim_priority)
        .min()
}

fn generated_raised_step_source_contact_points(
    source_constraints: &[RaisedStepSourceConstraint<'_>],
) -> BTreeSet<NodeRailPointKey> {
    let mut points = source_constraints
        .iter()
        .flat_map(|source_constraint| {
            generated_constraint_endpoint_keys(source_constraint.constraint)
        })
        .collect::<BTreeSet<_>>();
    for left_index in 0..source_constraints.len() {
        for right_index in left_index + 1..source_constraints.len() {
            let left = source_constraints[left_index].constraint;
            let right = source_constraints[right_index].constraint;
            for left_edge in generated_constraint_directed_edges(left) {
                for right_edge in generated_constraint_directed_edges(right) {
                    points.extend(generated_source_edge_contact_points(left_edge, right_edge));
                }
            }
        }
    }
    points
}

fn generated_source_edge_contact_points(
    left: GeneratedContourDirectedEdge,
    right: GeneratedContourDirectedEdge,
) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    if let Some(point) =
        quantized_proper_segment_intersection(left.start, left.end, right.start, right.end)
    {
        points.push(point);
    }
    for point in [left.start, left.end] {
        if generated_point_key_lies_on_segment(point, right.start, right.end) {
            points.push(point);
        }
    }
    for point in [right.start, right.end] {
        if generated_point_key_lies_on_segment(point, left.start, left.end) {
            points.push(point);
        }
    }
    points.sort_unstable();
    points.dedup();
    points
}

fn generated_raised_step_endpoint_source(
    constraint: &NodeRailConstraint,
) -> Option<GeneratedRaisedStepEndpointSource> {
    if constraint.kind != NodeRailConstraintKind::RaisedStepContact {
        return None;
    }
    let owner = constraint.owner?;
    let opposite_owner = constraint.opposite_owner?;
    let pair = GeneratedRaisedStepOwnerPair::new(owner, opposite_owner)?;
    Some(GeneratedRaisedStepEndpointSource {
        constraint_index: constraint.constraint_index,
        source_mouth_order_index: constraint.source_mouth_order_index,
        source_band_index: constraint.source_band_index,
        owners: [pair.owner, pair.opposite_owner],
    })
}

fn generated_constraint_endpoint_keys(constraint: &NodeRailConstraint) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    if let Some(point) = constraint.points_xz.first().copied() {
        points.push(road_point_key(point));
    }
    if let Some(point) = constraint.points_xz.last().copied() {
        points.push(road_point_key(point));
    }
    points.sort_unstable();
    points.dedup();
    points
}

pub(super) fn append_generated_material_point_contact_constraints(
    contours: &[NodeGeneratedContour],
    constraints: &mut Vec<NodeRailConstraint>,
) {
    let mut contact_points = BTreeSet::<GeneratedSameBandContactConstraint>::new();
    for left_index in 0..contours.len() {
        for right_index in left_index + 1..contours.len() {
            let left = &contours[left_index];
            let right = &contours[right_index];
            let Some(left_owner) = left.owner else {
                continue;
            };
            let Some(right_owner) = right.owner else {
                continue;
            };
            if left_owner == right_owner {
                continue;
            }
            let Some(left_kind) = generated_contour_band_kind(left) else {
                continue;
            };
            let Some(right_kind) = generated_contour_band_kind(right) else {
                continue;
            };
            if left_kind == right_kind {
                continue;
            }
            let Some(contact_kind) =
                generated_raised_step_contact_kind_for_owners(left_owner, right_owner)
            else {
                continue;
            };
            let mut points = shared_generated_contour_points(left, right);
            points.extend(generated_contact_points_from_contour_intersections(
                left, right,
            ));
            points.extend(generated_material_authority_points_on_counterpart_contour(
                contact_kind,
                left,
                right,
                left_owner,
                right_owner,
                constraints,
            ));
            points.extend(generated_contour_keys(left).into_iter().filter(|point| {
                generated_material_point_contact_authority(
                    contact_kind,
                    left_owner,
                    right_owner,
                    *point,
                    constraints,
                )
                .is_some_and(|authority| {
                    authority.owner == Some(right_owner)
                        || authority.opposite_owner == Some(right_owner)
                        || owners_match_unordered(
                            authority.owner,
                            authority.opposite_owner,
                            left_owner,
                            right_owner,
                        )
                })
            }));
            points.extend(generated_contour_keys(right).into_iter().filter(|point| {
                generated_material_point_contact_authority(
                    contact_kind,
                    left_owner,
                    right_owner,
                    *point,
                    constraints,
                )
                .is_some_and(|authority| {
                    authority.owner == Some(left_owner)
                        || authority.opposite_owner == Some(left_owner)
                        || owners_match_unordered(
                            authority.owner,
                            authority.opposite_owner,
                            left_owner,
                            right_owner,
                        )
                })
            }));
            points.sort_unstable();
            points.dedup();
            for point in points {
                let Some(contact_source) = generated_material_point_contact_authority(
                    contact_kind,
                    left_owner,
                    right_owner,
                    point,
                    constraints,
                ) else {
                    continue;
                };
                let Some(pair) = GeneratedRaisedStepOwnerPair::new(left_owner, right_owner) else {
                    continue;
                };
                contact_points.insert(GeneratedSameBandContactConstraint {
                    kind: contact_kind,
                    owner: pair.owner,
                    opposite_owner: pair.opposite_owner,
                    start: point,
                    end: point,
                    source_mouth_order_index: contact_source.source_mouth_order_index,
                    source_band_index: contact_source.source_band_index,
                });
            }
        }
    }

    let mut existing = constraints
        .iter()
        .filter_map(generated_same_band_contact_constraint_key)
        .collect::<BTreeSet<_>>();
    for contact in contact_points {
        let key = contact.key();
        if !existing.insert(key) {
            continue;
        }
        insert_generated_material_point_constraint(
            constraints,
            contact.kind,
            contact.owner,
            contact.opposite_owner,
            contact.start,
            contact.source_mouth_order_index,
            contact.source_band_index,
        );
    }
}

fn generated_material_authority_points_on_counterpart_contour(
    kind: NodeRailConstraintKind,
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
    constraints: &[NodeRailConstraint],
) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    for constraint in constraints
        .iter()
        .filter(|constraint| constraint.kind == kind)
        .filter(|constraint| {
            owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                left_owner,
                right_owner,
            )
        })
    {
        for point in constraint.points_xz.iter().copied().map(road_point_key) {
            if generated_contour_contains_key(right, point) {
                points.push(point);
            }
            if generated_contour_contains_key(left, point) {
                points.push(point);
            }
        }
        points.extend(generated_constraint_contour_contact_points(
            constraint, right,
        ));
        points.extend(generated_constraint_contour_contact_points(
            constraint, left,
        ));
    }
    points.sort_unstable();
    points.dedup();
    points
}

fn generated_constraint_contour_contact_points(
    constraint: &NodeRailConstraint,
    contour: &NodeGeneratedContour,
) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    for constraint_edge in generated_constraint_directed_edges(constraint) {
        for contour_edge in generated_contour_directed_edges(contour) {
            if let Some(point) = quantized_proper_segment_intersection(
                constraint_edge.start,
                constraint_edge.end,
                contour_edge.start,
                contour_edge.end,
            ) {
                points.push(point);
            }
            if generated_point_key_lies_on_segment(
                constraint_edge.start,
                contour_edge.start,
                contour_edge.end,
            ) {
                points.push(constraint_edge.start);
            }
            if generated_point_key_lies_on_segment(
                constraint_edge.end,
                contour_edge.start,
                contour_edge.end,
            ) {
                points.push(constraint_edge.end);
            }
            if generated_point_key_lies_on_segment(
                contour_edge.start,
                constraint_edge.start,
                constraint_edge.end,
            ) {
                points.push(contour_edge.start);
            }
            if generated_point_key_lies_on_segment(
                contour_edge.end,
                constraint_edge.start,
                constraint_edge.end,
            ) {
                points.push(contour_edge.end);
            }
        }
    }
    points.sort_unstable();
    points.dedup();
    points
}

fn generated_material_point_contact_authority(
    kind: NodeRailConstraintKind,
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
    point: NodeRailPointKey,
    constraints: &[NodeRailConstraint],
) -> Option<GeneratedMaterialPointContactAuthority> {
    constraints
        .iter()
        .filter(|constraint| constraint.kind == kind)
        .filter(|constraint| generated_constraint_touches_key(constraint, point))
        .filter(|constraint| {
            owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                left_owner,
                right_owner,
            )
        })
        .min_by_key(|constraint| constraint.constraint_index)
        .map(|constraint| GeneratedMaterialPointContactAuthority {
            source_mouth_order_index: constraint.source_mouth_order_index,
            source_band_index: constraint.source_band_index,
            owner: constraint.owner,
            opposite_owner: constraint.opposite_owner,
        })
}

fn generated_exact_owner_pair_contact_authority_for_edge(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    constraints: &[NodeRailConstraint],
    edge: GeneratedContourEdgeKey,
) -> Option<GeneratedMaterialPointContactAuthority> {
    constraints
        .iter()
        .filter(|constraint| constraint.kind == NodeRailConstraintKind::RaisedStepContact)
        .filter(|constraint| {
            owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                owner,
                opposite_owner,
            )
        })
        .filter(|constraint| {
            generated_constraint_contains_key_segment(constraint, edge.start, edge.end)
        })
        .min_by_key(|constraint| constraint.constraint_index)
        .map(|constraint| GeneratedMaterialPointContactAuthority {
            source_mouth_order_index: constraint.source_mouth_order_index,
            source_band_index: constraint.source_band_index,
            owner: constraint.owner,
            opposite_owner: constraint.opposite_owner,
        })
}

fn generated_exact_owner_pair_contact_authority_at_point(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    constraints: &[NodeRailConstraint],
    point: NodeRailPointKey,
) -> Option<GeneratedMaterialPointContactAuthority> {
    generated_material_point_contact_authority(
        NodeRailConstraintKind::RaisedStepContact,
        owner,
        opposite_owner,
        point,
        constraints,
    )
}

fn generated_contour_contains_key(contour: &NodeGeneratedContour, point: NodeRailPointKey) -> bool {
    doubled_point_inside_or_on_generated_contour(
        i128::from(point.0) * 2,
        i128::from(point.1) * 2,
        contour,
    )
}

fn generated_contour_boundary_contains_key(
    contour: &NodeGeneratedContour,
    point: NodeRailPointKey,
) -> bool {
    generated_contour_directed_edges(contour)
        .into_iter()
        .any(|edge| generated_point_key_lies_on_segment(point, edge.start, edge.end))
}

pub(super) fn append_generated_same_band_contact_constraints(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    constraints: &mut Vec<NodeRailConstraint>,
) {
    let mut contact_edges = BTreeSet::<GeneratedSameBandContactConstraint>::new();
    for left_index in 0..contours.len() {
        for right_index in left_index + 1..contours.len() {
            let left = &contours[left_index];
            let right = &contours[right_index];
            let Some(left_owner) = left.owner else {
                continue;
            };
            let Some(right_owner) = right.owner else {
                continue;
            };
            if left_owner == right_owner {
                continue;
            }
            let Some(kind) = generated_contour_band_kind(left) else {
                continue;
            };
            let Some(right_kind) = generated_contour_band_kind(right) else {
                continue;
            };
            let Some(contact_kind) =
                generated_raised_step_contact_kind_for_owners(left_owner, right_owner)
            else {
                continue;
            };
            let Some(pair) = GeneratedRaisedStepOwnerPair::new(left_owner, right_owner) else {
                continue;
            };
            let shared_edges = shared_generated_contour_edges(left, right);
            let shared_edge_points = shared_edges
                .iter()
                .flat_map(|edge| [edge.start, edge.end])
                .collect::<BTreeSet<_>>();
            for edge in shared_edges {
                if let Some(source) = generated_contact_edge_source_authority(
                    pair.owner,
                    pair.opposite_owner,
                    constraints,
                    edge,
                ) {
                    insert_generated_contact_constraint(
                        &mut contact_edges,
                        contact_kind,
                        pair.owner,
                        pair.opposite_owner,
                        edge,
                        source,
                    );
                }
            }
            for edge in generated_contact_edges_inside_contour(left, right) {
                if let Some(source) = generated_contact_edge_source_authority(
                    pair.owner,
                    pair.opposite_owner,
                    constraints,
                    edge,
                ) {
                    insert_generated_contact_constraint(
                        &mut contact_edges,
                        contact_kind,
                        pair.owner,
                        pair.opposite_owner,
                        edge,
                        source,
                    );
                }
            }
            for edge in generated_contact_edges_inside_contour(right, left) {
                if let Some(source) = generated_contact_edge_source_authority(
                    pair.owner,
                    pair.opposite_owner,
                    constraints,
                    edge,
                ) {
                    insert_generated_contact_constraint(
                        &mut contact_edges,
                        contact_kind,
                        pair.owner,
                        pair.opposite_owner,
                        edge,
                        source,
                    );
                }
            }
            for edge in generated_contact_edges_from_overlay_intersection(left, right) {
                if let Some(source) = generated_contact_edge_source_authority(
                    pair.owner,
                    pair.opposite_owner,
                    constraints,
                    edge,
                ) {
                    insert_generated_contact_constraint(
                        &mut contact_edges,
                        contact_kind,
                        pair.owner,
                        pair.opposite_owner,
                        edge,
                        source,
                    );
                }
            }
            for point in shared_generated_contour_points(left, right) {
                if shared_edge_points.contains(&point) {
                    continue;
                }
                if !generated_contact_point_has_explicit_roles(
                    kind,
                    right_kind,
                    left,
                    right,
                    constraints,
                    point,
                    contact_kind,
                ) {
                    continue;
                }
                let Some(source) = generated_exact_owner_pair_contact_authority_at_point(
                    pair.owner,
                    pair.opposite_owner,
                    constraints,
                    point,
                ) else {
                    continue;
                };
                contact_edges.insert(GeneratedSameBandContactConstraint {
                    kind: contact_kind,
                    owner: pair.owner,
                    opposite_owner: pair.opposite_owner,
                    start: point,
                    end: point,
                    source_mouth_order_index: source.source_mouth_order_index,
                    source_band_index: source.source_band_index,
                });
            }
            for point in generated_contact_points_from_contour_intersections(left, right) {
                if shared_edge_points.contains(&point) {
                    continue;
                }
                if !generated_contact_point_has_explicit_roles(
                    kind,
                    right_kind,
                    left,
                    right,
                    constraints,
                    point,
                    contact_kind,
                ) {
                    continue;
                }
                let Some(source) = generated_exact_owner_pair_contact_authority_at_point(
                    pair.owner,
                    pair.opposite_owner,
                    constraints,
                    point,
                ) else {
                    continue;
                };
                contact_edges.insert(GeneratedSameBandContactConstraint {
                    kind: contact_kind,
                    owner: pair.owner,
                    opposite_owner: pair.opposite_owner,
                    start: point,
                    end: point,
                    source_mouth_order_index: source.source_mouth_order_index,
                    source_band_index: source.source_band_index,
                });
            }
        }
    }
    collect_source_authorized_raised_step_contacts(
        piece_kind,
        contours,
        constraints,
        &mut contact_edges,
    );

    let mut existing = constraints
        .iter()
        .filter_map(generated_same_band_contact_constraint_key)
        .collect::<BTreeSet<_>>();
    for contact in contact_edges {
        let key = contact.key();
        if !existing.insert(key) {
            continue;
        }
        constraints.push(NodeRailConstraint {
            constraint_index: constraints.len(),
            kind: contact.kind,
            source_mouth_order_index: contact.source_mouth_order_index,
            source_band_index: contact.source_band_index,
            source_boundary_index: None,
            owner: Some(contact.owner),
            opposite_owner: Some(contact.opposite_owner),
            points_xz: vec![
                road_point_from_key(contact.start),
                road_point_from_key(contact.end),
            ],
        });
    }
}

fn source_authorized_target_groups(
    contours: &[NodeGeneratedContour],
) -> Vec<SourceAuthorizedTargetGroup> {
    let mut contour_indices_by_key = BTreeMap::<SourceAuthorizedTargetGroupKey, Vec<usize>>::new();
    for (contour_index, contour) in contours.iter().enumerate() {
        let Some(owner) = contour.owner else {
            continue;
        };
        let Some(kind) = generated_contour_band_kind(contour) else {
            continue;
        };
        contour_indices_by_key
            .entry(SourceAuthorizedTargetGroupKey {
                owner,
                kind,
                claim_priority: contour.claim_priority,
            })
            .or_default()
            .push(contour_index);
    }

    contour_indices_by_key
        .into_iter()
        .filter_map(|(key, contour_indices)| {
            let overlay_contours = contour_indices
                .iter()
                .map(|index| generated_overlay_contour(&contours[*index]))
                .collect::<Vec<_>>();
            let shapes = RoadSurfaceSystem::overlay_union_contours(&overlay_contours)?;
            Some(SourceAuthorizedTargetGroup {
                key,
                contour_indices,
                shape_edges: generated_overlay_shapes_directed_edges(&shapes),
                shapes,
            })
        })
        .collect()
}

fn insert_generated_contact_constraint(
    contact_edges: &mut BTreeSet<GeneratedSameBandContactConstraint>,
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    edge: GeneratedContourEdgeKey,
    source: GeneratedMaterialPointContactAuthority,
) {
    let (owner, opposite_owner) = if kind == NodeRailConstraintKind::RaisedStepContact {
        let Some(pair) = GeneratedRaisedStepOwnerPair::new(owner, opposite_owner) else {
            return;
        };
        (pair.owner, pair.opposite_owner)
    } else {
        (owner, opposite_owner)
    };
    for (start, end) in [
        (edge.start, edge.end),
        (edge.start, edge.start),
        (edge.end, edge.end),
    ] {
        contact_edges.insert(GeneratedSameBandContactConstraint {
            kind,
            owner,
            opposite_owner,
            start,
            end,
            source_mouth_order_index: source.source_mouth_order_index,
            source_band_index: source.source_band_index,
        });
    }
}

fn generated_contact_edge_source_authority(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    constraints: &[NodeRailConstraint],
    edge: GeneratedContourEdgeKey,
) -> Option<GeneratedMaterialPointContactAuthority> {
    generated_exact_owner_pair_contact_authority_for_edge(owner, opposite_owner, constraints, edge)
}

fn generated_contact_edges_inside_contour(
    edge_contour: &NodeGeneratedContour,
    containing_contour: &NodeGeneratedContour,
) -> Vec<GeneratedContourEdgeKey> {
    let mut edges = BTreeSet::new();
    for edge in generated_contour_directed_edges(edge_contour) {
        edges.extend(generated_role_edge_segments_inside_contour(
            edge,
            containing_contour,
        ));
    }
    edges.into_iter().collect()
}

fn generated_directed_edge_segments_inside_shape_edges(
    edge: GeneratedContourDirectedEdge,
    shape_edges: &[GeneratedContourDirectedEdge],
    containing_shapes: &NodeOverlayShapes,
) -> Vec<GeneratedContourEdgeKey> {
    let mut keys = vec![edge.start, edge.end];
    for shape_edge in shape_edges {
        if let Some(point) = quantized_proper_segment_intersection(
            edge.start,
            edge.end,
            shape_edge.start,
            shape_edge.end,
        ) {
            keys.push(point);
        }
        for point in [shape_edge.start, shape_edge.end] {
            if generated_point_key_lies_on_segment(point, edge.start, edge.end) {
                keys.push(point);
            }
        }
        for point in [edge.start, edge.end] {
            if generated_point_key_lies_on_segment(point, shape_edge.start, shape_edge.end) {
                keys.push(point);
            }
        }
    }
    keys.sort_by_key(|point| generated_segment_parameter_key(edge.start, edge.end, *point));
    keys.dedup();

    let mut edges = BTreeSet::new();
    for segment in keys.windows(2) {
        let start = segment[0];
        let end = segment[1];
        if start == end {
            continue;
        }
        let point_x2 = i128::from(start.0) + i128::from(end.0);
        let point_z2 = i128::from(start.1) + i128::from(end.1);
        if doubled_point_inside_or_on_overlay_shapes(point_x2, point_z2, containing_shapes) {
            edges.insert(GeneratedContourEdgeKey::new(start, end));
        }
    }
    edges.into_iter().collect()
}

fn generated_shape_boundary_segments_on_source_edge(
    source_edge: GeneratedContourDirectedEdge,
    shape_edges: &[GeneratedContourDirectedEdge],
) -> Vec<GeneratedContourEdgeKey> {
    let mut edges = BTreeSet::new();
    for shape_edge in shape_edges {
        let mut keys = Vec::new();
        for point in [shape_edge.start, shape_edge.end] {
            if generated_point_key_lies_on_segment(point, source_edge.start, source_edge.end) {
                keys.push(point);
            }
        }
        for point in [source_edge.start, source_edge.end] {
            if generated_point_key_lies_on_segment(point, shape_edge.start, shape_edge.end) {
                keys.push(point);
            }
        }
        keys.sort_by_key(|point| {
            generated_segment_parameter_key(source_edge.start, source_edge.end, *point)
        });
        keys.dedup();
        for segment in keys.windows(2) {
            let start = segment[0];
            let end = segment[1];
            if start != end {
                edges.insert(GeneratedContourEdgeKey::new(start, end));
            }
        }
    }
    edges.into_iter().collect()
}

fn generated_contact_edges_from_overlay_intersection(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
) -> Vec<GeneratedContourEdgeKey> {
    let Some(left_shapes) = generated_contour_overlay_shapes(left) else {
        return Vec::new();
    };
    let Some(right_shapes) = generated_contour_overlay_shapes(right) else {
        return Vec::new();
    };
    let Some(intersection) = RoadSurfaceSystem::overlay_binary_shapes(
        &left_shapes,
        &right_shapes,
        OverlayRule::Intersect,
    ) else {
        return Vec::new();
    };
    let mut edges = intersection
        .into_iter()
        .flat_map(|shape| shape.into_iter())
        .flat_map(|contour| {
            let keys = contour
                .into_iter()
                .map(|point| {
                    (
                        (point[0] * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
                        (point[1] * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
                    )
                })
                .collect::<Vec<_>>();
            let mut edges = Vec::new();
            for index in 0..keys.len() {
                let start = keys[index];
                let end = keys[(index + 1) % keys.len()];
                if start != end {
                    edges.push(GeneratedContourEdgeKey::new(start, end));
                }
            }
            edges
        })
        .collect::<Vec<_>>();
    edges.sort_unstable();
    edges.dedup();
    edges
}

fn generated_contact_points_from_contour_intersections(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    for left_edge in generated_contour_directed_edges(left) {
        for right_edge in generated_contour_directed_edges(right) {
            if let Some(point) = quantized_proper_segment_intersection(
                left_edge.start,
                left_edge.end,
                right_edge.start,
                right_edge.end,
            ) {
                points.push(point);
            }
            if generated_point_key_lies_on_segment(
                left_edge.start,
                right_edge.start,
                right_edge.end,
            ) {
                points.push(left_edge.start);
            }
            if generated_point_key_lies_on_segment(left_edge.end, right_edge.start, right_edge.end)
            {
                points.push(left_edge.end);
            }
            if generated_point_key_lies_on_segment(right_edge.start, left_edge.start, left_edge.end)
            {
                points.push(right_edge.start);
            }
            if generated_point_key_lies_on_segment(right_edge.end, left_edge.start, left_edge.end) {
                points.push(right_edge.end);
            }
        }
    }
    points.sort_unstable();
    points.dedup();
    points
}

fn generated_contour_overlay_shapes(contour: &NodeGeneratedContour) -> Option<NodeOverlayShapes> {
    RoadSurfaceSystem::overlay_union_contours(&[generated_overlay_contour(contour)])
}

fn generated_overlay_contour(contour: &NodeGeneratedContour) -> NodeOverlayContour {
    contour
        .points_xz
        .iter()
        .map(|point| [point.x, point.y])
        .collect()
}

fn generated_overlay_shapes_directed_edges(
    shapes: &NodeOverlayShapes,
) -> Vec<GeneratedContourDirectedEdge> {
    let mut edges = Vec::new();
    for contour in shapes.iter().flat_map(|shape| shape.iter()) {
        let keys = generated_overlay_contour_keys(contour);
        for index in 0..keys.len() {
            let start = keys[index];
            let end = keys[(index + 1) % keys.len()];
            if start != end {
                edges.push(GeneratedContourDirectedEdge { start, end });
            }
        }
    }
    edges
}

fn generated_overlay_contour_keys(contour: &NodeOverlayContour) -> Vec<NodeRailPointKey> {
    contour
        .iter()
        .copied()
        .map(generated_overlay_point_key)
        .collect()
}

fn generated_overlay_point_key(point: [f64; 2]) -> NodeRailPointKey {
    let key = SurfaceXzKey::from_overlay_point(point);
    (key.x_key(), key.z_key())
}

fn doubled_point_inside_or_on_generated_contour(
    point_x2: i128,
    point_z2: i128,
    contour: &NodeGeneratedContour,
) -> bool {
    let keys = generated_contour_keys(contour);
    doubled_point_inside_or_on_generated_keys(point_x2, point_z2, &keys)
}

fn doubled_point_inside_or_on_generated_keys(
    point_x2: i128,
    point_z2: i128,
    keys: &[NodeRailPointKey],
) -> bool {
    doubled_point_location_in_generated_keys(point_x2, point_z2, keys)
        != GeneratedPointContourLocation::Outside
}

fn doubled_point_inside_or_on_overlay_shapes(
    point_x2: i128,
    point_z2: i128,
    shapes: &NodeOverlayShapes,
) -> bool {
    shapes.iter().any(|shape| {
        let Some(outer) = shape.first() else {
            return false;
        };
        let outer_keys = generated_overlay_contour_keys(outer);
        match doubled_point_location_in_generated_keys(point_x2, point_z2, &outer_keys) {
            GeneratedPointContourLocation::Outside => false,
            GeneratedPointContourLocation::Boundary => true,
            GeneratedPointContourLocation::Inside => shape.iter().skip(1).all(|hole| {
                let hole_keys = generated_overlay_contour_keys(hole);
                doubled_point_location_in_generated_keys(point_x2, point_z2, &hole_keys)
                    != GeneratedPointContourLocation::Inside
            }),
        }
    })
}

fn doubled_point_location_in_generated_keys(
    point_x2: i128,
    point_z2: i128,
    keys: &[NodeRailPointKey],
) -> GeneratedPointContourLocation {
    if keys.len() < 3 {
        return GeneratedPointContourLocation::Outside;
    }
    let mut inside = false;
    for index in 0..keys.len() {
        let start = keys[index];
        let end = keys[(index + 1) % keys.len()];
        if doubled_point_lies_on_generated_segment(point_x2, point_z2, start, end) {
            return GeneratedPointContourLocation::Boundary;
        }
        let start_z2 = i128::from(start.1) * 2;
        let end_z2 = i128::from(end.1) * 2;
        if (start_z2 > point_z2) == (end_z2 > point_z2) {
            continue;
        }
        let start_x2 = i128::from(start.0) * 2;
        let end_x2 = i128::from(end.0) * 2;
        let denominator = end_z2 - start_z2;
        let lhs = (point_x2 - start_x2) * denominator;
        let rhs = (point_z2 - start_z2) * (end_x2 - start_x2);
        let crosses = if denominator > 0 {
            lhs < rhs
        } else {
            lhs > rhs
        };
        if crosses {
            inside = !inside;
        }
    }
    if inside {
        GeneratedPointContourLocation::Inside
    } else {
        GeneratedPointContourLocation::Outside
    }
}

fn doubled_point_lies_on_generated_segment(
    point_x2: i128,
    point_z2: i128,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
) -> bool {
    let start_x2 = i128::from(start.0) * 2;
    let start_z2 = i128::from(start.1) * 2;
    let end_x2 = i128::from(end.0) * 2;
    let end_z2 = i128::from(end.1) * 2;
    let dx = end_x2 - start_x2;
    let dz = end_z2 - start_z2;
    let px = point_x2 - start_x2;
    let pz = point_z2 - start_z2;
    if px * dz - pz * dx != 0 {
        return false;
    }
    point_x2 >= start_x2.min(end_x2)
        && point_x2 <= start_x2.max(end_x2)
        && point_z2 >= start_z2.min(end_z2)
        && point_z2 <= start_z2.max(end_z2)
}

pub(super) fn node_generated_contact_contours(
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
) -> Result<(), NodeRailGenerationError> {
    let max_passes = contours.len().saturating_mul(contours.len()).max(1) * 4;
    let mut previous_candidates = None;
    for _ in 0..max_passes {
        let candidates = generated_contact_contour_noding_candidates(contours, constraints);
        if candidates.is_empty() {
            return Ok(());
        };
        if previous_candidates.as_ref() == Some(&candidates) {
            return Ok(());
        }
        if !insert_contact_noding_candidates(contours, constraints, candidates.clone())? {
            return Ok(());
        }
        previous_candidates = Some(candidates);
    }
    Ok(())
}

fn generated_contact_contour_noding_candidates(
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
) -> Vec<(usize, GeneratedContourDirectedEdge, NodeRailPointKey)> {
    let mut candidates = Vec::new();
    for left_index in 0..contours.len() {
        for right_index in left_index + 1..contours.len() {
            let left = &contours[left_index];
            let right = &contours[right_index];
            if !generated_contours_support_contact_noding(left, right) {
                continue;
            }
            candidates.extend(
                generated_contact_point_on_edge_noding_candidates(left, right, constraints)
                    .into_iter()
                    .map(|(edge, insert_key)| (left_index, edge, insert_key)),
            );
            candidates.extend(
                generated_contact_point_on_edge_noding_candidates(right, left, constraints)
                    .into_iter()
                    .map(|(edge, insert_key)| (right_index, edge, insert_key)),
            );
            candidates.extend(
                generated_contact_edge_intersection_noding_candidates(left, right, constraints)
                    .into_iter()
                    .flat_map(|(left_edge, right_edge, insert_key)| {
                        [
                            (left_index, left_edge, insert_key),
                            (right_index, right_edge, insert_key),
                        ]
                    }),
            );
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    candidates
}

fn insert_contact_noding_candidates(
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
    candidates: Vec<(usize, GeneratedContourDirectedEdge, NodeRailPointKey)>,
) -> Result<bool, NodeRailGenerationError> {
    let mut insertions_by_contour =
        BTreeMap::<usize, BTreeMap<GeneratedContourDirectedEdge, BTreeSet<NodeRailPointKey>>>::new(
        );
    for (contour_index, edge, insert_key) in candidates {
        insertions_by_contour
            .entry(contour_index)
            .or_default()
            .entry(edge)
            .or_default()
            .insert(insert_key);
    }

    let mut inserted_any = false;
    for (contour_index, insertions_by_edge) in insertions_by_contour {
        inserted_any |= insert_keys_on_generated_contour_edges(
            contours,
            constraints,
            contour_index,
            insertions_by_edge,
        )?;
    }
    Ok(inserted_any)
}

fn insert_keys_on_generated_contour_edges(
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
    contour_index: usize,
    insertions_by_edge: BTreeMap<GeneratedContourDirectedEdge, BTreeSet<NodeRailPointKey>>,
) -> Result<bool, NodeRailGenerationError> {
    let Some(contour) = contours.get_mut(contour_index) else {
        return Ok(false);
    };
    let keys = generated_contour_keys(contour);
    if keys.len() < 2 {
        return Ok(false);
    }

    let height_points = contour.height_points_world.clone();
    let mut new_keys = Vec::with_capacity(keys.len());
    let mut new_height_points = height_points
        .as_ref()
        .filter(|points| points.len() == keys.len())
        .map(|_| Vec::with_capacity(keys.len()));
    let mut inserted_any = false;

    for index in 0..keys.len() {
        let next = (index + 1) % keys.len();
        let start = keys[index];
        let end = keys[next];
        new_keys.push(start);
        if let (Some(height_points), Some(new_height_points)) =
            (height_points.as_ref(), new_height_points.as_mut())
        {
            new_height_points.push(height_points[index]);
        }

        let edge = GeneratedContourDirectedEdge { start, end };
        let Some(insertions) = insertions_by_edge.get(&edge) else {
            continue;
        };
        let mut insertions = insertions
            .iter()
            .copied()
            .filter(|point| *point != start && *point != end)
            .filter(|point| generated_point_key_lies_on_segment(*point, start, end))
            .collect::<Vec<_>>();
        insertions.sort_by_key(|point| generated_segment_parameter_key(start, end, *point));
        insertions.dedup();
        for insert_key in insertions {
            inserted_any = true;
            new_keys.push(insert_key);
            if let (Some(height_points), Some(new_height_points)) =
                (height_points.as_ref(), new_height_points.as_mut())
            {
                let Some(height_m) = height_for_key_on_generated_edge(
                    insert_key,
                    start,
                    end,
                    height_points[index].y,
                    height_points[next].y,
                ) else {
                    contour.height_points_world = None;
                    continue;
                };
                let point = road_point_from_key(insert_key);
                new_height_points.push(RoadVec3::new(point.x, height_m, point.y));
            }
        }
    }

    if !inserted_any {
        return Ok(false);
    }
    remove_generated_contour_spikes(&mut new_keys);
    if new_keys == keys {
        return Ok(false);
    }
    contour.height_points_world = new_height_points;
    set_generated_contour_from_keys(contour, constraints, new_keys)?;
    Ok(generated_contour_keys(contour) != keys)
}

pub(super) fn node_generated_contact_source_constraints(
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
) -> BTreeMap<usize, BTreeMap<GeneratedContourDirectedEdge, BTreeSet<NodeRailPointKey>>> {
    let mut candidates =
        BTreeMap::<usize, BTreeMap<GeneratedContourDirectedEdge, BTreeSet<NodeRailPointKey>>>::new(
        );
    for constraint in constraints {
        if generated_contact_kind_from_constraint(constraint.kind).is_none()
            || constraint.owner.is_none()
            || constraint.opposite_owner.is_none()
        {
            continue;
        }
        for source_edge in generated_constraint_directed_edges(constraint) {
            for contour in contours {
                if !generated_contact_source_constraint_can_node_with_contour(constraint, contour) {
                    continue;
                }
                for point in generated_contour_keys(contour) {
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
                for contour_edge in generated_contour_directed_edges(contour) {
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

pub(super) fn node_generated_contact_sources_from_contour_backed_contacts(
    contours: &[NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
    generated_constraint_start_index: usize,
) {
    let generated_constraint_start_index = generated_constraint_start_index.min(constraints.len());
    let mut insertions =
        BTreeMap::<usize, BTreeMap<GeneratedContourDirectedEdge, BTreeSet<NodeRailPointKey>>>::new(
        );
    for constraint in constraints.iter().skip(generated_constraint_start_index) {
        let Some(contact) = generated_same_band_contact_constraint(constraint) else {
            continue;
        };
        for point in [contact.start, contact.end] {
            if !generated_contact_point_has_source_contour_authority(contours, contact, point) {
                continue;
            }
            for source_constraint in constraints.iter().take(generated_constraint_start_index) {
                if source_constraint.kind != contact.kind
                    || source_constraint.source_mouth_order_index
                        != contact.source_mouth_order_index
                    || source_constraint.source_band_index != contact.source_band_index
                    || !owners_match_unordered(
                        source_constraint.owner,
                        source_constraint.opposite_owner,
                        contact.owner,
                        contact.opposite_owner,
                    )
                {
                    continue;
                }
                for edge in generated_constraint_directed_edges(source_constraint) {
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
    insert_keys_on_generated_source_constraints(
        &mut constraints[..generated_constraint_start_index],
        insertions,
    );
}

fn generated_contact_point_has_source_contour_authority(
    contours: &[NodeGeneratedContour],
    contact: GeneratedSameBandContactConstraint,
    point: NodeRailPointKey,
) -> bool {
    let Some(source_band_index) = contact.source_band_index else {
        return false;
    };
    contours.iter().any(|contour| {
        contour.source_mouth_order_index == contact.source_mouth_order_index
            && contour.source_band_index == Some(source_band_index)
            && contour.claim_priority == NodeGeneratedContourClaimPriority::MouthBand
            && (contour.owner == Some(contact.owner)
                || contour.owner == Some(contact.opposite_owner))
            && generated_contour_boundary_contains_key(contour, point)
    })
}

fn insert_keys_on_generated_source_constraints(
    constraints: &mut [NodeRailConstraint],
    insertions_by_constraint: BTreeMap<
        usize,
        BTreeMap<GeneratedContourDirectedEdge, BTreeSet<NodeRailPointKey>>,
    >,
) -> bool {
    let mut inserted_any = false;
    for constraint in constraints {
        let Some(insertions_by_edge) = insertions_by_constraint.get(&constraint.constraint_index)
        else {
            continue;
        };
        let keys = constraint
            .points_xz
            .iter()
            .copied()
            .map(road_point_key)
            .collect::<Vec<_>>();
        if keys.len() < 2 {
            continue;
        }
        let mut new_keys = Vec::with_capacity(keys.len());
        for segment in keys.windows(2) {
            let start = segment[0];
            let end = segment[1];
            new_keys.push(start);
            let edge = GeneratedContourDirectedEdge { start, end };
            let Some(insertions) = insertions_by_edge.get(&edge) else {
                continue;
            };
            let mut insertions = insertions
                .iter()
                .copied()
                .filter(|point| *point != start && *point != end)
                .filter(|point| generated_point_key_lies_on_segment(*point, start, end))
                .collect::<Vec<_>>();
            insertions.sort_by_key(|point| generated_segment_parameter_key(start, end, *point));
            insertions.dedup();
            if !insertions.is_empty() {
                inserted_any = true;
            }
            new_keys.extend(insertions);
        }
        if let Some(last) = keys.last().copied() {
            new_keys.push(last);
        }
        new_keys.dedup();
        if new_keys != keys {
            constraint.points_xz = new_keys.into_iter().map(road_point_from_key).collect();
        }
    }
    inserted_any
}

pub(super) fn retain_source_authorized_generated_contact_constraints(
    contours: &[NodeGeneratedContour],
    authority_constraints: &[NodeRailConstraint],
    constraints: &mut Vec<NodeRailConstraint>,
    generated_constraint_start_index: usize,
) {
    let source_authority = ExactGeneratedSourceAuthority::from_sources(
        contours,
        authority_constraints,
        generated_constraint_start_index,
    );
    let mut index = 0usize;
    constraints.retain(|constraint| {
        let retain = index < generated_constraint_start_index
            || generated_contact_kind_from_constraint(constraint.kind).is_none()
            || generated_contact_constraint_has_exact_source_authority(
                constraint,
                &source_authority,
            );
        index += 1;
        retain
    });
}

fn generated_contact_constraint_has_exact_source_authority(
    constraint: &NodeRailConstraint,
    source_authority: &ExactGeneratedSourceAuthority,
) -> bool {
    let source_band_index = constraint.source_band_index;
    if constraint.owner.is_none() || constraint.opposite_owner.is_none() {
        return true;
    }
    let owners = [constraint.owner, constraint.opposite_owner];
    if !source_authority.has_any_source(
        owners,
        constraint.source_mouth_order_index,
        source_band_index,
    ) {
        return true;
    }
    constraint.points_xz.iter().copied().all(|point| {
        generated_contact_constraint_endpoint_has_exact_source_authority(
            constraint,
            source_authority,
            owners,
            source_band_index,
            road_point_key(point),
        )
    })
}

fn generated_contact_constraint_endpoint_has_exact_source_authority(
    constraint: &NodeRailConstraint,
    source_authority: &ExactGeneratedSourceAuthority,
    owners: [Option<NodeBandOwner>; 2],
    source_band_index: Option<usize>,
    key: NodeRailPointKey,
) -> bool {
    source_authority.has_exact_point(
        owners,
        constraint.source_mouth_order_index,
        source_band_index,
        key,
    ) || source_authority.has_exact_source_key(
        constraint.kind,
        owners,
        constraint.source_mouth_order_index,
        constraint.source_band_index,
        key,
    ) || source_authority.has_exact_same_kind_source_handoff_key(
        constraint.kind,
        owners,
        constraint.source_mouth_order_index,
        constraint.source_band_index,
        key,
    ) || source_authority.has_exact_cross_source_same_kind_contact_key(
        constraint.kind,
        owners,
        constraint.source_mouth_order_index,
        constraint.source_band_index,
        key,
    )
}

pub(super) fn validate_generated_contact_constraint_endpoints_from_sources(
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
    generated_constraint_start_index: usize,
) -> Result<(), NodeRailGenerationError> {
    let source_authority = ExactGeneratedSourceAuthority::from_sources(
        contours,
        constraints,
        generated_constraint_start_index,
    );
    for constraint in constraints.iter().skip(generated_constraint_start_index) {
        if generated_contact_kind_from_constraint(constraint.kind).is_none() {
            continue;
        }
        let source_band_index = constraint.source_band_index;
        if constraint.owner.is_none() || constraint.opposite_owner.is_none() {
            continue;
        }
        let owners = [constraint.owner, constraint.opposite_owner];
        if !source_authority.has_any_source(
            owners,
            constraint.source_mouth_order_index,
            source_band_index,
        ) {
            continue;
        }
        for point in &constraint.points_xz {
            let key = road_point_key(*point);
            if generated_contact_constraint_endpoint_has_exact_source_authority(
                constraint,
                &source_authority,
                owners,
                source_band_index,
                key,
            ) {
                continue;
            }
            return Err(
                NodeRailGenerationError::NonCanonicalGeneratedContactEndpoint {
                    kind: constraint.kind,
                    mouth_order_index: constraint.source_mouth_order_index,
                    band_index: constraint.source_band_index,
                    owner: constraint.owner,
                    opposite_owner: constraint.opposite_owner,
                    point_x_key: key.0,
                    point_z_key: key.1,
                },
            );
        }
    }
    Ok(())
}

struct ExactGeneratedSourceAuthority {
    keys_by_owner: BTreeMap<NodeBandOwner, BTreeSet<NodeRailPointKey>>,
    segments_by_owner: BTreeMap<NodeBandOwner, BTreeSet<GeneratedContourEdgeKey>>,
    keys_by_source: BTreeMap<(NodeBandOwner, usize, usize), BTreeSet<NodeRailPointKey>>,
    segments_by_contact_source: BTreeMap<
        (
            NodeRailConstraintKind,
            NodeBandOwner,
            NodeBandOwner,
            usize,
            Option<usize>,
        ),
        BTreeSet<GeneratedContourEdgeKey>,
    >,
}

impl ExactGeneratedSourceAuthority {
    fn from_sources(
        contours: &[NodeGeneratedContour],
        constraints: &[NodeRailConstraint],
        generated_constraint_start_index: usize,
    ) -> Self {
        let mut keys_by_owner = BTreeMap::<NodeBandOwner, BTreeSet<NodeRailPointKey>>::new();
        let mut segments_by_owner =
            BTreeMap::<NodeBandOwner, BTreeSet<GeneratedContourEdgeKey>>::new();
        let mut keys_by_source =
            BTreeMap::<(NodeBandOwner, usize, usize), BTreeSet<NodeRailPointKey>>::new();
        let mut segments_by_contact_source = BTreeMap::<
            (
                NodeRailConstraintKind,
                NodeBandOwner,
                NodeBandOwner,
                usize,
                Option<usize>,
            ),
            BTreeSet<GeneratedContourEdgeKey>,
        >::new();
        for contour in contours {
            let Some(owner) = contour.owner else {
                continue;
            };
            let keys = generated_contour_keys(contour);
            keys_by_owner
                .entry(owner)
                .or_default()
                .extend(keys.iter().copied());
            segments_by_owner.entry(owner).or_default().extend(
                generated_contour_directed_edges(contour)
                    .into_iter()
                    .map(|edge| GeneratedContourEdgeKey::new(edge.start, edge.end)),
            );
            let Some(source_band_index) = contour.source_band_index else {
                continue;
            };
            keys_by_source
                .entry((owner, contour.source_mouth_order_index, source_band_index))
                .or_default()
                .extend(keys.into_iter());
        }
        for constraint in constraints.iter().take(generated_constraint_start_index) {
            if generated_contact_kind_from_constraint(constraint.kind).is_none() {
                continue;
            }
            if let Some(source_band_index) = constraint.source_band_index {
                let owners = [constraint.owner, constraint.opposite_owner];
                for owner in owners.into_iter().flatten() {
                    keys_by_source
                        .entry((
                            owner,
                            constraint.source_mouth_order_index,
                            source_band_index,
                        ))
                        .or_default()
                        .extend(constraint.points_xz.iter().copied().map(road_point_key));
                }
            }
            let (Some(owner), Some(opposite_owner)) = (constraint.owner, constraint.opposite_owner)
            else {
                continue;
            };
            let Some((owner, opposite_owner)) =
                exact_generated_contact_owner_pair(constraint.kind, owner, opposite_owner)
            else {
                continue;
            };
            segments_by_contact_source
                .entry((
                    constraint.kind,
                    owner,
                    opposite_owner,
                    constraint.source_mouth_order_index,
                    constraint.source_band_index,
                ))
                .or_default()
                .extend(
                    generated_constraint_directed_edges(constraint)
                        .into_iter()
                        .map(|edge| GeneratedContourEdgeKey::new(edge.start, edge.end)),
                );
        }
        Self {
            keys_by_owner,
            segments_by_owner,
            keys_by_source,
            segments_by_contact_source,
        }
    }

    fn has_any_source(
        &self,
        owners: [Option<NodeBandOwner>; 2],
        source_mouth_order_index: usize,
        source_band_index: Option<usize>,
    ) -> bool {
        let has_source_contour = source_band_index.is_some_and(|source_band_index| {
            owners.into_iter().flatten().any(|owner| {
                self.keys_by_source.contains_key(&(
                    owner,
                    source_mouth_order_index,
                    source_band_index,
                ))
            })
        });
        if has_source_contour {
            return true;
        }
        let (Some(owner), Some(opposite_owner)) = (owners[0], owners[1]) else {
            return false;
        };
        self.segments_by_contact_source.keys().any(
            |(_, source_owner, source_opposite_owner, source_mouth, source_band)| {
                *source_mouth == source_mouth_order_index
                    && *source_band == source_band_index
                    && owners_match_unordered(
                        Some(*source_owner),
                        Some(*source_opposite_owner),
                        owner,
                        opposite_owner,
                    )
            },
        )
    }

    fn has_exact_point(
        &self,
        owners: [Option<NodeBandOwner>; 2],
        source_mouth_order_index: usize,
        source_band_index: Option<usize>,
        point: NodeRailPointKey,
    ) -> bool {
        let Some(source_band_index) = source_band_index else {
            return false;
        };
        owners.into_iter().flatten().any(|owner| {
            self.keys_by_source
                .get(&(owner, source_mouth_order_index, source_band_index))
                .is_some_and(|keys| keys.contains(&point))
        })
    }

    fn has_exact_source_key(
        &self,
        kind: NodeRailConstraintKind,
        owners: [Option<NodeBandOwner>; 2],
        source_mouth_order_index: usize,
        source_band_index: Option<usize>,
        point: NodeRailPointKey,
    ) -> bool {
        let (Some(owner), Some(opposite_owner)) = (owners[0], owners[1]) else {
            return false;
        };
        let Some((owner, opposite_owner)) =
            exact_generated_contact_owner_pair(kind, owner, opposite_owner)
        else {
            return false;
        };
        self.segments_by_contact_source
            .get(&(
                kind,
                owner,
                opposite_owner,
                source_mouth_order_index,
                source_band_index,
            ))
            .is_some_and(|segments| generated_segments_have_endpoint(segments, point))
    }

    fn has_exact_same_kind_source_handoff_key(
        &self,
        kind: NodeRailConstraintKind,
        owners: [Option<NodeBandOwner>; 2],
        source_mouth_order_index: usize,
        source_band_index: Option<usize>,
        point: NodeRailPointKey,
    ) -> bool {
        let (Some(left_owner), Some(right_owner)) = (owners[0], owners[1]) else {
            return false;
        };
        self.has_exact_same_kind_source_handoff_side(
            kind,
            left_owner,
            right_owner,
            source_mouth_order_index,
            source_band_index,
            point,
        ) || self.has_exact_same_kind_source_handoff_side(
            kind,
            right_owner,
            left_owner,
            source_mouth_order_index,
            source_band_index,
            point,
        )
    }

    fn has_exact_same_kind_source_handoff_side(
        &self,
        kind: NodeRailConstraintKind,
        retained_owner: NodeBandOwner,
        final_owner: NodeBandOwner,
        source_mouth_order_index: usize,
        source_band_index: Option<usize>,
        point: NodeRailPointKey,
    ) -> bool {
        if !self.owner_geometry_has_exact_key(final_owner, point) {
            return false;
        }
        self.segments_by_contact_source
            .iter()
            .filter(|((source_kind, _, _, source_mouth, source_band), _)| {
                *source_kind == kind
                    && *source_mouth == source_mouth_order_index
                    && *source_band == source_band_index
            })
            .any(
                |((_, source_owner, source_opposite_owner, _, _), segments)| {
                    let same_kind_handoff = (*source_owner == retained_owner
                        && source_opposite_owner.kind() == final_owner.kind())
                        || (*source_opposite_owner == retained_owner
                            && source_owner.kind() == final_owner.kind());
                    same_kind_handoff && generated_segments_have_endpoint(segments, point)
                },
            )
    }

    fn has_exact_cross_source_same_kind_contact_key(
        &self,
        kind: NodeRailConstraintKind,
        owners: [Option<NodeBandOwner>; 2],
        source_mouth_order_index: usize,
        source_band_index: Option<usize>,
        point: NodeRailPointKey,
    ) -> bool {
        let (Some(left_owner), Some(right_owner)) = (owners[0], owners[1]) else {
            return false;
        };
        (self.has_same_kind_source_key_for_owner(
            kind,
            left_owner,
            right_owner.kind(),
            Some(source_mouth_order_index),
            source_band_index,
            point,
        ) && self.has_same_kind_source_key_for_owner(
            kind,
            right_owner,
            left_owner.kind(),
            None,
            None,
            point,
        )) || (self.has_same_kind_source_key_for_owner(
            kind,
            left_owner,
            right_owner.kind(),
            None,
            None,
            point,
        ) && self.has_same_kind_source_key_for_owner(
            kind,
            right_owner,
            left_owner.kind(),
            Some(source_mouth_order_index),
            source_band_index,
            point,
        ))
    }

    fn has_same_kind_source_key_for_owner(
        &self,
        kind: NodeRailConstraintKind,
        owner: NodeBandOwner,
        counterpart_kind: RoadSurfaceBandKind,
        required_source_mouth_order_index: Option<usize>,
        required_source_band_index: Option<usize>,
        point: NodeRailPointKey,
    ) -> bool {
        self.segments_by_contact_source
            .iter()
            .filter(|((source_kind, _, _, source_mouth, source_band), _)| {
                *source_kind == kind
                    && required_source_mouth_order_index
                        .is_none_or(|required| *source_mouth == required)
                    && required_source_band_index
                        .is_none_or(|required| *source_band == Some(required))
            })
            .any(
                |((_, source_owner, source_opposite_owner, _, _), segments)| {
                    let owner_matches = (*source_owner == owner
                        && source_opposite_owner.kind() == counterpart_kind)
                        || (*source_opposite_owner == owner
                            && source_owner.kind() == counterpart_kind);
                    owner_matches && generated_segments_have_endpoint(segments, point)
                },
            )
    }

    fn owner_geometry_has_exact_key(&self, owner: NodeBandOwner, point: NodeRailPointKey) -> bool {
        self.keys_by_owner
            .get(&owner)
            .is_some_and(|keys| keys.contains(&point))
            || self
                .segments_by_owner
                .get(&owner)
                .is_some_and(|segments| generated_segments_have_endpoint(segments, point))
    }
}

fn generated_segments_have_endpoint(
    segments: &BTreeSet<GeneratedContourEdgeKey>,
    point: NodeRailPointKey,
) -> bool {
    segments
        .iter()
        .any(|segment| segment.start == point || segment.end == point)
}

fn exact_generated_contact_owner_pair(
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Option<(NodeBandOwner, NodeBandOwner)> {
    if generated_contact_kind_from_constraint(kind).is_none() {
        return None;
    }
    if kind == NodeRailConstraintKind::RaisedStepContact {
        let pair = GeneratedRaisedStepOwnerPair::new(owner, opposite_owner)?;
        return Some((pair.owner, pair.opposite_owner));
    }
    Some((owner.min(opposite_owner), owner.max(opposite_owner)))
}

fn generated_contours_support_contact_noding(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
) -> bool {
    let Some(left_owner) = left.owner else {
        return false;
    };
    let Some(right_owner) = right.owner else {
        return false;
    };
    generated_raised_step_contact_kind_for_owners(left_owner, right_owner).is_some()
}

fn generated_contact_point_on_edge_noding_candidates(
    edge_contour: &NodeGeneratedContour,
    point_contour: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
) -> Vec<(GeneratedContourDirectedEdge, NodeRailPointKey)> {
    let mut candidates = Vec::new();
    let edge_keys = generated_contour_keys(edge_contour);
    for edge in generated_contour_directed_edges(edge_contour) {
        for point_key in generated_contour_keys(point_contour) {
            if edge_keys.contains(&point_key)
                || !generated_point_key_lies_on_segment(point_key, edge.start, edge.end)
                || !generated_contact_noding_point_has_explicit_roles(
                    edge_contour,
                    point_contour,
                    constraints,
                    point_key,
                )
            {
                continue;
            }
            candidates.push((edge, point_key));
        }
    }
    candidates
}

fn generated_contact_edge_intersection_noding_candidates(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
) -> Vec<(
    GeneratedContourDirectedEdge,
    GeneratedContourDirectedEdge,
    NodeRailPointKey,
)> {
    let mut candidates = Vec::new();
    for left_edge in generated_contour_directed_edges(left) {
        for right_edge in generated_contour_directed_edges(right) {
            let Some(intersection) = quantized_proper_segment_intersection(
                left_edge.start,
                left_edge.end,
                right_edge.start,
                right_edge.end,
            ) else {
                continue;
            };
            if !generated_contact_noding_point_has_explicit_roles(
                left,
                right,
                constraints,
                intersection,
            ) {
                continue;
            }
            candidates.push((left_edge, right_edge, intersection));
        }
    }
    candidates
}

fn generated_contact_noding_point_has_explicit_roles(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    point: NodeRailPointKey,
) -> bool {
    let Some(left_kind) = generated_contour_band_kind(left) else {
        return false;
    };
    let Some(right_kind) = generated_contour_band_kind(right) else {
        return false;
    };
    let Some(left_owner) = left.owner else {
        return false;
    };
    let Some(right_owner) = right.owner else {
        return false;
    };
    let Some(contact_kind) = generated_raised_step_contact_kind_for_owners(left_owner, right_owner)
    else {
        return false;
    };
    generated_contact_point_has_explicit_roles(
        left_kind,
        right_kind,
        left,
        right,
        constraints,
        point,
        contact_kind,
    )
}

fn generated_raised_step_contact_kind_for_owners(
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
) -> Option<NodeRailConstraintKind> {
    GeneratedRaisedStepOwnerPair::new(left_owner, right_owner)
        .map(|_| NodeRailConstraintKind::RaisedStepContact)
}

pub(super) fn raised_step_band_kinds_can_contact(
    left_kind: RoadSurfaceBandKind,
    right_kind: RoadSurfaceBandKind,
) -> bool {
    raised_step_kinds_can_contact(left_kind, right_kind)
}

pub(super) fn generated_raised_step_boundary_role_for_owner(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Option<GeneratedSameBandBoundaryRole> {
    GeneratedRaisedStepOwnerPair::new(owner, opposite_owner)?;
    let owner_rank = raised_step_band_rank(owner.kind())?;
    let opposite_rank = raised_step_band_rank(opposite_owner.kind())?;
    if opposite_rank < owner_rank {
        Some(GeneratedSameBandBoundaryRole::LowerSide)
    } else if opposite_rank > owner_rank {
        Some(GeneratedSameBandBoundaryRole::RaisedSide)
    } else {
        None
    }
}

fn generated_same_band_point_contact_has_explicit_roles(
    kind: RoadSurfaceBandKind,
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    point: NodeRailPointKey,
) -> bool {
    generated_contour_supports_same_band_role(kind)
        && generated_same_band_boundary_role_at_contour_vertex(left, constraints, point).is_some()
        && generated_same_band_boundary_role_at_contour_vertex(right, constraints, point).is_some()
}

fn generated_contact_point_has_explicit_roles(
    left_kind: RoadSurfaceBandKind,
    right_kind: RoadSurfaceBandKind,
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    point: NodeRailPointKey,
    contact_kind: NodeRailConstraintKind,
) -> bool {
    if left_kind == right_kind {
        return generated_same_band_point_contact_has_explicit_roles(
            left_kind,
            left,
            right,
            constraints,
            point,
        );
    }
    match contact_kind {
        NodeRailConstraintKind::RaisedStepContact => {
            let Some(left_owner) = left.owner else {
                return false;
            };
            let Some(right_owner) = right.owner else {
                return false;
            };
            let Some(pair) = GeneratedRaisedStepOwnerPair::new(left_owner, right_owner) else {
                return false;
            };
            generated_exact_owner_pair_contact_authority_at_point(
                pair.owner,
                pair.opposite_owner,
                constraints,
                point,
            )
            .is_some()
        }
        _ => true,
    }
}

fn generated_same_band_contact_constraint_key(
    constraint: &NodeRailConstraint,
) -> Option<GeneratedSameBandContactConstraintKey> {
    generated_same_band_contact_constraint(constraint).map(GeneratedSameBandContactConstraint::key)
}

fn generated_same_band_contact_constraint(
    constraint: &NodeRailConstraint,
) -> Option<GeneratedSameBandContactConstraint> {
    let Some(kind) = generated_contact_kind_from_constraint(constraint.kind) else {
        return None;
    };
    let owner = constraint.owner?;
    let opposite_owner = constraint.opposite_owner?;
    if owner == opposite_owner {
        return None;
    }
    let points = constraint.points_xz.as_slice();
    if points.len() != 2 {
        return None;
    }
    let (owner, opposite_owner) = if kind == NodeRailConstraintKind::RaisedStepContact {
        let pair = GeneratedRaisedStepOwnerPair::new(owner, opposite_owner)?;
        (pair.owner, pair.opposite_owner)
    } else {
        (owner.min(opposite_owner), owner.max(opposite_owner))
    };
    Some(GeneratedSameBandContactConstraint {
        kind,
        owner,
        opposite_owner,
        start: road_point_key(points[0]),
        end: road_point_key(points[1]),
        source_mouth_order_index: constraint.source_mouth_order_index,
        source_band_index: constraint.source_band_index,
    })
}

fn generated_contact_kind_from_constraint(
    kind: NodeRailConstraintKind,
) -> Option<NodeRailConstraintKind> {
    match kind {
        NodeRailConstraintKind::AsphaltBoundary { .. }
        | NodeRailConstraintKind::RaisedStepContact => Some(kind),
        NodeRailConstraintKind::BandBoundary {
            left_kind,
            right_kind,
        } => raised_step_band_kinds_can_contact(left_kind, right_kind).then_some(kind),
        NodeRailConstraintKind::FullRoadbedContour
        | NodeRailConstraintKind::BandContour { .. }
        | NodeRailConstraintKind::SpanHandoff { .. }
        | NodeRailConstraintKind::FootprintSeam { .. } => None,
    }
}
