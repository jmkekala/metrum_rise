// SPDX-License-Identifier: GPL-2.0-only

//! Contact authority lookup for generated rail contact materialization.

use super::super::super::super::super::keys::surface_overlay_grid_collinearity_error_bound;
use super::*;
use std::collections::HashMap;

const GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS: i64 = 4096;
const GENERATED_CONTACT_AUTHORITY_TILE_KEYS: i64 = 8_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct GeneratedMaterialPointContactAuthority {
    pub(super) source_mouth_order_index: usize,
    pub(super) source_band_index: Option<usize>,
    pub(super) owner: Option<NodeBandOwner>,
    pub(super) opposite_owner: Option<NodeBandOwner>,
}

#[derive(Clone)]
pub(super) struct GeneratedContactAuthorityConstraint {
    pub(super) kind: NodeRailConstraintKind,
    pub(super) constraint_index: usize,
    pub(super) source_mouth_order_index: usize,
    pub(super) source_band_index: Option<usize>,
    pub(super) owner: Option<NodeBandOwner>,
    pub(super) opposite_owner: Option<NodeBandOwner>,
    pub(super) ordered_keys: Vec<NodeRailPointKey>,
    segments: Vec<GeneratedContactAuthoritySegment>,
    edges: Vec<GeneratedContactAuthorityEdge>,
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
}

#[derive(Clone, Copy)]
struct GeneratedContactAuthoritySegment {
    start: NodeRailPointKey,
    end: NodeRailPointKey,
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
    dx: i128,
    dz: i128,
    grid_collinearity_error_bound: i128,
}

#[derive(Clone, Copy)]
struct GeneratedContactAuthorityEdge {
    edge: GeneratedContourDirectedEdge,
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
}

