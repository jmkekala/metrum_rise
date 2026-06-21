//! Shared fixtures for rail-stage tests.

use super::*;

pub(super) fn band(kind: RoadSurfaceBandKind, start: RoadVec3, end: RoadVec3) -> IncidentMouthBand {
    IncidentMouthBand {
        kind,
        start_point_world: start,
        end_point_world: end,
    }
}
pub(super) fn profile(x: f64) -> IncidentMouthProfile {
    let boundary_points_world = vec![
        RoadVec3::new(x, 4.0, -4.0),
        RoadVec3::new(x, 4.1, -2.0),
        RoadVec3::new(x, 4.2, 0.0),
        RoadVec3::new(x, 4.3, 2.0),
        RoadVec3::new(x, 4.4, 4.0),
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
pub(super) fn terminal_profile(x: f64) -> IncidentMouthProfile {
    let boundary_points_world = vec![
        RoadVec3::new(x, 4.0, -4.0),
        RoadVec3::new(x, 4.1, -3.0),
        RoadVec3::new(x, 4.2, -1.0),
        RoadVec3::new(x, 4.0, 0.0),
        RoadVec3::new(x, 4.2, 1.0),
        RoadVec3::new(x, 4.1, 3.0),
        RoadVec3::new(x, 4.0, 4.0),
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
            RoadSurfaceBandKind::Carriageway,
            boundary_points_world[3],
            boundary_points_world[4],
        ),
        band(
            RoadSurfaceBandKind::CurbOrShoulder,
            boundary_points_world[4],
            boundary_points_world[5],
        ),
        band(
            RoadSurfaceBandKind::Sidewalk,
            boundary_points_world[5],
            boundary_points_world[6],
        ),
    ];
    IncidentMouthProfile {
        inward_direction_xz: RoadVec2::X,
        boundary_points_world,
        bands,
    }
}
pub(super) fn terminal_profile_z(z: f64) -> IncidentMouthProfile {
    let boundary_points_world = vec![
        RoadVec3::new(4.0, 4.0, z),
        RoadVec3::new(3.0, 4.1, z),
        RoadVec3::new(1.0, 4.2, z),
        RoadVec3::new(0.0, 4.0, z),
        RoadVec3::new(-1.0, 4.2, z),
        RoadVec3::new(-3.0, 4.1, z),
        RoadVec3::new(-4.0, 4.0, z),
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
            RoadSurfaceBandKind::Carriageway,
            boundary_points_world[3],
            boundary_points_world[4],
        ),
        band(
            RoadSurfaceBandKind::CurbOrShoulder,
            boundary_points_world[4],
            boundary_points_world[5],
        ),
        band(
            RoadSurfaceBandKind::Sidewalk,
            boundary_points_world[5],
            boundary_points_world[6],
        ),
    ];
    IncidentMouthProfile {
        inward_direction_xz: RoadVec2::Y,
        boundary_points_world,
        bands,
    }
}
pub(super) fn input_with_endpoint_x(endpoint_x: f64) -> NodeArrangementInput {
    let mouth = OrderedIncidentPieceMouth {
        profile: profile(10.0),
        endpoint_profile: profile(endpoint_x),
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
pub(super) fn terminal_input_with_endpoint_x(endpoint_x: f64) -> NodeArrangementInput {
    let mouth = OrderedIncidentPieceMouth {
        profile: terminal_profile(10.0),
        endpoint_profile: terminal_profile(endpoint_x),
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
        RoadSurfaceVisualNodePieceKind::Terminal,
        &[mouth],
    )
    .expect("test terminal mouth should produce canonical input")
}
pub(super) fn side_join_input(piece_kind: RoadSurfaceVisualNodePieceKind) -> NodeArrangementInput {
    let first = OrderedIncidentPieceMouth {
        profile: terminal_profile(10.0),
        endpoint_profile: terminal_profile(0.0),
        boundary_paths_world: Vec::new(),
        band_start_paths_world: Vec::new(),
        band_end_paths_world: Vec::new(),
        uses_explicit_band_domain_paths: false,
        direction_angle_ccw: 0.0,
        direction_xz: RoadVec2::X,
        edge_idx: 7,
        side: IncidentEdgeSide::Start,
    };
    let second = OrderedIncidentPieceMouth {
        profile: terminal_profile_z(12.0),
        endpoint_profile: terminal_profile_z(2.0),
        boundary_paths_world: Vec::new(),
        band_start_paths_world: Vec::new(),
        band_end_paths_world: Vec::new(),
        uses_explicit_band_domain_paths: false,
        direction_angle_ccw: std::f32::consts::FRAC_PI_2,
        direction_xz: RoadVec2::Y,
        edge_idx: 8,
        side: IncidentEdgeSide::Start,
    };
    NodeArrangementInput::from_ordered_mouths(42, piece_kind, &[first, second])
        .expect("test side-join mouths should produce canonical input")
}
pub(super) fn side_join_input_with_shared_endpoint_center(
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> NodeArrangementInput {
    let first = OrderedIncidentPieceMouth {
        profile: terminal_profile(10.0),
        endpoint_profile: terminal_profile(0.0),
        boundary_paths_world: Vec::new(),
        band_start_paths_world: Vec::new(),
        band_end_paths_world: Vec::new(),
        uses_explicit_band_domain_paths: false,
        direction_angle_ccw: 0.0,
        direction_xz: RoadVec2::X,
        edge_idx: 7,
        side: IncidentEdgeSide::Start,
    };
    let second = OrderedIncidentPieceMouth {
        profile: terminal_profile_z(12.0),
        endpoint_profile: terminal_profile_z(0.0),
        boundary_paths_world: Vec::new(),
        band_start_paths_world: Vec::new(),
        band_end_paths_world: Vec::new(),
        uses_explicit_band_domain_paths: false,
        direction_angle_ccw: std::f32::consts::FRAC_PI_2,
        direction_xz: RoadVec2::Y,
        edge_idx: 8,
        side: IncidentEdgeSide::Start,
    };
    NodeArrangementInput::from_ordered_mouths(42, piece_kind, &[first, second])
        .expect("shared-center side-join mouths should produce canonical input")
}
pub(super) fn nonterminal_input_with_side_join_candidate() -> NodeArrangementInput {
    side_join_input(RoadSurfaceVisualNodePieceKind::JunctionN)
}
pub(super) fn bend_input_with_curb_side_join() -> NodeArrangementInput {
    side_join_input(RoadSurfaceVisualNodePieceKind::Bend)
}
pub(super) fn same_owner_side_join_band() -> NodeInputSideJoinBand {
    NodeInputSideJoinBand {
        source_band_index: 3,
        band_kind: RoadSurfaceBandKind::Sidewalk,
        gap: NodeInputSideJoinGap {
            from_mouth_order_index: 0,
            to_mouth_order_index: 1,
            from_edge_idx: 7,
            to_edge_idx: 8,
            from_side: IncidentEdgeSide::Start,
            to_side: IncidentEdgeSide::Start,
            angle_rad: std::f64::consts::FRAC_PI_2,
            role: NodeInputSideJoinGapRole::Interior,
        },
        boundary_mode: NodeInputSideJoinBandBoundaryMode::SameOwnerOuterCap,
        inner_path_world: vec![RoadVec3::new(0.0, 4.4, 4.0), RoadVec3::new(2.0, 4.4, 4.0)],
        outer_path_world: vec![RoadVec3::new(0.9, 4.4, 6.0), RoadVec3::new(1.1, 4.4, 6.0)],
        outer_footprint_trim_world: Vec::new(),
        trims_outer_footprint: false,
        contour_world: vec![
            RoadVec3::new(0.0, 4.4, 4.0),
            RoadVec3::new(2.0, 4.4, 4.0),
            RoadVec3::new(1.0, 4.4, 6.0),
        ],
    }
}
pub(super) fn constraint_opposite_owner(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
) -> Option<NodeBandOwner> {
    match (constraint.owner, constraint.opposite_owner) {
        (Some(left), Some(right)) if left == owner => Some(right),
        (Some(left), Some(right)) if right == owner => Some(left),
        _ => None,
    }
}
