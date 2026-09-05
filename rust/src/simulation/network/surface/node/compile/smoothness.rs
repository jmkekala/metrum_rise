// SPDX-License-Identifier: GPL-2.0-only

//! Debug-only node top-surface smoothness metrics.

use super::super::arrangement::NodeBandHeightFieldId;
use super::*;
use std::collections::{BTreeMap, BTreeSet};

const SMOOTHNESS_MATERIALS: [RoadSurfaceBandKind; 3] = [
    RoadSurfaceBandKind::Carriageway,
    RoadSurfaceBandKind::CurbOrShoulder,
    RoadSurfaceBandKind::Sidewalk,
];
const PLANE_SOLVE_EPS: f64 = 1.0e-9;
const TRIANGLE_NORMAL_EPS: f64 = 1.0e-12;
const QUALITY_SAMPLE_LIMIT: usize = 6;
const SLIVER_AREA_M2: f64 = 1.0e-4;
const HIGH_ASPECT_RATIO: f64 = 50.0;
const STEEP_TRIANGLE_DEGREES: f64 = 45.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeSmoothnessEdgeKey {
    start: (i64, i64),
    end: (i64, i64),
}

#[derive(Clone, Copy)]
struct NodeSmoothnessEdgeSample {
    region_index: usize,
    owner_index: usize,
    triangle_index: usize,
    start: RoadVec3,
    end: RoadVec3,
    normal: RoadVec3,
    start_y_m: f64,
    end_y_m: f64,
    height_field_id: Option<NodeBandHeightFieldId>,
    triangle_aspect_ratio: f64,
    triangle_slope_degrees: f64,
}

#[derive(Clone, Copy)]
struct NodeSmoothnessPlaneFit {
    grade_x: f64,
    grade_z: f64,
    residual_max_m: f64,
    residual_avg_m: f64,
}

#[derive(Clone, Copy)]
struct NodeTriangleQualitySample {
    region_index: usize,
    owner_index: usize,
    triangle_index: usize,
    height_field_id: Option<NodeBandHeightFieldId>,
    area_m2: f64,
    min_edge_m: f64,
    max_edge_m: f64,
    aspect_ratio: f64,
    y_delta_m: f64,
    slope_degrees: f64,
    normal: RoadVec3,
    points: [RoadVec3; 3],
}

#[derive(Clone, Copy)]
struct NodeAdjacentQualitySample {
    left_region_index: usize,
    left_owner_index: usize,
    left_triangle_index: usize,
    left_height_field_id: Option<NodeBandHeightFieldId>,
    left_triangle_aspect_ratio: f64,
    left_triangle_slope_degrees: f64,
    right_region_index: usize,
    right_owner_index: usize,
    right_triangle_index: usize,
    right_height_field_id: Option<NodeBandHeightFieldId>,
    right_triangle_aspect_ratio: f64,
    right_triangle_slope_degrees: f64,
    start: RoadVec3,
    end: RoadVec3,
    shared_edge_length_m: f64,
    normal_angle_degrees: f64,
    endpoint_y_delta_m: f64,
}

#[derive(Default)]
pub(super) struct NodeSurfaceMaterialSmoothness {
    material: Option<RoadSurfaceBandKind>,
    region_count: usize,
    owner_count: usize,
    triangle_count: usize,
    vertex_sample_count: usize,
    height_field_count: usize,
    grade_authority_count: usize,
    y_min_m: f64,
    y_max_m: f64,
    min_triangle_area_m2: f64,
    min_triangle_edge_m: f64,
    max_triangle_edge_m: f64,
    max_triangle_aspect_ratio: f64,
    sliver_triangle_count: usize,
    high_aspect_triangle_count: usize,
    steep_triangle_count: usize,
    max_triangle_y_delta_m: f64,
    max_triangle_slope: f64,
    max_triangle_slope_degrees: f64,
    adjacent_edge_pairs: usize,
    same_height_field_adjacent_pairs: usize,
    cross_height_field_adjacent_pairs: usize,
    max_adjacent_normal_angle_degrees: f64,
    max_shared_edge_endpoint_y_delta_m: f64,
    max_cross_height_field_normal_angle_degrees: f64,
    max_cross_height_field_endpoint_y_delta_m: f64,
    plane_fit: Option<NodeSmoothnessPlaneFit>,
    triangle_quality_samples: Vec<NodeTriangleQualitySample>,
    adjacent_quality_samples: Vec<NodeAdjacentQualitySample>,
}

