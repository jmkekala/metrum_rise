//! Spade-backed triangulation for canonical node-owned height regions.

#![allow(dead_code)]

use super::arrangement::{NodeBandOwner, NodeHeightSource};
use super::backend::{RoadVec2, RoadVec3};
use super::height::{NodeHeightSolution, NodeHeightedRegion, NodeHeightedVertex};
use super::{
    NODE_OVERLAY_MIN_AREA_M2, NodeOverlayContour, NodeOverlayPoint, NodeOverlayShape,
    NodeOverlayShapes, RoadSurfaceBandKind, RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind,
    SurfaceCdt,
};
use i_overlay::core::overlay_rule::OverlayRule;
use spade::{Point2, Triangulation};
use std::collections::{BTreeMap, BTreeSet};

const NODE_TRIANGULATION_KEY_SCALE: f64 = 1000.0;
const NODE_CONTOUR_INTERSECTION_EPS_M: f64 = 1.0e-9;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeTriangulationSolution {
    pub(crate) node_id: u32,
    pub(crate) piece_kind: RoadSurfaceVisualNodePieceKind,
    pub(crate) regions: Vec<NodeTriangulatedRegion>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeTriangulatedRegion {
    pub(crate) kind: RoadSurfaceBandKind,
    pub(crate) owner: NodeBandOwner,
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: usize,
    pub(crate) vertices: Vec<NodeTriangulatedVertex>,
    pub(crate) boundary_constraints: Vec<[usize; 2]>,
    pub(crate) triangles: Vec<NodeTriangulatedTriangle>,
    pub(crate) area_m2: f32,
    pub(crate) height_sources: Vec<NodeHeightSource>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeTriangulatedVertex {
    pub(crate) point_world: RoadVec3,
    pub(crate) height_sources: Vec<NodeHeightSource>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct NodeTriangulatedTriangle {
    pub(crate) vertices: [usize; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeTriangulationError {
    EmptyHeightSolution {
        node_id: u32,
    },
    EmptyRegionShape {
        node_id: u32,
        region_index: usize,
    },
    DegenerateRegionContour {
        node_id: u32,
        region_index: usize,
        contour_index: usize,
        vertex_count: usize,
    },
    DuplicateVertexHeightConflict {
        node_id: u32,
        region_index: usize,
        x_mm: i64,
        z_mm: i64,
        existing_height_mm: i64,
        incoming_height_mm: i64,
    },
    InvalidConstraint {
        node_id: u32,
        region_index: usize,
        constraint_count: usize,
    },
    CdtBuildFailed {
        node_id: u32,
        region_index: usize,
    },
    EmptyTriangulation {
        node_id: u32,
        region_index: usize,
    },
    BooleanOperationFailed {
        node_id: u32,
        region_index: usize,
        stage: &'static str,
    },
    TriangleCoverageMismatch {
        node_id: u32,
        region_index: usize,
        missing_area_m2: f32,
        extra_area_m2: f32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeTriangulationPointKey {
    x_mm: i64,
    z_mm: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NodeTriangulationHeightKey(i64);

impl RoadSurfaceSystem {
    pub(super) fn split_self_touching_node_height_solution(
        mut heights: NodeHeightSolution,
    ) -> NodeHeightSolution {
        // Sharp node footprints can self-touch after ownership clipping. CDT needs simple loops.
        heights.regions = heights
            .regions
            .iter()
            .flat_map(split_self_touching_heighted_region)
            .collect();
        heights
    }

    pub(super) fn build_node_triangulation_from_height_solution(
        heights: &NodeHeightSolution,
    ) -> Result<NodeTriangulationSolution, NodeTriangulationError> {
        NodeTriangulationSolution::from_height_solution(heights)
    }
}

impl NodeTriangulationSolution {
    pub(crate) fn from_height_solution(
        heights: &NodeHeightSolution,
    ) -> Result<Self, NodeTriangulationError> {
        if heights.regions.is_empty() {
            return Err(NodeTriangulationError::EmptyHeightSolution {
                node_id: heights.node_id,
            });
        }

        let mut regions = Vec::with_capacity(heights.regions.len());
        for (region_index, region) in heights.regions.iter().enumerate() {
            regions.push(triangulate_region(heights.node_id, region_index, region)?);
        }

        Ok(Self {
            node_id: heights.node_id,
            piece_kind: heights.piece_kind,
            regions,
        })
    }
}

fn split_self_touching_heighted_region(region: &NodeHeightedRegion) -> Vec<NodeHeightedRegion> {
    let split_shapes = split_self_touching_heighted_shape(&region.shape);
    if split_shapes.is_empty() {
        return vec![region.clone()];
    }

    split_shapes
        .into_iter()
        .filter_map(|shape| {
            let area_m2 = heighted_shape_area_m2(&shape);
            (area_m2 > NODE_OVERLAY_MIN_AREA_M2).then(|| NodeHeightedRegion {
                kind: region.kind,
                owner: region.owner,
                source_mouth_order_index: region.source_mouth_order_index,
                source_band_index: region.source_band_index,
                shape,
                area_m2,
                height_sources: region.height_sources.clone(),
            })
        })
        .collect()
}

fn split_self_touching_heighted_shape(
    shape: &[Vec<NodeHeightedVertex>],
) -> Vec<Vec<Vec<NodeHeightedVertex>>> {
    let Some(outer) = shape.first() else {
        return Vec::new();
    };
    let outer_contours = split_self_touching_heighted_contour(outer.clone())
        .into_iter()
        .map(|contour| oriented_heighted_contour(contour, true))
        .collect::<Vec<_>>();
    if outer_contours.is_empty() {
        return Vec::new();
    }

    let hole_contours = shape
        .iter()
        .skip(1)
        .flat_map(|contour| split_self_touching_heighted_contour(contour.clone()))
        .map(|contour| oriented_heighted_contour(contour, false))
        .collect::<Vec<_>>();

    let mut shapes = Vec::new();
    for outer in outer_contours {
        let mut split_shape = vec![outer.clone()];
        split_shape.extend(
            hole_contours
                .iter()
                .filter(|hole| heighted_contour_inside_contour(hole, &outer))
                .cloned(),
        );
        if heighted_shape_area_m2(&split_shape) > NODE_OVERLAY_MIN_AREA_M2 {
            shapes.push(split_shape);
        }
    }
    shapes
}

fn split_self_touching_heighted_contour(
    contour: Vec<NodeHeightedVertex>,
) -> Vec<Vec<NodeHeightedVertex>> {
    let contour = dedup_consecutive_heighted_vertices(contour);
    if contour.len() < 3
        || signed_heighted_contour_area_m2(&contour).abs() <= NODE_OVERLAY_MIN_AREA_M2
    {
        return Vec::new();
    }

    if let Some((first, second)) = first_repeated_heighted_vertex_pair(&contour) {
        let first_loop = contour[first..second].to_vec();
        let mut second_loop = contour[second..].to_vec();
        second_loop.extend_from_slice(&contour[..first]);

        let mut loops = Vec::new();
        for candidate in [first_loop, second_loop] {
            loops.extend(split_self_touching_heighted_contour(candidate));
        }
        return loops;
    }

    if let Some(intersection) = first_strict_heighted_contour_intersection(&contour) {
        let mut noded = Vec::with_capacity(contour.len() + 2);
        for index in 0..contour.len() {
            noded.push(contour[index].clone());
            if index == intersection.first_edge_index || index == intersection.second_edge_index {
                noded.push(intersection.vertex.clone());
            }
        }
        return split_self_touching_heighted_contour(noded);
    }

    vec![contour]
}

fn dedup_consecutive_heighted_vertices(
    contour: Vec<NodeHeightedVertex>,
) -> Vec<NodeHeightedVertex> {
    let mut deduped = Vec::with_capacity(contour.len());
    for vertex in contour {
        if deduped
            .last()
            .is_some_and(|last| same_heighted_vertex_xz(last, &vertex))
        {
            continue;
        }
        deduped.push(vertex);
    }
    if deduped.len() >= 2
        && same_heighted_vertex_xz(deduped.first().unwrap(), deduped.last().unwrap())
    {
        deduped.pop();
    }
    deduped
}

fn first_repeated_heighted_vertex_pair(contour: &[NodeHeightedVertex]) -> Option<(usize, usize)> {
    for first in 0..contour.len() {
        for second in first + 1..contour.len() {
            if same_heighted_vertex_xz(&contour[first], &contour[second]) {
                return Some((first, second));
            }
        }
    }
    None
}

struct HeightedContourIntersection {
    first_edge_index: usize,
    second_edge_index: usize,
    vertex: NodeHeightedVertex,
}

fn first_strict_heighted_contour_intersection(
    contour: &[NodeHeightedVertex],
) -> Option<HeightedContourIntersection> {
    for first_edge_index in 0..contour.len() {
        let first_next = (first_edge_index + 1) % contour.len();
        for second_edge_index in first_edge_index + 1..contour.len() {
            let second_next = (second_edge_index + 1) % contour.len();
            if first_edge_index == second_next || first_next == second_edge_index {
                continue;
            }
            let Some((first_t, second_t, point_xz)) = strict_segment_intersection_xz(
                contour[first_edge_index].point_xz,
                contour[first_next].point_xz,
                contour[second_edge_index].point_xz,
                contour[second_next].point_xz,
            ) else {
                continue;
            };
            let first_height =
                interpolate_height(&contour[first_edge_index], &contour[first_next], first_t);
            let second_height =
                interpolate_height(&contour[second_edge_index], &contour[second_next], second_t);
            let mut height_sources = contour[first_edge_index].height_sources.clone();
            height_sources.extend(contour[first_next].height_sources.iter().cloned());
            height_sources.extend(contour[second_edge_index].height_sources.iter().cloned());
            height_sources.extend(contour[second_next].height_sources.iter().cloned());
            height_sources.sort();
            height_sources.dedup();
            return Some(HeightedContourIntersection {
                first_edge_index,
                second_edge_index,
                vertex: NodeHeightedVertex {
                    point_xz,
                    height_m: (first_height + second_height) * 0.5,
                    height_sources,
                },
            });
        }
    }
    None
}

fn strict_segment_intersection_xz(
    a: RoadVec2,
    b: RoadVec2,
    c: RoadVec2,
    d: RoadVec2,
) -> Option<(f64, f64, RoadVec2)> {
    let ab = b - a;
    let cd = d - c;
    let denominator = cross_xz(ab, cd);
    if denominator.abs() <= NODE_CONTOUR_INTERSECTION_EPS_M {
        return None;
    }

    let ac = c - a;
    let first_t = cross_xz(ac, cd) / denominator;
    let second_t = cross_xz(ac, ab) / denominator;
    if !(NODE_CONTOUR_INTERSECTION_EPS_M..=1.0 - NODE_CONTOUR_INTERSECTION_EPS_M).contains(&first_t)
        || !(NODE_CONTOUR_INTERSECTION_EPS_M..=1.0 - NODE_CONTOUR_INTERSECTION_EPS_M)
            .contains(&second_t)
    {
        return None;
    }

    Some((first_t, second_t, a + ab * first_t))
}

fn cross_xz(a: RoadVec2, b: RoadVec2) -> f64 {
    a.x * b.y - a.y * b.x
}

fn interpolate_height(start: &NodeHeightedVertex, end: &NodeHeightedVertex, t: f64) -> f64 {
    start.height_m + (end.height_m - start.height_m) * t
}

fn same_heighted_vertex_xz(a: &NodeHeightedVertex, b: &NodeHeightedVertex) -> bool {
    NodeTriangulationPointKey::from_vertex(a) == NodeTriangulationPointKey::from_vertex(b)
}

fn oriented_heighted_contour(
    mut contour: Vec<NodeHeightedVertex>,
    positive_area: bool,
) -> Vec<NodeHeightedVertex> {
    let is_positive = signed_heighted_contour_area_m2(&contour) >= 0.0;
    if is_positive != positive_area {
        contour.reverse();
    }
    contour
}

fn heighted_shape_area_m2(shape: &[Vec<NodeHeightedVertex>]) -> f32 {
    let Some(outer) = shape.first() else {
        return 0.0;
    };
    let holes = shape
        .iter()
        .skip(1)
        .map(|hole| signed_heighted_contour_area_m2(hole).abs())
        .sum::<f32>();
    (signed_heighted_contour_area_m2(outer).abs() - holes).max(0.0)
}

fn signed_heighted_contour_area_m2(contour: &[NodeHeightedVertex]) -> f32 {
    if contour.len() < 3 {
        return 0.0;
    }
    let mut area = 0.0;
    for index in 0..contour.len() {
        let current = contour[index].point_xz;
        let next = contour[(index + 1) % contour.len()].point_xz;
        area += current.x * next.y - next.x * current.y;
    }
    (area * 0.5) as f32
}

fn heighted_contour_inside_contour(
    inner: &[NodeHeightedVertex],
    outer: &[NodeHeightedVertex],
) -> bool {
    let Some(point) =
        heighted_contour_centroid(inner).or_else(|| inner.first().map(|v| v.point_xz))
    else {
        return false;
    };
    heighted_contour_contains_point(outer, point)
}

fn heighted_contour_centroid(contour: &[NodeHeightedVertex]) -> Option<RoadVec2> {
    if contour.is_empty() {
        return None;
    }
    let mut point = RoadVec2::ZERO;
    for vertex in contour {
        point += vertex.point_xz;
    }
    Some(point / contour.len() as f64)
}

fn heighted_contour_contains_point(contour: &[NodeHeightedVertex], point: RoadVec2) -> bool {
    if contour.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut previous = contour.len() - 1;
    for current in 0..contour.len() {
        let a = contour[current].point_xz;
        let b = contour[previous].point_xz;
        if (a.y > point.y) != (b.y > point.y) {
            let intersection_x = (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x;
            if point.x < intersection_x {
                inside = !inside;
            }
        }
        previous = current;
    }
    inside
}

fn triangulate_region(
    node_id: u32,
    region_index: usize,
    region: &NodeHeightedRegion,
) -> Result<NodeTriangulatedRegion, NodeTriangulationError> {
    if region.shape.is_empty() {
        return Err(NodeTriangulationError::EmptyRegionShape {
            node_id,
            region_index,
        });
    }

    let mut vertices = Vec::new();
    let mut vertex_lookup = BTreeMap::new();
    let mut constraints = BTreeSet::new();
    for (contour_index, contour) in region.shape.iter().enumerate() {
        push_region_constraint_loop(
            node_id,
            region_index,
            contour_index,
            contour,
            &mut vertices,
            &mut vertex_lookup,
            &mut constraints,
        )?;
    }

    let spade_vertices = vertices
        .iter()
        .map(|vertex| Point2::new(vertex.point_world.x, vertex.point_world.z))
        .collect::<Vec<_>>();
    let mut invalid_constraints = 0usize;
    let cdt = SurfaceCdt::try_bulk_load_cdt(
        spade_vertices,
        constraints.iter().copied().collect(),
        |_| invalid_constraints += 1,
    )
    .map_err(|_| NodeTriangulationError::CdtBuildFailed {
        node_id,
        region_index,
    })?;
    if invalid_constraints > 0 {
        return Err(NodeTriangulationError::InvalidConstraint {
            node_id,
            region_index,
            constraint_count: invalid_constraints,
        });
    }

    let owner_shape = overlay_shape_from_heighted_region(region);
    let mut triangles = Vec::new();
    for face in cdt.inner_faces() {
        let [a, b, c] = face.vertices();
        let triangle = NodeTriangulatedTriangle {
            vertices: [a.fix().index(), b.fix().index(), c.fix().index()],
        };
        if triangle_double_area_m2(&triangle, &vertices) <= f64::from(NODE_OVERLAY_MIN_AREA_M2) {
            continue;
        }
        if triangle_is_inside_owner(node_id, region_index, &triangle, &vertices, &owner_shape)? {
            triangles.push(triangle);
        }
    }
    triangles.sort_by(|a, b| triangle_sort_key(a, &vertices).cmp(&triangle_sort_key(b, &vertices)));
    triangles.dedup();
    if triangles.is_empty() {
        return Err(NodeTriangulationError::EmptyTriangulation {
            node_id,
            region_index,
        });
    }

    reject_triangle_coverage_mismatch(node_id, region_index, &owner_shape, &triangles, &vertices)?;

    Ok(NodeTriangulatedRegion {
        kind: region.kind,
        owner: region.owner,
        source_mouth_order_index: region.source_mouth_order_index,
        source_band_index: region.source_band_index,
        vertices,
        boundary_constraints: constraints.into_iter().collect(),
        triangles,
        area_m2: region.area_m2,
        height_sources: region.height_sources.clone(),
    })
}

fn push_region_constraint_loop(
    node_id: u32,
    region_index: usize,
    contour_index: usize,
    contour: &[NodeHeightedVertex],
    vertices: &mut Vec<NodeTriangulatedVertex>,
    vertex_lookup: &mut BTreeMap<NodeTriangulationPointKey, (usize, NodeTriangulationHeightKey)>,
    constraints: &mut BTreeSet<[usize; 2]>,
) -> Result<(), NodeTriangulationError> {
    if contour.len() < 3 {
        return Err(NodeTriangulationError::DegenerateRegionContour {
            node_id,
            region_index,
            contour_index,
            vertex_count: contour.len(),
        });
    }

    let indices = contour
        .iter()
        .map(|vertex| insert_region_vertex(node_id, region_index, vertex, vertices, vertex_lookup))
        .collect::<Result<Vec<_>, _>>()?;
    for index in 0..indices.len() {
        let constraint =
            normalized_constraint(indices[index], indices[(index + 1) % indices.len()]);
        if constraint[0] != constraint[1] {
            constraints.insert(constraint);
        }
    }
    Ok(())
}

fn insert_region_vertex(
    node_id: u32,
    region_index: usize,
    vertex: &NodeHeightedVertex,
    vertices: &mut Vec<NodeTriangulatedVertex>,
    vertex_lookup: &mut BTreeMap<NodeTriangulationPointKey, (usize, NodeTriangulationHeightKey)>,
) -> Result<usize, NodeTriangulationError> {
    let point_key = NodeTriangulationPointKey::from_vertex(vertex);
    let height_key = NodeTriangulationHeightKey::from_vertex(vertex);
    if let Some((index, existing_height_key)) = vertex_lookup.get(&point_key).copied() {
        if existing_height_key != height_key {
            return Err(NodeTriangulationError::DuplicateVertexHeightConflict {
                node_id,
                region_index,
                x_mm: point_key.x_mm,
                z_mm: point_key.z_mm,
                existing_height_mm: existing_height_key.0,
                incoming_height_mm: height_key.0,
            });
        }
        return Ok(index);
    }

    let index = vertices.len();
    vertices.push(NodeTriangulatedVertex {
        point_world: RoadVec3::new(vertex.point_xz.x, vertex.height_m, vertex.point_xz.y),
        height_sources: vertex.height_sources.clone(),
    });
    vertex_lookup.insert(point_key, (index, height_key));
    Ok(index)
}

fn triangle_is_inside_owner(
    node_id: u32,
    region_index: usize,
    triangle: &NodeTriangulatedTriangle,
    vertices: &[NodeTriangulatedVertex],
    owner_shape: &NodeOverlayShape,
) -> Result<bool, NodeTriangulationError> {
    let triangle_shape = vec![positive_triangle_contour(triangle, vertices)];
    let area_budget_m2 = RoadSurfaceSystem::overlay_numeric_area_budget_for_shape(&triangle_shape);
    let triangle_shapes = vec![triangle_shape];
    let owner_shapes = vec![owner_shape.clone()];
    let residual = overlay_difference(
        node_id,
        region_index,
        &triangle_shapes,
        &owner_shapes,
        "triangle_minus_owner",
    )?;
    Ok(overlay_area_m2(&residual) <= area_budget_m2)
}

fn reject_triangle_coverage_mismatch(
    node_id: u32,
    region_index: usize,
    owner_shape: &NodeOverlayShape,
    triangles: &[NodeTriangulatedTriangle],
    vertices: &[NodeTriangulatedVertex],
) -> Result<(), NodeTriangulationError> {
    let owner_shapes = vec![owner_shape.clone()];
    let triangle_contours = triangles
        .iter()
        .map(|triangle| positive_triangle_contour(triangle, vertices))
        .collect::<Vec<_>>();
    let triangle_shapes =
        overlay_union(node_id, region_index, &triangle_contours, "triangle_union")?;
    let missing = overlay_difference(
        node_id,
        region_index,
        &owner_shapes,
        &triangle_shapes,
        "owner_minus_triangles",
    )?;
    let extra = overlay_difference(
        node_id,
        region_index,
        &triangle_shapes,
        &owner_shapes,
        "triangles_minus_owner",
    )?;
    let missing_area_m2 = overlay_area_m2(&missing);
    let extra_area_m2 = overlay_area_m2(&extra);
    let area_budget_m2 = RoadSurfaceSystem::overlay_numeric_area_budget_for_shape(owner_shape);
    if missing_area_m2 > area_budget_m2 || extra_area_m2 > area_budget_m2 {
        return Err(NodeTriangulationError::TriangleCoverageMismatch {
            node_id,
            region_index,
            missing_area_m2,
            extra_area_m2,
        });
    }
    Ok(())
}

fn overlay_union(
    node_id: u32,
    region_index: usize,
    contours: &[NodeOverlayContour],
    stage: &'static str,
) -> Result<NodeOverlayShapes, NodeTriangulationError> {
    let mut shapes = RoadSurfaceSystem::overlay_union_contours(contours).ok_or(
        NodeTriangulationError::BooleanOperationFailed {
            node_id,
            region_index,
            stage,
        },
    )?;
    RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);
    Ok(shapes)
}

fn overlay_difference(
    node_id: u32,
    region_index: usize,
    subject: &NodeOverlayShapes,
    clip: &NodeOverlayShapes,
    stage: &'static str,
) -> Result<NodeOverlayShapes, NodeTriangulationError> {
    let mut shapes =
        RoadSurfaceSystem::overlay_binary_shapes(subject, clip, OverlayRule::Difference).ok_or(
            NodeTriangulationError::BooleanOperationFailed {
                node_id,
                region_index,
                stage,
            },
        )?;
    RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);
    Ok(shapes)
}

fn overlay_shape_from_heighted_region(region: &NodeHeightedRegion) -> NodeOverlayShape {
    region
        .shape
        .iter()
        .map(|contour| {
            contour
                .iter()
                .map(|vertex| [vertex.point_xz.x, vertex.point_xz.y])
                .collect::<Vec<_>>()
        })
        .collect()
}

fn positive_triangle_contour(
    triangle: &NodeTriangulatedTriangle,
    vertices: &[NodeTriangulatedVertex],
) -> NodeOverlayContour {
    let mut contour = triangle
        .vertices
        .iter()
        .map(|index| overlay_point_from_vertex(&vertices[*index]))
        .collect::<Vec<_>>();
    if signed_overlay_area_m2(&contour) < 0.0 {
        contour.swap(1, 2);
    }
    contour
}

fn overlay_point_from_vertex(vertex: &NodeTriangulatedVertex) -> NodeOverlayPoint {
    [vertex.point_world.x, vertex.point_world.z]
}

fn overlay_area_m2(shapes: &NodeOverlayShapes) -> f32 {
    shapes
        .iter()
        .map(RoadSurfaceSystem::overlay_shape_area_m2)
        .sum()
}

fn triangle_double_area_m2(
    triangle: &NodeTriangulatedTriangle,
    vertices: &[NodeTriangulatedVertex],
) -> f64 {
    let a = vertices[triangle.vertices[0]].point_world;
    let b = vertices[triangle.vertices[1]].point_world;
    let c = vertices[triangle.vertices[2]].point_world;
    ((b.x - a.x) * (c.z - a.z) - (b.z - a.z) * (c.x - a.x)).abs()
}

fn signed_overlay_area_m2(contour: &NodeOverlayContour) -> f32 {
    if contour.len() < 3 {
        return 0.0;
    }
    let mut area = 0.0;
    for index in 0..contour.len() {
        let start = contour[index];
        let end = contour[(index + 1) % contour.len()];
        area += start[0] * end[1] - end[0] * start[1];
    }
    (area * 0.5) as f32
}

fn triangle_sort_key(
    triangle: &NodeTriangulatedTriangle,
    vertices: &[NodeTriangulatedVertex],
) -> [NodeTriangulationPointKey; 3] {
    let mut keys = triangle
        .vertices
        .map(|index| NodeTriangulationPointKey::from_world(vertices[index].point_world));
    keys.sort();
    keys
}

fn normalized_constraint(a: usize, b: usize) -> [usize; 2] {
    if a < b { [a, b] } else { [b, a] }
}

fn quantize_m(value: f64) -> i64 {
    (value * NODE_TRIANGULATION_KEY_SCALE).round() as i64
}

impl NodeTriangulationPointKey {
    fn from_vertex(vertex: &NodeHeightedVertex) -> Self {
        Self {
            x_mm: quantize_m(vertex.point_xz.x),
            z_mm: quantize_m(vertex.point_xz.y),
        }
    }

    fn from_world(point: RoadVec3) -> Self {
        Self {
            x_mm: quantize_m(point.x),
            z_mm: quantize_m(point.z),
        }
    }
}

impl NodeTriangulationHeightKey {
    fn from_vertex(vertex: &NodeHeightedVertex) -> Self {
        Self(quantize_m(vertex.height_m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::height::NodeHeightedRegion;
    use crate::simulation::network::surface::input::NodeArrangementInput;
    use crate::simulation::network::surface::ownership::NodeBooleanOwnership;
    use crate::simulation::network::surface::rails::NodeRailContourSet;
    use crate::simulation::network::surface::{
        IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, OrderedIncidentPieceMouth,
    };
    use godot::prelude::{Vector2, Vector3};

    fn band(kind: RoadSurfaceBandKind, start: Vector3, end: Vector3) -> IncidentMouthBand {
        IncidentMouthBand {
            kind,
            start_point_world: start,
            end_point_world: end,
        }
    }

    fn profile(x: f32, base_height: f32) -> IncidentMouthProfile {
        let boundary_points_world = vec![
            Vector3::new(x, base_height, -4.0),
            Vector3::new(x, base_height + 0.1, -2.0),
            Vector3::new(x, base_height + 0.2, 0.0),
            Vector3::new(x, base_height + 0.3, 2.0),
            Vector3::new(x, base_height + 0.4, 4.0),
        ];
        let bands = vec![
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[0],
                boundary_points_world[1],
            ),
            band(
                RoadSurfaceBandKind::CurbOrShoulder,
                boundary_points_world[1],
                boundary_points_world[2],
            ),
            band(
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[2],
                boundary_points_world[3],
            ),
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[3],
                boundary_points_world[4],
            ),
        ];
        IncidentMouthProfile {
            inward_direction_xz: Vector2::RIGHT,
            boundary_points_world,
            bands,
        }
    }

    fn solved_height_solution() -> NodeHeightSolution {
        let mouth = OrderedIncidentPieceMouth {
            profile: profile(10.0, 4.0),
            endpoint_profile: profile(0.0, 2.0),
            direction_angle_ccw: 0.0,
            direction_xz: Vector2::RIGHT,
            edge_idx: 7,
            side: IncidentEdgeSide::Start,
        };
        let input = NodeArrangementInput::from_ordered_mouths(
            42,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &[mouth],
        )
        .expect("test mouth should produce canonical input");
        let rails =
            NodeRailContourSet::from_input(&input).expect("test input should produce rails");
        let ownership =
            NodeBooleanOwnership::from_rails(&rails).expect("test rails should produce ownership");
        NodeHeightSolution::from_ownership_and_input(&input, &ownership)
            .expect("test ownership should height canonical regions")
    }

    #[test]
    fn triangulates_heighted_owned_regions_with_spade() {
        let heights = solved_height_solution();
        let solution = NodeTriangulationSolution::from_height_solution(&heights)
            .expect("heighted regions should triangulate");

        assert_eq!(solution.node_id, 42);
        assert_eq!(
            solution.piece_kind,
            RoadSurfaceVisualNodePieceKind::JunctionN
        );
        assert_eq!(solution.regions.len(), heights.regions.len());
        assert!(solution.regions.iter().all(|region| {
            !region.vertices.is_empty()
                && !region.boundary_constraints.is_empty()
                && !region.triangles.is_empty()
                && !region.height_sources.is_empty()
        }));
        assert!(
            solution
                .regions
                .iter()
                .any(|region| region.kind == RoadSurfaceBandKind::Carriageway
                    && region.triangles.len() == 2)
        );
    }

    #[test]
    fn triangulates_owned_region_with_hole_without_filling_the_hole() {
        let heights = NodeHeightSolution {
            node_id: 91,
            piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
            regions: vec![flat_region_with_hole()],
        };
        let solution = NodeTriangulationSolution::from_height_solution(&heights)
            .expect("owned region with an explicit hole should triangulate");
        let region = &solution.regions[0];

        assert!(!region.triangles.is_empty());
        for triangle in &region.triangles {
            let centroid = triangle_centroid_xz(triangle, &region.vertices);
            assert!(
                centroid[0] <= 1.0
                    || centroid[0] >= 3.0
                    || centroid[1] <= 1.0
                    || centroid[1] >= 3.0,
                "triangle centroid must not land inside the hole: {:?}",
                centroid
            );
        }
    }

    #[test]
    fn rejects_degenerate_region_contours() {
        let heights = NodeHeightSolution {
            node_id: 92,
            piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
            regions: vec![NodeHeightedRegion {
                kind: RoadSurfaceBandKind::Carriageway,
                owner: NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0),
                source_mouth_order_index: 0,
                source_band_index: 0,
                shape: vec![vec![flat_vertex(0.0, 0.0), flat_vertex(1.0, 0.0)]],
                area_m2: 0.0,
                height_sources: Vec::new(),
            }],
        };

        assert_eq!(
            NodeTriangulationSolution::from_height_solution(&heights),
            Err(NodeTriangulationError::DegenerateRegionContour {
                node_id: 92,
                region_index: 0,
                contour_index: 0,
                vertex_count: 2,
            })
        );
    }

    fn flat_region_with_hole() -> NodeHeightedRegion {
        NodeHeightedRegion {
            kind: RoadSurfaceBandKind::Sidewalk,
            owner: NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 0),
            source_mouth_order_index: 0,
            source_band_index: 0,
            shape: vec![
                vec![
                    flat_vertex(0.0, 0.0),
                    flat_vertex(4.0, 0.0),
                    flat_vertex(4.0, 4.0),
                    flat_vertex(0.0, 4.0),
                ],
                vec![
                    flat_vertex(1.0, 3.0),
                    flat_vertex(3.0, 3.0),
                    flat_vertex(3.0, 1.0),
                    flat_vertex(1.0, 1.0),
                ],
            ],
            area_m2: 12.0,
            height_sources: Vec::new(),
        }
    }

    fn flat_vertex(x: f64, z: f64) -> NodeHeightedVertex {
        NodeHeightedVertex {
            point_xz: super::super::backend::RoadVec2::new(x, z),
            height_m: 2.0,
            height_sources: Vec::new(),
        }
    }

    fn triangle_centroid_xz(
        triangle: &NodeTriangulatedTriangle,
        vertices: &[NodeTriangulatedVertex],
    ) -> [f64; 2] {
        let a = vertices[triangle.vertices[0]].point_world;
        let b = vertices[triangle.vertices[1]].point_world;
        let c = vertices[triangle.vertices[2]].point_world;
        [(a.x + b.x + c.x) / 3.0, (a.z + b.z + c.z) / 3.0]
    }
}
