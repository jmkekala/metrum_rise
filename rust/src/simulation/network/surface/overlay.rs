//! Overlay boolean geometry and node-region reconstruction for road surfaces.

use super::edge::{CURB_BAND_WIDTH_M, CURB_STEP_HEIGHT_M};
use super::{
    NODE_OVERLAY_MIN_AREA_M2, NodeBandHeightDomain, NodeBandHeightSample,
    NodeNonRoadCandidatePolygon, NodeOverlayContour, NodeOverlayEdgeKey, NodeOverlayPoint,
    NodeOverlayPointKey, NodeOverlayShape, NodeOverlayShapes, NodeSurfaceRegionResult,
    RoadSurfaceBandKind, RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind,
    RoadSurfaceVisualPolygon, SAMPLE_EPSILON_M, SurfaceCdt, WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2,
};
use godot::prelude::{Vector2, Vector3};
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::scale::FixedScaleFloatOverlay;
use i_overlay::float::simplify::SimplifyShape;
use spade::{Point2, Triangulation};
use std::collections::{BTreeMap, BTreeSet};

// Overlay boolean operations quantize coordinates to millimetres for deterministic keys.
const NODE_OVERLAY_SCALE: f32 = 1000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct NodeTargetHeightSample {
    height_delta_m: f32,
    priority: u8,
    distance_squared: f32,
    domain_index: usize,
    height_m: f32,
}

impl RoadSurfaceSystem {
    pub(super) fn resolve_node_surface_regions_with_overlay(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        road_candidates: &[RoadSurfaceVisualPolygon],
        non_road_candidates: &[NodeNonRoadCandidatePolygon],
        non_road_height_domains: &[NodeBandHeightDomain],
    ) -> Option<NodeSurfaceRegionResult> {
        let mut height_candidates = Vec::with_capacity(
            road_candidates
                .len()
                .saturating_add(non_road_candidates.len()),
        );
        height_candidates.extend(road_candidates.iter().cloned());
        height_candidates.extend(
            non_road_candidates
                .iter()
                .map(|candidate| candidate.polygon.clone()),
        );
        let road_height_domains = road_candidates
            .iter()
            .cloned()
            .map(|polygon| NodeBandHeightDomain {
                kind: RoadSurfaceBandKind::Carriageway,
                polygon,
            })
            .collect::<Vec<_>>();

        let road_contours = Self::overlay_contours_from_polygons(road_candidates);
        let mut road_shapes = Self::overlay_union_contours(&road_contours)?;

        let footprint_contours = Self::overlay_contours_from_polygons(&height_candidates);
        let mut footprint_shapes = Self::overlay_union_contours(&footprint_contours)?;

        let mut non_road_shapes = if road_shapes.is_empty() {
            footprint_shapes.clone()
        } else if footprint_shapes.is_empty() {
            Vec::new()
        } else {
            Self::overlay_binary_shapes(&footprint_shapes, &road_shapes, OverlayRule::Difference)?
        };
        Self::sort_overlay_shapes(&mut road_shapes);
        Self::sort_overlay_shapes(&mut non_road_shapes);
        Self::sort_overlay_shapes(&mut footprint_shapes);
        let mut resolved_non_road_height_domains = Self::boundary_curb_transition_domains(
            &road_shapes,
            &non_road_shapes,
            &road_height_domains,
            non_road_height_domains,
        );
        resolved_non_road_height_domains.extend(non_road_height_domains.iter().cloned());

        let mut road_surface_polygons = Self::visual_polygons_from_overlay_shapes_with_band_heights(
            node_id,
            piece_kind,
            "carriageway",
            &road_shapes,
            &road_height_domains,
        );
        let mut sidewalk_surface_polygons =
            Self::visual_non_road_band_polygons_from_height_domains(
                node_id,
                piece_kind,
                &non_road_shapes,
                &road_shapes,
                &resolved_non_road_height_domains,
            )?;
        let mut outer_boundary_loops = Self::outer_boundary_polygons_from_overlay_shapes(
            &footprint_shapes,
            &height_candidates,
        );

        if road_surface_polygons.is_empty() && sidewalk_surface_polygons.is_empty() {
            return None;
        }
        if outer_boundary_loops.is_empty() {
            return None;
        }

        Self::sort_visual_polygons(&mut road_surface_polygons);
        Self::sort_visual_polygons(&mut sidewalk_surface_polygons);
        Self::sort_visual_polygons(&mut outer_boundary_loops);
        Some(NodeSurfaceRegionResult {
            outer_boundary_loops,
            road_surface_polygons,
            sidewalk_surface_polygons,
        })
    }

    fn overlay_contours_from_polygons(
        polygons: &[RoadSurfaceVisualPolygon],
    ) -> Vec<NodeOverlayContour> {
        let mut contours = Vec::new();
        for polygon in polygons {
            let contour = Self::overlay_contour_from_world_points(&polygon.points_world);
            if Self::overlay_contour_area(&contour).abs() > NODE_OVERLAY_MIN_AREA_M2 {
                contours.push(contour);
            }
        }
        contours
    }

    fn overlay_contour_from_world_points(points_world: &[Vector3]) -> NodeOverlayContour {
        let mut contour = Vec::with_capacity(points_world.len());
        for point in points_world {
            let overlay_point = Self::overlay_point_from_world_point(*point);
            if contour
                .last()
                .is_none_or(|last: &NodeOverlayPoint| *last != overlay_point)
            {
                contour.push(overlay_point);
            }
        }
        if contour.len() >= 2 && contour.first() == contour.last() {
            contour.pop();
        }
        contour
    }

    fn overlay_point_from_world_point(point: Vector3) -> NodeOverlayPoint {
        [
            (point.x * NODE_OVERLAY_SCALE).round() / NODE_OVERLAY_SCALE,
            (point.z * NODE_OVERLAY_SCALE).round() / NODE_OVERLAY_SCALE,
        ]
    }

    pub(super) fn overlay_union_contours(
        contours: &[NodeOverlayContour],
    ) -> Option<NodeOverlayShapes> {
        if contours.is_empty() {
            return Some(Vec::new());
        }
        let shapes = contours.simplify_shape(FillRule::Positive);
        Some(Self::filter_overlay_shapes_by_area(shapes))
    }

    fn overlay_binary_shapes(
        subject: &NodeOverlayShapes,
        clip: &NodeOverlayShapes,
        rule: OverlayRule,
    ) -> Option<NodeOverlayShapes> {
        if subject.is_empty() {
            return Some(Vec::new());
        }
        if clip.is_empty() {
            return Some(subject.clone());
        }
        let shapes = subject
            .overlay_with_fixed_scale(clip, rule, FillRule::Positive, NODE_OVERLAY_SCALE)
            .ok()?;
        Some(Self::filter_overlay_shapes_by_area(shapes))
    }

