//! Debug overlay contour and shape helpers.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface::debug) fn debug_overlay_contours_from_polygons(
        polygons: &[RoadSurfaceVisualPolygon],
    ) -> Vec<NodeOverlayContour> {
        polygons
            .iter()
            .filter_map(|polygon| {
                Self::debug_overlay_contour_from_world_points(&polygon.points_world)
            })
            .collect()
    }

    pub(in crate::simulation::network::surface::debug) fn debug_overlay_contours_from_top_polygons<
        'a,
    >(
        polygons: impl IntoIterator<Item = &'a RoadSurfaceVisualPolygon>,
    ) -> Vec<NodeOverlayContour> {
        let mut contours = Vec::new();
        for polygon in polygons {
            if polygon.triangles_world.is_empty() {
                if let Some(contour) =
                    Self::debug_overlay_contour_from_world_points(&polygon.points_world)
                {
                    contours.push(contour);
                }
                continue;
            }
            for triangle in &polygon.triangles_world {
                if let Some(contour) = Self::debug_overlay_contour_from_world_points(triangle) {
                    contours.push(contour);
                }
            }
        }
        contours
    }

    pub(in crate::simulation::network::surface::debug) fn debug_overlay_contour_from_world_points(
        points: &[backend::RoadVec3],
    ) -> Option<NodeOverlayContour> {
        let mut contour = Vec::with_capacity(points.len());
        for point in points {
            let point = backend::road_vec2_to_overlay_point(backend::road_vec3_xz(*point));
            if contour.last().is_none_or(|last| *last != point) {
                contour.push(point);
            }
        }
        if contour.len() >= 2 && contour.first() == contour.last() {
            contour.pop();
        }
        (contour.len() >= 3).then_some(contour)
    }

    pub(in crate::simulation::network::surface::debug) fn debug_overlay_area_m2(
        shapes: &NodeOverlayShapes,
    ) -> f32 {
        shapes.iter().map(Self::overlay_shape_area_m2).sum()
    }

    pub(in crate::simulation::network::surface::debug) fn append_overlay_shape_samples(
        dump: &mut String,
        shapes: &[NodeOverlayShape],
    ) {
        for (index, shape) in shapes.iter().take(DEBUG_MAX_PROBLEM_SAMPLES).enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            let area_m2 = Self::overlay_shape_area_m2(shape);
            let thin_width_m = Self::debug_overlay_shape_thin_width_m(shape);
            let (centroid_x, centroid_z) = Self::overlay_shape_centroid_xz(shape);
            let (min_x, min_z, max_x, max_z) = Self::overlay_shape_bounds_xz(shape);
            let _ = write!(
                dump,
                "{{\"area_m2\":{:.6},\"thin_width_m\":{:.6},\"centroid\":[{:.6}, {:.6}],\"bounds\":[[{:.6}, {:.6}], [{:.6}, {:.6}]],\"contours\":[",
                area_m2, thin_width_m, centroid_x, centroid_z, min_x, min_z, max_x, max_z
            );
            for (contour_index, contour) in shape.iter().enumerate() {
                if contour_index > 0 {
                    dump.push_str(", ");
                }
                dump.push('[');
                for (point_index, point) in contour.iter().enumerate() {
                    if point_index > 0 {
                        dump.push_str(", ");
                    }
                    let _ = write!(dump, "[{:.6}, {:.6}]", point[0], point[1]);
                }
                dump.push(']');
            }
            dump.push_str("]}");
        }
    }

    pub(in crate::simulation::network::surface::debug) fn debug_overlay_shape_thin_width_m(
        shape: &NodeOverlayShape,
    ) -> f32 {
        let area_m2 = Self::overlay_shape_area_m2(shape);
        let perimeter_m = shape
            .iter()
            .map(|contour| Self::overlay_contour_perimeter_m(contour))
            .sum::<f32>();
        if perimeter_m <= f32::EPSILON {
            return 0.0;
        }
        2.0 * area_m2 / perimeter_m
    }

    pub(in crate::simulation::network::surface::debug) fn overlay_shape_centroid_xz(
        shape: &NodeOverlayShape,
    ) -> (f64, f64) {
        let mut weighted_x = 0.0;
        let mut weighted_z = 0.0;
        let mut total_weight = 0.0;
        for contour in shape {
            let area = Self::overlay_contour_area_f64(contour);
            let weight = area.abs();
            let (x, z) = Self::overlay_contour_average_xz(contour);
            weighted_x += x * weight;
            weighted_z += z * weight;
            total_weight += weight;
        }
        if total_weight <= f64::EPSILON {
            return Self::overlay_shape_average_xz(shape);
        }
        (weighted_x / total_weight, weighted_z / total_weight)
    }

    pub(in crate::simulation::network::surface::debug) fn overlay_shape_average_xz(
        shape: &NodeOverlayShape,
    ) -> (f64, f64) {
        let mut x = 0.0;
        let mut z = 0.0;
        let mut count = 0usize;
        for contour in shape {
            for point in contour {
                x += point[0];
                z += point[1];
                count += 1;
            }
        }
        if count == 0 {
            return (0.0, 0.0);
        }
        (x / count as f64, z / count as f64)
    }

    pub(in crate::simulation::network::surface::debug) fn overlay_contour_average_xz(
        contour: &[NodeOverlayPoint],
    ) -> (f64, f64) {
        if contour.is_empty() {
            return (0.0, 0.0);
        }
        let mut x = 0.0;
        let mut z = 0.0;
        for point in contour {
            x += point[0];
            z += point[1];
        }
        (x / contour.len() as f64, z / contour.len() as f64)
    }

    pub(in crate::simulation::network::surface::debug) fn overlay_shape_bounds_xz(
        shape: &NodeOverlayShape,
    ) -> (f64, f64, f64, f64) {
        let mut min_x = f64::INFINITY;
        let mut min_z = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_z = f64::NEG_INFINITY;
        for point in shape.iter().flat_map(|contour| contour.iter()) {
            min_x = min_x.min(point[0]);
            min_z = min_z.min(point[1]);
            max_x = max_x.max(point[0]);
            max_z = max_z.max(point[1]);
        }
        if !min_x.is_finite() {
            return (0.0, 0.0, 0.0, 0.0);
        }
        (min_x, min_z, max_x, max_z)
    }
}