impl GeneratedContactAuthorityEdge {
    fn new(edge: GeneratedContourDirectedEdge) -> Self {
        Self {
            edge,
            min_x: edge.start.0.min(edge.end.0),
            min_z: edge.start.1.min(edge.end.1),
            max_x: edge.start.0.max(edge.end.0),
            max_z: edge.start.1.max(edge.end.1),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct GeneratedContactAuthorityTile {
    x: i64,
    z: i64,
}

#[derive(Default)]
struct GeneratedContactAuthorityBucket {
    constraints: Vec<GeneratedContactAuthorityConstraint>,
    constraint_indices_by_tile: HashMap<GeneratedContactAuthorityTile, Vec<usize>>,
}

/// Pair-scoped view that avoids repeating the owner-pair lookup for every candidate point.
pub(in crate::simulation::network::surface::node::rails::contacts) struct GeneratedContactAuthorityPointQuery<
    'a,
> {
    bucket: Option<&'a GeneratedContactAuthorityBucket>,
}

pub(in crate::simulation::network::surface::node::rails::contacts) struct GeneratedContactAuthorityIndex
{
    buckets: Vec<GeneratedContactAuthorityBucket>,
    bucket_indices_by_owner_pair: HashMap<(NodeBandOwner, NodeBandOwner), usize>,
}

impl GeneratedContactAuthorityIndex {
    pub(in crate::simulation::network::surface::node::rails::contacts) fn new(
        constraints: &[NodeRailConstraint],
    ) -> Self {
        let mut buckets = Vec::<GeneratedContactAuthorityBucket>::new();
        let mut bucket_indices_by_owner_pair = HashMap::new();
        for constraint in constraints {
            if constraint.kind != NodeRailConstraintKind::RaisedStepContact {
                continue;
            }
            let (Some(owner), Some(opposite_owner)) = (constraint.owner, constraint.opposite_owner)
            else {
                continue;
            };
            let bucket_index =
                if let Some(&index) = bucket_indices_by_owner_pair.get(&(owner, opposite_owner)) {
                    index
                } else {
                    let index = buckets.len();
                    buckets.push(GeneratedContactAuthorityBucket::default());
                    bucket_indices_by_owner_pair.insert((owner, opposite_owner), index);
                    bucket_indices_by_owner_pair.insert((opposite_owner, owner), index);
                    index
                };
            buckets[bucket_index].push(GeneratedContactAuthorityConstraint::new(constraint));
        }
        Self {
            buckets,
            bucket_indices_by_owner_pair,
        }
    }

    fn bucket_for(
        &self,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
    ) -> Option<&GeneratedContactAuthorityBucket> {
        self.bucket_indices_by_owner_pair
            .get(&(owner, opposite_owner))
            .and_then(|&index| self.buckets.get(index))
    }

    fn constraints_for(
        &self,
        kind: NodeRailConstraintKind,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
    ) -> &[GeneratedContactAuthorityConstraint] {
        if kind != NodeRailConstraintKind::RaisedStepContact {
            return &[];
        }
        self.bucket_for(owner, opposite_owner)
            .map(|bucket| bucket.constraints.as_slice())
            .unwrap_or(&[])
    }

    fn constraints_for_point(
        &self,
        kind: NodeRailConstraintKind,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
        point: NodeRailPointKey,
    ) -> impl Iterator<Item = &GeneratedContactAuthorityConstraint> {
        (kind == NodeRailConstraintKind::RaisedStepContact)
            .then(|| self.bucket_for(owner, opposite_owner))
            .flatten()
            .into_iter()
            .flat_map(move |bucket| bucket.constraints_for_point(point))
    }

    /// Prepares repeated point-role queries for one raised-step owner pair.
    pub(in crate::simulation::network::surface::node::rails::contacts) fn point_query(
        &self,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
    ) -> GeneratedContactAuthorityPointQuery<'_> {
        GeneratedContactAuthorityPointQuery {
            bucket: self.bucket_for(owner, opposite_owner),
        }
    }

    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) fn has_constraints_touching_contour_pair(
        &self,
        kind: NodeRailConstraintKind,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
        left: &GeneratedContactContourSummary,
        right: &GeneratedContactContourSummary,
    ) -> bool {
        self.constraints_for(kind, owner, opposite_owner)
            .iter()
            .any(|authority_constraint| {
                authority_constraint.bounds_touch_summary(left)
                    && authority_constraint.bounds_touch_summary(right)
            })
    }

    pub(in crate::simulation::network::surface::node::rails::contacts) fn has_constraints_touching_bounds(
        &self,
        kind: NodeRailConstraintKind,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
        left_bounds: (i64, i64, i64, i64),
        right_bounds: (i64, i64, i64, i64),
    ) -> bool {
        self.constraints_for(kind, owner, opposite_owner)
            .iter()
            .any(|authority_constraint| {
                authority_constraint.bounds_touch_bounds(left_bounds)
                    && authority_constraint.bounds_touch_bounds(right_bounds)
            })
    }

    /// Visits only constraints in the exact kind/owner bucket whose cached bounds touch both contours.
    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) fn visit_constraints_touching_contour_pair(
        &self,
        kind: NodeRailConstraintKind,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
        left: &GeneratedContactContourSummary,
        right: &GeneratedContactContourSummary,
        mut visit: impl FnMut(&GeneratedContactAuthorityConstraint),
    ) {
        for authority_constraint in self.constraints_for(kind, owner, opposite_owner) {
            if authority_constraint.bounds_touch_summary(left)
                && authority_constraint.bounds_touch_summary(right)
            {
                visit(authority_constraint);
            }
        }
    }
}

impl GeneratedContactAuthorityPointQuery<'_> {
    /// Returns whether a point lies on an explicit constraint in this owner-pair bucket.
    pub(in crate::simulation::network::surface::node::rails::contacts) fn has_explicit_roles(
        &self,
        point: NodeRailPointKey,
    ) -> bool {
        self.bucket.is_some_and(|bucket| {
            bucket.constraints_for_point(point).any(|constraint| {
                constraint.bounds_touch_point(point) && constraint.contains_point(point)
            })
        })
    }
}

impl GeneratedContactAuthorityBucket {
    fn push(&mut self, constraint: GeneratedContactAuthorityConstraint) {
        let index = self.constraints.len();
        if constraint.min_x <= constraint.max_x && constraint.min_z <= constraint.max_z {
            let min_tile = GeneratedContactAuthorityTile::from_point((
                constraint
                    .min_x
                    .saturating_sub(GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS),
                constraint
                    .min_z
                    .saturating_sub(GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS),
            ));
            let max_tile = GeneratedContactAuthorityTile::from_point((
                constraint
                    .max_x
                    .saturating_add(GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS),
                constraint
                    .max_z
                    .saturating_add(GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS),
            ));
            for x in min_tile.x..=max_tile.x {
                for z in min_tile.z..=max_tile.z {
                    self.constraint_indices_by_tile
                        .entry(GeneratedContactAuthorityTile { x, z })
                        .or_default()
                        .push(index);
                }
            }
        }
        self.constraints.push(constraint);
    }

