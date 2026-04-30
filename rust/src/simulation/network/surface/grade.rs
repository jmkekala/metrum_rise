//! Deterministic node-local grade fields for road-surface visual pieces.

use super::{
    NodeBandHeightDomain, RoadSurfaceBandKind, RoadSurfaceSystem, SAMPLE_EPSILON_M,
    WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2,
};
use godot::prelude::Vector2;

const NODE_GRADE_SNAP_DISTANCE_M: f32 = 0.003;
const NODE_GRADE_MAX_WEIGHTED_SAMPLES: usize = 6;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NodeSeamKind {
    SpanHandoff,
    AsphaltCurb,
    CurbSidewalk,
    Derived,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct NodeGradeConstraint {
    pub(super) kind: RoadSurfaceBandKind,
    pub(super) seam_kind: NodeSeamKind,
    pub(super) start_xz: Vector2,
    pub(super) end_xz: Vector2,
    pub(super) start_y_m: f32,
    pub(super) end_y_m: f32,
    pub(super) order: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct NodeGradeSample {
    pub(super) kind: RoadSurfaceBandKind,
    pub(super) height_m: f32,
    pub(super) distance_squared: f32,
    pub(super) constraint_order: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct NodeGradeField {
    constraints: Vec<NodeGradeConstraint>,
}

impl NodeGradeField {
    pub(super) fn from_domains(domains: &[NodeBandHeightDomain]) -> Self {
        let mut constraints = Vec::new();
        for (domain_index, domain) in domains.iter().enumerate() {
            let points = &domain.polygon.points_world;
            if points.is_empty() {
                continue;
            }
            if points.len() == 1 {
                let point = points[0];
                constraints.push(NodeGradeConstraint {
                    kind: domain.kind,
                    seam_kind: NodeSeamKind::Derived,
                    start_xz: Vector2::new(point.x, point.z),
                    end_xz: Vector2::new(point.x, point.z),
                    start_y_m: point.y,
                    end_y_m: point.y,
                    order: domain_index,
                });
                continue;
            }
            for point_index in 0..points.len() {
                let start = points[point_index];
                let end = points[(point_index + 1) % points.len()];
                constraints.push(NodeGradeConstraint {
                    kind: domain.kind,
                    seam_kind: Self::constraint_seam_kind(domain.kind),
                    start_xz: Vector2::new(start.x, start.z),
                    end_xz: Vector2::new(end.x, end.z),
                    start_y_m: start.y,
                    end_y_m: end.y,
                    order: domain_index
                        .saturating_mul(points.len())
                        .saturating_add(point_index),
                });
            }
        }
        constraints.sort_by(|a, b| {
            RoadSurfaceSystem::band_kind_sort_key(a.kind)
                .cmp(&RoadSurfaceSystem::band_kind_sort_key(b.kind))
                .then(a.order.cmp(&b.order))
                .then(a.start_xz.x.total_cmp(&b.start_xz.x))
                .then(a.start_xz.y.total_cmp(&b.start_xz.y))
                .then(a.end_xz.x.total_cmp(&b.end_xz.x))
                .then(a.end_xz.y.total_cmp(&b.end_xz.y))
        });
        Self { constraints }
    }

    pub(super) fn constraints_for_kind(
        &self,
        kind: RoadSurfaceBandKind,
    ) -> impl Iterator<Item = &NodeGradeConstraint> {
        self.constraints
            .iter()
            .filter(move |constraint| constraint.kind == kind)
    }

    pub(super) fn sample_material_height(
        &self,
        kind: RoadSurfaceBandKind,
        point_xz: Vector2,
    ) -> Option<f32> {
        self.sample_with_filter(point_xz, |constraint_kind| constraint_kind == kind)
            .map(|sample| sample.height_m)
    }

    pub(super) fn sample_walkable_height(&self, point_xz: Vector2) -> Option<f32> {
        self.sample_with_filter(point_xz, Self::is_walkable_height_kind)
            .map(|sample| sample.height_m)
    }

    pub(super) fn sample_footprint_height(&self, point_xz: Vector2) -> Option<f32> {
        self.sample_with_filter(point_xz, |_| true)
            .map(|sample| sample.height_m)
    }

    fn sample_with_filter(
        &self,
        point_xz: Vector2,
        accepts_kind: impl Fn(RoadSurfaceBandKind) -> bool,
    ) -> Option<NodeGradeSample> {
        let mut nearest = [None; NODE_GRADE_MAX_WEIGHTED_SAMPLES];
        for constraint in self
            .constraints
            .iter()
            .filter(|constraint| accepts_kind(constraint.kind))
        {
            Self::retain_nearest_sample(
                &mut nearest,
                Self::project_constraint(point_xz, *constraint),
            );
        }
        let closest = nearest[0]?;

        let snap_distance_squared = NODE_GRADE_SNAP_DISTANCE_M * NODE_GRADE_SNAP_DISTANCE_M;
        if closest.distance_squared <= snap_distance_squared {
            return Some(closest);
        }

        let mut weighted_height = 0.0;
        let mut weight_sum = 0.0;
        let weight_floor = 0.25 * SAMPLE_EPSILON_M * SAMPLE_EPSILON_M;
        for projection in nearest.into_iter().flatten() {
            let weight = 1.0 / projection.distance_squared.max(weight_floor);
            weighted_height += projection.height_m * weight;
            weight_sum += weight;
        }
        (weight_sum > 0.0).then_some(NodeGradeSample {
            kind: closest.kind,
            height_m: weighted_height / weight_sum,
            distance_squared: closest.distance_squared,
            constraint_order: closest.constraint_order,
        })
    }

    fn retain_nearest_sample(
        nearest: &mut [Option<NodeGradeSample>; NODE_GRADE_MAX_WEIGHTED_SAMPLES],
        candidate: NodeGradeSample,
    ) {
        let Some(insert_at) = nearest.iter().position(|current| {
            current.is_none_or(|current| Self::grade_sample_order(candidate, current).is_lt())
        }) else {
            return;
        };
        for index in (insert_at + 1..nearest.len()).rev() {
            nearest[index] = nearest[index - 1];
        }
        nearest[insert_at] = Some(candidate);
    }

    fn grade_sample_order(a: NodeGradeSample, b: NodeGradeSample) -> std::cmp::Ordering {
        a.distance_squared
            .total_cmp(&b.distance_squared)
            .then(a.constraint_order.cmp(&b.constraint_order))
            .then(
                RoadSurfaceSystem::band_kind_sort_key(a.kind)
                    .cmp(&RoadSurfaceSystem::band_kind_sort_key(b.kind)),
            )
    }

    fn project_constraint(point_xz: Vector2, constraint: NodeGradeConstraint) -> NodeGradeSample {
        let segment = constraint.end_xz - constraint.start_xz;
        let length_squared = segment.length_squared();
        let t = if length_squared <= WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2 {
            0.0
        } else {
            ((point_xz - constraint.start_xz).dot(segment) / length_squared).clamp(0.0, 1.0)
        };
        let closest = constraint.start_xz + segment * t;
        NodeGradeSample {
            kind: constraint.kind,
            height_m: constraint.start_y_m + (constraint.end_y_m - constraint.start_y_m) * t,
            distance_squared: point_xz.distance_squared_to(closest),
            constraint_order: constraint.order,
        }
    }

    fn constraint_seam_kind(kind: RoadSurfaceBandKind) -> NodeSeamKind {
        match kind {
            RoadSurfaceBandKind::Carriageway => NodeSeamKind::SpanHandoff,
            RoadSurfaceBandKind::CurbOrShoulder => NodeSeamKind::AsphaltCurb,
            RoadSurfaceBandKind::Sidewalk | RoadSurfaceBandKind::Footpath => {
                NodeSeamKind::CurbSidewalk
            }
            RoadSurfaceBandKind::Median
            | RoadSurfaceBandKind::Parking
            | RoadSurfaceBandKind::CycleTrack
            | RoadSurfaceBandKind::TramReservation => NodeSeamKind::Derived,
        }
    }

    fn is_walkable_height_kind(kind: RoadSurfaceBandKind) -> bool {
        matches!(
            kind,
            RoadSurfaceBandKind::Sidewalk
                | RoadSurfaceBandKind::Footpath
                | RoadSurfaceBandKind::CycleTrack
                | RoadSurfaceBandKind::Median
                | RoadSurfaceBandKind::Parking
                | RoadSurfaceBandKind::TramReservation
                | RoadSurfaceBandKind::CurbOrShoulder
        )
    }
}