impl RoadSurfaceSystem {
    pub(super) fn log_node_surface_smoothness_detail(
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        regions: &NodeSurfaceRegionResult,
    ) {
        if !matches!(
            kind,
            RoadSurfaceVisualNodePieceKind::Bend | RoadSurfaceVisualNodePieceKind::JunctionN
        ) {
            return;
        }

        for material in SMOOTHNESS_MATERIALS {
            let smoothness = Self::node_surface_smoothness_for_material(regions, material);
            if smoothness.triangle_count == 0 && smoothness.region_count == 0 {
                continue;
            }
            Self::debug_log_node_surface_smoothness(node_id, kind, &smoothness);
            Self::debug_log_node_surface_triangle_quality(node_id, kind, &smoothness);
        }
    }

    fn debug_log_node_surface_smoothness(
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        smoothness: &NodeSurfaceMaterialSmoothness,
    ) {
        let Some(material) = smoothness.material else {
            return;
        };
        let (
            plane_fit,
            plane_grade,
            plane_slope_degrees,
            plane_residual_max_m,
            plane_residual_avg_m,
        ) = if let Some(fit) = smoothness.plane_fit {
            let grade = fit.grade_x.hypot(fit.grade_z);
            (
                "ok",
                grade,
                grade.atan().to_degrees(),
                fit.residual_max_m,
                fit.residual_avg_m,
            )
        } else {
            ("none", 0.0, 0.0, 0.0, 0.0)
        };

        crate::debug_log!(
            "road",
            "node_surface_smoothness_detail node={} kind={:?} material={:?} regions={} owners={} triangles={} vertex_samples={} height_fields={} grade_authorities={} y_min={:.3} y_max={:.3} y_span={:.3} max_triangle_y_delta={:.3} max_triangle_slope={:.5} max_triangle_slope_deg={:.3} adjacent_edge_pairs={} same_height_field_adjacent_pairs={} cross_height_field_adjacent_pairs={} max_adjacent_normal_angle_deg={:.3} max_shared_edge_endpoint_y_delta={:.3} max_cross_height_field_normal_angle_deg={:.3} max_cross_height_field_endpoint_y_delta={:.3} plane_fit={} plane_grade={:.5} plane_slope_deg={:.3} plane_residual_max={:.3} plane_residual_avg={:.3}",
            node_id,
            kind,
            material,
            smoothness.region_count,
            smoothness.owner_count,
            smoothness.triangle_count,
            smoothness.vertex_sample_count,
            smoothness.height_field_count,
            smoothness.grade_authority_count,
            smoothness.y_min_m,
            smoothness.y_max_m,
            smoothness.y_span_m(),
            smoothness.max_triangle_y_delta_m,
            smoothness.max_triangle_slope,
            smoothness.max_triangle_slope_degrees,
            smoothness.adjacent_edge_pairs,
            smoothness.same_height_field_adjacent_pairs,
            smoothness.cross_height_field_adjacent_pairs,
            smoothness.max_adjacent_normal_angle_degrees,
            smoothness.max_shared_edge_endpoint_y_delta_m,
            smoothness.max_cross_height_field_normal_angle_degrees,
            smoothness.max_cross_height_field_endpoint_y_delta_m,
            plane_fit,
            plane_grade,
            plane_slope_degrees,
            plane_residual_max_m,
            plane_residual_avg_m
        );
    }

