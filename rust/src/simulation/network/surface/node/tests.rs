//! Node surface export tests.

use super::super::NodeFootprintBoundaryVertexSource;
use super::super::arrangement::{NodeBandHeightFieldId, NodeRegionSeamConstraint, NodeSeamSource};
use super::super::backend::RoadVec2;
use super::super::height::{NodeHeightSolution, NodeHeightedRegion, NodeHeightedVertex};
use super::super::node_grade::{NodeGradeCarrierDecision, NodeGradeVertexAuthority};
use super::*;

fn owner(kind: RoadSurfaceBandKind, owner_index: usize) -> NodeBandOwner {
    NodeBandOwner::new(kind, owner_index)
}

fn height_field(owner: NodeBandOwner) -> NodeBandHeightFieldId {
    NodeBandHeightFieldId::new(owner.owner_index(), owner.owner_index(), owner.kind())
}

fn raised_step_seam(
    lower_owner: NodeBandOwner,
    raised_owner: NodeBandOwner,
    start: RoadVec2,
    end: RoadVec2,
) -> NodeRegionSeamConstraint {
    NodeRegionSeamConstraint {
        constraint_index: 7,
        seam_source: NodeSeamSource::RaisedStepContact {
            owner_index: raised_owner.owner_index(),
        },
        owner: Some(lower_owner),
        opposite_owner: Some(raised_owner),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: start,
        end_xz: end,
    }
}

fn arrangement_with_vertical_step_support(
    raised_start: RoadVec2,
    raised_end: RoadVec2,
) -> (NodeArrangement, Vec<NodeExplicitVerticalStepSegment>) {
    arrangement_with_owner_pair_vertical_step_support(
        RoadSurfaceBandKind::Carriageway,
        RoadSurfaceBandKind::CurbOrShoulder,
        raised_start,
        raised_end,
    )
}

fn arrangement_with_owner_pair_vertical_step_support(
    lower_kind: RoadSurfaceBandKind,
    raised_kind: RoadSurfaceBandKind,
    raised_start: RoadVec2,
    raised_end: RoadVec2,
) -> (NodeArrangement, Vec<NodeExplicitVerticalStepSegment>) {
    let lower_owner = owner(lower_kind, 0);
    let raised_owner = owner(raised_kind, 1);
    let lower_height = height_field(lower_owner);
    let raised_height = height_field(raised_owner);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(2.0, 0.0);
    let seam = raised_step_seam(lower_owner, raised_owner, start, end);
    let mut arrangement = NodeArrangement::new(42, RoadSurfaceVisualNodePieceKind::Bend);

    let lower_start = arrangement
        .insert_vertex(start, 0.0, [lower_owner], lower_height, [])
        .expect("lower start vertex is valid");
    let lower_end = arrangement
        .insert_vertex(end, 0.0, [lower_owner], lower_height, [])
        .expect("lower end vertex is valid");
    let lower_apex = arrangement
        .insert_vertex(
            RoadVec2::new(0.0, -1.0),
            0.0,
            [lower_owner],
            lower_height,
            [],
        )
        .expect("lower apex vertex is valid");
    let lower_edge = arrangement.push_edge(
        lower_start,
        lower_end,
        lower_owner,
        lower_height,
        Some(raised_owner),
        Some(raised_height),
        false,
        false,
        true,
        NodeSeamSource::RaisedStepContact {
            owner_index: raised_owner.owner_index(),
        },
        vec![seam.constraint_index],
    );
    let lower_region = arrangement.push_region(
        lower_owner,
        lower_height,
        vec![lower_start, lower_end, lower_apex],
        Vec::new(),
        vec![lower_edge],
        1.0,
        vec![seam.clone()],
    );
    arrangement.push_face(
        lower_region,
        lower_owner,
        [lower_start, lower_end, lower_apex],
    );

    let upper_start = arrangement
        .insert_vertex(raised_start, 0.12, [raised_owner], raised_height, [])
        .expect("upper start vertex is valid");
    let upper_end = arrangement
        .insert_vertex(raised_end, 0.12, [raised_owner], raised_height, [])
        .expect("upper end vertex is valid");
    let upper_apex = arrangement
        .insert_vertex(
            RoadVec2::new(raised_start.x, 1.0),
            0.12,
            [raised_owner],
            raised_height,
            [],
        )
        .expect("upper apex vertex is valid");
    let upper_region = arrangement.push_region(
        raised_owner,
        raised_height,
        vec![upper_start, upper_apex, upper_end],
        Vec::new(),
        Vec::new(),
        1.0,
        vec![seam],
    );
    arrangement.push_face(
        upper_region,
        raised_owner,
        [upper_start, upper_apex, upper_end],
    );

    let segments = arrangement.explicit_vertical_step_segments();
    (arrangement, segments)
}