    fn constraints_for_point(
        &self,
        point: NodeRailPointKey,
    ) -> impl Iterator<Item = &GeneratedContactAuthorityConstraint> {
        self.constraint_indices_by_tile
            .get(&GeneratedContactAuthorityTile::from_point(point))
            .into_iter()
            .flatten()
            .filter_map(|&index| self.constraints.get(index))
    }
}

impl GeneratedContactAuthorityTile {
    fn from_point(point: NodeRailPointKey) -> Self {
        Self {
            x: point.0.div_euclid(GENERATED_CONTACT_AUTHORITY_TILE_KEYS),
            z: point.1.div_euclid(GENERATED_CONTACT_AUTHORITY_TILE_KEYS),
        }
    }
}

impl GeneratedContactAuthorityConstraint {
    fn new(constraint: &NodeRailConstraint) -> Self {
        let ordered_keys = constraint
            .points_xz
            .iter()
            .copied()
            .map(road_point_key)
            .collect::<Vec<_>>();
        let (mut min_x, mut min_z) = (i64::MAX, i64::MAX);
        let (mut max_x, mut max_z) = (i64::MIN, i64::MIN);
        for &point in &ordered_keys {
            min_x = min_x.min(point.0);
            min_z = min_z.min(point.1);
            max_x = max_x.max(point.0);
            max_z = max_z.max(point.1);
        }
        if constraint.points_xz.is_empty() {
            min_x = 1;
            min_z = 1;
            max_x = 0;
            max_z = 0;
        }
        let segments = ordered_keys
            .windows(2)
            .map(|segment| GeneratedContactAuthoritySegment::new(segment[0], segment[1]))
            .collect();
        let edges = ordered_keys
            .windows(2)
            .filter_map(|segment| {
                (segment[0] != segment[1]).then_some(GeneratedContourDirectedEdge {
                    start: segment[0],
                    end: segment[1],
                })
            })
            .map(GeneratedContactAuthorityEdge::new)
            .collect();
        Self {
            kind: constraint.kind,
            constraint_index: constraint.constraint_index,
            source_mouth_order_index: constraint.source_mouth_order_index,
            source_band_index: constraint.source_band_index,
            owner: constraint.owner,
            opposite_owner: constraint.opposite_owner,
            ordered_keys,
            segments,
            edges,
            min_x,
            min_z,
            max_x,
            max_z,
        }
    }

    fn bounds_touch_summary(&self, summary: &GeneratedContactContourSummary) -> bool {
        self.bounds_touch_bounds((summary.min_x, summary.min_z, summary.max_x, summary.max_z))
    }

    fn bounds_touch_bounds(&self, bounds: (i64, i64, i64, i64)) -> bool {
        let (min_x, min_z, max_x, max_z) = bounds;
        if self.min_x > self.max_x || self.min_z > self.max_z || min_x > max_x || min_z > max_z {
            return false;
        }
        self.min_x - GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS <= max_x
            && min_x <= self.max_x + GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS
            && self.min_z - GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS <= max_z
            && min_z <= self.max_z + GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS
    }

    fn bounds_touch_edge(&self, edge: GeneratedContourEdgeKey) -> bool {
        let min_x = edge.start.0.min(edge.end.0);
        let min_z = edge.start.1.min(edge.end.1);
        let max_x = edge.start.0.max(edge.end.0);
        let max_z = edge.start.1.max(edge.end.1);
        self.min_x - GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS <= max_x
            && min_x <= self.max_x + GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS
            && self.min_z - GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS <= max_z
            && min_z <= self.max_z + GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS
    }

    fn bounds_touch_point(&self, point: NodeRailPointKey) -> bool {
        self.min_x - GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS <= point.0
            && point.0 <= self.max_x + GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS
            && self.min_z - GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS <= point.1
            && point.1 <= self.max_z + GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS
    }