    fn filter_overlay_shapes_by_area(shapes: NodeOverlayShapes) -> NodeOverlayShapes {
        shapes
            .into_iter()
            .filter_map(|shape| {
                let filtered = shape
                    .into_iter()
                    .filter(|contour| contour.len() >= 3)
                    .collect::<Vec<_>>();
                let outer = filtered.first()?;
                (Self::overlay_contour_area(outer).abs() > NODE_OVERLAY_MIN_AREA_M2)
                    .then_some(filtered)
            })
            .collect()
    }

    fn sort_overlay_shapes(shapes: &mut [NodeOverlayShape]) {
        shapes.sort_by(|a, b| {
            let area_a = a
                .first()
                .map(|contour| Self::overlay_contour_area(contour).abs())
                .unwrap_or(0.0);
            let area_b = b
                .first()
                .map(|contour| Self::overlay_contour_area(contour).abs())
                .unwrap_or(0.0);
            area_b
                .total_cmp(&area_a)
                .then_with(|| Self::overlay_shape_sort_key(a).cmp(&Self::overlay_shape_sort_key(b)))
        });
    }

    fn overlay_shape_sort_key(shape: &NodeOverlayShape) -> (i64, i64, usize) {
        let mut min_x = i64::MAX;
        let mut min_z = i64::MAX;
        let mut points = 0usize;
        for contour in shape {
            points += contour.len();
            for point in contour {
                min_x = min_x.min((point[0] * NODE_OVERLAY_SCALE).round() as i64);
                min_z = min_z.min((point[1] * NODE_OVERLAY_SCALE).round() as i64);
            }
        }
        (min_x, min_z, points)
    }

    fn overlay_contour_area(contour: &NodeOverlayContour) -> f32 {
        if contour.len() < 3 {
            return 0.0;
        }
        let mut signed_area = 0.0;
        for index in 0..contour.len() {
            let current = contour[index];
            let next = contour[(index + 1) % contour.len()];
            signed_area += current[0] * next[1] - next[0] * current[1];
        }
        signed_area * 0.5
    }

    fn visual_polygons_from_overlay_shapes_with_band_heights(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        material_name: &'static str,
        shapes: &[NodeOverlayShape],
        height_domains: &[NodeBandHeightDomain],
    ) -> Vec<RoadSurfaceVisualPolygon> {
        let mut polygons = Vec::new();
        for shape in shapes {
            let Some(polygon) = Self::visual_polygon_from_overlay_shape_with_band_heights(
                node_id,
                piece_kind,
                material_name,
                shape,
                height_domains,
                true,
            ) else {
                continue;
            };
            polygons.push(polygon);
        }
        Self::sort_visual_polygons(&mut polygons);
        polygons
    }

    pub(super) fn union_terrain_clip_polygons(
        polygons: &[RoadSurfaceVisualPolygon],
    ) -> Vec<RoadSurfaceVisualPolygon> {
        if polygons.is_empty() {
            return Vec::new();
        }

        let contours = Self::overlay_contours_from_polygons(polygons);
        let Some(mut shapes) = Self::overlay_union_contours(&contours) else {
            return Vec::new();
        };
        Self::sort_overlay_shapes(&mut shapes);
        Self::outer_boundary_polygons_from_overlay_shapes(&shapes, polygons)
    }

    fn outer_boundary_polygons_from_overlay_shapes(
        shapes: &[NodeOverlayShape],
        height_candidates: &[RoadSurfaceVisualPolygon],
    ) -> Vec<RoadSurfaceVisualPolygon> {
        let mut polygons = Vec::new();
        for shape in shapes {
            let Some(polygon) = Self::visual_polygon_from_overlay_shape_with_footprint_heights(
                shape,
                height_candidates,
                false,
            ) else {
                continue;
            };
            polygons.push(polygon);
        }
        Self::sort_visual_polygons(&mut polygons);
        polygons
    }

    fn visual_polygon_from_overlay_shape_with_band_heights(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        material_name: &'static str,
        shape: &NodeOverlayShape,
        height_domains: &[NodeBandHeightDomain],
        preserve_holes: bool,
    ) -> Option<RoadSurfaceVisualPolygon> {
        let outer_contour = shape.first()?;
        let mut outer_points = Self::world_points_from_overlay_contour_with_band_heights(
            node_id,
            piece_kind,
            material_name,
            outer_contour,
            height_domains,
        )?;
        if Self::signed_polygon_area_xz(&outer_points).abs() <= NODE_OVERLAY_MIN_AREA_M2 {
            return None;
        }
        if Self::signed_polygon_area_xz(&outer_points) < 0.0 {
            outer_points.reverse();
        }

        let mut hole_points = Vec::new();
        if preserve_holes {
            for contour in shape.iter().skip(1) {
                let mut points = Self::world_points_from_overlay_contour_with_band_heights(
                    node_id,
                    piece_kind,
                    material_name,
                    contour,
                    height_domains,
                )?;
                if Self::signed_polygon_area_xz(&points).abs() <= NODE_OVERLAY_MIN_AREA_M2 {
                    continue;
                }
                if Self::signed_polygon_area_xz(&points) > 0.0 {
                    points.reverse();
                }
                hole_points.push(points);
            }
        }

        Self::canonicalize_world_loop(&mut outer_points)?;
        for hole in &mut hole_points {
            Self::canonicalize_world_loop(hole)?;
        }
        let triangles_world = Self::triangulate_constrained_shape_xz(&outer_points, &hole_points)?;
        Some(RoadSurfaceVisualPolygon {
            points_world: outer_points,
            triangles_world,
        })
    }