fn heighted_vertex_with_grade_decision(
    point_xz: RoadVec2,
    height_m: f64,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    decision: NodeGradeCarrierDecision,
) -> NodeHeightedVertex {
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
            decision,
        )),
    }
}

fn footprint_shapes_from_points(points: &[RoadVec2]) -> NodeOverlayShapes {
    vec![vec![
        points
            .iter()
            .copied()
            .map(backend::road_vec2_to_overlay_point)
            .collect(),
    ]]
}

fn footprint_loop_contains_xz(loop_points: &[Vector3], point_xz: RoadVec2) -> bool {
    let key = NodeArrangementKey::from_point(point_xz);
    loop_points
        .iter()
        .any(|point| ArrangementBoundaryPointKey::from_world(*point).xz_key() == key)
}

#[test]
fn node_top_surface_sources_preserve_explicit_material_seam_authority() {
    let owner = owner(RoadSurfaceBandKind::Carriageway, 6);
    let height_field_id = height_field(owner);
    let decision = NodeGradeCarrierDecision::ExplicitMaterialSeam;
    let heights = NodeHeightSolution {
        node_id: 82,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![NodeHeightedRegion {
            kind: RoadSurfaceBandKind::Carriageway,
            owner,
            height_field_id,
            shape: vec![vec![
                heighted_vertex_with_grade_decision(
                    RoadVec2::new(0.0, 0.0),
                    2.0,
                    owner,
                    height_field_id,
                    decision,
                ),
                heighted_vertex_with_grade_decision(
                    RoadVec2::new(1.0, 0.0),
                    2.0,
                    owner,
                    height_field_id,
                    decision,
                ),
                heighted_vertex_with_grade_decision(
                    RoadVec2::new(0.0, 1.0),
                    2.0,
                    owner,
                    height_field_id,
                    decision,
                ),
            ]],
            area_m2: 0.5,
            seam_constraints: Vec::new(),
        }],
    };
    let mut arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("grade-authorized explicit seam should arrange");
    let triangulation = RoadSurfaceSystem::build_node_triangulation_from_arrangement(&arrangement)
        .expect("grade-authorized explicit seam should triangulate");
    arrangement
        .attach_triangulation(&triangulation)
        .expect("grade-authorized explicit seam should attach triangulation");
    let footprint_shapes = footprint_shapes_from_points(&[
        RoadVec2::new(0.0, 0.0),
        RoadVec2::new(1.0, 0.0),
        RoadVec2::new(0.0, 1.0),
    ]);
    let regions =
        RoadSurfaceSystem::node_surface_regions_from_arrangement(&arrangement, &footprint_shapes)
            .expect("grade-authorized explicit seam should export node top provenance");

    assert_eq!(regions.node_top_surface_sources.len(), 1);
    let source = &regions.node_top_surface_sources[0];
    assert_eq!(source.kind, RoadSurfaceBandKind::Carriageway);
    assert_eq!(source.owner_index, owner.owner_index());
    assert_eq!(source.height_field_id, height_field_id);
    assert_eq!(source.vertex_sources.len(), 3);
    assert_eq!(source.triangle_sources.len(), 1);
    for grade_authority_index in source
        .vertex_sources
        .iter()
        .map(|source| source.grade_authority_index)
        .chain(
            source
                .triangle_sources
                .iter()
                .flat_map(|triangle| triangle.iter().map(|source| source.grade_authority_index)),
        )
    {
        assert_eq!(
            regions.node_grade_authorities[grade_authority_index].decision,
            NodeGradeCarrierDecision::ExplicitMaterialSeam
        );
    }
}