    fn debug_log_node_surface_triangle_quality(
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        smoothness: &NodeSurfaceMaterialSmoothness,
    ) {
        let Some(material) = smoothness.material else {
            return;
        };
        crate::debug_log!(
            "road",
            "node_surface_triangle_quality_detail node={} kind={:?} material={:?} triangles={} sliver_triangles={} high_aspect_triangles={} steep_triangles={} min_area_m2={:.8} min_edge_m={:.6} max_edge_m={:.3} max_aspect_ratio={:.3} max_slope_deg={:.3} sample_triangles={} sample_adjacent_pairs={}",
            node_id,
            kind,
            material,
            smoothness.triangle_count,
            smoothness.sliver_triangle_count,
            smoothness.high_aspect_triangle_count,
            smoothness.steep_triangle_count,
            smoothness.min_triangle_area_m2,
            smoothness.min_triangle_edge_m,
            smoothness.max_triangle_edge_m,
            smoothness.max_triangle_aspect_ratio,
            smoothness.max_triangle_slope_degrees,
            smoothness.triangle_quality_samples.len(),
            smoothness.adjacent_quality_samples.len(),
        );
        for (rank, sample) in smoothness.triangle_quality_samples.iter().enumerate() {
            crate::debug_log!(
                "road",
                "node_surface_triangle_quality_sample node={} kind={:?} material={:?} rank={} region={} owner={} triangle={} height_field={:?} area_m2={:.8} min_edge_m={:.6} max_edge_m={:.3} aspect_ratio={:.3} y_delta_m={:.3} slope_deg={:.3} normal=({:.5},{:.5},{:.5}) p0=({:.3},{:.3},{:.3}) p1=({:.3},{:.3},{:.3}) p2=({:.3},{:.3},{:.3})",
                node_id,
                kind,
                material,
                rank,
                sample.region_index,
                sample.owner_index,
                sample.triangle_index,
                sample.height_field_id,
                sample.area_m2,
                sample.min_edge_m,
                sample.max_edge_m,
                sample.aspect_ratio,
                sample.y_delta_m,
                sample.slope_degrees,
                sample.normal.x,
                sample.normal.y,
                sample.normal.z,
                sample.points[0].x,
                sample.points[0].y,
                sample.points[0].z,
                sample.points[1].x,
                sample.points[1].y,
                sample.points[1].z,
                sample.points[2].x,
                sample.points[2].y,
                sample.points[2].z,
            );
        }
        for (rank, sample) in smoothness.adjacent_quality_samples.iter().enumerate() {
            crate::debug_log!(
                "road",
                "node_surface_adjacent_quality_sample node={} kind={:?} material={:?} rank={} normal_angle_deg={:.3} endpoint_y_delta_m={:.3} shared_edge_length_m={:.6} left_region={} left_owner={} left_triangle={} left_height_field={:?} left_aspect_ratio={:.3} left_slope_deg={:.3} right_region={} right_owner={} right_triangle={} right_height_field={:?} right_aspect_ratio={:.3} right_slope_deg={:.3} edge_start=({:.3},{:.3},{:.3}) edge_end=({:.3},{:.3},{:.3})",
                node_id,
                kind,
                material,
                rank,
                sample.normal_angle_degrees,
                sample.endpoint_y_delta_m,
                sample.shared_edge_length_m,
                sample.left_region_index,
                sample.left_owner_index,
                sample.left_triangle_index,
                sample.left_height_field_id,
                sample.left_triangle_aspect_ratio,
                sample.left_triangle_slope_degrees,
                sample.right_region_index,
                sample.right_owner_index,
                sample.right_triangle_index,
                sample.right_height_field_id,
                sample.right_triangle_aspect_ratio,
                sample.right_triangle_slope_degrees,
                sample.start.x,
                sample.start.y,
                sample.start.z,
                sample.end.x,
                sample.end.y,
                sample.end.z,
            );
        }
    }

