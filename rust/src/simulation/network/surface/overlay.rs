//! Deterministic overlay boolean geometry helpers for road surfaces.

use super::band_semantics::band_kind_sort_key;
use super::{
    NODE_OVERLAY_MIN_AREA_M2, NODE_OVERLAY_NUMERIC_AREA_CAP_M2, NODE_OVERLAY_NUMERIC_AREA_EPS_M2,
    NODE_OVERLAY_NUMERIC_DUST_WIDTH_M, NodeOverlayContour, NodeOverlayPoint, NodeOverlayPointKey,
    NodeOverlayShape, NodeOverlayShapes, RoadSurfaceBandKind, RoadSurfaceSystem,
};
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay::{IntOverlayOptions, Overlay};
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::i_float::int::point::IntPoint;

use super::backend::ROAD_OVERLAY_COORDINATE_SCALE;

// Overlay boolean operations quantize coordinates onto the project overlay grid.
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

    /// Computes signed contour area in square metres without narrowing to `f32`.
    pub(super) fn overlay_contour_area_f64(contour: &NodeOverlayContour) -> f64 {
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

    pub(super) fn overlay_contour_area(contour: &NodeOverlayContour) -> f32 {
        Self::overlay_contour_area_f64(contour) as f32
    }

    pub(super) fn overlay_shape_area_m2(shape: &NodeOverlayShape) -> f32 {
        let Some((outer, holes)) = shape.split_first() else {
            return 0.0;
        };
        let holes_area = holes
            .iter()
            .map(|hole| Self::overlay_contour_area(hole).abs())
            .sum::<f32>();
        (Self::overlay_contour_area(outer).abs() - holes_area).max(0.0)
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

    pub(super) fn overlay_numeric_area_budget_m2(perimeter_m: f32, vertex_count: usize) -> f32 {
        let boundary_strip_m2 = perimeter_m * NODE_OVERLAY_NUMERIC_DUST_WIDTH_M;
        let vertex_floor_m2 = vertex_count.max(1) as f32 * NODE_OVERLAY_MIN_AREA_M2;
        (NODE_OVERLAY_NUMERIC_AREA_EPS_M2 + boundary_strip_m2 + vertex_floor_m2)
            .min(NODE_OVERLAY_NUMERIC_AREA_CAP_M2)
    }

    pub(super) fn overlay_contour_perimeter_m(contour: &NodeOverlayContour) -> f32 {
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

    pub(super) fn band_kind_sort_key(kind: RoadSurfaceBandKind) -> u8 {
        band_kind_sort_key(kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_contour_area_preserves_signed_winding() {
        let contour = vec![[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [0.0, 1.0]];
        let mut reversed = contour.clone();
        reversed.reverse();

        assert_eq!(RoadSurfaceSystem::overlay_contour_area_f64(&contour), 2.0);
        assert_eq!(RoadSurfaceSystem::overlay_contour_area_f64(&reversed), -2.0);
        assert_eq!(
            RoadSurfaceSystem::overlay_contour_area(&contour),
            RoadSurfaceSystem::overlay_contour_area_f64(&contour) as f32
        );
    }

    #[test]
    fn overlay_contour_area_rejects_degenerate_point_counts() {
        let line = vec![[0.0, 0.0], [1.0, 0.0]];

        assert_eq!(RoadSurfaceSystem::overlay_contour_area_f64(&line), 0.0);
        assert_eq!(RoadSurfaceSystem::overlay_contour_area(&line), 0.0);
    }
}