    pub(super) fn boundary_curb_transition_domains(
        road_shapes: &NodeOverlayShapes,
        non_road_shapes: &NodeOverlayShapes,
        road_height_domains: &[NodeBandHeightDomain],
        non_road_height_domains: &[NodeBandHeightDomain],
    ) -> Vec<NodeBandHeightDomain> {
        let shared_segments = Self::overlay_shared_collinear_segments(road_shapes, non_road_shapes);
        if shared_segments.is_empty() {
            return Vec::new();
        }

        let mut domains = Vec::new();
        let mut joint_samples: BTreeMap<NodeOverlayPointKey, Vec<(Vector2, f32, Vector2, f32)>> =
            BTreeMap::new();
        for (start, end) in shared_segments {
            let start_xz = Vector2::new(start[0], start[1]);
            let end_xz = Vector2::new(end[0], end[1]);
            let Some(non_road_normal) = Self::non_road_normal_for_shared_overlay_segment(
                start_xz,
                end_xz,
                road_shapes,
                non_road_shapes,
            ) else {
                continue;
            };
            let outer_start_xz = start_xz + non_road_normal * CURB_BAND_WIDTH_M;
            let outer_end_xz = end_xz + non_road_normal * CURB_BAND_WIDTH_M;
            let Some(inner_start_y) =
                Self::sample_node_band_height_from_domains(start_xz, road_height_domains)
            else {
                continue;
            };
            let Some(inner_end_y) =
                Self::sample_node_band_height_from_domains(end_xz, road_height_domains)
            else {
                continue;
            };
            let Some(outer_start_y) = Self::sample_node_walkable_boundary_height(
                outer_start_xz,
                non_road_height_domains,
                inner_start_y + CURB_STEP_HEIGHT_M,
            ) else {
                continue;
            };
            let Some(outer_end_y) = Self::sample_node_walkable_boundary_height(
                outer_end_xz,
                non_road_height_domains,
                inner_end_y + CURB_STEP_HEIGHT_M,
            ) else {
                continue;
            };
            let Some(polygon) = Self::make_visual_polygon(vec![
                Vector3::new(start_xz.x, inner_start_y, start_xz.y),
                Vector3::new(end_xz.x, inner_end_y, end_xz.y),
                Vector3::new(outer_end_xz.x, outer_end_y, outer_end_xz.y),
                Vector3::new(outer_start_xz.x, outer_start_y, outer_start_xz.y),
            ]) else {
                continue;
            };
            domains.push(NodeBandHeightDomain {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
                polygon,
            });
            Self::push_boundary_curb_joint_sample(
                &mut joint_samples,
                start_xz,
                inner_start_y,
                outer_start_xz,
                outer_start_y,
            );
            Self::push_boundary_curb_joint_sample(
                &mut joint_samples,
                end_xz,
                inner_end_y,
                outer_end_xz,
                outer_end_y,
            );
        }
        Self::append_boundary_curb_joint_domains(&mut domains, joint_samples, road_shapes);
        domains
    }

    fn push_boundary_curb_joint_sample(
        samples_by_point: &mut BTreeMap<NodeOverlayPointKey, Vec<(Vector2, f32, Vector2, f32)>>,
        inner_xz: Vector2,
        inner_y: f32,
        outer_xz: Vector2,
        outer_y: f32,
    ) {
        let key = Self::overlay_point_key([inner_xz.x, inner_xz.y]);
        let samples = samples_by_point.entry(key).or_default();
        if samples.iter().any(|(_, _, existing_outer_xz, _)| {
            existing_outer_xz.distance_squared_to(outer_xz) <= WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2
        }) {
            return;
        }
        samples.push((inner_xz, inner_y, outer_xz, outer_y));
    }

    fn append_boundary_curb_joint_domains(
        domains: &mut Vec<NodeBandHeightDomain>,
        joint_samples: BTreeMap<NodeOverlayPointKey, Vec<(Vector2, f32, Vector2, f32)>>,
        road_shapes: &NodeOverlayShapes,
    ) {
        for samples in joint_samples.into_values() {
            if samples.len() < 2 {
                continue;
            }
            let (inner_xz, inner_y, _, _) = samples[0];
            let mut outer_samples = samples
                .iter()
                .map(|(_, _, outer_xz, outer_y)| (*outer_xz, *outer_y))
                .collect::<Vec<_>>();
            outer_samples.sort_by(|(a, _), (b, _)| {
                (a.y - inner_xz.y)
                    .atan2(a.x - inner_xz.x)
                    .total_cmp(&(b.y - inner_xz.y).atan2(b.x - inner_xz.x))
                    .then(a.x.total_cmp(&b.x))
                    .then(a.y.total_cmp(&b.y))
            });
            let pair_count = if outer_samples.len() == 2 {
                1
            } else {
                outer_samples.len()
            };
            for index in 0..pair_count {
                let (outer_a_xz, outer_a_y) = outer_samples[index];
                let (outer_b_xz, outer_b_y) = outer_samples[(index + 1) % outer_samples.len()];
                if outer_a_xz.distance_squared_to(outer_b_xz)
                    <= WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2
                {
                    continue;
                }
                let centroid = (inner_xz + outer_a_xz + outer_b_xz) / 3.0;
                if Self::overlay_shapes_contain_point(road_shapes, centroid) {
                    continue;
                }
                let Some(polygon) = Self::make_visual_polygon(vec![
                    Vector3::new(inner_xz.x, inner_y, inner_xz.y),
                    Vector3::new(outer_a_xz.x, outer_a_y, outer_a_xz.y),
                    Vector3::new(outer_b_xz.x, outer_b_y, outer_b_xz.y),
                ]) else {
                    continue;
                };
                domains.push(NodeBandHeightDomain {
                    kind: RoadSurfaceBandKind::CurbOrShoulder,
                    polygon,
                });
            }
        }
    }