    fn node_surface_smoothness_for_material(
        regions: &NodeSurfaceRegionResult,
        material: RoadSurfaceBandKind,
    ) -> NodeSurfaceMaterialSmoothness {
        let mut smoothness = NodeSurfaceMaterialSmoothness {
            material: Some(material),
            y_min_m: f64::INFINITY,
            y_max_m: f64::NEG_INFINITY,
            min_triangle_area_m2: f64::INFINITY,
            min_triangle_edge_m: f64::INFINITY,
            ..NodeSurfaceMaterialSmoothness::default()
        };
        let mut owners = BTreeSet::new();
        let mut height_fields = BTreeSet::new();
        let mut grade_authorities = BTreeSet::new();
        let mut plane_points = Vec::new();
        let mut edge_samples =
            BTreeMap::<NodeSmoothnessEdgeKey, Vec<NodeSmoothnessEdgeSample>>::new();

        for (region_index, region) in regions.owned_regions.iter().enumerate() {
            if region.kind != material {
                continue;
            }
            smoothness.region_count += 1;
            owners.insert(region.owner_index);
            let source = regions.node_top_surface_sources.get(region_index);
            if let Some(source) = source {
                height_fields.insert(source.height_field_id);
                grade_authorities.extend(
                    source
                        .vertex_sources
                        .iter()
                        .map(|source| source.grade_authority_index),
                );
                grade_authorities.extend(
                    source.triangle_sources.iter().flat_map(|sources| {
                        sources.iter().map(|source| source.grade_authority_index)
                    }),
                );
            }

            let height_field_id = source.map(|source| source.height_field_id);
            if region.polygon.triangles_world.is_empty() {
                for point in &region.polygon.points_world {
                    smoothness.record_point(*point);
                    plane_points.push(*point);
                }
                continue;
            }

            for (triangle_index, triangle) in region.polygon.triangles_world.iter().enumerate() {
                let Some(sample) = NodeTriangleQualitySample::from_triangle(
                    region_index,
                    region.owner_index,
                    triangle_index,
                    height_field_id,
                    *triangle,
                ) else {
                    continue;
                };
                smoothness.triangle_count += 1;
                smoothness.record_triangle(sample);
                plane_points.extend(triangle.iter().copied());
                Self::append_node_smoothness_edge_samples(&mut edge_samples, sample);
            }
        }

        smoothness.owner_count = owners.len();
        smoothness.height_field_count = height_fields.len();
        smoothness.grade_authority_count = grade_authorities.len();
        if smoothness.vertex_sample_count == 0 {
            smoothness.y_min_m = 0.0;
            smoothness.y_max_m = 0.0;
        }
        if smoothness.triangle_count == 0 {
            smoothness.min_triangle_area_m2 = 0.0;
            smoothness.min_triangle_edge_m = 0.0;
        }
        smoothness.plane_fit = node_smoothness_plane_fit(&plane_points);
        smoothness.record_adjacency(&edge_samples);
        smoothness.sort_and_limit_quality_samples();
        smoothness
    }

    fn append_node_smoothness_edge_samples(
        edge_samples: &mut BTreeMap<NodeSmoothnessEdgeKey, Vec<NodeSmoothnessEdgeSample>>,
        triangle: NodeTriangleQualitySample,
    ) {
        for edge_index in 0..3 {
            let start = triangle.points[edge_index];
            let end = triangle.points[(edge_index + 1) % 3];
            let Some((key, start, end)) = NodeSmoothnessEdgeKey::from_points(start, end) else {
                continue;
            };
            edge_samples
                .entry(key)
                .or_default()
                .push(NodeSmoothnessEdgeSample {
                    region_index: triangle.region_index,
                    owner_index: triangle.owner_index,
                    triangle_index: triangle.triangle_index,
                    start,
                    end,
                    normal: triangle.normal,
                    start_y_m: start.y,
                    end_y_m: end.y,
                    height_field_id: triangle.height_field_id,
                    triangle_aspect_ratio: triangle.aspect_ratio,
                    triangle_slope_degrees: triangle.slope_degrees,
                });
        }
    }
}

impl NodeSurfaceMaterialSmoothness {
    fn record_point(&mut self, point: RoadVec3) {
        self.vertex_sample_count += 1;
        self.y_min_m = self.y_min_m.min(point.y);
        self.y_max_m = self.y_max_m.max(point.y);
    }

