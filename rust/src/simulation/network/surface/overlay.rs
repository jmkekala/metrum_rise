//! Deterministic overlay boolean geometry helpers for road surfaces.

use super::{
    NODE_OVERLAY_MIN_AREA_M2, NODE_OVERLAY_NUMERIC_AREA_CAP_M2, NODE_OVERLAY_NUMERIC_AREA_EPS_M2,
    NODE_OVERLAY_NUMERIC_DUST_WIDTH_M, NodeOverlayContour, NodeOverlayPoint, NodeOverlayPointKey,
    NodeOverlayShape, NodeOverlayShapes, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceTerrainClipEdgeKind, RoadSurfaceTerrainClipLoop, RoadSurfaceVisualPolygon,
    WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2,
};
use godot::prelude::Vector3;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay::{IntOverlayOptions, Overlay};
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::i_float::int::point::IntPoint;
use std::collections::BTreeMap;

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

#[derive(Clone, Copy)]
struct TerrainClipSourceEdge {
    start: Vector3,
    end: Vector3,
    kind: RoadSurfaceTerrainClipEdgeKind,
    source_index: usize,
    edge_index: usize,
}

#[derive(Clone, Copy)]
struct TerrainClipSegmentHeights {
    start_y: f32,
    end_y: f32,
}

#[derive(Clone, Copy)]
struct TerrainClipEndpointSample {
    kind: RoadSurfaceTerrainClipEdgeKind,
    source_index: usize,
    edge_index: usize,
    y: f32,
}

#[derive(Clone, Copy)]
struct TerrainClipSourceInterval {
    start_t: f64,
    end_t: f64,
    start_y: f32,
    end_y: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OverlaySegmentParameter {
    numerator: i128,
    denominator: i128,
}

impl OverlaySegmentParameter {
    fn new(numerator: i128, denominator: i128) -> Option<Self> {
        (denominator > 0).then_some(Self {
            numerator,
            denominator,
        })
    }