#[test]
fn node_export_uses_boolean_footprint_boundary_vertices() {
    let owner = owner(RoadSurfaceBandKind::Carriageway, 6);
    let height_field_id = height_field(owner);
    let heights = NodeHeightSolution {
        node_id: 83,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![NodeHeightedRegion {
            kind: RoadSurfaceBandKind::Carriageway,
            owner,
            height_field_id,
            shape: vec![vec![
                heighted_vertex_with_grade_decision(
                    RoadVec2::new(0.0, 0.0),
                    2.0,
                    owner,
                    height_field_id,
                    NodeGradeCarrierDecision::SourceCarrier { authority: None },
                ),
                heighted_vertex_with_grade_decision(
                    RoadVec2::new(1.0, 0.0),
                    2.0,
                    owner,
                    height_field_id,
                    NodeGradeCarrierDecision::SourceCarrier { authority: None },
                ),
                heighted_vertex_with_grade_decision(
                    RoadVec2::new(0.0, 1.0),
                    2.0,
                    owner,
                    height_field_id,
                    NodeGradeCarrierDecision::SourceCarrier { authority: None },
                ),
            ]],
            area_m2: 0.5,
            seam_constraints: Vec::new(),
        }],
    };
    let mut arrangement =
        NodeArrangement::from_height_solution(&heights).expect("test triangle should arrange");
    let triangulation = RoadSurfaceSystem::build_node_triangulation_from_arrangement(&arrangement)
        .expect("test triangle should triangulate");
    arrangement
        .attach_triangulation(&triangulation)
        .expect("test triangle should attach triangulation");
    let footprint_shapes = footprint_shapes_from_points(&[
        RoadVec2::new(0.0, 0.0),
        RoadVec2::new(0.5, 0.0),
        RoadVec2::new(1.0, 0.0),
        RoadVec2::new(0.0, 1.0),
    ]);

    let regions =
        RoadSurfaceSystem::node_surface_regions_from_arrangement(&arrangement, &footprint_shapes)
            .expect("footprint export should use boolean footprint contour vertices");

    assert!(
        regions.outer_boundary_loops.iter().any(|polygon| {
            footprint_loop_contains_xz(&polygon.points_world, RoadVec2::new(0.5, 0.0))
        }),
        "node footprint export must preserve final boolean footprint vertices instead of rebuilding from top triangles"
    );
}

#[test]
fn node_export_uses_surface_provenance_for_boolean_footprint_vertex_height() {
    let owner = owner(RoadSurfaceBandKind::Carriageway, 6);
    let height_field_id = height_field(owner);
    let heights = NodeHeightSolution {
        node_id: 85,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![NodeHeightedRegion {
            kind: RoadSurfaceBandKind::Carriageway,
            owner,
            height_field_id,
            shape: vec![vec![
                heighted_vertex_with_grade_decision(
                    RoadVec2::new(0.0, 0.0),
                    0.0,
                    owner,
                    height_field_id,
                    NodeGradeCarrierDecision::SourceCarrier { authority: None },
                ),
                heighted_vertex_with_grade_decision(
                    RoadVec2::new(1.0, 0.0),
                    1.0,
                    owner,
                    height_field_id,
                    NodeGradeCarrierDecision::SourceCarrier { authority: None },
                ),
                heighted_vertex_with_grade_decision(
                    RoadVec2::new(0.0, 1.0),
                    0.0,
                    owner,
                    height_field_id,
                    NodeGradeCarrierDecision::SourceCarrier { authority: None },
                ),
            ]],
            area_m2: 0.5,
            seam_constraints: Vec::new(),
        }],
    };
    let mut arrangement =
        NodeArrangement::from_height_solution(&heights).expect("test triangle should arrange");
    let triangulation = RoadSurfaceSystem::build_node_triangulation_from_arrangement(&arrangement)
        .expect("test triangle should triangulate");
    arrangement
        .attach_triangulation(&triangulation)
        .expect("test triangle should attach triangulation");
    let footprint_shapes = footprint_shapes_from_points(&[
        RoadVec2::new(0.0, 0.0),
        RoadVec2::new(0.5, 0.25),
        RoadVec2::new(1.0, 0.0),
        RoadVec2::new(0.0, 1.0),
    ]);

    let regions =
        RoadSurfaceSystem::node_surface_regions_from_arrangement(&arrangement, &footprint_shapes)
            .expect("footprint export should use visible top-surface provenance");

    let boundary_point = regions
        .outer_boundary_loops
        .iter()
        .flat_map(|polygon| polygon.points_world.iter())
        .find(|point| (point.x - 0.5).abs() <= 1.0e-6 && (point.z - 0.25).abs() <= 1.0e-6)
        .expect("boolean footprint vertex should be preserved");
    assert!((boundary_point.y - 0.5).abs() <= 1.0e-6);

    let has_surface_source = regions
        .earthwork_boundary_segments
        .iter()
        .flatten()
        .any(|segment| {
            let RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                boundary_source: Some(boundary_source),
                ..
            } = segment.source
            else {
                return false;
            };
            [boundary_source.start, boundary_source.end]
                .into_iter()
                .any(|source| {
                    matches!(
                        source,
                        NodeFootprintBoundaryVertexSource::SurfaceInterpolation {
                            height_mm: 500,
                            ..
                        }
                    )
                })
        });
    assert!(
        has_surface_source,
        "boolean footprint vertices inside visible top triangles must export surface provenance"
    );
}

