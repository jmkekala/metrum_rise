//! Height triangle construction and canonical polygon support checks.

use super::model::*;
use super::seams::*;
use super::source_edges::*;
use super::vertices::canonical_height_vertices;
use super::*;

impl NodeBandHeightTriangle {
    pub(super) fn height_at(&self, point_xz: RoadVec2) -> Option<f64> {
        let a = height_source_point_key(self.a_xz);
        let b = height_source_point_key(self.b_xz);
        let c = height_source_point_key(self.c_xz);
        let p = height_source_point_key(point_xz);
        let area = height_triangle_area2(a, b, c);
        if area == 0 {
            return None;
        }
        let abp = height_triangle_area2(a, b, p);
        let bcp = height_triangle_area2(b, c, p);
        let cap = height_triangle_area2(c, a, p);
        let has_negative = abp < 0 || bcp < 0 || cap < 0;
        let has_positive = abp > 0 || bcp > 0 || cap > 0;
        if has_negative && has_positive {
            return None;
        }
        let area_f = area as f64;
        let wa = bcp as f64 / area_f;
        let wb = cap as f64 / area_f;
        let wc = abp as f64 / area_f;
        Some(self.a_height_m * wa + self.b_height_m * wb + self.c_height_m * wc)
    }
}

pub(super) fn path_band_height_triangles(
    start_path_world: &[RoadVec3],
    end_path_world: &[RoadVec3],
) -> Option<Vec<NodeBandHeightTriangle>> {
    if start_path_world.len() != end_path_world.len() || start_path_world.len() < 2 {
        return None;
    }

    let mut triangles = Vec::with_capacity((start_path_world.len() - 1) * 2);
    for index in 0..start_path_world.len() - 1 {
        let start_current = start_path_world[index];
        let start_next = start_path_world[index + 1];
        let end_next = end_path_world[index + 1];
        let end_current = end_path_world[index];
        push_height_triangle(&mut triangles, start_current, start_next, end_next);
        push_height_triangle(&mut triangles, start_current, end_next, end_current);
    }
    (!triangles.is_empty()).then_some(triangles)
}

pub(super) fn path_band_height_edges(
    start_path_world: &[RoadVec3],
    end_path_world: &[RoadVec3],
) -> Result<Option<Vec<NodeBandHeightEdge>>, HeightCarrierContourError> {
    if start_path_world.len() != end_path_world.len() || start_path_world.len() < 2 {
        return Ok(None);
    }

    let mut contour = Vec::with_capacity(start_path_world.len() + end_path_world.len());
    contour.extend_from_slice(start_path_world);
    contour.extend(end_path_world.iter().rev().copied());
    let edges = height_edges_from_vertices(&contour)?;
    Ok((!edges.is_empty()).then_some(edges))
}

pub(super) fn terminal_cap_band_height_triangles(
    id: NodeBandHeightFieldId,
    source_kind: RoadSurfaceBandKind,
    authority: NodeHeightPatchAuthority,
    cap_band: &NodeTerminalCapBand,
) -> Result<Vec<NodeBandHeightTriangle>, NodeHeightFieldError> {
    if let Some(triangles) = terminal_material_band_height_triangles(&cap_band.contour_world) {
        return Ok(triangles);
    }

    height_triangles_from_contour(id, source_kind, authority, &cap_band.contour_world)
}

pub(super) fn terminal_material_band_height_triangles(
    points: &[RoadVec3],
) -> Option<Vec<NodeBandHeightTriangle>> {
    if points.len() < 4 || points.len() % 2 != 0 {
        return None;
    }

    let rail_point_count = points.len() / 2;
    let mut triangles = Vec::with_capacity((rail_point_count - 1) * 2);
    for index in 0..rail_point_count - 1 {
        let inner_start = points[index];
        let inner_end = points[index + 1];
        let outer_end = points[points.len() - 2 - index];
        let outer_start = points[points.len() - 1 - index];
        push_height_triangle(&mut triangles, inner_start, inner_end, outer_end);
        push_height_triangle(&mut triangles, inner_start, outer_end, outer_start);
    }

    (!triangles.is_empty()).then_some(triangles)
}