    fn contains_point(&self, point: NodeRailPointKey) -> bool {
        self.segments
            .iter()
            .any(|segment| segment.contains_point(point))
    }

    fn contains_edge(&self, edge: GeneratedContourEdgeKey) -> bool {
        self.segments
            .iter()
            .any(|segment| segment.contains_point(edge.start) && segment.contains_point(edge.end))
    }
}

impl GeneratedContactAuthoritySegment {
    fn new(start: NodeRailPointKey, end: NodeRailPointKey) -> Self {
        let dx = i128::from(end.0 - start.0);
        let dz = i128::from(end.1 - start.1);
        Self {
            start,
            end,
            min_x: start.0.min(end.0),
            min_z: start.1.min(end.1),
            max_x: start.0.max(end.0),
            max_z: start.1.max(end.1),
            dx,
            dz,
            grid_collinearity_error_bound: surface_overlay_grid_collinearity_error_bound(dx, dz),
        }
    }

    fn contains_point(&self, point: NodeRailPointKey) -> bool {
        if point == self.start || point == self.end {
            return true;
        }
        if self.start == self.end
            || (self.dx != 0 && (point.0 < self.min_x || point.0 > self.max_x))
            || (self.dz != 0 && (point.1 < self.min_z || point.1 > self.max_z))
        {
            return false;
        }
        let point_dx = i128::from(point.0 - self.start.0);
        let point_dz = i128::from(point.1 - self.start.1);
        let cross = self.dx * point_dz - self.dz * point_dx;
        cross == 0 || cross.abs() <= self.grid_collinearity_error_bound
    }
}

pub(super) fn generated_contact_authority_source_edges_touching_contour_pair(
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    left_summary: &GeneratedContactContourSummary,
    right_summary: &GeneratedContactContourSummary,
    authority_index: &GeneratedContactAuthorityIndex,
) -> Vec<GeneratedContourDirectedEdge> {
    let mut edges = authority_index
        .constraints_for(kind, owner, opposite_owner)
        .iter()
        .filter(|authority_constraint| {
            authority_constraint.bounds_touch_summary(left_summary)
                && authority_constraint.bounds_touch_summary(right_summary)
        })
        .flat_map(|authority_constraint| {
            authority_constraint
                .edges
                .iter()
                .filter(|edge| {
                    generated_edge_bounds_touch_summary(edge, left_summary)
                        && generated_edge_bounds_touch_summary(edge, right_summary)
                })
                .map(|edge| edge.edge)
        })
        .collect::<Vec<_>>();
    edges.sort_unstable();
    edges.dedup();
    edges
}

fn generated_edge_bounds_touch_summary(
    edge: &GeneratedContactAuthorityEdge,
    summary: &GeneratedContactContourSummary,
) -> bool {
    edge.min_x - GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS <= summary.max_x
        && summary.min_x <= edge.max_x + GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS
        && edge.min_z - GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS <= summary.max_z
        && summary.min_z <= edge.max_z + GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS
}

pub(super) fn append_generated_material_authority_points_on_counterpart_contour(
    kind: NodeRailConstraintKind,
    left_summary: &GeneratedContactContourSummary,
    right_summary: &GeneratedContactContourSummary,
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
    authority_index: &GeneratedContactAuthorityIndex,
    points: &mut Vec<NodeRailPointKey>,
) {
    for authority_constraint in authority_index.constraints_for(kind, left_owner, right_owner) {
        let touches_left = authority_constraint.bounds_touch_summary(left_summary);
        let touches_right = authority_constraint.bounds_touch_summary(right_summary);
        if !touches_left && !touches_right {
            continue;
        }
        for &point in &authority_constraint.ordered_keys {
            if touches_right
                && right_summary.bounds_contain_key(point)
                && right_summary.point_location.contains_key(point)
            {
                points.push(point);
            }
            if touches_left
                && left_summary.bounds_contain_key(point)
                && left_summary.point_location.contains_key(point)
            {
                points.push(point);
            }
        }
        if touches_right {
            append_generated_constraint_contour_contact_points(
                authority_constraint,
                right_summary,
                points,
            );
        }
        if touches_left {
            append_generated_constraint_contour_contact_points(
                authority_constraint,
                left_summary,
                points,
            );
        }
    }
}

