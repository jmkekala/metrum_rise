//! Deterministic overlay boolean geometry helpers for road surfaces.

use super::{
    NODE_OVERLAY_MIN_AREA_M2, NODE_OVERLAY_NUMERIC_AREA_CAP_M2, NODE_OVERLAY_NUMERIC_AREA_EPS_M2,
    NODE_OVERLAY_NUMERIC_DUST_WIDTH_M, NodeOverlayContour, NodeOverlayPoint, NodeOverlayPointKey,
    NodeOverlayShape, NodeOverlayShapes, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceVisualPolygon, SAMPLE_EPSILON_M,
};
use godot::prelude::{Vector2, Vector3};
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay::{IntOverlayOptions, Overlay};
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::i_float::int::point::IntPoint;

use super::backend::ROAD_OVERLAY_COORDINATE_SCALE;

// Overlay boolean operations quantize coordinates to millimetres for deterministic keys.
const NODE_OVERLAY_SCALE: f64 = ROAD_OVERLAY_COORDINATE_SCALE;
type NodeIntContour = Vec<IntPoint>;
type NodeIntShape = Vec<NodeIntContour>;
type NodeIntShapes = Vec<NodeIntShape>;

#[derive(Clone, Copy)]
struct NodeIntGridOrigin {
    x: i64,
    y: i64,
}