pub(super) fn push_height_triangle(
    triangles: &mut Vec<NodeBandHeightTriangle>,
    a_world: RoadVec3,
    b_world: RoadVec3,
    c_world: RoadVec3,
) {
    let a_xz = quantize_road_vec2_to_overlay_grid(xz(a_world));
    let b_xz = quantize_road_vec2_to_overlay_grid(xz(b_world));
    let c_xz = quantize_road_vec2_to_overlay_grid(xz(c_world));
    if height_triangle_area2(
        height_source_point_key(a_xz),
        height_source_point_key(b_xz),
        height_source_point_key(c_xz),
    ) == 0
    {
        return;
    }
    triangles.push(NodeBandHeightTriangle {
        a_xz,
        b_xz,
        c_xz,
        a_height_m: quantize_source_height_m(a_world.y),
        b_height_m: quantize_source_height_m(b_world.y),
        c_height_m: quantize_source_height_m(c_world.y),
    });
}

pub(super) fn terminal_cap_band_height_edges(
    cap_band: &NodeTerminalCapBand,
) -> Result<Vec<NodeBandHeightEdge>, HeightCarrierContourError> {
    height_edges_from_vertices(&cap_band.contour_world)
}

pub(super) fn height_triangles_from_vertices(
    points: &[RoadVec3],
) -> Result<Vec<NodeBandHeightTriangle>, HeightCarrierContourError> {
    let vertices = canonical_height_vertices(points)?;
    Ok(fan_height_triangles_from_vertices(&vertices))
}

pub(super) fn height_triangles_from_contour(
    id: NodeBandHeightFieldId,
    source_kind: RoadSurfaceBandKind,
    authority: NodeHeightPatchAuthority,
    points: &[RoadVec3],
) -> Result<Vec<NodeBandHeightTriangle>, NodeHeightFieldError> {
    constrained_height_triangles_from_vertices(points).map_err(|error| {
        NodeHeightFieldError::InvalidHeightCarrierContour {
            mouth_order_index: id.mouth_order_index(),
            band_index: id.band_index(),
            source_kind,
            height_field_id: id,
            authority: authority.source(),
            reason: error.diagnostic_reason(),
        }
    })
}

pub(super) fn constrained_height_triangles_from_vertices(
    points: &[RoadVec3],
) -> Result<Vec<NodeBandHeightTriangle>, HeightCarrierContourError> {
    let vertices = canonical_height_vertices(points)?;
    if vertices.len() < 3 {
        return Err(HeightCarrierContourError::TooFewVertices);
    }
    if vertices.len() == 3 {
        let triangles = fan_height_triangles_from_vertices(&vertices);
        return (!triangles.is_empty())
            .then_some(triangles)
            .ok_or(HeightCarrierContourError::DegenerateContour);
    }

    let spade_vertices = vertices
        .iter()
        .map(|(point_xz, _)| Point2::new(point_xz.x, point_xz.y))
        .collect::<Vec<_>>();
    let constraints = (0..vertices.len())
        .map(|index| [index, (index + 1) % vertices.len()])
        .collect::<Vec<_>>();
    let mut invalid_constraints = 0usize;
    let cdt = SurfaceCdt::try_bulk_load_cdt(spade_vertices, constraints, |_| {
        invalid_constraints += 1;
    })
    .map_err(|_| HeightCarrierContourError::CdtBuildFailed)?;
    if invalid_constraints > 0 {
        return Err(HeightCarrierContourError::InvalidConstraint);
    }

    let mut triangles = Vec::new();
    for face in cdt.inner_faces() {
        let [a, b, c] = face.vertices();
        let indices = [a.fix().index(), b.fix().index(), c.fix().index()];
        let centroid =
            (vertices[indices[0]].0 + vertices[indices[1]].0 + vertices[indices[2]].0) / 3.0;
        if !height_polygon_contains_point_xz(&vertices, centroid) {
            continue;
        }
        push_height_triangle_from_vertices(
            &mut triangles,
            vertices[indices[0]],
            vertices[indices[1]],
            vertices[indices[2]],
        );
    }
    triangles.sort_by(|a, b| height_triangle_sort_key(a).cmp(&height_triangle_sort_key(b)));
    triangles.dedup_by_key(|triangle| height_triangle_sort_key(triangle));
    (!triangles.is_empty())
        .then_some(triangles)
        .ok_or(HeightCarrierContourError::EmptyInteriorTriangulation)
}