    fn record_triangle(&mut self, sample: NodeTriangleQualitySample) {
        for point in sample.points {
            self.record_point(point);
        }
        self.min_triangle_area_m2 = self.min_triangle_area_m2.min(sample.area_m2);
        self.min_triangle_edge_m = self.min_triangle_edge_m.min(sample.min_edge_m);
        self.max_triangle_edge_m = self.max_triangle_edge_m.max(sample.max_edge_m);
        self.max_triangle_aspect_ratio = self.max_triangle_aspect_ratio.max(sample.aspect_ratio);
        if sample.area_m2 < SLIVER_AREA_M2 {
            self.sliver_triangle_count += 1;
        }
        if sample.aspect_ratio > HIGH_ASPECT_RATIO {
            self.high_aspect_triangle_count += 1;
        }
        if sample.slope_degrees > STEEP_TRIANGLE_DEGREES {
            self.steep_triangle_count += 1;
        }
        self.max_triangle_y_delta_m = self.max_triangle_y_delta_m.max(sample.y_delta_m);
        self.max_triangle_slope = self.max_triangle_slope.max(sample.slope_ratio());
        self.max_triangle_slope_degrees = self.max_triangle_slope_degrees.max(sample.slope_degrees);
        self.triangle_quality_samples.push(sample);
    }

    fn y_span_m(&self) -> f64 {
        self.y_max_m - self.y_min_m
    }

    fn record_adjacency(
        &mut self,
        edge_samples: &BTreeMap<NodeSmoothnessEdgeKey, Vec<NodeSmoothnessEdgeSample>>,
    ) {
        for samples in edge_samples.values() {
            if samples.len() < 2 {
                continue;
            }
            for left_index in 0..samples.len() {
                for right in samples.iter().skip(left_index + 1) {
                    let left = samples[left_index];
                    self.record_adjacent_pair(left, *right);
                }
            }
        }
    }

    fn record_adjacent_pair(
        &mut self,
        left: NodeSmoothnessEdgeSample,
        right: NodeSmoothnessEdgeSample,
    ) {
        self.adjacent_edge_pairs += 1;
        let normal_angle_degrees = normal_angle_degrees(left.normal, right.normal);
        let endpoint_y_delta_m = (left.start_y_m - right.start_y_m)
            .abs()
            .max((left.end_y_m - right.end_y_m).abs());
        self.max_adjacent_normal_angle_degrees = self
            .max_adjacent_normal_angle_degrees
            .max(normal_angle_degrees);
        self.max_shared_edge_endpoint_y_delta_m = self
            .max_shared_edge_endpoint_y_delta_m
            .max(endpoint_y_delta_m);
        self.adjacent_quality_samples
            .push(NodeAdjacentQualitySample {
                left_region_index: left.region_index,
                left_owner_index: left.owner_index,
                left_triangle_index: left.triangle_index,
                left_height_field_id: left.height_field_id,
                left_triangle_aspect_ratio: left.triangle_aspect_ratio,
                left_triangle_slope_degrees: left.triangle_slope_degrees,
                right_region_index: right.region_index,
                right_owner_index: right.owner_index,
                right_triangle_index: right.triangle_index,
                right_height_field_id: right.height_field_id,
                right_triangle_aspect_ratio: right.triangle_aspect_ratio,
                right_triangle_slope_degrees: right.triangle_slope_degrees,
                start: left.start,
                end: left.end,
                shared_edge_length_m: xz_distance(left.start, left.end),
                normal_angle_degrees,
                endpoint_y_delta_m,
            });

        match (left.height_field_id, right.height_field_id) {
            (Some(left_field), Some(right_field)) if left_field == right_field => {
                self.same_height_field_adjacent_pairs += 1;
            }
            (Some(_), Some(_)) => {
                self.cross_height_field_adjacent_pairs += 1;
                self.max_cross_height_field_normal_angle_degrees = self
                    .max_cross_height_field_normal_angle_degrees
                    .max(normal_angle_degrees);
                self.max_cross_height_field_endpoint_y_delta_m = self
                    .max_cross_height_field_endpoint_y_delta_m
                    .max(endpoint_y_delta_m);
            }
            _ => {}
        }
    }

