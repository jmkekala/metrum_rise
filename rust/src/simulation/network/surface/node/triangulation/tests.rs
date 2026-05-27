//! Triangulation tests kept beside the stage while out of the production router.

use super::*;
use crate::simulation::network::surface::arrangement::{NodeRegionSeamConstraint, NodeSeamSource};
use crate::simulation::network::surface::backend::{RoadVec2, RoadVec3};
use crate::simulation::network::surface::height::{
    NodeHeightSolution, NodeHeightedRegion, NodeHeightedVertex,
};
use crate::simulation::network::surface::input::NodeArrangementInput;
use crate::simulation::network::surface::ownership::NodeBooleanOwnership;
use crate::simulation::network::surface::rails::NodeRailContourSet;
use crate::simulation::network::surface::{
    IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, OrderedIncidentPieceMouth,
};

fn band(kind: RoadSurfaceBandKind, start: RoadVec3, end: RoadVec3) -> IncidentMouthBand {
    IncidentMouthBand {
        kind,
        start_point_world: start,
        end_point_world: end,
    }
}

fn profile(x: f64, base_height: f64) -> IncidentMouthProfile {
    let boundary_points_world = vec![
        RoadVec3::new(x, base_height, -4.0),
        RoadVec3::new(x, base_height + 0.1, -2.0),
        RoadVec3::new(x, base_height + 0.2, 0.0),
        RoadVec3::new(x, base_height + 0.3, 2.0),
        RoadVec3::new(x, base_height + 0.4, 4.0),
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
        inward_direction_xz: RoadVec2::X,
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
        direction_xz: RoadVec2::X,
        edge_idx: 7,
        side: IncidentEdgeSide::Start,
    };
    let input = NodeArrangementInput::from_ordered_mouths(
        42,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &[mouth],
    )
    .expect("test mouth should produce canonical input");
    let rails = NodeRailContourSet::from_input(&input).expect("test input should produce rails");
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
    let solution =
        triangulation_from_height_solution(&heights).expect("arranged regions should triangulate");

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
    assert!(solution.regions.iter().any(|region| {
        region.kind == RoadSurfaceBandKind::Carriageway && region.triangles.len() == 2
    }));
}

#[test]
fn triangulation_does_not_derive_step_authority_from_shared_boundaries() {
    let lower_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let raised_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let lower_height_field = NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Carriageway);
    let raised_height_field = NodeBandHeightFieldId::new(0, 1, RoadSurfaceBandKind::CurbOrShoulder);
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
            centroid[0] <= 1.0 || centroid[0] >= 3.0 || centroid[1] <= 1.0 || centroid[1] >= 3.0,
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
        source_provenance: None,
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