impl RoadSurfaceSystem {
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
            (f64::from(point.x) * NODE_OVERLAY_SCALE).round() / NODE_OVERLAY_SCALE,
            (f64::from(point.z) * NODE_OVERLAY_SCALE).round() / NODE_OVERLAY_SCALE,
        ]
    }

    pub(super) fn overlay_union_contours(
        contours: &[NodeOverlayContour],
    ) -> Option<NodeOverlayShapes> {
        if contours.is_empty() {
            return Some(Vec::new());
        }
        let origin = Self::int_grid_origin_for_contours(contours);
        let subject = Self::int_contours_from_overlay_contours(contours, origin)?;
        let clip = Vec::new();
        let mut overlay = Overlay::with_contours_custom(
            &subject,
            &clip,
            Self::int_overlay_options(),
            Default::default(),
        );
        let shapes = overlay.overlay(OverlayRule::Union, FillRule::Positive);
        Some(Self::filter_overlay_shapes_by_area(
            Self::overlay_shapes_from_int_shapes(shapes, origin),
        ))
    }

    pub(super) fn overlay_binary_shapes(
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
        let origin = Self::int_grid_origin_for_shapes(subject, clip);
        let subject = Self::int_shapes_from_overlay_shapes(subject, origin)?;
        let clip = Self::int_shapes_from_overlay_shapes(clip, origin)?;
        let mut overlay = Overlay::with_shapes_options(
            &subject,
            &clip,
            Self::int_overlay_options(),
            Default::default(),
        );
        let shapes = overlay.overlay(rule, FillRule::Positive);
        Some(Self::filter_overlay_shapes_by_area(
            Self::overlay_shapes_from_int_shapes(shapes, origin),
        ))
    }

    fn int_overlay_options() -> IntOverlayOptions {
        IntOverlayOptions {
            min_output_area: 0,
            ..Default::default()
        }
    }

    fn int_contours_from_overlay_contours(
        contours: &[NodeOverlayContour],
        origin: NodeIntGridOrigin,
    ) -> Option<Vec<NodeIntContour>> {
        contours
            .iter()
            .map(|contour| Self::int_contour_from_overlay_contour(contour, origin))
            .collect::<Option<Vec<_>>>()
            .map(|contours| {
                contours
                    .into_iter()
                    .filter(|contour| contour.len() >= 3)
                    .collect()
            })
    }

    fn int_shapes_from_overlay_shapes(
        shapes: &[NodeOverlayShape],
        origin: NodeIntGridOrigin,
    ) -> Option<NodeIntShapes> {
        shapes
            .iter()
            .map(|shape| {
                shape
                    .iter()
                    .map(|contour| Self::int_contour_from_overlay_contour(contour, origin))
                    .collect::<Option<Vec<_>>>()
                    .map(|shape| {
                        shape
                            .into_iter()
                            .filter(|contour| contour.len() >= 3)
                            .collect::<Vec<_>>()
                    })
            })
            .collect::<Option<Vec<_>>>()
            .map(|shapes| {
                shapes
                    .into_iter()
                    .filter(|shape| !shape.is_empty())
                    .collect()
            })
    }

    fn int_contour_from_overlay_contour(
        contour: &NodeOverlayContour,
        origin: NodeIntGridOrigin,
    ) -> Option<NodeIntContour> {
        let mut int_contour = Vec::with_capacity(contour.len());
        for point in contour {
            let point = Self::int_point_from_overlay_point(*point, origin)?;
            if int_contour.last().is_none_or(|last| *last != point) {
                int_contour.push(point);
            }
        }
        if int_contour.len() >= 2 && int_contour.first() == int_contour.last() {
            int_contour.pop();
        }
        Some(int_contour)
    }

    fn int_grid_origin_for_contours(contours: &[NodeOverlayContour]) -> NodeIntGridOrigin {
        let mut min_x = i64::MAX;
        let mut min_y = i64::MAX;
        for contour in contours {
            for point in contour {
                let key = Self::overlay_point_grid_key(*point);
                min_x = min_x.min(key.0);
                min_y = min_y.min(key.1);
            }
        }
        if min_x == i64::MAX {
            return NodeIntGridOrigin { x: 0, y: 0 };
        }
        NodeIntGridOrigin { x: min_x, y: min_y }
    }

    fn int_grid_origin_for_shapes(
        subject: &[NodeOverlayShape],
        clip: &[NodeOverlayShape],
    ) -> NodeIntGridOrigin {
        let mut min_x = i64::MAX;
        let mut min_y = i64::MAX;
        for shape in subject.iter().chain(clip.iter()) {
            for contour in shape {
                for point in contour {
                    let key = Self::overlay_point_grid_key(*point);
                    min_x = min_x.min(key.0);
                    min_y = min_y.min(key.1);
                }
            }
        }
        if min_x == i64::MAX {
            return NodeIntGridOrigin { x: 0, y: 0 };
        }
        NodeIntGridOrigin { x: min_x, y: min_y }
    }

    fn overlay_point_grid_key(point: NodeOverlayPoint) -> NodeOverlayPointKey {
        (
            (point[0] * NODE_OVERLAY_SCALE).round() as i64,
            (point[1] * NODE_OVERLAY_SCALE).round() as i64,
        )
    }

    fn int_point_from_overlay_point(
        point: NodeOverlayPoint,
        origin: NodeIntGridOrigin,
    ) -> Option<IntPoint> {
        let key = Self::overlay_point_grid_key(point);
        Some(IntPoint::new(
            i32::try_from(key.0 - origin.x).ok()?,
            i32::try_from(key.1 - origin.y).ok()?,
        ))
    }

    fn overlay_shapes_from_int_shapes(
        shapes: NodeIntShapes,
        origin: NodeIntGridOrigin,
    ) -> NodeOverlayShapes {
        shapes
            .into_iter()
            .map(|shape| {
                shape
                    .into_iter()
                    .map(|contour| {
                        contour
                            .into_iter()
                            .map(|point| Self::overlay_point_from_int_point(point, origin))
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn overlay_point_from_int_point(
        point: IntPoint,
        origin: NodeIntGridOrigin,
    ) -> NodeOverlayPoint {
        [
            (origin.x + i64::from(point.x)) as f64 / NODE_OVERLAY_SCALE,
            (origin.y + i64::from(point.y)) as f64 / NODE_OVERLAY_SCALE,
        ]
    }

    fn filter_overlay_shapes_by_area(shapes: NodeOverlayShapes) -> NodeOverlayShapes {
        shapes
            .into_iter()
            .filter_map(|shape| {
                let filtered = shape
                    .into_iter()
                    .filter_map(Self::canonical_overlay_contour)
                    .collect::<Vec<_>>();
                let outer = filtered.first()?;
                (Self::overlay_contour_area(outer).abs() > NODE_OVERLAY_MIN_AREA_M2)
                    .then_some(filtered)
            })
            .collect()
    }

    fn canonical_overlay_contour(contour: NodeOverlayContour) -> Option<NodeOverlayContour> {
        let mut canonical = Vec::with_capacity(contour.len());
        for point in contour {
            let point = Self::canonical_overlay_point(point);
            if canonical.last().is_none_or(|last| *last != point) {
                canonical.push(point);
            }
        }
        if canonical.len() >= 2 && canonical.first() == canonical.last() {
            canonical.pop();
        }
        (canonical.len() >= 3
            && Self::overlay_contour_area(&canonical).abs() > NODE_OVERLAY_MIN_AREA_M2)
            .then_some(canonical)
    }

    fn canonical_overlay_point(point: NodeOverlayPoint) -> NodeOverlayPoint {
        [
            (point[0] * NODE_OVERLAY_SCALE).round() / NODE_OVERLAY_SCALE,
            (point[1] * NODE_OVERLAY_SCALE).round() / NODE_OVERLAY_SCALE,
        ]
    }

    pub(super) fn sort_overlay_shapes(shapes: &mut [NodeOverlayShape]) {
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
        (signed_area * 0.5) as f32
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
        Self::terrain_clip_polygons_from_overlay_shapes_with_candidate_heights(&shapes, polygons)
    }

    fn terrain_clip_polygons_from_overlay_shapes_with_candidate_heights(
        shapes: &[NodeOverlayShape],
        candidates: &[RoadSurfaceVisualPolygon],
    ) -> Vec<RoadSurfaceVisualPolygon> {
        let mut polygons = Vec::new();
        for shape in shapes {
            let Some(polygon) =
                Self::terrain_clip_polygon_from_overlay_shape_with_candidate_heights(
                    shape, candidates,
                )
            else {
                continue;
            };
            polygons.push(polygon);
        }
        Self::sort_visual_polygons(&mut polygons);
        polygons
    }

    pub(super) fn overlay_shape_area_m2(shape: &NodeOverlayShape) -> f32 {
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

    pub(super) fn overlay_numeric_area_budget_for_shape(shape: &NodeOverlayShape) -> f32 {
        let perimeter_m = shape
            .iter()
            .map(|contour| Self::overlay_contour_perimeter_m(contour))
            .sum::<f32>();
        let vertex_count = shape.iter().map(Vec::len).sum::<usize>();
        Self::overlay_numeric_area_budget_m2(perimeter_m, vertex_count)
    }

    pub(super) fn overlay_numeric_area_budget_for_shapes(shapes: &NodeOverlayShapes) -> f32 {
        let perimeter_m = shapes
            .iter()
            .flat_map(|shape| shape.iter())
            .map(|contour| Self::overlay_contour_perimeter_m(contour))
            .sum::<f32>();
        let vertex_count = shapes
            .iter()
            .flat_map(|shape| shape.iter())
            .map(Vec::len)
            .sum::<usize>();
        Self::overlay_numeric_area_budget_m2(perimeter_m, vertex_count)
    }

    pub(super) fn overlay_numeric_area_budget_for_world_loop(points_world: &[Vector3]) -> f32 {
        if points_world.len() < 2 {
            return NODE_OVERLAY_NUMERIC_AREA_EPS_M2;
        }
        let perimeter_m = points_world
            .iter()
            .zip(points_world.iter().cycle().skip(1))
            .take(points_world.len())
            .map(|(start, end)| start.distance_to(*end))
            .sum::<f32>();
        Self::overlay_numeric_area_budget_m2(perimeter_m, points_world.len())
    }

    fn overlay_numeric_area_budget_m2(perimeter_m: f32, vertex_count: usize) -> f32 {
        let boundary_strip_m2 = perimeter_m * NODE_OVERLAY_NUMERIC_DUST_WIDTH_M;
        let vertex_floor_m2 = vertex_count.max(1) as f32 * NODE_OVERLAY_MIN_AREA_M2;
        (NODE_OVERLAY_NUMERIC_AREA_EPS_M2 + boundary_strip_m2 + vertex_floor_m2)
            .min(NODE_OVERLAY_NUMERIC_AREA_CAP_M2)
    }

    fn overlay_contour_perimeter_m(contour: &NodeOverlayContour) -> f32 {
        if contour.len() < 2 {
            return 0.0;
        }
        contour
            .iter()
            .zip(contour.iter().cycle().skip(1))
            .take(contour.len())
            .map(|(start, end)| {
                let dx = start[0] - end[0];
                let dz = start[1] - end[1];
                (dx * dx + dz * dz).sqrt() as f32
            })
            .sum()
    }

    fn overlay_point_key(point: NodeOverlayPoint) -> NodeOverlayPointKey {
        (
            (point[0] * NODE_OVERLAY_SCALE).round() as i64,
            (point[1] * NODE_OVERLAY_SCALE).round() as i64,
        )
    }

    pub(super) fn band_kind_sort_key(kind: RoadSurfaceBandKind) -> u8 {
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

    fn terrain_clip_polygon_from_overlay_shape_with_candidate_heights(
        shape: &NodeOverlayShape,
        candidates: &[RoadSurfaceVisualPolygon],
    ) -> Option<RoadSurfaceVisualPolygon> {
        let outer_contour = shape.first()?;
        let outer_points = Self::world_points_from_overlay_contour_with_candidate_heights(
            outer_contour,
            candidates,
        )?;
        Self::make_boundary_loop_polygon(outer_points)
    }

    fn world_points_from_overlay_contour_with_candidate_heights(
        contour: &NodeOverlayContour,
        candidates: &[RoadSurfaceVisualPolygon],
    ) -> Option<Vec<Vector3>> {
        contour
            .iter()
            .map(|point| {
                let xz = Vector2::new(point[0] as f32, point[1] as f32);
                let y = Self::sample_height_from_candidate_coverage(xz, candidates)?;
                Some(Vector3::new(point[0] as f32, y, point[1] as f32))
            })
            .collect()
    }

    fn sample_height_from_candidate_coverage(
        point_xz: Vector2,
        candidates: &[RoadSurfaceVisualPolygon],
    ) -> Option<f32> {
        let point_key = Self::overlay_point_key([f64::from(point_xz.x), f64::from(point_xz.y)]);
        let mut vertex_heights = Vec::new();
        for polygon in candidates {
            for point in &polygon.points_world {
                if Self::overlay_point_key([f64::from(point.x), f64::from(point.z)]) == point_key {
                    vertex_heights.push(point.y);
                }
            }
            for triangle in &polygon.triangles_world {
                for point in triangle {
                    if Self::overlay_point_key([f64::from(point.x), f64::from(point.z)])
                        == point_key
                    {
                        vertex_heights.push(point.y);
                    }
                }
            }
        }
        if !vertex_heights.is_empty() {
            return Self::highest_height_sample(vertex_heights);
        }

        let mut edge_heights = Vec::new();
        for polygon in candidates {
            for index in 0..polygon.points_world.len() {
                let start = polygon.points_world[index];
                let end = polygon.points_world[(index + 1) % polygon.points_world.len()];
                if let Some(height) = Self::sample_height_from_candidate_edge(point_xz, start, end)
                {
                    edge_heights.push(height);
                }
            }
        }
        if !edge_heights.is_empty() {
            return Self::highest_height_sample(edge_heights);
        }

        let mut covered_heights = Vec::new();
        for polygon in candidates {
            for triangle in &polygon.triangles_world {
                if let Some((wa, wb, wc)) =
                    Self::triangle_barycentric_weights_xz(*triangle, point_xz)
                {
                    covered_heights
                        .push(triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc);
                }
            }
        }
        Self::highest_height_sample(covered_heights)
    }

    fn sample_height_from_candidate_edge(
        point_xz: Vector2,
        start: Vector3,
        end: Vector3,
    ) -> Option<f32> {
        let start_xz = Vector2::new(start.x, start.z);
        let end_xz = Vector2::new(end.x, end.z);
        let segment = end_xz - start_xz;
        let length_squared = segment.length_squared();
        if length_squared <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
            return None;
        }
        let t = ((point_xz - start_xz).dot(segment) / length_squared).clamp(0.0, 1.0);
        let closest = start_xz + segment * t;
        if point_xz.distance_squared_to(closest) > SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
            return None;
        }
        Some(start.y + (end.y - start.y) * t)
    }

    // Unioned terrain cutters may touch several visible top surfaces at the same XZ seam.
    // Use the highest deterministic top height so terrain cannot survive above a road surface.
    fn highest_height_sample<I>(heights: I) -> Option<f32>
    where
        I: IntoIterator<Item = f32>,
    {
        let mut heights = heights.into_iter().collect::<Vec<_>>();
        if heights.is_empty() {
            return None;
        }
        heights.sort_by(|a, b| a.total_cmp(b));
        heights.last().copied()
    }
}