    fn sort_and_limit_quality_samples(&mut self) {
        self.triangle_quality_samples.sort_by(|a, b| {
            b.slope_degrees
                .total_cmp(&a.slope_degrees)
                .then_with(|| b.aspect_ratio.total_cmp(&a.aspect_ratio))
                .then_with(|| a.area_m2.total_cmp(&b.area_m2))
                .then_with(|| a.region_index.cmp(&b.region_index))
                .then_with(|| a.triangle_index.cmp(&b.triangle_index))
        });
        self.triangle_quality_samples.truncate(QUALITY_SAMPLE_LIMIT);
        self.adjacent_quality_samples.sort_by(|a, b| {
            b.normal_angle_degrees
                .total_cmp(&a.normal_angle_degrees)
                .then_with(|| b.endpoint_y_delta_m.total_cmp(&a.endpoint_y_delta_m))
                .then_with(|| a.left_region_index.cmp(&b.left_region_index))
                .then_with(|| a.left_triangle_index.cmp(&b.left_triangle_index))
                .then_with(|| {
                    a.right_region_index
                        .cmp(&b.right_region_index)
                        .then_with(|| a.right_triangle_index.cmp(&b.right_triangle_index))
                })
        });
        self.adjacent_quality_samples.truncate(QUALITY_SAMPLE_LIMIT);
    }
}

impl NodeSmoothnessEdgeKey {
    fn from_points(start: RoadVec3, end: RoadVec3) -> Option<(Self, RoadVec3, RoadVec3)> {
        let start_key = keys::SurfaceXzKey::from_world_xz(start).raw_tuple();
        let end_key = keys::SurfaceXzKey::from_world_xz(end).raw_tuple();
        if start_key == end_key {
            return None;
        }
        if start_key <= end_key {
            Some((
                Self {
                    start: start_key,
                    end: end_key,
                },
                start,
                end,
            ))
        } else {
            Some((
                Self {
                    start: end_key,
                    end: start_key,
                },
                end,
                start,
            ))
        }
    }
}

impl NodeTriangleQualitySample {
    fn from_triangle(
        region_index: usize,
        owner_index: usize,
        triangle_index: usize,
        height_field_id: Option<NodeBandHeightFieldId>,
        points: [RoadVec3; 3],
    ) -> Option<Self> {
        let normal = normalized_triangle_normal(points)?;
        let edge_ab_m = xz_distance(points[0], points[1]);
        let edge_bc_m = xz_distance(points[1], points[2]);
        let edge_ca_m = xz_distance(points[2], points[0]);
        let min_edge_m = edge_ab_m.min(edge_bc_m).min(edge_ca_m);
        let max_edge_m = edge_ab_m.max(edge_bc_m).max(edge_ca_m);
        let area_m2 = triangle_xz_area_m2(points);
        let aspect_ratio = if area_m2 <= TRIANGLE_NORMAL_EPS {
            f64::INFINITY
        } else {
            max_edge_m * max_edge_m / (2.0 * area_m2)
        };
        let y_min = points
            .iter()
            .map(|point| point.y)
            .fold(f64::INFINITY, f64::min);
        let y_max = points
            .iter()
            .map(|point| point.y)
            .fold(f64::NEG_INFINITY, f64::max);

        Some(Self {
            region_index,
            owner_index,
            triangle_index,
            height_field_id,
            area_m2,
            min_edge_m,
            max_edge_m,
            aspect_ratio,
            y_delta_m: y_max - y_min,
            slope_degrees: triangle_slope_degrees_from_normal(normal),
            normal,
            points,
        })
    }

    fn slope_ratio(self) -> f64 {
        let vertical = self.normal.y.abs();
        if vertical <= TRIANGLE_NORMAL_EPS {
            f64::INFINITY
        } else {
            self.normal.x.hypot(self.normal.z) / vertical
        }
    }
}

fn normalized_triangle_normal(triangle: [RoadVec3; 3]) -> Option<RoadVec3> {
    let normal = (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]);
    let length_squared = normal.length_squared();
    if length_squared <= TRIANGLE_NORMAL_EPS {
        return None;
    }
    Some(normal / length_squared.sqrt())
}

