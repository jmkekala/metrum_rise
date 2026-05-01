//! Deterministic overlay boolean geometry helpers for road surfaces.

use super::{
    NODE_OVERLAY_MIN_AREA_M2, NodeOverlayContour, NodeOverlayPoint, NodeOverlayPointKey,
    NodeOverlayShape, NodeOverlayShapes, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceVisualPolygon, SAMPLE_EPSILON_M, SurfaceCdt, WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2,
};
use godot::prelude::{Vector2, Vector3};
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::scale::FixedScaleFloatOverlay;
use i_overlay::float::simplify::SimplifyShape;
use spade::{Point2, Triangulation};
use std::collections::{BTreeMap, BTreeSet};

use super::backend::ROAD_OVERLAY_COORDINATE_SCALE;

// Overlay boolean operations quantize coordinates to millimetres for deterministic keys.
const NODE_OVERLAY_SCALE: f32 = ROAD_OVERLAY_COORDINATE_SCALE as f32;
const NODE_SURFACE_HEIGHT_EPSILON_M: f32 = 1.0 / NODE_OVERLAY_SCALE;

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
        signed_area * 0.5
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
        Self::outer_boundary_polygons_from_overlay_shapes_with_candidate_heights(&shapes, polygons)
    }

    fn outer_boundary_polygons_from_overlay_shapes_with_candidate_heights(
        shapes: &[NodeOverlayShape],
        candidates: &[RoadSurfaceVisualPolygon],
    ) -> Vec<RoadSurfaceVisualPolygon> {
        let mut polygons = Vec::new();
        for shape in shapes {
            let Some(polygon) = Self::visual_polygon_from_overlay_shape_with_candidate_heights(
                shape, candidates, false,
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

    fn visual_polygon_from_overlay_shape_with_candidate_heights(
        shape: &NodeOverlayShape,
        candidates: &[RoadSurfaceVisualPolygon],
        preserve_holes: bool,
    ) -> Option<RoadSurfaceVisualPolygon> {
        let outer_contour = shape.first()?;
        let mut outer_points = Self::world_points_from_overlay_contour_with_candidate_heights(
            outer_contour,
            candidates,
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
                let mut points = Self::world_points_from_overlay_contour_with_candidate_heights(
                    contour, candidates,
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

    fn world_points_from_overlay_contour_with_candidate_heights(
        contour: &NodeOverlayContour,
        candidates: &[RoadSurfaceVisualPolygon],
    ) -> Option<Vec<Vector3>> {
        contour
            .iter()
            .map(|point| {
                let xz = Vector2::new(point[0], point[1]);
                let y = Self::sample_height_from_candidate_coverage(xz, candidates)?;
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

    fn sample_height_from_candidate_coverage(
        point_xz: Vector2,
        candidates: &[RoadSurfaceVisualPolygon],
    ) -> Option<f32> {
        let point_key = Self::overlay_point_key([point_xz.x, point_xz.y]);
        let mut vertex_heights = Vec::new();
        for polygon in candidates {
            for point in &polygon.points_world {
                if Self::overlay_point_key([point.x, point.z]) == point_key {
                    vertex_heights.push(point.y);
                }
            }
            for triangle in &polygon.triangles_world {
                for point in triangle {
                    if Self::overlay_point_key([point.x, point.z]) == point_key {
                        vertex_heights.push(point.y);
                    }
                }
            }
        }
        if !vertex_heights.is_empty() {
            return Self::canonical_height_sample(vertex_heights);
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
        Self::canonical_height_sample(covered_heights)
    }

    fn canonical_height_sample<I>(heights: I) -> Option<f32>
    where
        I: IntoIterator<Item = f32>,
    {
        let mut heights = heights.into_iter().collect::<Vec<_>>();
        if heights.is_empty() {
            return None;
        }
        heights.sort_by(|a, b| a.total_cmp(b));
        let min_height = *heights.first()?;
        let max_height = *heights.last()?;
        (max_height - min_height <= NODE_SURFACE_HEIGHT_EPSILON_M).then_some(min_height)
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

        Self::triangulate_surface_cdt_vertices(vertices, constraints, outer_points, holes)
    }

    fn triangulate_surface_cdt_vertices(
        vertices: Vec<Vector3>,
        constraints: BTreeSet<[usize; 2]>,
        outer_points: &[Vector3],
        holes: &[Vec<Vector3>],
    ) -> Option<Vec<[Vector3; 3]>> {
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
