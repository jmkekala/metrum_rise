//! Earthwork skirt geometry, transition vectors, and top-surface intrusion checks.

use super::super::{
    NodeOverlayContour, NodeOverlayShapes, RoadSurfaceSystem, RoadSurfaceVisualPolygon,
    SAMPLE_EPSILON_M, backend,
};
use super::{
    EARTHWORK_CUT_SLOPE_RATE, EARTHWORK_FILL_SLOPE_RATE, EARTHWORK_MARGIN_SAMPLE_STEP_M,
    EARTHWORK_MAX_MARGIN_M, EARTHWORK_MIN_MARGIN_M, EARTHWORK_RETAINING_WALL_SLOPE_THRESHOLD,
    RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkFaceKind,
    RoadSurfaceEarthworkGeometryError, RoadSurfaceEarthworkRenderFace,
};
use crate::config;
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};
use i_overlay::core::overlay_rule::OverlayRule;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn build_closed_earthwork_geometry_from_boundary_segments(
        &self,
        boundary_segment_loops: &[Vec<RoadSurfaceEarthworkBoundarySegment>],
        terrain: &TerrainSystem,
        top_surface_shapes: Option<&NodeOverlayShapes>,
    ) -> Result<
        (
            Vec<RoadSurfaceVisualPolygon>,
            Vec<RoadSurfaceVisualPolygon>,
            Vec<RoadSurfaceEarthworkRenderFace>,
        ),
        RoadSurfaceEarthworkGeometryError,
    > {
        let mut earthwork_surface_polygons = Vec::new();
        let mut earthwork_outer_boundary_loops = Vec::new();
        let mut render_earthwork_faces = Vec::new();

        for boundary_segments in boundary_segment_loops {
            let Some((outer_loop, side_polygons, render_faces)) = self
                .build_closed_earthwork_loop_geometry(
                    boundary_segments,
                    terrain,
                    top_surface_shapes,
                )?
            else {
                continue;
            };
            if let Some(outer_loop) = outer_loop {
                earthwork_outer_boundary_loops.push(outer_loop);
            }
            earthwork_surface_polygons.extend(side_polygons);
            render_earthwork_faces.extend(render_faces);
        }

        Self::sort_visual_polygons(&mut earthwork_surface_polygons);
        Self::sort_visual_polygons(&mut earthwork_outer_boundary_loops);
        Self::sort_earthwork_render_faces(&mut render_earthwork_faces);
        Ok((
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        ))
    }

    fn build_closed_earthwork_loop_geometry(
        &self,
        boundary_segments: &[RoadSurfaceEarthworkBoundarySegment],
        terrain: &TerrainSystem,
        top_surface_shapes: Option<&NodeOverlayShapes>,
    ) -> Result<
        Option<(
            Option<RoadSurfaceVisualPolygon>,
            Vec<RoadSurfaceVisualPolygon>,
            Vec<RoadSurfaceEarthworkRenderFace>,
        )>,
        RoadSurfaceEarthworkGeometryError,
    > {
        if boundary_segments.len() < 3 {
            return Err(RoadSurfaceEarthworkGeometryError::DegenerateBoundaryLoop {
                point_count: boundary_segments.len(),
            });
        }
        let boundary_points = boundary_segments
            .iter()
            .map(|segment| segment.inner_start)
            .collect::<Vec<_>>();

        let mut vertex_outer_points = Vec::with_capacity(boundary_points.len());
        for (index, point) in boundary_points.iter().enumerate() {
            let Some(outward) = Self::closed_loop_vertex_outward_xz(&boundary_points, index) else {
                vertex_outer_points.clear();
                break;
            };
            let Some(outer_point) = self.earthwork_transition_point(*point, outward, terrain)
            else {
                vertex_outer_points.clear();
                break;
            };
            vertex_outer_points.push(outer_point);
        }
        // A cusp has no unique vertex miter. Keep edge-based faces, but do not invent a loop point.
        let outer_loop = if vertex_outer_points.len() == boundary_points.len() {
            Self::make_visual_polygon(vertex_outer_points)
        } else {
            None
        };
        let mut side_polygons = Vec::new();
        let mut render_faces = Vec::new();
        let winding_ccw = Self::signed_polygon_area_xz(&boundary_points) > 0.0;
        for segment in boundary_segments {
            let current = segment.inner_start;
            let next = segment.inner_end;
            let (outer_current, outer_next) = self.earthwork_edge_transition_points(
                current,
                next,
                winding_ccw,
                terrain,
                top_surface_shapes,
            )?;
            // Handoff and internal seam edges can be part of a closed footprint loop. They are not
            // terrain tie-ins, so a skirt whose plan area enters solved top ownership is rejected.
            if let Some(top_surface_shapes) = top_surface_shapes
                && Self::earthwork_candidate_intrudes_top(
                    [current, next, outer_next, outer_current],
                    top_surface_shapes,
                )
            {
                continue;
            }
            let Some(polygon) =
                Self::make_visual_polygon(vec![current, next, outer_next, outer_current])
            else {
                continue;
            };
            let face_kind =
                Self::classify_earthwork_face_kind(current, next, outer_next, outer_current);
            render_faces.push(RoadSurfaceEarthworkRenderFace {
                kind: face_kind,
                source: segment.source,
                inner_start: current,
                inner_end: next,
                polygon: polygon.clone(),
            });
            side_polygons.push(polygon);
        }

        if outer_loop.is_none() && side_polygons.is_empty() {
            return Ok(None);
        }
        Ok(Some((outer_loop, side_polygons, render_faces)))
    }

    fn earthwork_edge_transition_points(
        &self,
        current: Vector3,
        next: Vector3,
        winding_ccw: bool,
        terrain: &TerrainSystem,
        top_surface_shapes: Option<&NodeOverlayShapes>,
    ) -> Result<(Vector3, Vector3), RoadSurfaceEarthworkGeometryError> {
        let edge = Vector2::new(next.x - current.x, next.z - current.z);
        let Some(outward) = Self::edge_outward_normal_xz(edge, winding_ccw) else {
            return Err(
                RoadSurfaceEarthworkGeometryError::DegenerateOutwardDirection {
                    point_count: 2,
                    point_index: 0,
                },
            );
        };
        let Some(outer_current) = self.earthwork_transition_point(current, outward, terrain) else {
            return Err(
                RoadSurfaceEarthworkGeometryError::DegenerateOutwardDirection {
                    point_count: 2,
                    point_index: 0,
                },
            );
        };
        let Some(outer_next) = self.earthwork_transition_point(next, outward, terrain) else {
            return Err(
                RoadSurfaceEarthworkGeometryError::DegenerateOutwardDirection {
                    point_count: 2,
                    point_index: 1,
                },
            );
        };
        let Some(top_surface_shapes) = top_surface_shapes else {
            return Ok((outer_current, outer_next));
        };

        let Some(opposite_outer_current) =
            self.earthwork_transition_point(current, -outward, terrain)
        else {
            return Err(
                RoadSurfaceEarthworkGeometryError::DegenerateOutwardDirection {
                    point_count: 2,
                    point_index: 0,
                },
            );
        };
        let Some(opposite_outer_next) = self.earthwork_transition_point(next, -outward, terrain)
        else {
            return Err(
                RoadSurfaceEarthworkGeometryError::DegenerateOutwardDirection {
                    point_count: 2,
                    point_index: 1,
                },
            );
        };
        let Some(nominal_overlap) = Self::earthwork_candidate_top_overlap_area_m2(
            [current, next, outer_next, outer_current],
            top_surface_shapes,
        ) else {
            return Ok((outer_current, outer_next));
        };
        let Some(opposite_overlap) = Self::earthwork_candidate_top_overlap_area_m2(
            [current, next, opposite_outer_next, opposite_outer_current],
            top_surface_shapes,
        ) else {
            return Ok((outer_current, outer_next));
        };
        if opposite_overlap < nominal_overlap {
            Ok((opposite_outer_current, opposite_outer_next))
        } else {
            Ok((outer_current, outer_next))
        }
    }

    fn earthwork_candidate_intrudes_top(
        points: [Vector3; 4],
        top_surface_shapes: &NodeOverlayShapes,
    ) -> bool {
        let Some((overlap_area_m2, budget_m2)) =
            Self::earthwork_candidate_top_overlap_metrics_m2(points, top_surface_shapes)
        else {
            return true;
        };
        overlap_area_m2 > budget_m2
    }

    fn earthwork_candidate_top_overlap_area_m2(
        points: [Vector3; 4],
        top_surface_shapes: &NodeOverlayShapes,
    ) -> Option<f32> {
        Self::earthwork_candidate_top_overlap_metrics_m2(points, top_surface_shapes)
            .map(|(overlap_area_m2, _)| overlap_area_m2)
    }

    fn earthwork_candidate_top_overlap_metrics_m2(
        mut points: [Vector3; 4],
        top_surface_shapes: &NodeOverlayShapes,
    ) -> Option<(f32, f32)> {
        if Self::signed_polygon_area_xz(&points) < 0.0 {
            points.reverse();
        }
        let candidate_shapes =
            Self::overlay_union_contours(&[Self::earthwork_overlay_contour_from_points(&points)])?;
        let overlap = Self::overlay_binary_shapes(
            &candidate_shapes,
            top_surface_shapes,
            OverlayRule::Intersect,
        )?;
        let overlap_area_m2 = overlap.iter().map(Self::overlay_shape_area_m2).sum();
        let budget_m2 = Self::overlay_numeric_area_budget_for_shapes(&candidate_shapes).max(
            Self::overlay_numeric_area_budget_for_shapes(top_surface_shapes),
        );
        Some((overlap_area_m2, budget_m2))
    }

    fn earthwork_overlay_contour_from_points(points: &[Vector3]) -> NodeOverlayContour {
        let mut contour = Vec::with_capacity(points.len());
        for point in points {
            let point = backend::road_vec2_to_overlay_point(backend::godot_vec3_xz_to_road(*point));
            if contour.last().is_none_or(|last| *last != point) {
                contour.push(point);
            }
        }
        if contour.len() >= 2 && contour.first() == contour.last() {
            contour.pop();
        }
        contour
    }

    pub(in crate::simulation::network::surface) fn top_surface_overlay_shapes<'a>(
        polygons: impl IntoIterator<Item = &'a RoadSurfaceVisualPolygon>,
    ) -> Option<NodeOverlayShapes> {
        let mut contours = Vec::new();
        for polygon in polygons {
            if polygon.points_world.len() >= 3 {
                contours.push(Self::earthwork_overlay_contour_from_points(
                    &polygon.points_world,
                ));
            }
        }
        Self::overlay_union_contours(&contours)
    }

    fn closed_loop_vertex_outward_xz(boundary_points: &[Vector3], index: usize) -> Option<Vector2> {
        if boundary_points.len() < 3 {
            return None;
        }

        let len = boundary_points.len();
        let prev = boundary_points[(index + len - 1) % len];
        let current = boundary_points[index];
        let next = boundary_points[(index + 1) % len];
        let incoming = Vector2::new(current.x - prev.x, current.z - prev.z);
        let outgoing = Vector2::new(next.x - current.x, next.z - current.z);
        let winding_ccw = Self::signed_polygon_area_xz(boundary_points) > 0.0;
        let outward_incoming = Self::edge_outward_normal_xz(incoming, winding_ccw)?;
        let outward_outgoing = Self::edge_outward_normal_xz(outgoing, winding_ccw)?;
        let outward = outward_incoming + outward_outgoing;
        if outward.length_squared() <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
            None
        } else {
            Some(outward.normalized())
        }
    }

    fn edge_outward_normal_xz(edge_xz: Vector2, winding_ccw: bool) -> Option<Vector2> {
        if edge_xz.length_squared() <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
            return None;
        }
        let tangent = edge_xz.normalized();
        if winding_ccw {
            Some(Vector2::new(tangent.y, -tangent.x))
        } else {
            Some(Vector2::new(-tangent.y, tangent.x))
        }
    }

    pub(in crate::simulation::network::surface) fn classify_earthwork_face_kind(
        inner_start: Vector3,
        inner_end: Vector3,
        outer_end: Vector3,
        outer_start: Vector3,
    ) -> RoadSurfaceEarthworkFaceKind {
        let setback_a =
            Vector2::new(outer_start.x - inner_start.x, outer_start.z - inner_start.z).length();
        let setback_b = Vector2::new(outer_end.x - inner_end.x, outer_end.z - inner_end.z).length();
        let avg_setback = (setback_a + setback_b) * 0.5;
        if avg_setback <= SAMPLE_EPSILON_M {
            return RoadSurfaceEarthworkFaceKind::RetainingWall;
        }

        let max_height_delta = (outer_start.y - inner_start.y)
            .abs()
            .max((outer_end.y - inner_end.y).abs());
        let slope_ratio = max_height_delta / avg_setback.max(SAMPLE_EPSILON_M);
        if slope_ratio >= EARTHWORK_RETAINING_WALL_SLOPE_THRESHOLD {
            RoadSurfaceEarthworkFaceKind::RetainingWall
        } else {
            RoadSurfaceEarthworkFaceKind::Slope
        }
    }

    pub(in crate::simulation::network::surface) fn sort_earthwork_render_faces(
        faces: &mut [RoadSurfaceEarthworkRenderFace],
    ) {
        faces.sort_by(|a, b| {
            let kind_order = match (a.kind, b.kind) {
                (
                    RoadSurfaceEarthworkFaceKind::Slope,
                    RoadSurfaceEarthworkFaceKind::RetainingWall,
                ) => std::cmp::Ordering::Less,
                (
                    RoadSurfaceEarthworkFaceKind::RetainingWall,
                    RoadSurfaceEarthworkFaceKind::Slope,
                ) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            };
            if kind_order != std::cmp::Ordering::Equal {
                return kind_order;
            }
            a.source
                .source_ordering(b.source)
                .then(
                    a.inner_start
                        .x
                        .total_cmp(&b.inner_start.x)
                        .then(a.inner_start.z.total_cmp(&b.inner_start.z))
                        .then(a.inner_start.y.total_cmp(&b.inner_start.y)),
                )
                .then(
                    a.polygon
                        .points_world
                        .len()
                        .cmp(&b.polygon.points_world.len()),
                )
                .then_with(|| {
                    match a
                        .polygon
                        .points_world
                        .iter()
                        .zip(&b.polygon.points_world)
                        .find_map(|(point_a, point_b)| {
                            let ordering = point_a
                                .x
                                .total_cmp(&point_b.x)
                                .then(point_a.z.total_cmp(&point_b.z))
                                .then(point_a.y.total_cmp(&point_b.y));
                            (ordering != std::cmp::Ordering::Equal).then_some(ordering)
                        }) {
                        Some(ordering) => ordering,
                        None => std::cmp::Ordering::Equal,
                    }
                })
        });
    }

    pub(in crate::simulation::network::surface) fn earthwork_transition_point(
        &self,
        road_point: Vector3,
        outward_xz: Vector2,
        terrain: &TerrainSystem,
    ) -> Option<Vector3> {
        if outward_xz.length_squared() <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
            return None;
        }
        let outward_xz = outward_xz.normalized();
        let distance_m = self.earthwork_transition_distance_m(road_point, outward_xz, terrain);
        let outer_xz = Vector2::new(road_point.x, road_point.z) + outward_xz * distance_m;
        let outer_height_m =
            terrain.sample_height_world(outer_xz.x, outer_xz.y) * config::HEIGHT_SCALE;
        Some(Vector3::new(outer_xz.x, outer_height_m, outer_xz.y))
    }

    fn earthwork_transition_distance_m(
        &self,
        road_point: Vector3,
        outward_xz: Vector2,
        terrain: &TerrainSystem,
    ) -> f32 {
        let source_height_at_edge =
            terrain.sample_height_world(road_point.x, road_point.z) * config::HEIGHT_SCALE;
        let cut_side = source_height_at_edge > road_point.y;
        let slope_rate = if cut_side {
            EARTHWORK_CUT_SLOPE_RATE
        } else {
            EARTHWORK_FILL_SLOPE_RATE
        };

        let mut distance_m = EARTHWORK_MIN_MARGIN_M;
        while distance_m < EARTHWORK_MAX_MARGIN_M {
            let sample_x = road_point.x + outward_xz.x * distance_m;
            let sample_z = road_point.z + outward_xz.y * distance_m;
            let source_height =
                terrain.sample_height_world(sample_x, sample_z) * config::HEIGHT_SCALE;
            let transition_height = if cut_side {
                road_point.y + slope_rate * distance_m
            } else {
                road_point.y - slope_rate * distance_m
            };
            let rejoins_source = if cut_side {
                transition_height >= source_height
            } else {
                transition_height <= source_height
            };
            if rejoins_source {
                return distance_m;
            }
            distance_m += EARTHWORK_MARGIN_SAMPLE_STEP_M;
        }

        EARTHWORK_MAX_MARGIN_M
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earthwork_vertex_outward_rejects_degenerate_spur() {
        let points = vec![
            Vector3::new(-1.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(-1.0, 0.0, 0.0),
        ];

        assert!(RoadSurfaceSystem::closed_loop_vertex_outward_xz(&points, 1).is_none());
    }

    #[test]
    fn earthwork_edge_outward_accepts_short_nonzero_edges() {
        let outward = RoadSurfaceSystem::edge_outward_normal_xz(
            Vector2::new(SAMPLE_EPSILON_M * 10.0, 0.0),
            true,
        );

        assert_eq!(outward, Some(Vector2::new(0.0, -1.0)));
    }
}