    fn as_f64(self) -> f64 {
        self.numerator as f64 / self.denominator as f64
    }
}

impl RoadSurfaceSystem {
    fn overlay_contours_from_terrain_clip_boundary_loops(
        boundary_loops: &[RoadSurfaceTerrainClipLoop],
    ) -> Vec<NodeOverlayContour> {
        let mut contours = Vec::new();
        for boundary_loop in boundary_loops {
            let contour = Self::overlay_contour_from_world_points(&boundary_loop.points_world);
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

    pub(super) fn overlay_contour_area(contour: &NodeOverlayContour) -> f32 {
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

    pub(super) fn union_terrain_clip_boundary_loops(
        boundary_loops: &[RoadSurfaceTerrainClipLoop],
    ) -> Vec<RoadSurfaceVisualPolygon> {
        if boundary_loops.is_empty() {
            return Vec::new();
        }

        let contours = Self::overlay_contours_from_terrain_clip_boundary_loops(boundary_loops);
        let Some(mut shapes) = Self::overlay_union_contours(&contours) else {
            return Vec::new();
        };
        Self::sort_overlay_shapes(&mut shapes);
        let source_edges = Self::terrain_clip_source_edges_from_boundary_loops(boundary_loops);
        Self::terrain_clip_polygons_from_overlay_shapes_with_source_edges(&shapes, &source_edges)
    }

    fn terrain_clip_polygons_from_overlay_shapes_with_source_edges(
        shapes: &[NodeOverlayShape],
        source_edges: &[TerrainClipSourceEdge],
    ) -> Vec<RoadSurfaceVisualPolygon> {
        let mut polygons = Vec::new();
        for (shape_index, shape) in shapes.iter().enumerate() {
            let Some(polygon) = Self::terrain_clip_polygon_from_overlay_shape_with_source_edges(
                shape,
                shape_index,
                source_edges,
            ) else {
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

    pub(super) fn overlay_numeric_area_budget_m2(perimeter_m: f32, vertex_count: usize) -> f32 {
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

    fn terrain_clip_polygon_from_overlay_shape_with_source_edges(
        shape: &NodeOverlayShape,
        shape_index: usize,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<RoadSurfaceVisualPolygon> {
        let outer_contour = shape.first()?;
        let outer_points = Self::world_points_from_overlay_contour_with_source_edges(
            outer_contour,
            shape_index,
            source_edges,
        )?;
        Self::make_boundary_loop_polygon(outer_points)
    }

    fn world_points_from_overlay_contour_with_source_edges(
        contour: &NodeOverlayContour,
        shape_index: usize,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<Vec<Vector3>> {
        let contour = Self::compact_overlay_contour_by_key(contour);
        if contour.len() < 3 {
            return None;
        }

        let mut points = Vec::new();
        for index in 0..contour.len() {
            let start = contour[index];
            let end = contour[(index + 1) % contour.len()];
            let Some(segment_points) =
                Self::terrain_clip_segment_points_from_source_edges(start, end, source_edges)
                    .or_else(|| {
                        Self::terrain_clip_dust_connector_points_from_source_edges(
                            &contour,
                            index,
                            source_edges,
                        )
                    })
            else {
                let context =
                    Self::terrain_clip_missing_source_context_label(start, end, source_edges);
                crate::debug_log!(
                    "road",
                    "terrain_clip_missing_outer_boundary_owner shape={} start=({:.3},{:.3}) end=({:.3},{:.3}) {}",
                    shape_index,
                    start[0],
                    start[1],
                    end[0],
                    end[1],
                    context
                );
                return None;
            };
            Self::append_terrain_clip_segment_points(&mut points, segment_points);
        }

        Self::close_terrain_clip_contour_points(&mut points);
        (points.len() >= 3).then_some(points)
    }

    fn compact_overlay_contour_by_key(contour: &NodeOverlayContour) -> NodeOverlayContour {
        let mut compact = Vec::with_capacity(contour.len());
        for &point in contour {
            if compact
                .last()
                .is_none_or(|last| !overlay_points_same_for_boundary(*last, point))
            {
                compact.push(point);
            }
        }
        while compact.len() >= 2
            && overlay_points_same_for_boundary(*compact.first().unwrap(), *compact.last().unwrap())
        {
            compact.pop();
        }
        remove_repeated_overlay_point_spurs(&mut compact);
        compact
    }

    fn terrain_clip_source_edges_from_boundary_loops(
        boundary_loops: &[RoadSurfaceTerrainClipLoop],
    ) -> Vec<TerrainClipSourceEdge> {
        let mut edges = Vec::new();
        for (source_index, boundary_loop) in boundary_loops.iter().enumerate() {
            for (edge_index, source_edge) in boundary_loop.source_edges.iter().copied().enumerate()
            {
                let start = Self::terrain_clip_canonical_loop_point(
                    source_edge.start,
                    &boundary_loop.points_world,
                );
                let end = Self::terrain_clip_canonical_loop_point(
                    source_edge.end,
                    &boundary_loop.points_world,
                );
                if Self::overlay_point_key([f64::from(start.x), f64::from(start.z)])
                    == Self::overlay_point_key([f64::from(end.x), f64::from(end.z)])
                {
                    continue;
                }
                edges.push(TerrainClipSourceEdge {
                    start,
                    end,
                    kind: source_edge.kind,
                    source_index,
                    edge_index,
                });
            }
        }
        // Keep source endpoints on one canonical coordinate only when they are already the same
        // solved-height seam endpoint after boundary-loop representation cleanup.
        Self::canonicalize_terrain_clip_source_endpoint_groups(&mut edges);

        edges.sort_by_key(|edge| {
            let start_key =
                Self::overlay_point_key([f64::from(edge.start.x), f64::from(edge.start.z)]);
            let end_key = Self::overlay_point_key([f64::from(edge.end.x), f64::from(edge.end.z)]);
            (
                start_key.min(end_key),
                start_key.max(end_key),
                terrain_clip_edge_kind_priority(edge.kind),
                Self::overlay_height_key(edge.start.y),
                Self::overlay_height_key(edge.end.y),
                edge.source_index,
                edge.edge_index,
            )
        });
        edges
    }

    fn canonicalize_terrain_clip_source_endpoint_groups(edges: &mut [TerrainClipSourceEdge]) {
        let mut groups: Vec<Vec<Vector3>> = Vec::new();
        for point in edges.iter().flat_map(|edge| [edge.start, edge.end]) {
            if let Some(group) = groups.iter_mut().find(|group| {
                group.iter().any(|candidate| {
                    Self::terrain_clip_source_points_share_canonical_endpoint(*candidate, point)
                })
            }) {
                group.push(point);
            } else {
                groups.push(vec![point]);
            }
        }

        let mut replacements = BTreeMap::new();
        for group in groups {
            if group.len() < 2 {
                continue;
            }
            let mut point_counts = BTreeMap::<(i64, i64, i64), (usize, Vector3)>::new();
            for point in group {
                let key = Self::terrain_clip_source_point_group_key(point);
                let entry = point_counts.entry(key).or_insert((0, point));
                entry.0 += 1;
            }
            let mut counted_points = point_counts.into_iter().collect::<Vec<_>>();
            counted_points.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(&b.0)));
            let Some((_, (_, replacement))) = counted_points.first().copied() else {
                continue;
            };
            for (key, _) in counted_points {
                replacements.insert(key, replacement);
            }
        }

        for edge in edges {
            if let Some(point) =
                replacements.get(&Self::terrain_clip_source_point_group_key(edge.start))
            {
                edge.start = *point;
            }
            if let Some(point) =
                replacements.get(&Self::terrain_clip_source_point_group_key(edge.end))
            {
                edge.end = *point;
            }
        }
    }

    fn terrain_clip_source_points_share_canonical_endpoint(a: Vector3, b: Vector3) -> bool {
        let dx = a.x - b.x;
        let dz = a.z - b.z;
        dx * dx + dz * dz <= WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2
            && Self::overlay_height_key(a.y) == Self::overlay_height_key(b.y)
    }

    fn terrain_clip_source_point_group_key(point: Vector3) -> (i64, i64, i64) {
        let key = Self::overlay_point_key([f64::from(point.x), f64::from(point.z)]);
        (key.0, key.1, Self::overlay_height_key(point.y))
    }

    fn terrain_clip_canonical_loop_point(point: Vector3, loop_points: &[Vector3]) -> Vector3 {
        loop_points
            .iter()
            .copied()
            .find(|candidate| {
                let dx = candidate.x - point.x;
                let dz = candidate.z - point.z;
                dx * dx + dz * dz <= WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2
                    && Self::overlay_height_key(candidate.y) == Self::overlay_height_key(point.y)
            })
            .unwrap_or(point)
    }

    fn terrain_clip_segment_heights_from_source_edges(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<TerrainClipSegmentHeights> {
        let points = Self::terrain_clip_segment_points_from_source_edges(start, end, source_edges)?;
        Some(TerrainClipSegmentHeights {
            start_y: points.first()?.y,
            end_y: points.last()?.y,
        })
    }

    fn terrain_clip_segment_points_from_source_edges(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<Vec<Vector3>> {
        if Self::overlay_point_key(start) == Self::overlay_point_key(end) {
            return None;
        }

        let mut samples = Vec::new();
        for &source_edge in source_edges {
            if let Some(interval) =
                Self::terrain_clip_source_interval_on_segment(start, end, source_edge)
            {
                samples.push(interval);
            }
        }
        Self::terrain_clip_points_from_interval_coverage(start, end, samples)
    }

    fn terrain_clip_points_from_interval_coverage<I>(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        intervals: I,
    ) -> Option<Vec<Vector3>>
    where
        I: IntoIterator<Item = TerrainClipSourceInterval>,
    {
        let intervals = intervals.into_iter().collect::<Vec<_>>();
        if intervals.is_empty() {
            return None;
        }

        let mut breakpoints = Self::terrain_clip_interval_breakpoints(&intervals);
        Self::append_terrain_clip_height_crossings(&intervals, &mut breakpoints);
        breakpoints.sort_by(|a, b| a.total_cmp(b));
        breakpoints.dedup_by(|a, b| *a == *b);

        let mut heights = vec![None; breakpoints.len()];
        let mut covered_any = false;
        for index in 0..breakpoints.len().saturating_sub(1) {
            let start_t = breakpoints[index];
            let end_t = breakpoints[index + 1];
            if end_t <= start_t {
                continue;
            }

            let covering = intervals
                .iter()
                .copied()
                .filter(|interval| Self::terrain_clip_interval_covers(*interval, start_t, end_t))
                .collect::<Vec<_>>();
            if covering.is_empty() {
                return None;
            }
            covered_any = true;

            let start_y = Self::terrain_clip_highest_source_height_at_t(&covering, start_t)?;
            let end_y = Self::terrain_clip_highest_source_height_at_t(&covering, end_t)?;
            Self::merge_terrain_clip_height(&mut heights[index], start_y);
            Self::merge_terrain_clip_height(&mut heights[index + 1], end_y);
        }
        if !covered_any {
            return None;
        }

        let mut points = Vec::new();
        for (t, height) in breakpoints.into_iter().zip(heights) {
            let Some(height) = height else {
                continue;
            };
            let point = interpolate_overlay_point(start, end, t);
            points.push(Vector3::new(point[0] as f32, height, point[1] as f32));
        }
        Self::dedup_terrain_clip_segment_points(&mut points);
        (points.len() >= 2).then_some(points)
    }

    fn terrain_clip_interval_breakpoints(intervals: &[TerrainClipSourceInterval]) -> Vec<f64> {
        let mut breakpoints = Vec::with_capacity(intervals.len() * 2 + 2);
        breakpoints.push(0.0);
        breakpoints.push(1.0);
        for interval in intervals {
            breakpoints.push(interval.start_t.clamp(0.0, 1.0));
            breakpoints.push(interval.end_t.clamp(0.0, 1.0));
        }
        breakpoints
    }

    fn append_terrain_clip_height_crossings(
        intervals: &[TerrainClipSourceInterval],
        breakpoints: &mut Vec<f64>,
    ) {
        for first_index in 0..intervals.len() {
            for second_index in first_index + 1..intervals.len() {
                let first = intervals[first_index];
                let second = intervals[second_index];
                let start_t = first.start_t.max(second.start_t).max(0.0);
                let end_t = first.end_t.min(second.end_t).min(1.0);
                if end_t <= start_t {
                    continue;
                }
                let start_delta =
                    interval_height_at(first, start_t) - interval_height_at(second, start_t);
                let end_delta =
                    interval_height_at(first, end_t) - interval_height_at(second, end_t);
                if Self::overlay_heights_equal(start_delta, 0.0) {
                    breakpoints.push(start_t);
                }
                if Self::overlay_heights_equal(end_delta, 0.0) {
                    breakpoints.push(end_t);
                }
                if start_delta.signum() == end_delta.signum() {
                    continue;
                }
                let denominator = f64::from(start_delta - end_delta);
                if denominator == 0.0 {
                    continue;
                }
                let crossing_t = start_t + (end_t - start_t) * f64::from(start_delta) / denominator;
                if crossing_t > start_t && crossing_t < end_t {
                    breakpoints.push(crossing_t);
                }
            }
        }
    }

    fn terrain_clip_interval_covers(
        interval: TerrainClipSourceInterval,
        start_t: f64,
        end_t: f64,
    ) -> bool {
        interval.start_t <= start_t && interval.end_t >= end_t
    }

    fn terrain_clip_highest_source_height_at_t(
        intervals: &[TerrainClipSourceInterval],
        t: f64,
    ) -> Option<f32> {
        intervals
            .iter()
            .copied()
            .map(|interval| interval_height_at(interval, t))
            .max_by(|a, b| a.total_cmp(b))
    }

    fn merge_terrain_clip_height(height: &mut Option<f32>, candidate: f32) {
        *height = Some(height.map_or(candidate, |current| current.max(candidate)));
    }

    fn dedup_terrain_clip_segment_points(points: &mut Vec<Vector3>) {
        let mut deduped = Vec::with_capacity(points.len());
        for &point in points.iter() {
            if let Some(last) = deduped.last_mut() {
                if Self::world_points_same_for_boundary(*last, point) {
                    if point.y > last.y {
                        last.y = point.y;
                    }
                    continue;
                }
            }
            deduped.push(point);
        }
        *points = deduped;
    }

    fn terrain_clip_dust_connector_heights_from_source_edges(
        contour: &NodeOverlayContour,
        segment_index: usize,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<TerrainClipSegmentHeights> {
        let len = contour.len();
        if len < 3 {
            return None;
        }

        let start = contour[segment_index];
        let end = contour[(segment_index + 1) % len];
        if !Self::terrain_clip_connector_is_numeric_dust(contour, segment_index) {
            return None;
        }

        let previous = contour[(segment_index + len - 1) % len];
        let next = contour[(segment_index + 2) % len];
        if let (Some(previous_heights), Some(next_heights)) = (
            Self::terrain_clip_segment_heights_from_source_edges(previous, start, source_edges),
            Self::terrain_clip_segment_heights_from_source_edges(end, next, source_edges),
        ) {
            return Some(TerrainClipSegmentHeights {
                start_y: previous_heights.end_y,
                end_y: next_heights.start_y,
            });
        }

        let heights =
            Self::terrain_clip_contour_vertex_heights_from_source_edges(contour, source_edges)?;
        Some(TerrainClipSegmentHeights {
            start_y: heights[segment_index],
            end_y: heights[(segment_index + 1) % len],
        })
    }

    fn terrain_clip_contour_vertex_heights_from_source_edges(
        contour: &NodeOverlayContour,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<Vec<f32>> {
        let len = contour.len();
        if len < 3 {
            return None;
        }

        let mut heights = contour
            .iter()
            .copied()
            .map(|point| {
                Self::terrain_clip_overlay_point_height_from_source_edges(point, source_edges)
            })
            .collect::<Vec<_>>();
        let anchor = heights.iter().position(Option::is_some)?;
        let mut offset = 1usize;
        while offset < len {
            let index = (anchor + offset) % len;
            if heights[index].is_some() {
                offset += 1;
                continue;
            }

            let run_start_offset = offset;
            while offset < len && heights[(anchor + offset) % len].is_none() {
                offset += 1;
            }
            let prev_index = (anchor + run_start_offset - 1) % len;
            let next_index = (anchor + offset) % len;
            Self::interpolate_terrain_clip_dust_run_heights(
                contour,
                &mut heights,
                prev_index,
                next_index,
            )?;
        }

        heights.into_iter().collect()
    }

    fn terrain_clip_overlay_point_height_from_source_edges(
        point: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<f32> {
        let mut height = None;
        for &source_edge in source_edges {
            let source_start = [
                f64::from(source_edge.start.x),
                f64::from(source_edge.start.z),
            ];
            let source_end = [f64::from(source_edge.end.x), f64::from(source_edge.end.z)];
            let Some(t) = Self::overlay_segment_parameter(point, source_start, source_end) else {
                continue;
            };
            Self::merge_terrain_clip_height(
                &mut height,
                interpolate_height_f64(source_edge.start.y, source_edge.end.y, t),
            );
        }
        height
    }

    fn interpolate_terrain_clip_dust_run_heights(
        contour: &NodeOverlayContour,
        heights: &mut [Option<f32>],
        prev_index: usize,
        next_index: usize,
    ) -> Option<()> {
        let start_y = heights[prev_index]?;
        let end_y = heights[next_index]?;
        let mut total_length_m = 0.0f64;
        let mut edge_index = prev_index;
        while edge_index != next_index {
            if !Self::terrain_clip_connector_is_numeric_dust(contour, edge_index) {
                return None;
            }
            let next = (edge_index + 1) % contour.len();
            total_length_m += overlay_segment_length_m(contour[edge_index], contour[next]);
            edge_index = next;
        }

        let mut distance_m = 0.0f64;
        edge_index = prev_index;
        while edge_index != next_index {
            let next = (edge_index + 1) % contour.len();
            distance_m += overlay_segment_length_m(contour[edge_index], contour[next]);
            if heights[next].is_none() {
                let t = if total_length_m > 0.0 {
                    distance_m / total_length_m
                } else {
                    0.0
                };
                heights[next] = Some(interpolate_height_f64(start_y, end_y, t));
            }
            edge_index = next;
        }

        Some(())
    }

    fn terrain_clip_dust_connector_points_from_source_edges(
        contour: &NodeOverlayContour,
        segment_index: usize,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<Vec<Vector3>> {
        let len = contour.len();
        let heights = Self::terrain_clip_dust_connector_heights_from_source_edges(
            contour,
            segment_index,
            source_edges,
        )?;
        let start = contour[segment_index];
        let end = contour[(segment_index + 1) % len];
        Some(vec![
            Vector3::new(start[0] as f32, heights.start_y, start[1] as f32),
            Vector3::new(end[0] as f32, heights.end_y, end[1] as f32),
        ])
    }

    fn append_terrain_clip_segment_points(out: &mut Vec<Vector3>, points: Vec<Vector3>) {
        for point in points {
            if let Some(last) = out.last_mut() {
                if Self::world_points_same_for_boundary(*last, point) {
                    if point.y > last.y {
                        last.y = point.y;
                    }
                    continue;
                }
            }
            out.push(point);
        }
    }

    fn close_terrain_clip_contour_points(points: &mut Vec<Vector3>) {
        while points.len() >= 2
            && Self::world_points_same_for_boundary(
                *points.first().unwrap(),
                *points.last().unwrap(),
            )
        {
            let last = points.pop().unwrap();
            if last.y > points[0].y {
                points[0].y = last.y;
            }
        }
    }

    fn world_points_same_for_boundary(a: Vector3, b: Vector3) -> bool {
        Self::overlay_point_key([f64::from(a.x), f64::from(a.z)])
            == Self::overlay_point_key([f64::from(b.x), f64::from(b.z)])
    }

    fn terrain_clip_connector_is_numeric_dust(
        contour: &NodeOverlayContour,
        segment_index: usize,
    ) -> bool {
        let len = contour.len();
        if len < 4 {
            return false;
        }

        let start = contour[segment_index];
        let end = contour[(segment_index + 1) % len];
        let connector_length_squared_m2 =
            (start[0] - end[0]) * (start[0] - end[0]) + (start[1] - end[1]) * (start[1] - end[1]);
        if connector_length_squared_m2
            <= f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M)
                * f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M)
        {
            return true;
        }
        let budget_m2 = f64::from(Self::overlay_numeric_area_budget_m2(
            Self::overlay_contour_perimeter_m(contour),
            contour.len(),
        ));
        if connector_length_squared_m2 > budget_m2 {
            return false;
        }

        let area_m2 = overlay_contour_area_local(contour).abs();
        let remove_start_delta = contour_area_delta_after_removing_vertex(contour, segment_index)
            .map(|area| (area - area_m2).abs());
        let remove_end_delta =
            contour_area_delta_after_removing_vertex(contour, (segment_index + 1) % len)
                .map(|area| (area - area_m2).abs());
        remove_start_delta
            .into_iter()
            .chain(remove_end_delta)
            .any(|delta| delta <= budget_m2)
    }

    fn terrain_clip_endpoint_samples(
        point: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Vec<TerrainClipEndpointSample> {
        let point_key = Self::overlay_point_key(point);
        let mut samples = Vec::new();
        for &source_edge in source_edges {
            let source_start = [
                f64::from(source_edge.start.x),
                f64::from(source_edge.start.z),
            ];
            let source_end = [f64::from(source_edge.end.x), f64::from(source_edge.end.z)];
            if Self::overlay_point_key(source_start) == point_key {
                samples.push(TerrainClipEndpointSample {
                    kind: source_edge.kind,
                    source_index: source_edge.source_index,
                    edge_index: source_edge.edge_index,
                    y: source_edge.start.y,
                });
            }
            if Self::overlay_point_key(source_end) == point_key {
                samples.push(TerrainClipEndpointSample {
                    kind: source_edge.kind,
                    source_index: source_edge.source_index,
                    edge_index: source_edge.edge_index,
                    y: source_edge.end.y,
                });
            }
        }
        samples
    }

    fn terrain_clip_missing_source_context_label(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> String {
        format!(
            "start_sources={} end_sources={}",
            Self::terrain_clip_endpoint_context_label(start, source_edges),
            Self::terrain_clip_endpoint_context_label(end, source_edges)
        )
    }

    fn terrain_clip_endpoint_context_label(
        point: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> String {
        let mut samples = Self::terrain_clip_endpoint_samples(point, source_edges)
            .into_iter()
            .map(|sample| {
                format!(
                    "{:?}:{}:{}@{:.3}",
                    sample.kind, sample.source_index, sample.edge_index, sample.y
                )
            })
            .collect::<Vec<_>>();
        samples.sort();
        samples.dedup();
        if samples.is_empty() {
            "none".to_string()
        } else {
            samples.into_iter().take(6).collect::<Vec<_>>().join("|")
        }
    }

    fn terrain_clip_source_interval_on_segment(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edge: TerrainClipSourceEdge,
    ) -> Option<TerrainClipSourceInterval> {
        let source_start = [
            f64::from(source_edge.start.x),
            f64::from(source_edge.start.z),
        ];
        let source_end = [f64::from(source_edge.end.x), f64::from(source_edge.end.z)];
        let source_start_t = Self::overlay_line_parameter(source_start, start, end)?;
        let source_end_t = Self::overlay_line_parameter(source_end, start, end)?;
        let mut overlap_start_t = source_start_t.min(source_end_t).max(0.0);
        let mut overlap_end_t = source_start_t.max(source_end_t).min(1.0);
        let endpoint_dust_t = Self::overlay_endpoint_dust_parameter(start, end)?;
        if overlap_start_t <= endpoint_dust_t {
            overlap_start_t = 0.0;
        }
        if overlap_end_t >= 1.0 - endpoint_dust_t {
            overlap_end_t = 1.0;
        }
        if overlap_end_t <= overlap_start_t {
            return None;
        }

        let overlap_start = interpolate_overlay_point(start, end, overlap_start_t);
        let overlap_end = interpolate_overlay_point(start, end, overlap_end_t);
        let edge_overlap_start_t =
            Self::overlay_segment_parameter(overlap_start, source_start, source_end)?;
        let edge_overlap_end_t =
            Self::overlay_segment_parameter(overlap_end, source_start, source_end)?;
        Some(TerrainClipSourceInterval {
            start_t: overlap_start_t,
            end_t: overlap_end_t,
            start_y: interpolate_height_f64(
                source_edge.start.y,
                source_edge.end.y,
                edge_overlap_start_t,
            ),
            end_y: interpolate_height_f64(
                source_edge.start.y,
                source_edge.end.y,
                edge_overlap_end_t,
            ),
        })
    }

    fn overlay_line_parameter(
        point: NodeOverlayPoint,
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
    ) -> Option<f64> {
        Self::overlay_segment_parameter_unbounded(point, start, end)
            .map(OverlaySegmentParameter::as_f64)
            .or_else(|| Self::overlay_numeric_dust_line_parameter(point, start, end))
    }

    fn overlay_segment_parameter(
        point: NodeOverlayPoint,
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
    ) -> Option<f64> {
        if let Some(parameter) = Self::overlay_segment_parameter_unbounded(point, start, end) {
            return Self::overlay_segment_parameter_with_endpoint_dust(
                parameter.as_f64(),
                start,
                end,
            );
        }
        let parameter = Self::overlay_numeric_dust_line_parameter(point, start, end)?;
        Self::overlay_segment_parameter_with_endpoint_dust(parameter, start, end)
    }

    fn overlay_segment_parameter_unbounded(
        point: NodeOverlayPoint,
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
    ) -> Option<OverlaySegmentParameter> {
        let start_key = Self::overlay_point_key(start);
        let end_key = Self::overlay_point_key(end);
        let point_key = Self::overlay_point_key(point);
        let dx = end_key.0 - start_key.0;
        let dz = end_key.1 - start_key.1;
        let px = point_key.0 - start_key.0;
        let pz = point_key.1 - start_key.1;
        let length_squared = i128::from(dx) * i128::from(dx) + i128::from(dz) * i128::from(dz);
        if length_squared == 0
            || i128::from(dx) * i128::from(pz) - i128::from(dz) * i128::from(px) != 0
        {
            return None;
        }
        OverlaySegmentParameter::new(
            i128::from(px) * i128::from(dx) + i128::from(pz) * i128::from(dz),
            length_squared,
        )
    }

    fn overlay_numeric_dust_line_parameter(
        point: NodeOverlayPoint,
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
    ) -> Option<f64> {
        // `i_overlay` may emit split vertices microscopically off the source edge's integer line.
        // The source edge still owns the height; this only recovers its interval parameter.
        let dx = end[0] - start[0];
        let dz = end[1] - start[1];
        let length_squared = dx * dx + dz * dz;
        if length_squared == 0.0 {
            return None;
        }
        let point_dx = point[0] - start[0];
        let point_dz = point[1] - start[1];
        let length = length_squared.sqrt();
        let cross = point_dx * dz - point_dz * dx;
        if cross.abs() > f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M) * length {
            return None;
        }
        Some((point_dx * dx + point_dz * dz) / length_squared)
    }

    fn overlay_segment_parameter_with_endpoint_dust(
        parameter: f64,
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
    ) -> Option<f64> {
        let endpoint_dust = Self::overlay_endpoint_dust_parameter(start, end)?;
        (parameter >= -endpoint_dust && parameter <= 1.0 + endpoint_dust)
            .then_some(parameter.clamp(0.0, 1.0))
    }

    fn overlay_endpoint_dust_parameter(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
    ) -> Option<f64> {
        let dx = end[0] - start[0];
        let dz = end[1] - start[1];
        let length = (dx * dx + dz * dz).sqrt();
        if length == 0.0 {
            return None;
        }
        Some(f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M) / length)
    }

    fn overlay_height_key(height_m: f32) -> i64 {
        (f64::from(height_m) * NODE_OVERLAY_SCALE).round() as i64
    }

    fn overlay_heights_equal(a: f32, b: f32) -> bool {
        Self::overlay_height_key(a) == Self::overlay_height_key(b)
    }
}

fn interpolate_height_f64(start_y: f32, end_y: f32, t: f64) -> f32 {
    (f64::from(start_y) + f64::from(end_y - start_y) * t) as f32
}

fn overlay_segment_length_m(start: NodeOverlayPoint, end: NodeOverlayPoint) -> f64 {
    let dx = end[0] - start[0];
    let dz = end[1] - start[1];
    (dx * dx + dz * dz).sqrt()
}

fn overlay_points_same_for_boundary(a: NodeOverlayPoint, b: NodeOverlayPoint) -> bool {
    RoadSurfaceSystem::overlay_point_key(a) == RoadSurfaceSystem::overlay_point_key(b)
}

fn remove_repeated_overlay_point_spurs(points: &mut NodeOverlayContour) {
    while points.len() >= 3 {
        let Some((first, second)) = first_repeated_overlay_point_pair(points) else {
            break;
        };
        let cycle = points[first..second].to_vec();
        let mut remainder = Vec::with_capacity(points.len() - (second - first));
        remainder.extend_from_slice(&points[..=first]);
        remainder.extend_from_slice(&points[second + 1..]);

        let cycle_area = overlay_contour_area_local(&cycle).abs();
        let remainder_area = overlay_contour_area_local(&remainder).abs();
        if remainder.len() >= 3 && remainder_area >= cycle_area {
            *points = remainder;
        } else if cycle.len() >= 3 {
            *points = cycle;
        } else {
            break;
        }
    }
}

fn first_repeated_overlay_point_pair(points: &NodeOverlayContour) -> Option<(usize, usize)> {
    for first in 0..points.len() {
        for second in first + 2..points.len() {
            if first == 0 && second + 1 == points.len() {
                continue;
            }
            if overlay_points_same_for_boundary(points[first], points[second]) {
                return Some((first, second));
            }
        }
    }
    None
}

fn overlay_contour_area_local(contour: &NodeOverlayContour) -> f64 {
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

fn contour_area_delta_after_removing_vertex(
    contour: &NodeOverlayContour,
    index: usize,
) -> Option<f64> {
    if contour.len() <= 3 || index >= contour.len() {
        return None;
    }
    let mut reduced = Vec::with_capacity(contour.len() - 1);
    reduced.extend_from_slice(&contour[..index]);
    reduced.extend_from_slice(&contour[index + 1..]);
    Some(overlay_contour_area_local(&reduced).abs())
}

fn interpolate_overlay_point(
    start: NodeOverlayPoint,
    end: NodeOverlayPoint,
    t: f64,
) -> NodeOverlayPoint {
    [
        start[0] + (end[0] - start[0]) * t,
        start[1] + (end[1] - start[1]) * t,
    ]
}

fn interval_height_at(interval: TerrainClipSourceInterval, t: f64) -> f32 {
    let span = interval.end_t - interval.start_t;
    if span == 0.0 {
        return interval.start_y;
    }
    interpolate_height_f64(
        interval.start_y,
        interval.end_y,
        (t - interval.start_t) / span,
    )
}

fn terrain_clip_edge_kind_priority(kind: RoadSurfaceTerrainClipEdgeKind) -> u8 {
    match kind {
        RoadSurfaceTerrainClipEdgeKind::SidewalkOuter => 0,
        RoadSurfaceTerrainClipEdgeKind::ShoulderOuter => 1,
        RoadSurfaceTerrainClipEdgeKind::FootprintBoundary => 2,
        RoadSurfaceTerrainClipEdgeKind::SpanHandoff => 3,
    }
}