#[test]
fn node_export_rejects_conflicting_footprint_boundary_heights() {
    let lower_owner = owner(RoadSurfaceBandKind::Carriageway, 0);
    let raised_owner = owner(RoadSurfaceBandKind::Sidewalk, 1);
    let lower_height = height_field(lower_owner);
    let raised_height = height_field(raised_owner);
    let mut arrangement = NodeArrangement::new(84, RoadSurfaceVisualNodePieceKind::JunctionN);
    let lower_start = arrangement
        .insert_vertex(
            RoadVec2::new(0.0, 0.0),
            0.0,
            [lower_owner],
            lower_height,
            [],
        )
        .expect("lower start vertex is valid");
    let lower_end = arrangement
        .insert_vertex(
            RoadVec2::new(1.0, 0.0),
            0.0,
            [lower_owner],
            lower_height,
            [],
        )
        .expect("lower end vertex is valid");
    let lower_apex = arrangement
        .insert_vertex(
            RoadVec2::new(0.0, 1.0),
            0.0,
            [lower_owner],
            lower_height,
            [],
        )
        .expect("lower apex vertex is valid");
    let lower_region = arrangement.push_region(
        lower_owner,
        lower_height,
        vec![lower_start, lower_end, lower_apex],
        Vec::new(),
        Vec::new(),
        0.5,
        Vec::new(),
    );
    arrangement.push_face(
        lower_region,
        lower_owner,
        [lower_start, lower_end, lower_apex],
    );

    let raised_start = arrangement
        .insert_vertex(
            RoadVec2::new(0.0, 0.0),
            0.1,
            [raised_owner],
            raised_height,
            [],
        )
        .expect("raised start vertex is valid");
    let raised_end = arrangement
        .insert_vertex(
            RoadVec2::new(-1.0, 0.0),
            0.1,
            [raised_owner],
            raised_height,
            [],
        )
        .expect("raised end vertex is valid");
    let raised_apex = arrangement
        .insert_vertex(
            RoadVec2::new(0.0, -1.0),
            0.1,
            [raised_owner],
            raised_height,
            [],
        )
        .expect("raised apex vertex is valid");
    let raised_region = arrangement.push_region(
        raised_owner,
        raised_height,
        vec![raised_start, raised_end, raised_apex],
        Vec::new(),
        Vec::new(),
        0.5,
        Vec::new(),
    );
    arrangement.push_face(
        raised_region,
        raised_owner,
        [raised_start, raised_end, raised_apex],
    );
    let footprint_shapes = footprint_shapes_from_points(&[
        RoadVec2::new(0.0, 0.0),
        RoadVec2::new(1.0, 0.0),
        RoadVec2::new(0.0, 1.0),
    ]);

    let error =
        RoadSurfaceSystem::node_surface_regions_from_arrangement(&arrangement, &footprint_shapes)
            .expect_err("footprint boundary height conflicts must not be resolved by max/min");

    assert!(matches!(
        error,
        NodeBoundaryExportError::ConflictingFootprintBoundaryHeight { .. }
    ));
}

#[test]
fn vertical_step_export_uses_exact_canonical_arrangement_keys() {
    let (arrangement, segments) =
        arrangement_with_vertical_step_support(RoadVec2::new(0.0, 0.0), RoadVec2::new(2.0, 0.0));

    let faces =
        RoadSurfaceSystem::raised_step_face_polygons_from_arrangement(&arrangement, &segments);

    assert_eq!(segments.len(), 1);
    assert_eq!(faces.len(), 1);
}

#[test]
fn vertical_step_export_uses_generic_curb_sidewalk_owner_pair() {
    let (arrangement, segments) = arrangement_with_owner_pair_vertical_step_support(
        RoadSurfaceBandKind::CurbOrShoulder,
        RoadSurfaceBandKind::Sidewalk,
        RoadVec2::new(0.0, 0.0),
        RoadVec2::new(2.0, 0.0),
    );

    let faces =
        RoadSurfaceSystem::raised_step_face_polygons_from_arrangement(&arrangement, &segments);

    assert_eq!(segments.len(), 1);
    assert_eq!(faces.len(), 1);
}

#[test]
fn vertical_step_export_does_not_repair_overlay_sibling_support() {
    let (arrangement, segments) = arrangement_with_vertical_step_support(
        RoadVec2::new(0.0, 0.000001),
        RoadVec2::new(2.0, 0.000001),
    );

    let faces =
        RoadSurfaceSystem::raised_step_face_polygons_from_arrangement(&arrangement, &segments);

    assert_eq!(segments.len(), 1);
    assert!(
        faces.is_empty(),
        "overlay-neighbor support must not synthesize a vertical face"
    );
}