fn normal_angle_degrees(left: RoadVec3, right: RoadVec3) -> f64 {
    let dot = left.dot(right).abs().clamp(0.0, 1.0);
    dot.acos().to_degrees()
}

fn triangle_slope_degrees_from_normal(normal: RoadVec3) -> f64 {
    let vertical = normal.y.abs();
    if vertical <= TRIANGLE_NORMAL_EPS {
        90.0
    } else {
        (normal.x.hypot(normal.z) / vertical).atan().to_degrees()
    }
}

fn xz_distance(a: RoadVec3, b: RoadVec3) -> f64 {
    (a.x - b.x).hypot(a.z - b.z)
}

fn triangle_xz_area_m2(points: [RoadVec3; 3]) -> f64 {
    ((points[1].x - points[0].x) * (points[2].z - points[0].z)
        - (points[1].z - points[0].z) * (points[2].x - points[0].x))
        .abs()
        * 0.5
}

fn node_smoothness_plane_fit(points: &[RoadVec3]) -> Option<NodeSmoothnessPlaneFit> {
    if points.len() < 3 {
        return None;
    }
    let inv_count = 1.0 / points.len() as f64;
    let x_origin = points.iter().map(|point| point.x).sum::<f64>() * inv_count;
    let z_origin = points.iter().map(|point| point.z).sum::<f64>() * inv_count;

    let mut s_x = 0.0;
    let mut s_z = 0.0;
    let mut s_y = 0.0;
    let mut s_xx = 0.0;
    let mut s_xz = 0.0;
    let mut s_zz = 0.0;
    let mut s_xy = 0.0;
    let mut s_zy = 0.0;
    for point in points {
        let x = point.x - x_origin;
        let z = point.z - z_origin;
        s_x += x;
        s_z += z;
        s_y += point.y;
        s_xx += x * x;
        s_xz += x * z;
        s_zz += z * z;
        s_xy += x * point.y;
        s_zy += z * point.y;
    }

    let solution = solve_3x3(
        [
            [s_xx, s_xz, s_x],
            [s_xz, s_zz, s_z],
            [s_x, s_z, points.len() as f64],
        ],
        [s_xy, s_zy, s_y],
    )?;
    let grade_x = solution[0];
    let grade_z = solution[1];
    let plane_y_at_origin = solution[2];

    let mut residual_max_m: f64 = 0.0;
    let mut residual_sum_m = 0.0;
    for point in points {
        let fitted_y =
            grade_x * (point.x - x_origin) + grade_z * (point.z - z_origin) + plane_y_at_origin;
        let residual = (point.y - fitted_y).abs();
        residual_max_m = residual_max_m.max(residual);
        residual_sum_m += residual;
    }

    Some(NodeSmoothnessPlaneFit {
        grade_x,
        grade_z,
        residual_max_m,
        residual_avg_m: residual_sum_m * inv_count,
    })
}