    fn non_road_normal_for_shared_overlay_segment(
        start_xz: Vector2,
        end_xz: Vector2,
        road_shapes: &NodeOverlayShapes,
        non_road_shapes: &NodeOverlayShapes,
    ) -> Option<Vector2> {
        let segment = end_xz - start_xz;
        if segment.length_squared() <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
            return None;
        }
        let direction = segment.normalized();
        let left = Vector2::new(-direction.y, direction.x);
        let midpoint = (start_xz + end_xz) * 0.5;
        for normal in [left, -left] {
            for probe_distance_m in [
                (SAMPLE_EPSILON_M * 4.0).max(0.002),
                CURB_BAND_WIDTH_M * 0.5,
                CURB_BAND_WIDTH_M,
            ] {
                let probe = midpoint + normal * probe_distance_m;
                if Self::overlay_shapes_contain_point(non_road_shapes, probe)
                    && !Self::overlay_shapes_contain_point(road_shapes, probe)
                {
                    return Some(normal);
                }
            }
        }
        None
    }

    fn sample_node_walkable_boundary_height(
        point_xz: Vector2,
        domains: &[NodeBandHeightDomain],
        target_height_m: f32,
    ) -> Option<f32> {
        let mut best = None;
        Self::visit_node_band_height_samples(
            point_xz,
            domains,
            Self::is_walkable_boundary_height_kind,
            |domain_index, kind, distance_squared, height_m| {
                Self::retain_best_target_height_sample(
                    &mut best,
                    NodeTargetHeightSample {
                        height_delta_m: (height_m - target_height_m).abs(),
                        priority: Self::walkable_boundary_height_priority(kind),
                        distance_squared,
                        domain_index,
                        height_m,
                    },
                );
            },
        );
        best.map(|sample| sample.height_m)
    }

    pub(super) fn visual_non_road_band_polygons_from_height_domains(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        target_non_road_shapes: &NodeOverlayShapes,
        road_shapes: &NodeOverlayShapes,
        height_domains: &[NodeBandHeightDomain],
    ) -> Option<Vec<RoadSurfaceVisualPolygon>> {
        let mut polygons = Vec::new();
        let mut claimed_shapes = Vec::new();
        for kind in Self::non_road_visual_band_order() {
            let kind_domains = height_domains
                .iter()
                .filter(|domain| domain.kind == kind)
                .cloned()
                .collect::<Vec<_>>();
            if kind_domains.is_empty() {
                continue;
            }

            let contours = kind_domains
                .iter()
                .map(|domain| Self::overlay_contour_from_world_points(&domain.polygon.points_world))
                .filter(|contour| {
                    Self::overlay_contour_area(contour).abs() > NODE_OVERLAY_MIN_AREA_M2
                })
                .collect::<Vec<_>>();
            let mut band_shapes = Self::overlay_union_contours(&contours)?;
            band_shapes = Self::overlay_binary_shapes(
                &band_shapes,
                target_non_road_shapes,
                OverlayRule::Intersect,
            )?;
            if !road_shapes.is_empty() {
                band_shapes = Self::overlay_binary_shapes(
                    &band_shapes,
                    road_shapes,
                    OverlayRule::Difference,
                )?;
            }
            if !claimed_shapes.is_empty() {
                band_shapes = Self::overlay_binary_shapes(
                    &band_shapes,
                    &claimed_shapes,
                    OverlayRule::Difference,
                )?;
            }
            Self::sort_overlay_shapes(&mut band_shapes);

            if kind == RoadSurfaceBandKind::CurbOrShoulder {
                let mut band_polygons = Self::visual_polygons_from_overlay_shapes_with_band_heights(
                    node_id,
                    piece_kind,
                    "non_road_band",
                    &band_shapes,
                    &kind_domains,
                );
                polygons.append(&mut band_polygons);
                claimed_shapes = Self::overlay_union_shape_sets(&claimed_shapes, &band_shapes)?;
                continue;
            }

            let mut kind_claimed_shapes = Vec::new();
            for domain in &kind_domains {
                // Same-material domains may carry different height planes on sloped junctions;
                // claiming them one at a time preserves that deterministic seam for CDT.
                let contour = Self::overlay_contour_from_world_points(&domain.polygon.points_world);
                if Self::overlay_contour_area(&contour).abs() <= NODE_OVERLAY_MIN_AREA_M2 {
                    continue;
                }

                let mut domain_shapes = Self::overlay_union_contours(&[contour])?;
                domain_shapes = Self::overlay_binary_shapes(
                    &domain_shapes,
                    &band_shapes,
                    OverlayRule::Intersect,
                )?;
                if !kind_claimed_shapes.is_empty() {
                    domain_shapes = Self::overlay_binary_shapes(
                        &domain_shapes,
                        &kind_claimed_shapes,
                        OverlayRule::Difference,
                    )?;
                }
                Self::sort_overlay_shapes(&mut domain_shapes);
                if domain_shapes.is_empty() {
                    continue;
                }

                let mut domain_polygons =
                    Self::visual_polygons_from_overlay_shapes_with_band_heights(
                        node_id,
                        piece_kind,
                        "non_road_band",
                        &domain_shapes,
                        std::slice::from_ref(domain),
                    );
                polygons.append(&mut domain_polygons);
                kind_claimed_shapes =
                    Self::overlay_union_shape_sets(&kind_claimed_shapes, &domain_shapes)?;
            }

            let kind_residual_shapes = if band_shapes.is_empty() {
                Vec::new()
            } else if kind_claimed_shapes.is_empty() {
                band_shapes.clone()
            } else {
                Self::overlay_binary_shapes(
                    &band_shapes,
                    &kind_claimed_shapes,
                    OverlayRule::Difference,
                )?
            };
            if !kind_residual_shapes.is_empty() {
                let mut residual_polygons =
                    Self::visual_polygons_from_overlay_shapes_with_band_heights(
                        node_id,
                        piece_kind,
                        "non_road_band_residual",
                        &kind_residual_shapes,
                        &kind_domains,
                    );
                polygons.append(&mut residual_polygons);
            }

            claimed_shapes = Self::overlay_union_shape_sets(&claimed_shapes, &band_shapes)?;
        }

        let residual_shapes = if target_non_road_shapes.is_empty() {
            Vec::new()
        } else if claimed_shapes.is_empty() {
            target_non_road_shapes.clone()
        } else {
            Self::overlay_binary_shapes(
                target_non_road_shapes,
                &claimed_shapes,
                OverlayRule::Difference,
            )?
        };
        if !residual_shapes.is_empty() {
            for shape in &residual_shapes {
                let residual_kind = Self::residual_non_road_height_kind_for_shape(
                    shape,
                    road_shapes,
                    height_domains,
                )?;
                let residual_domains = height_domains
                    .iter()
                    .filter(|domain| domain.kind == residual_kind)
                    .cloned()
                    .collect::<Vec<_>>();
                let Some(polygon) = Self::visual_polygon_from_overlay_shape_with_band_heights(
                    node_id,
                    piece_kind,
                    "non_road_residual",
                    shape,
                    &residual_domains,
                    true,
                ) else {
                    continue;
                };
                polygons.push(polygon);
            }
        }
        Self::sort_visual_polygons(&mut polygons);
        Some(polygons)
    }

    fn overlay_union_shape_sets(
        existing: &NodeOverlayShapes,
        added: &NodeOverlayShapes,
    ) -> Option<NodeOverlayShapes> {
        if existing.is_empty() {
            return Some(added.clone());
        }
        if added.is_empty() {
            return Some(existing.clone());
        }
        let contours = existing
            .iter()
            .chain(added.iter())
            .flat_map(|shape| shape.iter().cloned())
            .collect::<Vec<_>>();
        Self::overlay_union_contours(&contours)
    }

    fn residual_non_road_height_kind_for_shape(
        shape: &NodeOverlayShape,
        road_shapes: &NodeOverlayShapes,
        domains: &[NodeBandHeightDomain],
    ) -> Option<RoadSurfaceBandKind> {
        if domains
            .iter()
            .any(|domain| domain.kind == RoadSurfaceBandKind::CurbOrShoulder)
            && Self::residual_shape_is_narrow_road_edge_closure(shape, road_shapes)
        {
            return Some(RoadSurfaceBandKind::CurbOrShoulder);
        }

        if domains
            .iter()
            .any(|domain| domain.kind == RoadSurfaceBandKind::Sidewalk)
        {
            return Some(RoadSurfaceBandKind::Sidewalk);
        }

        let centroid = Self::overlay_shape_centroid_xz(shape)?;
        let mut best = None;
        for domain in domains {
            if domain.kind == RoadSurfaceBandKind::Carriageway {
                continue;
            }
            let distance_squared =
                Self::distance_squared_to_visual_polygon_xz(centroid, &domain.polygon);
            let priority = Self::non_road_residual_band_priority(domain.kind);
            let candidate = (distance_squared, priority, domain.kind);
            let replace = best.is_none_or(|current: (f32, u8, RoadSurfaceBandKind)| {
                candidate
                    .0
                    .total_cmp(&current.0)
                    .then(candidate.1.cmp(&current.1))
                    .then(
                        Self::band_kind_sort_key(candidate.2)
                            .cmp(&Self::band_kind_sort_key(current.2)),
                    )
                    .is_lt()
            });
            if replace {
                best = Some(candidate);
            }
        }
        best.map(|(_, _, kind)| kind)
    }

    fn residual_shape_is_narrow_road_edge_closure(
        shape: &NodeOverlayShape,
        road_shapes: &NodeOverlayShapes,
    ) -> bool {
        let road_edge_keys = Self::overlay_shape_set_edge_keys(road_shapes);
        if road_edge_keys.is_empty() {
            return false;
        }

        let shared_road_length_m = Self::overlay_shape_shared_edge_length_m(shape, &road_edge_keys);
        if shared_road_length_m <= SAMPLE_EPSILON_M {
            return false;
        }

        let area_m2 = Self::overlay_shape_area_m2(shape).abs();
        if area_m2 <= NODE_OVERLAY_MIN_AREA_M2 {
            return true;
        }

        let effective_width_m = area_m2 / shared_road_length_m.max(SAMPLE_EPSILON_M);
        effective_width_m <= CURB_BAND_WIDTH_M * 2.0
    }

    fn overlay_shape_area_m2(shape: &NodeOverlayShape) -> f32 {
        let Some(outer) = shape.first() else {
            return 0.0;
        };
        let holes = shape
            .iter()
            .skip(1)
            .map(|hole| Self::overlay_contour_area(hole).abs())
            .sum::<f32>();
        (Self::overlay_contour_area(outer).abs() - holes).max(0.0)
    }

    fn overlay_shapes_contain_point(shapes: &NodeOverlayShapes, point: Vector2) -> bool {
        shapes
            .iter()
            .any(|shape| Self::overlay_shape_contains_point(shape, point))
    }

    fn overlay_shape_contains_point(shape: &NodeOverlayShape, point: Vector2) -> bool {
        let Some(outer) = shape.first() else {
            return false;
        };
        if !Self::overlay_contour_contains_point(outer, point) {
            return false;
        }
        !shape
            .iter()
            .skip(1)
            .any(|hole| Self::overlay_contour_contains_point(hole, point))
    }

    fn overlay_contour_contains_point(contour: &NodeOverlayContour, point: Vector2) -> bool {
        if contour.len() < 3 {
            return false;
        }
        let mut inside = false;
        for index in 0..contour.len() {
            let start = contour[index];
            let end = contour[(index + 1) % contour.len()];
            let start_xz = Vector2::new(start[0], start[1]);
            let end_xz = Vector2::new(end[0], end[1]);
            if Self::distance_squared_to_segment_xz(point, start_xz, end_xz) <= 0.0001 {
                return true;
            }
            if (start[1] > point.y) != (end[1] > point.y) {
                let edge_x_at_point_z =
                    (end[0] - start[0]) * (point.y - start[1]) / (end[1] - start[1]) + start[0];
                if point.x < edge_x_at_point_z {
                    inside = !inside;
                }
            }
        }
        inside
    }

    fn overlay_shape_set_edge_keys(shapes: &NodeOverlayShapes) -> BTreeSet<NodeOverlayEdgeKey> {
        let mut keys = BTreeSet::new();
        for shape in shapes {
            for contour in shape {
                if contour.len() < 2 {
                    continue;
                }
                for index in 0..contour.len() {
                    keys.insert(Self::overlay_edge_key(
                        contour[index],
                        contour[(index + 1) % contour.len()],
                    ));
                }
            }
        }
        keys
    }

    fn overlay_shape_segments(
        shapes: &NodeOverlayShapes,
    ) -> Vec<(NodeOverlayPoint, NodeOverlayPoint)> {
        let mut segments = Vec::new();
        for shape in shapes {
            for contour in shape {
                if contour.len() < 2 {
                    continue;
                }
                for index in 0..contour.len() {
                    let start = contour[index];
                    let end = contour[(index + 1) % contour.len()];
                    if Vector2::new(end[0] - start[0], end[1] - start[1]).length_squared()
                        > SAMPLE_EPSILON_M * SAMPLE_EPSILON_M
                    {
                        segments.push((start, end));
                    }
                }
            }
        }
        segments
    }

    pub(super) fn overlay_shared_collinear_segments(
        road_shapes: &NodeOverlayShapes,
        non_road_shapes: &NodeOverlayShapes,
    ) -> Vec<(NodeOverlayPoint, NodeOverlayPoint)> {
        let road_segments = Self::overlay_shape_segments(road_shapes);
        let non_road_segments = Self::overlay_shape_segments(non_road_shapes);
        if road_segments.is_empty() || non_road_segments.is_empty() {
            return Vec::new();
        }

        let mut segments = Vec::new();
        let mut emitted = BTreeSet::new();
        for (non_road_start, non_road_end) in non_road_segments {
            for (road_start, road_end) in &road_segments {
                let Some((start, end)) = Self::overlay_collinear_overlap_segment(
                    non_road_start,
                    non_road_end,
                    *road_start,
                    *road_end,
                ) else {
                    continue;
                };
                let key = Self::overlay_edge_key(start, end);
                if emitted.insert(key) {
                    segments.push((start, end));
                }
            }
        }

        segments.sort_by(|a, b| {
            a.0[0]
                .total_cmp(&b.0[0])
                .then(a.0[1].total_cmp(&b.0[1]))
                .then(a.1[0].total_cmp(&b.1[0]))
                .then(a.1[1].total_cmp(&b.1[1]))
        });
        segments
    }

    fn overlay_collinear_overlap_segment(
        segment_start: NodeOverlayPoint,
        segment_end: NodeOverlayPoint,
        other_start: NodeOverlayPoint,
        other_end: NodeOverlayPoint,
    ) -> Option<(NodeOverlayPoint, NodeOverlayPoint)> {
        let start = Vector2::new(segment_start[0], segment_start[1]);
        let end = Vector2::new(segment_end[0], segment_end[1]);
        let other_start = Vector2::new(other_start[0], other_start[1]);
        let other_end = Vector2::new(other_end[0], other_end[1]);
        let segment = end - start;
        let other = other_end - other_start;
        let segment_length_squared = segment.length_squared();
        let other_length_squared = other.length_squared();
        if segment_length_squared <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M
            || other_length_squared <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M
        {
            return None;
        }

        let segment_length = segment_length_squared.sqrt();
        let other_length = other_length_squared.sqrt();
        let tolerance_m = 2.0 / NODE_OVERLAY_SCALE;
        let direction_cross =
            Self::cross_xz(segment, other).abs() / (segment_length * other_length);
        if direction_cross > tolerance_m {
            return None;
        }

        let other_start_distance =
            Self::cross_xz(segment, other_start - start).abs() / segment_length;
        let other_end_distance = Self::cross_xz(segment, other_end - start).abs() / segment_length;
        if other_start_distance > tolerance_m || other_end_distance > tolerance_m {
            return None;
        }

        let other_start_t = (other_start - start).dot(segment) / segment_length_squared;
        let other_end_t = (other_end - start).dot(segment) / segment_length_squared;
        let overlap_start_t = other_start_t.min(other_end_t).max(0.0);
        let overlap_end_t = other_start_t.max(other_end_t).min(1.0);
        if (overlap_end_t - overlap_start_t) * segment_length <= tolerance_m {
            return None;
        }

        let overlap_start = start + segment * overlap_start_t;
        let overlap_end = start + segment * overlap_end_t;
        let start_point = Self::quantize_overlay_point([overlap_start.x, overlap_start.y]);
        let end_point = Self::quantize_overlay_point([overlap_end.x, overlap_end.y]);
        let edge_key = Self::overlay_edge_key(start_point, end_point);
        if edge_key.0 == edge_key.1 {
            return None;
        }
        Some((start_point, end_point))
    }

    fn overlay_shape_shared_edge_length_m(
        shape: &NodeOverlayShape,
        edge_keys: &BTreeSet<NodeOverlayEdgeKey>,
    ) -> f32 {
        let mut length_m = 0.0;
        for contour in shape {
            if contour.len() < 2 {
                continue;
            }
            for index in 0..contour.len() {
                let start = contour[index];
                let end = contour[(index + 1) % contour.len()];
                if edge_keys.contains(&Self::overlay_edge_key(start, end)) {
                    length_m += Vector2::new(end[0] - start[0], end[1] - start[1]).length();
                }
            }
        }
        length_m
    }

    fn overlay_edge_key(a: NodeOverlayPoint, b: NodeOverlayPoint) -> NodeOverlayEdgeKey {
        let a_key = Self::overlay_point_key(a);
        let b_key = Self::overlay_point_key(b);
        if a_key <= b_key {
            (a_key, b_key)
        } else {
            (b_key, a_key)
        }
    }

    fn overlay_point_key(point: NodeOverlayPoint) -> NodeOverlayPointKey {
        (
            (point[0] * NODE_OVERLAY_SCALE).round() as i64,
            (point[1] * NODE_OVERLAY_SCALE).round() as i64,
        )
    }

    fn quantize_overlay_point(point: NodeOverlayPoint) -> NodeOverlayPoint {
        let key = Self::overlay_point_key(point);
        [
            key.0 as f32 / NODE_OVERLAY_SCALE,
            key.1 as f32 / NODE_OVERLAY_SCALE,
        ]
    }

    fn overlay_shape_centroid_xz(shape: &NodeOverlayShape) -> Option<Vector2> {
        let contour = shape.first()?;
        if contour.is_empty() {
            return None;
        }
        let mut signed_cross_sum = 0.0;
        let mut centroid_x = 0.0;
        let mut centroid_z = 0.0;
        for index in 0..contour.len() {
            let current = contour[index];
            let next = contour[(index + 1) % contour.len()];
            let cross = current[0] * next[1] - next[0] * current[1];
            signed_cross_sum += cross;
            centroid_x += (current[0] + next[0]) * cross;
            centroid_z += (current[1] + next[1]) * cross;
        }
        if signed_cross_sum.abs() > SAMPLE_EPSILON_M {
            return Some(Vector2::new(
                centroid_x / (3.0 * signed_cross_sum),
                centroid_z / (3.0 * signed_cross_sum),
            ));
        }

        let (sum_x, sum_z) = contour.iter().fold((0.0, 0.0), |acc, point| {
            (acc.0 + point[0], acc.1 + point[1])
        });
        Some(Vector2::new(
            sum_x / contour.len() as f32,
            sum_z / contour.len() as f32,
        ))
    }

    fn distance_squared_to_visual_polygon_xz(
        point_xz: Vector2,
        polygon: &RoadSurfaceVisualPolygon,
    ) -> f32 {
        if Self::polygon_contains_point_xz(&polygon.points_world, point_xz) {
            return 0.0;
        }
        let mut best = f32::INFINITY;
        for index in 0..polygon.points_world.len() {
            let start = polygon.points_world[index];
            let end = polygon.points_world[(index + 1) % polygon.points_world.len()];
            let start_xz = Vector2::new(start.x, start.z);
            let end_xz = Vector2::new(end.x, end.z);
            best = best.min(Self::distance_squared_to_segment_xz(
                point_xz, start_xz, end_xz,
            ));
        }
        best
    }

    fn distance_squared_to_segment_xz(point: Vector2, start: Vector2, end: Vector2) -> f32 {
        let segment = end - start;
        let length_squared = segment.length_squared();
        if length_squared <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
            return point.distance_squared_to(start);
        }
        let t = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
        point.distance_squared_to(start + segment * t)
    }

    fn non_road_residual_band_priority(kind: RoadSurfaceBandKind) -> u8 {
        match kind {
            RoadSurfaceBandKind::CurbOrShoulder => 0,
            RoadSurfaceBandKind::Sidewalk => 1,
            RoadSurfaceBandKind::Footpath => 2,
            RoadSurfaceBandKind::CycleTrack => 3,
            RoadSurfaceBandKind::Median => 4,
            RoadSurfaceBandKind::Parking => 5,
            RoadSurfaceBandKind::TramReservation => 6,
            RoadSurfaceBandKind::Carriageway => 7,
        }
    }

    fn band_kind_sort_key(kind: RoadSurfaceBandKind) -> u8 {
        match kind {
            RoadSurfaceBandKind::Carriageway => 0,
            RoadSurfaceBandKind::CurbOrShoulder => 1,
            RoadSurfaceBandKind::Sidewalk => 2,
            RoadSurfaceBandKind::Footpath => 3,
            RoadSurfaceBandKind::Median => 4,
            RoadSurfaceBandKind::Parking => 5,
            RoadSurfaceBandKind::CycleTrack => 6,
            RoadSurfaceBandKind::TramReservation => 7,
        }
    }

    fn non_road_visual_band_order() -> [RoadSurfaceBandKind; 7] {
        [
            RoadSurfaceBandKind::CurbOrShoulder,
            RoadSurfaceBandKind::Sidewalk,
            RoadSurfaceBandKind::Footpath,
            RoadSurfaceBandKind::CycleTrack,
            RoadSurfaceBandKind::Median,
            RoadSurfaceBandKind::Parking,
            RoadSurfaceBandKind::TramReservation,
        ]
    }

    fn visual_polygon_from_overlay_shape_with_footprint_heights(
        shape: &NodeOverlayShape,
        height_candidates: &[RoadSurfaceVisualPolygon],
        preserve_holes: bool,
    ) -> Option<RoadSurfaceVisualPolygon> {
        let outer_contour = shape.first()?;
        let mut outer_points = Self::world_points_from_overlay_contour_with_footprint_heights(
            outer_contour,
            height_candidates,
        )?;
        if Self::signed_polygon_area_xz(&outer_points).abs() <= NODE_OVERLAY_MIN_AREA_M2 {
            return None;
        }
        if Self::signed_polygon_area_xz(&outer_points) < 0.0 {
            outer_points.reverse();
        }

        let mut hole_points = Vec::new();
        if preserve_holes {
            for contour in shape.iter().skip(1) {
                let mut points = Self::world_points_from_overlay_contour_with_footprint_heights(
                    contour,
                    height_candidates,
                )?;
                if Self::signed_polygon_area_xz(&points).abs() <= NODE_OVERLAY_MIN_AREA_M2 {
                    continue;
                }
                if Self::signed_polygon_area_xz(&points) > 0.0 {
                    points.reverse();
                }
                hole_points.push(points);
            }
        }

        Self::canonicalize_world_loop(&mut outer_points)?;
        for hole in &mut hole_points {
            Self::canonicalize_world_loop(hole)?;
        }
        let triangles_world = Self::triangulate_constrained_shape_xz(&outer_points, &hole_points)?;
        Some(RoadSurfaceVisualPolygon {
            points_world: outer_points,
            triangles_world,
        })
    }

    fn world_points_from_overlay_contour_with_band_heights(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        material_name: &'static str,
        contour: &NodeOverlayContour,
        height_domains: &[NodeBandHeightDomain],
    ) -> Option<Vec<Vector3>> {
        let mut points_world = Vec::with_capacity(contour.len());
        for point in contour {
            let xz = Vector2::new(point[0], point[1]);
            let Some(y) = Self::sample_node_band_height_from_domains(xz, height_domains) else {
                crate::debug_log!(
                    "road",
                    "node_band_height_missing node={} piece={:?} material={} x={:.3} z={:.3} domain_count={}",
                    node_id,
                    piece_kind,
                    material_name,
                    xz.x,
                    xz.y,
                    height_domains.len()
                );
                return None;
            };
            points_world.push(Vector3::new(point[0], y, point[1]));
        }
        Some(points_world)
    }

    fn world_points_from_overlay_contour_with_footprint_heights(
        contour: &NodeOverlayContour,
        height_candidates: &[RoadSurfaceVisualPolygon],
    ) -> Option<Vec<Vector3>> {
        contour
            .iter()
            .map(|point| {
                let xz = Vector2::new(point[0], point[1]);
                let y =
                    Self::sample_overlay_footprint_height_from_candidates(xz, height_candidates)?;
                Some(Vector3::new(point[0], y, point[1]))
            })
            .collect()
    }

    fn canonicalize_world_loop(points_world: &mut Vec<Vector3>) -> Option<()> {
        points_world
            .dedup_by(|a, b| (*a - *b).length_squared() <= WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2);
        if points_world.len() >= 2
            && (points_world.first().copied()? - points_world.last().copied()?).length_squared()
                <= WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2
        {
            points_world.pop();
        }
        if points_world.len() < 3 {
            return None;
        }
        let (start_index, _) = points_world.iter().enumerate().min_by(|(_, a), (_, b)| {
            a.x.total_cmp(&b.x)
                .then(a.z.total_cmp(&b.z))
                .then(a.y.total_cmp(&b.y))
        })?;
        points_world.rotate_left(start_index);
        Some(())
    }

    pub(super) fn sample_node_band_height_from_domains(
        point_xz: Vector2,
        domains: &[NodeBandHeightDomain],
    ) -> Option<f32> {
        Self::sample_node_band_height_from_filtered_domains(point_xz, domains, |_| true)
    }

    fn sample_node_band_height_from_filtered_domains(
        point_xz: Vector2,
        domains: &[NodeBandHeightDomain],
        accepts_kind: impl Fn(RoadSurfaceBandKind) -> bool,
    ) -> Option<f32> {
        let mut best = None;
        Self::visit_node_band_height_samples(
            point_xz,
            domains,
            accepts_kind,
            |domain_index, kind, distance_squared, height_m| {
                Self::retain_best_node_band_height_sample(
                    &mut best,
                    NodeBandHeightSample {
                        priority: Self::node_band_height_priority(kind),
                        domain_index,
                        distance_squared,
                        height_m,
                    },
                );
            },
        );

        best.map(|sample| sample.height_m)
    }

    fn visit_node_band_height_samples(
        point_xz: Vector2,
        domains: &[NodeBandHeightDomain],
        accepts_kind: impl Fn(RoadSurfaceBandKind) -> bool,
        mut visit: impl FnMut(usize, RoadSurfaceBandKind, f32, f32),
    ) {
        let mut found_containing_triangle = false;
        for (domain_index, domain) in domains.iter().enumerate() {
            if !accepts_kind(domain.kind) {
                continue;
            }
            for triangle in &domain.polygon.triangles_world {
                let Some((wa, wb, wc)) = Self::triangle_barycentric_weights_xz(*triangle, point_xz)
                else {
                    continue;
                };
                found_containing_triangle = true;
                visit(
                    domain_index,
                    domain.kind,
                    0.0,
                    triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc,
                );
            }
        }
        if found_containing_triangle {
            return;
        }

        for (domain_index, domain) in domains.iter().enumerate() {
            if !accepts_kind(domain.kind) || domain.polygon.points_world.len() < 2 {
                continue;
            }
            for index in 0..domain.polygon.points_world.len() {
                let start = domain.polygon.points_world[index];
                let end =
                    domain.polygon.points_world[(index + 1) % domain.polygon.points_world.len()];
                let start_xz = Vector2::new(start.x, start.z);
                let end_xz = Vector2::new(end.x, end.z);
                let segment = end_xz - start_xz;
                let length_squared = segment.length_squared();
                let t = if length_squared <= SAMPLE_EPSILON_M {
                    0.0
                } else {
                    ((point_xz - start_xz).dot(segment) / length_squared).clamp(0.0, 1.0)
                };
                let closest = start_xz + segment * t;
                visit(
                    domain_index,
                    domain.kind,
                    point_xz.distance_squared_to(closest),
                    start.y + (end.y - start.y) * t,
                );
            }
        }
    }

    fn retain_best_node_band_height_sample(
        best: &mut Option<NodeBandHeightSample>,
        candidate: NodeBandHeightSample,
    ) {
        let replace = best.is_none_or(|current| {
            candidate
                .priority
                .cmp(&current.priority)
                .then_with(|| {
                    candidate
                        .distance_squared
                        .total_cmp(&current.distance_squared)
                })
                .then(candidate.domain_index.cmp(&current.domain_index))
                .is_lt()
        });
        if replace {
            *best = Some(candidate);
        }
    }

    fn retain_best_target_height_sample(
        best: &mut Option<NodeTargetHeightSample>,
        candidate: NodeTargetHeightSample,
    ) {
        let replace = best.is_none_or(|current| {
            candidate
                .height_delta_m
                .total_cmp(&current.height_delta_m)
                .then(candidate.priority.cmp(&current.priority))
                .then_with(|| {
                    candidate
                        .distance_squared
                        .total_cmp(&current.distance_squared)
                })
                .then(candidate.domain_index.cmp(&current.domain_index))
                .is_lt()
        });
        if replace {
            *best = Some(candidate);
        }
    }

    fn is_walkable_boundary_height_kind(kind: RoadSurfaceBandKind) -> bool {
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

    fn walkable_boundary_height_priority(kind: RoadSurfaceBandKind) -> u8 {
        match kind {
            RoadSurfaceBandKind::Sidewalk => 0,
            RoadSurfaceBandKind::Footpath => 1,
            RoadSurfaceBandKind::CycleTrack => 2,
            RoadSurfaceBandKind::Median => 3,
            RoadSurfaceBandKind::Parking => 4,
            RoadSurfaceBandKind::TramReservation => 5,
            RoadSurfaceBandKind::CurbOrShoulder => 6,
            RoadSurfaceBandKind::Carriageway => 7,
        }
    }

    fn node_band_height_priority(kind: RoadSurfaceBandKind) -> u8 {
        match kind {
            RoadSurfaceBandKind::Carriageway | RoadSurfaceBandKind::CurbOrShoulder => 0,
            RoadSurfaceBandKind::Sidewalk | RoadSurfaceBandKind::Footpath => 1,
            RoadSurfaceBandKind::Median => 3,
            RoadSurfaceBandKind::Parking => 4,
            RoadSurfaceBandKind::CycleTrack => 5,
            RoadSurfaceBandKind::TramReservation => 6,
        }
    }

    fn sample_overlay_footprint_height_from_candidates(
        point_xz: Vector2,
        candidates: &[RoadSurfaceVisualPolygon],
    ) -> Option<f32> {
        for polygon in candidates {
            for triangle in &polygon.triangles_world {
                if let Some((wa, wb, wc)) =
                    Self::triangle_barycentric_weights_xz(*triangle, point_xz)
                {
                    return Some(triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc);
                }
            }
        }

        let mut best_distance_squared = f32::INFINITY;
        let mut best_height = None;
        for polygon in candidates {
            if polygon.points_world.len() < 2 {
                continue;
            }
            for index in 0..polygon.points_world.len() {
                let start = polygon.points_world[index];
                let end = polygon.points_world[(index + 1) % polygon.points_world.len()];
                let start_xz = Vector2::new(start.x, start.z);
                let end_xz = Vector2::new(end.x, end.z);
                let segment = end_xz - start_xz;
                let length_squared = segment.length_squared();
                let t = if length_squared <= SAMPLE_EPSILON_M {
                    0.0
                } else {
                    ((point_xz - start_xz).dot(segment) / length_squared).clamp(0.0, 1.0)
                };
                let closest = start_xz + segment * t;
                let distance_squared = point_xz.distance_squared_to(closest);
                if distance_squared < best_distance_squared {
                    best_distance_squared = distance_squared;
                    best_height = Some(start.y + (end.y - start.y) * t);
                }
            }
        }
        best_height
    }

    fn triangulate_constrained_shape_xz(
        outer_points: &[Vector3],
        holes: &[Vec<Vector3>],
    ) -> Option<Vec<[Vector3; 3]>> {
        if outer_points.len() < 3 {
            return None;
        }

        let mut vertices = Vec::new();
        let mut vertex_lookup = BTreeMap::new();
        let mut constraints = BTreeSet::new();
        Self::push_surface_cdt_loop(
            outer_points,
            &mut vertices,
            &mut vertex_lookup,
            &mut constraints,
        );
        for hole in holes {
            Self::push_surface_cdt_loop(hole, &mut vertices, &mut vertex_lookup, &mut constraints);
        }

        let spade_vertices = vertices
            .iter()
            .map(|point| Point2::new(f64::from(point.x), f64::from(point.z)))
            .collect::<Vec<_>>();
        let mut invalid_constraints = 0usize;
        let cdt = SurfaceCdt::try_bulk_load_cdt(
            spade_vertices,
            constraints.into_iter().collect(),
            |_| invalid_constraints += 1,
        )
        .ok()?;
        if invalid_constraints > 0 {
            return None;
        }

        let mut triangles = Vec::new();
        for face in cdt.inner_faces() {
            let [a, b, c] = face.vertices();
            let triangle = [
                vertices[a.fix().index()],
                vertices[b.fix().index()],
                vertices[c.fix().index()],
            ];
            let centroid = Vector2::new(
                (triangle[0].x + triangle[1].x + triangle[2].x) / 3.0,
                (triangle[0].z + triangle[1].z + triangle[2].z) / 3.0,
            );
            if !Self::triangle_has_area_xz(triangle) {
                continue;
            }
            if !Self::polygon_contains_point_xz(outer_points, centroid) {
                continue;
            }
            if holes
                .iter()
                .any(|hole| Self::polygon_contains_point_xz(hole, centroid))
            {
                continue;
            }
            triangles.push(triangle);
        }

        (!triangles.is_empty()).then_some(triangles)
    }

    fn push_surface_cdt_loop(
        points_world: &[Vector3],
        vertices: &mut Vec<Vector3>,
        vertex_lookup: &mut BTreeMap<(i64, i64), usize>,
        constraints: &mut BTreeSet<[usize; 2]>,
    ) {
        if points_world.len() < 2 {
            return;
        }
        let indices = points_world
            .iter()
            .map(|point| Self::insert_surface_cdt_vertex(*point, vertices, vertex_lookup))
            .collect::<Vec<_>>();
        for index in 0..indices.len() {
            let edge = Self::normalize_surface_edge_array(
                indices[index],
                indices[(index + 1) % indices.len()],
            );
            if edge[0] != edge[1] {
                constraints.insert(edge);
            }
        }
    }

    fn insert_surface_cdt_vertex(
        point: Vector3,
        vertices: &mut Vec<Vector3>,
        vertex_lookup: &mut BTreeMap<(i64, i64), usize>,
    ) -> usize {
        let key = Self::surface_cdt_vertex_key(point);
        if let Some(index) = vertex_lookup.get(&key) {
            return *index;
        }
        let index = vertices.len();
        vertices.push(point);
        vertex_lookup.insert(key, index);
        index
    }

    fn surface_cdt_vertex_key(point: Vector3) -> (i64, i64) {
        (
            (point.x / SAMPLE_EPSILON_M).round() as i64,
            (point.z / SAMPLE_EPSILON_M).round() as i64,
        )
    }

    fn normalize_surface_edge_array(a: usize, b: usize) -> [usize; 2] {
        if a < b { [a, b] } else { [b, a] }
    }
}