fn append_generated_constraint_contour_contact_points(
    authority_constraint: &GeneratedContactAuthorityConstraint,
    contour_summary: &GeneratedContactContourSummary,
    points: &mut Vec<NodeRailPointKey>,
) {
    for constraint_edge in &authority_constraint.edges {
        let x_last = contour_summary
            .edges_by_min_x
            .partition_point(|edge| edge.min_x <= constraint_edge.max_x);
        let z_last = contour_summary
            .edges_by_min_z
            .partition_point(|edge| edge.min_z <= constraint_edge.max_z);
        let contour_edges = if x_last <= z_last {
            &contour_summary.edges_by_min_x[..x_last]
        } else {
            &contour_summary.edges_by_min_z[..z_last]
        };
        for contour_edge in contour_edges {
            if contour_edge.max_x < constraint_edge.min_x
                || constraint_edge.max_x < contour_edge.min_x
                || contour_edge.max_z < constraint_edge.min_z
                || constraint_edge.max_z < contour_edge.min_z
            {
                continue;
            }
            let contour_edge = contour_edge.edge;
            append_quantized_segment_contact_points(
                constraint_edge.edge.start,
                constraint_edge.edge.end,
                contour_edge.start,
                contour_edge.end,
                points,
            );
        }
    }
}

pub(super) fn generated_material_point_contact_authority(
    kind: NodeRailConstraintKind,
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
    point: NodeRailPointKey,
    authority_index: &GeneratedContactAuthorityIndex,
) -> Option<GeneratedMaterialPointContactAuthority> {
    authority_index
        .constraints_for_point(kind, left_owner, right_owner, point)
        .filter(|authority_constraint| authority_constraint.bounds_touch_point(point))
        .filter(|authority_constraint| authority_constraint.contains_point(point))
        .min_by_key(|authority_constraint| authority_constraint.constraint_index)
        .map(
            |authority_constraint| GeneratedMaterialPointContactAuthority {
                source_mouth_order_index: authority_constraint.source_mouth_order_index,
                source_band_index: authority_constraint.source_band_index,
                owner: authority_constraint.owner,
                opposite_owner: authority_constraint.opposite_owner,
            },
        )
}

fn generated_exact_owner_pair_contact_authority_for_edge(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    authority_index: &GeneratedContactAuthorityIndex,
    edge: GeneratedContourEdgeKey,
) -> Option<GeneratedMaterialPointContactAuthority> {
    authority_index
        .constraints_for_point(
            NodeRailConstraintKind::RaisedStepContact,
            owner,
            opposite_owner,
            edge.start,
        )
        .filter(|authority_constraint| authority_constraint.bounds_touch_edge(edge))
        .filter(|authority_constraint| authority_constraint.contains_edge(edge))
        .min_by_key(|authority_constraint| authority_constraint.constraint_index)
        .map(
            |authority_constraint| GeneratedMaterialPointContactAuthority {
                source_mouth_order_index: authority_constraint.source_mouth_order_index,
                source_band_index: authority_constraint.source_band_index,
                owner: authority_constraint.owner,
                opposite_owner: authority_constraint.opposite_owner,
            },
        )
}

pub(super) fn generated_exact_owner_pair_contact_authority_at_point(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    authority_index: &GeneratedContactAuthorityIndex,
    point: NodeRailPointKey,
) -> Option<GeneratedMaterialPointContactAuthority> {
    generated_material_point_contact_authority(
        NodeRailConstraintKind::RaisedStepContact,
        owner,
        opposite_owner,
        point,
        authority_index,
    )
}

pub(super) fn generated_contact_edge_source_authority(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    authority_index: &GeneratedContactAuthorityIndex,
    edge: GeneratedContourEdgeKey,
) -> Option<GeneratedMaterialPointContactAuthority> {
    generated_exact_owner_pair_contact_authority_for_edge(
        owner,
        opposite_owner,
        authority_index,
        edge,
    )
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

pub(in crate::simulation::network::surface::node::rails::contacts) fn generated_contact_point_has_explicit_roles(
    left_kind: RoadSurfaceBandKind,
    right_kind: RoadSurfaceBandKind,
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    authority_index: &GeneratedContactAuthorityIndex,
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
                authority_index,
                point,
            )
            .is_some()
        }
        _ => true,
    }
}
