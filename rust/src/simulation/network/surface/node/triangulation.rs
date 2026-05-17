//! Spade-backed triangulation for canonical node-owned height regions.

use super::arrangement::{
    NodeArrangement, NodeArrangementVertex, NodeArrangementVertexId, NodeBandHeightFieldId,
    NodeBandOwner, NodeExplicitVerticalStepSegment, NodeOwnedRegion,
};
use super::backend::RoadVec3;
use super::height::NodeGradeVertexAuthority;
use super::indices::normalized_vertex_edge;
use super::keys::{SurfaceHeightMmKey, SurfaceXzKey};
use super::{
    NODE_OVERLAY_MIN_AREA_M2, NodeOverlayContour, NodeOverlayPoint, NodeOverlayShape,
    NodeOverlayShapes, RoadSurfaceBandKind, RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind,
    SurfaceCdt,
};
use i_overlay::core::overlay_rule::OverlayRule;
use spade::{Point2, Triangulation};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeTriangulationSolution {
    pub(crate) node_id: u32,
    pub(crate) piece_kind: RoadSurfaceVisualNodePieceKind,
    pub(crate) regions: Vec<NodeTriangulatedRegion>,
    pub(crate) explicit_vertical_step_segments: Vec<NodeExplicitVerticalStepSegment>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeTriangulatedRegion {
    pub(crate) kind: RoadSurfaceBandKind,
    pub(crate) owner: NodeBandOwner,
    pub(crate) height_field_id: NodeBandHeightFieldId,
    pub(crate) vertices: Vec<NodeTriangulatedVertex>,
    pub(crate) boundary_constraints: Vec<[usize; 2]>,
    pub(crate) triangles: Vec<NodeTriangulatedTriangle>,
    pub(crate) area_m2: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeTriangulatedVertex {
    pub(crate) point_world: RoadVec3,
    pub(crate) height_field_id: NodeBandHeightFieldId,
    pub(crate) grade_authority: NodeGradeVertexAuthority,
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
    pub(in crate::simulation::network::surface) fn build_node_triangulation_from_arrangement(
        arrangement: &NodeArrangement,
    ) -> Result<NodeTriangulationSolution, NodeTriangulationError> {
        NodeTriangulationSolution::from_arrangement(arrangement)
    }
}

impl NodeTriangulationSolution {
    pub(crate) fn from_arrangement(
        arrangement: &NodeArrangement,
    ) -> Result<Self, NodeTriangulationError> {
        if arrangement.regions().is_empty() {
            return Err(NodeTriangulationError::EmptyHeightSolution {
                node_id: arrangement.node_id(),
            });
        }

        let mut regions = Vec::with_capacity(arrangement.regions().len());
        for (region_index, region) in arrangement.regions().iter().enumerate() {
            regions.push(triangulate_arrangement_region(
                arrangement.node_id(),
                region_index,
                arrangement,
                region,
            )?);
        }

        Ok(Self {
            node_id: arrangement.node_id(),
            piece_kind: arrangement.piece_kind(),
            regions,
            explicit_vertical_step_segments: arrangement.explicit_vertical_step_segments(),
        })
    }
}

fn triangulate_arrangement_region(
    node_id: u32,
    region_index: usize,
    arrangement: &NodeArrangement,
    region: &NodeOwnedRegion,
) -> Result<NodeTriangulatedRegion, NodeTriangulationError> {
    if region.outer_loop().is_empty() {
        return Err(NodeTriangulationError::EmptyRegionShape {
            node_id,
            region_index,
        });
    }

    let mut vertices = Vec::new();
    let mut vertex_lookup = BTreeMap::new();
    let mut constraints = BTreeSet::new();
    push_arrangement_constraint_loop(
        node_id,
        region_index,
        0,
        region.outer_loop(),
        arrangement,
        &mut vertices,
        &mut vertex_lookup,
        &mut constraints,
    )?;
    for (hole_index, hole) in region.holes().iter().enumerate() {
        push_arrangement_constraint_loop(
            node_id,
            region_index,
            hole_index + 1,
            hole,
            arrangement,
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

    let owner_shape = overlay_shape_from_arrangement_region(arrangement, region);
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

    let owner = region.owner();
    Ok(NodeTriangulatedRegion {
        kind: owner.kind(),
        owner,
        height_field_id: region.height_field_id(),
        vertices,
        boundary_constraints: constraints.into_iter().collect(),
        triangles,
        area_m2: region.area_m2(),
    })
}

fn push_arrangement_constraint_loop(
    node_id: u32,
    region_index: usize,
    contour_index: usize,
    contour: &[NodeArrangementVertexId],
    arrangement: &NodeArrangement,
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
        .map(|vertex_id| {
            let vertex = arrangement.vertices().get(vertex_id.index()).ok_or(
                NodeTriangulationError::DegenerateRegionContour {
                    node_id,
                    region_index,
                    contour_index,
                    vertex_count: contour.len(),
                },
            )?;
            insert_arrangement_vertex(node_id, region_index, vertex, vertices, vertex_lookup)
        })
        .collect::<Result<Vec<_>, _>>()?;
    for index in 0..indices.len() {
        let constraint =
            normalized_vertex_edge(indices[index], indices[(index + 1) % indices.len()]);
        if constraint[0] != constraint[1] {
            constraints.insert(constraint);
        }
    }
    Ok(())
}

fn insert_arrangement_vertex(
    node_id: u32,
    region_index: usize,
    vertex: &NodeArrangementVertex,
    vertices: &mut Vec<NodeTriangulatedVertex>,
    vertex_lookup: &mut BTreeMap<NodeTriangulationPointKey, (usize, NodeTriangulationHeightKey)>,
) -> Result<usize, NodeTriangulationError> {
    let point_key = NodeTriangulationPointKey::from_arrangement_vertex(vertex);
    let height_key = NodeTriangulationHeightKey::from_arrangement_vertex(vertex);
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
    let point_xz = vertex.point_xz();
    vertices.push(NodeTriangulatedVertex {
        point_world: RoadVec3::new(point_xz.x, vertex.height_m(), point_xz.y),
        height_field_id: vertex.height_field_id(),
        grade_authority: vertex.grade_authority(),
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
    let triangle_shapes = vec![triangle_shape];
    let owner_shapes = vec![owner_shape.clone()];
    let residual = overlay_difference(
        node_id,
        region_index,
        &triangle_shapes,
        &owner_shapes,
        "triangle_minus_owner",
    )?;
    Ok(residual.is_empty())
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

fn overlay_shape_from_arrangement_region(
    arrangement: &NodeArrangement,
    region: &NodeOwnedRegion,
) -> NodeOverlayShape {
    std::iter::once(region.outer_loop())
        .chain(region.holes().iter().map(Vec::as_slice))
        .map(|contour| {
            contour
                .iter()
                .filter_map(|vertex_id| arrangement.vertices().get(vertex_id.index()))
                .map(|vertex| {
                    let point = vertex.point_xz();
                    [point.x, point.y]
                })
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
    if RoadSurfaceSystem::overlay_contour_area(&contour) < 0.0 {
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
    RoadSurfaceSystem::road_triangle_double_area_xz_m2([a, b, c])
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

fn quantize_m(value: f64) -> i64 {
    SurfaceHeightMmKey::from_m_f64(value).as_i64()
}

impl NodeTriangulationPointKey {
    fn from_arrangement_vertex(vertex: &NodeArrangementVertex) -> Self {
        let key = SurfaceXzKey::from_road_xz(vertex.point_xz());
        Self {
            x_mm: key.x_key(),
            z_mm: key.z_key(),
        }
    }

    fn from_world(point: RoadVec3) -> Self {
        let key = SurfaceXzKey::from_world_xz(point);
        Self {
            x_mm: key.x_key(),
            z_mm: key.z_key(),
        }
    }
}

impl NodeTriangulationHeightKey {
    fn from_arrangement_vertex(vertex: &NodeArrangementVertex) -> Self {
        Self(quantize_m(vertex.height_m()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::arrangement::{
        NodeRegionSeamConstraint, NodeSeamSource,
    };
    use crate::simulation::network::surface::height::{
        NodeHeightSolution, NodeHeightedRegion, NodeHeightedVertex,
    };
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
            boundary_paths_world: Vec::new(),
            band_start_paths_world: Vec::new(),
            band_end_paths_world: Vec::new(),
            uses_explicit_band_domain_paths: false,
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

    fn triangulation_from_height_solution(
        heights: &NodeHeightSolution,
    ) -> Result<NodeTriangulationSolution, NodeTriangulationError> {
        let arrangement = NodeArrangement::from_height_solution(heights)
            .expect("height solution should produce canonical arrangement before CDT");
        NodeTriangulationSolution::from_arrangement(&arrangement)
    }

    #[test]
    fn triangulates_heighted_owned_regions_with_spade() {
        let heights = solved_height_solution();
        let solution = triangulation_from_height_solution(&heights)
            .expect("arranged regions should triangulate");

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
    fn triangulation_does_not_derive_step_authority_from_shared_boundaries() {
        let lower_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let raised_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let lower_height_field = NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Carriageway);
        let raised_height_field =
            NodeBandHeightFieldId::new(0, 1, RoadSurfaceBandKind::CurbOrShoulder);
        let mut arrangement = NodeArrangement::new(94, RoadSurfaceVisualNodePieceKind::JunctionN);
        let seam = NodeRegionSeamConstraint {
            constraint_index: 27,
            seam_source: NodeSeamSource::RaisedStepContact {
                owner_index: raised_owner.owner_index(),
            },
            owner: Some(lower_owner),
            opposite_owner: Some(raised_owner),
            constrains_shared_height: false,
            is_material_transition: true,
            start_xz: super::super::backend::RoadVec2::new(0.0, 1.0),
            end_xz: super::super::backend::RoadVec2::new(1.0, 1.0),
        };

        let lower_a = arrangement_test_vertex(
            &mut arrangement,
            0.0,
            0.0,
            0.0,
            lower_owner,
            lower_height_field,
        );
        let lower_b = arrangement_test_vertex(
            &mut arrangement,
            1.0,
            0.0,
            0.0,
            lower_owner,
            lower_height_field,
        );
        let lower_c = arrangement_test_vertex(
            &mut arrangement,
            1.0,
            1.0,
            0.0,
            lower_owner,
            lower_height_field,
        );
        let lower_d = arrangement_test_vertex(
            &mut arrangement,
            0.0,
            1.0,
            0.0,
            lower_owner,
            lower_height_field,
        );
        arrangement.push_region(
            lower_owner,
            lower_height_field,
            vec![lower_a, lower_b, lower_c, lower_d],
            Vec::new(),
            Vec::new(),
            1.0,
            vec![seam.clone()],
        );

        let raised_a = arrangement_test_vertex(
            &mut arrangement,
            0.0,
            1.0,
            0.12,
            raised_owner,
            raised_height_field,
        );
        let raised_b = arrangement_test_vertex(
            &mut arrangement,
            1.0,
            1.0,
            0.12,
            raised_owner,
            raised_height_field,
        );
        let raised_c = arrangement_test_vertex(
            &mut arrangement,
            1.0,
            2.0,
            0.12,
            raised_owner,
            raised_height_field,
        );
        let raised_d = arrangement_test_vertex(
            &mut arrangement,
            0.0,
            2.0,
            0.12,
            raised_owner,
            raised_height_field,
        );
        arrangement.push_region(
            raised_owner,
            raised_height_field,
            vec![raised_a, raised_b, raised_c, raised_d],
            Vec::new(),
            Vec::new(),
            1.0,
            vec![seam],
        );

        let solution = NodeTriangulationSolution::from_arrangement(&arrangement)
            .expect("adjacent owned regions should triangulate");

        assert!(
            arrangement.explicit_vertical_step_segments().is_empty(),
            "test arrangement must not carry source step authority"
        );
        assert!(
            solution.explicit_vertical_step_segments.is_empty(),
            "CDT boundary contact plus source seam evidence must not synthesize explicit vertical step authority"
        );
    }

    #[test]
    fn triangulates_owned_region_with_hole_without_filling_the_hole() {
        let heights = NodeHeightSolution {
            node_id: 91,
            piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
            regions: vec![flat_region_with_hole()],
        };
        let solution = triangulation_from_height_solution(&heights)
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
    fn triangulation_vertex_pool_preserves_overlay_grid_distinct_points_inside_same_millimetre() {
        let height_field_id = NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Sidewalk);
        let heights = NodeHeightSolution {
            node_id: 93,
            piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            regions: vec![NodeHeightedRegion {
                kind: RoadSurfaceBandKind::Sidewalk,
                owner: NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 0),
                height_field_id,
                shape: vec![vec![
                    flat_vertex(0.00051, 0.0),
                    flat_vertex(0.00149, 0.0),
                    flat_vertex(1.0, 0.0),
                    flat_vertex(1.0, 1.0),
                    flat_vertex(0.0, 1.0),
                ]],
                area_m2: 1.0,
                seam_constraints: Vec::new(),
            }],
        };
        let solution = triangulation_from_height_solution(&heights)
            .expect("overlay-grid-distinct boundary vertices must remain valid CDT input");
        let region = &solution.regions[0];

        assert_eq!(
            region.vertices.len(),
            5,
            "triangulation must not collapse source-distinct canonical XZ vertices into a millimetre pool"
        );
        assert!(region.vertices.iter().any(|vertex| {
            (vertex.point_world.x - 0.00051).abs() < 1.0e-9 && vertex.point_world.z == 0.0
        }));
        assert!(region.vertices.iter().any(|vertex| {
            (vertex.point_world.x - 0.00149).abs() < 1.0e-9 && vertex.point_world.z == 0.0
        }));
    }

    #[test]
    fn rejects_degenerate_region_contours() {
        let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let mut arrangement = NodeArrangement::new(92, RoadSurfaceVisualNodePieceKind::Bend);
        let first = arrangement
            .insert_vertex(
                super::super::backend::RoadVec2::new(0.0, 0.0),
                2.0,
                [owner],
                NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Carriageway),
                [NodeSeamSource::FootprintBoundary { owner_index: 0 }],
            )
            .expect("test vertex should enter arrangement");
        let second = arrangement
            .insert_vertex(
                super::super::backend::RoadVec2::new(1.0, 0.0),
                2.0,
                [owner],
                NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Carriageway),
                [NodeSeamSource::FootprintBoundary { owner_index: 0 }],
            )
            .expect("test vertex should enter arrangement");
        arrangement.push_region(
            owner,
            NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Carriageway),
            vec![first, second],
            Vec::new(),
            Vec::new(),
            0.0,
            Vec::new(),
        );

        assert_eq!(
            NodeTriangulationSolution::from_arrangement(&arrangement),
            Err(NodeTriangulationError::DegenerateRegionContour {
                node_id: 92,
                region_index: 0,
                contour_index: 0,
                vertex_count: 2,
            })
        );
    }

    fn flat_region_with_hole() -> NodeHeightedRegion {
        let height_field_id = NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Sidewalk);
        NodeHeightedRegion {
            kind: RoadSurfaceBandKind::Sidewalk,
            owner: NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 0),
            height_field_id,
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
            seam_constraints: Vec::new(),
        }
    }

    fn flat_vertex(x: f64, z: f64) -> NodeHeightedVertex {
        let point_xz = super::super::backend::RoadVec2::new(x, z);
        let height_m = 2.0;
        let height_field_id = NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Sidewalk);
        let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 0);
        NodeHeightedVertex {
            point_xz,
            height_m,
            height_field_id,
            height_authority: None,
            grade_authority: Some(NodeGradeVertexAuthority::new(
                point_xz,
                height_m,
                owner,
                height_field_id,
                super::super::height::NodeGradeCarrierDecision::SourceCarrier { authority: None },
            )),
        }
    }

    fn arrangement_test_vertex(
        arrangement: &mut NodeArrangement,
        x: f64,
        z: f64,
        height_m: f64,
        owner: NodeBandOwner,
        height_field_id: NodeBandHeightFieldId,
    ) -> NodeArrangementVertexId {
        arrangement
            .insert_vertex(
                super::super::backend::RoadVec2::new(x, z),
                height_m,
                [owner],
                height_field_id,
                [NodeSeamSource::FootprintBoundary {
                    owner_index: owner.owner_index(),
                }],
            )
            .expect("test vertex should enter arrangement")
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