fn solve_3x3(mut matrix: [[f64; 3]; 3], mut rhs: [f64; 3]) -> Option<[f64; 3]> {
    for column in 0..3 {
        let mut pivot_row = column;
        let mut pivot_abs = matrix[column][column].abs();
        for (row, values) in matrix.iter().enumerate().skip(column + 1) {
            let candidate_abs = values[column].abs();
            if candidate_abs > pivot_abs {
                pivot_abs = candidate_abs;
                pivot_row = row;
            }
        }
        if pivot_abs <= PLANE_SOLVE_EPS {
            return None;
        }
        if pivot_row != column {
            matrix.swap(column, pivot_row);
            rhs.swap(column, pivot_row);
        }
        let pivot = matrix[column][column];
        for value in matrix[column].iter_mut().skip(column) {
            *value /= pivot;
        }
        rhs[column] /= pivot;

        let pivot_values = matrix[column];
        let pivot_rhs = rhs[column];
        for row in 0..3 {
            if row == column {
                continue;
            }
            let factor = matrix[row][column];
            if factor.abs() <= PLANE_SOLVE_EPS {
                continue;
            }
            for value_column in column..3 {
                matrix[row][value_column] -= factor * pivot_values[value_column];
            }
            rhs[row] -= factor * pivot_rhs;
        }
    }
    Some(rhs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(kind: RoadSurfaceBandKind, triangles: Vec<[RoadVec3; 3]>) -> NodeOwnedRegion {
        NodeOwnedRegion {
            kind,
            owner_index: 0,
            polygon: RoadSurfaceVisualPolygon::from_parts(Vec::new(), triangles),
        }
    }

    fn source(kind: RoadSurfaceBandKind) -> NodeTopSurfacePolygonSource {
        NodeTopSurfacePolygonSource {
            kind,
            owner_index: 0,
            height_field_id: NodeBandHeightFieldId::new(0, 0, kind),
            vertex_keys: Vec::new(),
            vertex_height_mm: Vec::new(),
            vertex_sources: vec![NodeTopSurfaceVertexSource {
                grade_authority_index: 0,
            }],
            triangle_sources: Vec::new(),
        }
    }

    fn region_result(
        region: NodeOwnedRegion,
        source: NodeTopSurfacePolygonSource,
    ) -> NodeSurfaceRegionResult {
        NodeSurfaceRegionResult {
            outer_boundary_loops: Vec::new(),
            earthwork_boundary_segments: Vec::new(),
            terrain_clip_boundary_loops: Vec::new(),
            road_surface_polygons: Vec::new(),
            curb_surface_polygons: Vec::new(),
            raised_step_faces: Vec::new(),
            sidewalk_surface_polygons: Vec::new(),
            explicit_vertical_step_segments: Vec::new(),
            node_grade_authorities: Vec::new(),
            node_top_surface_sources: vec![source],
            owned_regions: vec![region],
            boolean_debug: None,
        }
    }

    #[test]
    fn smoothness_reports_planar_asphalt_without_residual() {
        let result = region_result(
            region(
                RoadSurfaceBandKind::Carriageway,
                vec![
                    [
                        RoadVec3::new(0.0, 0.0, 0.0),
                        RoadVec3::new(10.0, 1.0, 0.0),
                        RoadVec3::new(10.0, 1.0, 10.0),
                    ],
                    [
                        RoadVec3::new(0.0, 0.0, 0.0),
                        RoadVec3::new(10.0, 1.0, 10.0),
                        RoadVec3::new(0.0, 0.0, 10.0),
                    ],
                ],
            ),
            source(RoadSurfaceBandKind::Carriageway),
        );

        let smoothness = RoadSurfaceSystem::node_surface_smoothness_for_material(
            &result,
            RoadSurfaceBandKind::Carriageway,
        );

        assert_eq!(smoothness.triangle_count, 2);
        assert_eq!(smoothness.adjacent_edge_pairs, 1);
        assert!(smoothness.max_adjacent_normal_angle_degrees <= 1.0e-6);
        let fit = smoothness
            .plane_fit
            .expect("planar asphalt should expose a plane fit");
        assert!(fit.residual_max_m <= 1.0e-6);
    }

    #[test]
    fn smoothness_reports_folded_asphalt_residual_and_normal_angle() {
        let result = region_result(
            region(
                RoadSurfaceBandKind::Carriageway,
                vec![
                    [
                        RoadVec3::new(0.0, 0.0, 0.0),
                        RoadVec3::new(10.0, 0.0, 0.0),
                        RoadVec3::new(10.0, 0.0, 10.0),
                    ],
                    [
                        RoadVec3::new(0.0, 0.0, 0.0),
                        RoadVec3::new(10.0, 0.0, 10.0),
                        RoadVec3::new(0.0, 2.0, 10.0),
                    ],
                ],
            ),
            source(RoadSurfaceBandKind::Carriageway),
        );

        let smoothness = RoadSurfaceSystem::node_surface_smoothness_for_material(
            &result,
            RoadSurfaceBandKind::Carriageway,
        );

        assert!(smoothness.max_adjacent_normal_angle_degrees > 5.0);
        let fit = smoothness
            .plane_fit
            .expect("folded asphalt should still expose a best-fit plane");
        assert!(fit.residual_max_m > 0.1);
    }
}