pub(super) fn fan_height_triangles_from_vertices(
    vertices: &[(RoadVec2, f64)],
) -> Vec<NodeBandHeightTriangle> {
    let mut triangles = Vec::new();
    if vertices.len() < 3 {
        return triangles;
    }
    let (a_xz, a_height_m) = vertices[0];
    for index in 1..vertices.len() - 1 {
        let (b_xz, b_height_m) = vertices[index];
        let (c_xz, c_height_m) = vertices[index + 1];
        if height_triangle_area2(
            height_source_point_key(a_xz),
            height_source_point_key(b_xz),
            height_source_point_key(c_xz),
        ) == 0
        {
            continue;
        }
        triangles.push(NodeBandHeightTriangle {
            a_xz,
            b_xz,
            c_xz,
            a_height_m,
            b_height_m,
            c_height_m,
        });
    }
    triangles
}

pub(super) fn push_height_triangle_from_vertices(
    triangles: &mut Vec<NodeBandHeightTriangle>,
    a: (RoadVec2, f64),
    b: (RoadVec2, f64),
    c: (RoadVec2, f64),
) {
    if height_triangle_area2(
        height_source_point_key(a.0),
        height_source_point_key(b.0),
        height_source_point_key(c.0),
    ) == 0
    {
        return;
    }
    triangles.push(NodeBandHeightTriangle {
        a_xz: a.0,
        b_xz: b.0,
        c_xz: c.0,
        a_height_m: a.1,
        b_height_m: b.1,
        c_height_m: c.1,
    });
}

pub(super) fn height_triangle_sort_key(
    triangle: &NodeBandHeightTriangle,
) -> [NodeHeightSourcePointKey; 3] {
    let mut keys = [
        height_source_point_key(triangle.a_xz),
        height_source_point_key(triangle.b_xz),
        height_source_point_key(triangle.c_xz),
    ];
    keys.sort();
    keys
}

pub(super) fn height_polygon_contains_point_xz(
    vertices: &[(RoadVec2, f64)],
    point: RoadVec2,
) -> bool {
    if vertices.len() < 3 {
        return false;
    }
    let point_key = height_source_point_key(point);
    for index in 0..vertices.len() {
        let start = vertices[index].0;
        let end = vertices[(index + 1) % vertices.len()].0;
        if raw_tuple_key_lies_exactly_on_segment(
            point_key,
            height_source_point_key(start),
            height_source_point_key(end),
        ) {
            return true;
        }
    }

    let mut inside = false;
    for index in 0..vertices.len() {
        let start = vertices[index].0;
        let end = vertices[(index + 1) % vertices.len()].0;
        if (start.y > point.y) != (end.y > point.y) {
            let edge_x_at_point_z =
                (end.x - start.x) * (point.y - start.y) / (end.y - start.y) + start.x;
            if point.x < edge_x_at_point_z {
                inside = !inside;
            }
        }
    }
    inside
}

pub(super) fn height_triangle_area2(
    a: NodeHeightSourcePointKey,
    b: NodeHeightSourcePointKey,
    c: NodeHeightSourcePointKey,
) -> i128 {
    SurfaceXzKey::raw_tuple_triangle_area2(a, b, c)
}
