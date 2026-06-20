//! Shared fixtures for node-height tests.

use super::*;
use crate::simulation::network::surface::ownership::NodeSourceCarrierRegistry;

pub(super) fn band(kind: RoadSurfaceBandKind, start: RoadVec3, end: RoadVec3) -> IncidentMouthBand {
    IncidentMouthBand {
        kind,
        start_point_world: start,
        end_point_world: end,
    }
}
pub(super) fn profile(x: f64, base_height: f64) -> IncidentMouthProfile {
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
pub(super) fn solved_input() -> NodeArrangementInput {
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
    NodeArrangementInput::from_ordered_mouths(
        42,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &[mouth],
    )
    .expect("test mouth should produce canonical input")
}
pub(super) fn solved_ownership(input: &NodeArrangementInput) -> NodeBooleanOwnership {
    let rails = NodeRailContourSet::from_input(input).expect("test input should produce rails");
    NodeBooleanOwnership::from_rails(&rails).expect("test rails should produce ownership")
}
pub(super) fn manual_rail_contours(
    node_id: u32,
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: Vec<NodeGeneratedContour>,
) -> NodeRailContourSet {
    let source_carriers = NodeSourceCarrierRegistry::from_rail_parts(
        &contours,
        &[],
        &BTreeMap::new(),
        &BTreeMap::new(),
    );
    NodeRailContourSet {
        node_id,
        piece_kind,
        contours,
        corner_trims: Vec::new(),
        constraints: Vec::new(),
        height_carrier_paths_by_source: BTreeMap::new(),
        height_carrier_points_by_source: BTreeMap::new(),
        source_carriers,
    }
}
pub(super) fn terminal_cap_band_for_height_test(
    x: f64,
    height_m: f64,
    role: TerminalCapBandRole,
) -> NodeTerminalCapBand {
    let inner_start = RoadVec3::new(x, height_m, -1.0);
    let inner_center = RoadVec3::new(x, height_m, 0.0);
    let inner_end = RoadVec3::new(x, height_m, 1.0);
    let outer_start = RoadVec3::new(x + 0.15, height_m, -1.0);
    let outer_center = RoadVec3::new(x + 0.15, height_m, 0.0);
    let outer_end = RoadVec3::new(x + 0.15, height_m, 1.0);
    NodeTerminalCapBand {
        source_band_index: 0,
        band_kind: RoadSurfaceBandKind::CurbOrShoulder,
        provenance: TerminalCapBandProvenance {
            layer_index: 0,
            role,
            left_source_band_index: 0,
            right_source_band_index: 1,
            source_boundary_start_index: 0,
            source_boundary_end_index: 1,
            inner_offset_m: 0.0,
            outer_offset_m: 0.15,
        },
        inner_path_world: vec![inner_start, inner_center, inner_end],
        outer_path_world: vec![outer_start, outer_center, outer_end],
        contour_world: vec![
            inner_start,
            inner_center,
            inner_end,
            outer_end,
            outer_center,
            outer_start,
        ],
    }
}
pub(super) fn generated_band_contour(
    kind: RoadSurfaceBandKind,
    points_xz: Vec<RoadVec2>,
    height_points_world: Option<Vec<RoadVec3>>,
) -> NodeGeneratedContour {
    NodeGeneratedContour {
        kind: NodeGeneratedContourKind::Band { kind },
        purpose: NodeGeneratedContourPurpose::NonRoadBand,
        source_mouth_order_index: 0,
        source_band_index: Some(0),
        owner: Some(NodeBandOwner::new(kind, 0)),
        claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
        backend_polyline: road_points_to_polyline(points_xz.clone(), true),
        points_xz,
        height_points_world,
    }
}
pub(super) fn has_vertex_height(
    region: &NodeHeightedRegion,
    expected_x: f64,
    expected_z: f64,
    expected_height: f64,
) -> bool {
    region.shape.iter().flatten().any(|vertex| {
        (vertex.point_xz.x - expected_x).abs() <= 1.0e-6
            && (vertex.point_xz.y - expected_z).abs() <= 1.0e-6
            && (vertex.height_m - expected_height).abs() <= 1.0e-6
    })
}
pub(super) fn conflicting_manual_input() -> NodeArrangementInput {
    NodeArrangementInput {
        node_id: 77,
        piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
        mouths: vec![NodeInputMouth {
            order_index: 0,
            edge_idx: 9,
            side: IncidentEdgeSide::Start,
            direction_xz: RoadVec2::X,
            direction_angle_ccw: 0.0,
            conflict_handoff_distance_m: 10.0,
            mouth_rails: Vec::new(),
            endpoint_rails: Vec::new(),
            boundary_rails: Vec::new(),
            band_intervals: vec![
                manual_interval(0, RoadSurfaceBandKind::Carriageway, 2.0, 4.0),
                manual_interval(1, RoadSurfaceBandKind::Sidewalk, 5.0, 7.0),
            ],
            uses_explicit_band_domain_paths: false,
        }],
    }
}
pub(super) fn manual_interval(
    band_index: usize,
    band_kind: RoadSurfaceBandKind,
    endpoint_height: f64,
    mouth_height: f64,
) -> NodeInputBandInterval {
    NodeInputBandInterval {
        band_index,
        band_kind,
        mouth_start_world: RoadVec3::new(10.0, mouth_height, 0.0),
        mouth_end_world: RoadVec3::new(10.0, mouth_height, 2.0),
        endpoint_start_world: RoadVec3::new(0.0, endpoint_height, 0.0),
        endpoint_end_world: RoadVec3::new(0.0, endpoint_height, 2.0),
        start_path_world: Vec::new(),
        end_path_world: Vec::new(),
    }
}
pub(super) fn manual_region(
    kind: RoadSurfaceBandKind,
    band_index: usize,
    area_m2: f32,
) -> NodeBooleanOwnedRegion {
    NodeBooleanOwnedRegion {
        kind,
        owner: NodeBandOwner::new(kind, band_index),
        claim_priority:
            crate::simulation::network::surface::rails::NodeGeneratedContourClaimPriority::MouthBand,
        source_mouth_order_index: 0,
        source_band_index: Some(band_index),
        shape: vec![vec![[0.0, 0.0], [10.0, 0.0], [10.0, 2.0], [0.0, 2.0]]],
        area_m2,
        seam_constraints: Vec::new(),
    }
}
pub(super) fn manual_heighted_region(
    kind: RoadSurfaceBandKind,
    owner_index: usize,
    area_m2: f32,
    contour: NodeHeightedContour,
) -> NodeHeightedRegion {
    manual_heighted_region_with_seams(kind, owner_index, area_m2, contour, Vec::new())
}
pub(super) fn manual_heighted_region_with_seams(
    kind: RoadSurfaceBandKind,
    owner_index: usize,
    area_m2: f32,
    contour: NodeHeightedContour,
    seam_constraints: Vec<NodeRegionSeamConstraint>,
) -> NodeHeightedRegion {
    let height_field_id = NodeBandHeightFieldId::new(owner_index, owner_index, kind);
    let contour = contour
        .into_iter()
        .map(|mut vertex| {
            vertex.height_field_id = height_field_id;
            vertex
        })
        .collect();
    NodeHeightedRegion {
        kind,
        owner: NodeBandOwner::new(kind, owner_index),
        height_field_id,
        shape: vec![contour],
        area_m2,
        seam_constraints,
    }
}
pub(super) fn manual_seam_constraint(
    constraint_index: usize,
    seam_source: NodeSeamSource,
    constrains_shared_height: bool,
    is_material_transition: bool,
) -> NodeRegionSeamConstraint {
    NodeRegionSeamConstraint {
        constraint_index,
        seam_source,
        owner: None,
        opposite_owner: None,
        constrains_shared_height,
        is_material_transition,
        start_xz: RoadVec2::new(0.0, 0.0),
        end_xz: RoadVec2::new(1.0, 0.0),
    }
}
pub(super) fn manual_owned_pair_seam_constraint(
    constraint_index: usize,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    constrains_shared_height: bool,
) -> NodeRegionSeamConstraint {
    NodeRegionSeamConstraint {
        constraint_index,
        seam_source: NodeSeamSource::RaisedStepContact {
            owner_index: owner.owner_index(),
        },
        owner: Some(owner),
        opposite_owner: Some(opposite_owner),
        constrains_shared_height,
        is_material_transition: true,
        start_xz: RoadVec2::new(0.0, 0.0),
        end_xz: RoadVec2::new(1.0, 0.0),
    }
}
pub(super) fn manual_heighted_vertex(x: f64, z: f64, height_m: f64) -> NodeHeightedVertex {
    NodeHeightedVertex {
        point_xz: RoadVec2::new(x, z),
        height_m,
        height_field_id: NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Sidewalk),
        height_authority: None,
        source_provenance: None,
        grade_authority: None,
    }
}
pub(super) fn manual_heighted_vertex_with_source_provenance(
    kind: RoadSurfaceBandKind,
    owner_index: usize,
    x: f64,
    z: f64,
    height_m: f64,
) -> NodeHeightedVertex {
    let mut vertex = manual_heighted_vertex(x, z, height_m);
    let owner = NodeBandOwner::new(kind, owner_index);
    let height_field_id = NodeBandHeightFieldId::new(owner_index, owner_index, kind);
    vertex.height_field_id = height_field_id;
    vertex.height_authority = Some(NodeHeightAuthoritySource::SourceInterval);
    vertex.source_provenance = Some(NodeHeightCarrierProvenanceKey {
        owner,
        source_kind: kind,
        source_mouth_order_index: owner_index,
        source_band_index: owner_index,
        height_field_id,
        claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
        point: NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(x, z)),
        origin: NodeCarrierProvenanceOrigin::SourceVertex,
    });
    vertex
}
