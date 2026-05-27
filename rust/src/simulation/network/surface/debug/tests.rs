//! Debug extraction regression tests.

use super::*;
use crate::simulation::network::surface::{
    IncidentMouthBand, NodeOwnedRegion, RoadSurfaceVisualNodePieceKind,
    backend::{RoadVec2, RoadVec3},
};

fn polygon(points_world: Vec<RoadVec3>) -> RoadSurfaceVisualPolygon {
    RoadSurfaceVisualPolygon {
        points_world,
        triangles_world: Vec::new(),
    }
}

fn empty_node_piece() -> RoadSurfaceVisualNodePiece {
    RoadSurfaceVisualNodePiece {
        node_id: 0,
        kind: RoadSurfaceVisualNodePieceKind::Terminal,
        outer_boundary_loops: Vec::new(),
        terrain_clip_boundary_loops: Vec::new(),
        road_surface_polygons: Vec::new(),
        curb_surface_polygons: Vec::new(),
        raised_step_face_polygons: Vec::new(),
        raised_step_face_sources: Vec::new(),
        sidewalk_surface_polygons: Vec::new(),
        explicit_vertical_step_segments: Vec::new(),
        node_grade_authorities: Vec::new(),
        node_top_surface_sources: Vec::new(),
        owned_regions: Vec::new(),
        earthwork_owner_sources: Vec::new(),
        earthwork_surface_polygons: Vec::new(),
        earthwork_outer_boundary_loops: Vec::new(),
        render_earthwork_faces: Vec::new(),
    }
}

#[test]
fn mouth_seam_debug_matches_vertical_step_anchors_by_material() {
    let curb_anchor = RoadVec3::new(0.0, 0.12, 0.0);
    let road_anchor = RoadVec3::new(0.0, 0.0, 0.0);
    let mouth = IncidentMouthProfile {
        inward_direction_xz: RoadVec2::X,
        boundary_points_world: vec![
            RoadVec3::new(-1.0, 0.12, 0.0),
            curb_anchor,
            RoadVec3::new(1.0, 0.0, 0.0),
        ],
        bands: vec![
            IncidentMouthBand {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
                start_point_world: RoadVec3::new(-1.0, 0.12, 0.0),
                end_point_world: curb_anchor,
            },
            IncidentMouthBand {
                kind: RoadSurfaceBandKind::Carriageway,
                start_point_world: road_anchor,
                end_point_world: RoadVec3::new(1.0, 0.0, 0.0),
            },
        ],
    };

    let mut piece = empty_node_piece();
    piece.road_surface_polygons.push(polygon(vec![
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(1.0, 0.0, 0.0),
        RoadVec3::new(0.0, 0.0, -1.0),
    ]));
    piece.curb_surface_polygons.push(polygon(vec![
        RoadVec3::new(-1.0, 0.12, -1.0),
        RoadVec3::new(1.0, 0.12, 1.0),
        RoadVec3::new(-1.0, 0.12, 1.0),
    ]));

    let material_blind = RoadSurfaceSystem::closest_debug_top_vertex(
        curb_anchor,
        &RoadSurfaceSystem::debug_top_vertices(&piece),
    )
    .expect("test piece should expose a top vertex");
    assert_eq!(material_blind.material, "road");
    assert!((material_blind.y_delta_m - 0.12).abs() <= f32::EPSILON);

    let anchors = RoadSurfaceSystem::mouth_top_match_anchors(&mouth);
    let curb_seam_anchor = anchors
        .iter()
        .find(|anchor| anchor.role == "material_seam_before")
        .expect("curb side of asphalt-curb mouth seam should be checked");
    assert_eq!(curb_seam_anchor.material, "curb");
    let curb_match = RoadSurfaceSystem::closest_debug_top_support_for_material(
        curb_seam_anchor.point,
        curb_seam_anchor.material,
        &piece,
    )
    .expect("curb material should support its seam anchor");
    assert_eq!(curb_match.material, "curb");
    assert!(curb_match.xz_error_m <= f32::EPSILON);
    assert!(curb_match.y_delta_m.abs() <= f32::EPSILON);

    let road_seam_anchor = anchors
        .iter()
        .find(|anchor| anchor.role == "material_seam_after")
        .expect("road side of asphalt-curb mouth seam should be checked");
    assert_eq!(road_seam_anchor.material, "road");
    let road_match = RoadSurfaceSystem::closest_debug_top_support_for_material(
        road_seam_anchor.point,
        road_seam_anchor.material,
        &piece,
    )
    .expect("road material should support its seam anchor");
    assert_eq!(road_match.material, "road");
    assert!(road_match.xz_error_m <= f32::EPSILON);
    assert!(road_match.y_delta_m.abs() <= f32::EPSILON);
}

#[test]
fn raised_step_face_debug_reports_exact_top_edge_closure() {
    let mut piece = empty_node_piece();
    piece.owned_regions.push(NodeOwnedRegion {
        kind: RoadSurfaceBandKind::Carriageway,
        owner_index: 7,
        polygon: polygon(vec![
            RoadVec3::new(-1.0, 0.0, -1.0),
            RoadVec3::new(1.0, 0.0, -1.0),
            RoadVec3::new(1.0, 0.0, 0.0),
            RoadVec3::new(-1.0, 0.0, 0.0),
        ]),
    });
    piece.owned_regions.push(NodeOwnedRegion {
        kind: RoadSurfaceBandKind::CurbOrShoulder,
        owner_index: 11,
        polygon: polygon(vec![
            RoadVec3::new(-1.0, 0.12, 0.0),
            RoadVec3::new(1.0, 0.12, 0.0),
            RoadVec3::new(1.0, 0.12, 1.0),
            RoadVec3::new(-1.0, 0.12, 1.0),
        ]),
    });
    piece.raised_step_face_polygons.push(polygon(vec![
        RoadVec3::new(-1.0, 0.12, 0.0),
        RoadVec3::new(-1.0, 0.0, 0.0),
        RoadVec3::new(1.0, 0.0, 0.0),
        RoadVec3::new(1.0, 0.12, 0.0),
    ]));

    let mut dump = String::new();
    RoadSurfaceSystem::append_raised_step_face_details_debug_literal(&mut dump, &piece);

    assert!(dump.contains("\"face_count\":1"));
    assert!(dump.contains("\"expected_raised_step_count\":1"));
    assert!(dump.contains("\"problem_count\":0"));
    assert!(dump.contains("\"matches_raised_step_owner_pair\":true"));
    assert!(dump.contains("\"visible_from_lower_owner\":true"));
}

#[test]
fn raised_step_face_debug_reports_generic_curb_sidewalk_steps() {
    let mut piece = empty_node_piece();
    let canonical_segment = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(backend::RoadVec2::new(-1.0, 0.0)),
        NodeArrangementKey::from_point(backend::RoadVec2::new(1.0, 0.0)),
        NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 7),
        NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 11),
    )
    .expect("curb-sidewalk step should be non-degenerate");
    piece
        .explicit_vertical_step_segments
        .push(canonical_segment);
    piece.owned_regions.push(NodeOwnedRegion {
        kind: RoadSurfaceBandKind::CurbOrShoulder,
        owner_index: 7,
        polygon: polygon(vec![
            RoadVec3::new(-1.0, 0.12, -1.0),
            RoadVec3::new(1.0, 0.12, -1.0),
            RoadVec3::new(1.0, 0.12, 0.0),
            RoadVec3::new(-1.0, 0.12, 0.0),
        ]),
    });
    piece.owned_regions.push(NodeOwnedRegion {
        kind: RoadSurfaceBandKind::Sidewalk,
        owner_index: 11,
        polygon: polygon(vec![
            RoadVec3::new(-1.0, 0.18, 0.0),
            RoadVec3::new(1.0, 0.18, 0.0),
            RoadVec3::new(1.0, 0.18, 1.0),
            RoadVec3::new(-1.0, 0.18, 1.0),
        ]),
    });
    piece.raised_step_face_polygons.push(polygon(vec![
        RoadVec3::new(-1.0, 0.18, 0.0),
        RoadVec3::new(-1.0, 0.12, 0.0),
        RoadVec3::new(1.0, 0.12, 0.0),
        RoadVec3::new(1.0, 0.18, 0.0),
    ]));
    piece
        .raised_step_face_sources
        .push(RoadSurfaceVerticalFaceSource::CanonicalStep {
            explicit_vertical_step_index: 0,
            segment: canonical_segment,
        });

    let mut dump = String::new();
    RoadSurfaceSystem::append_raised_step_face_details_debug_literal(&mut dump, &piece);

    assert!(dump.contains("\"canonical_raised_step_count\":1"));
    assert!(dump.contains("\"canonical_raised_step_problem_count\":0"));
    assert!(dump.contains("\"raised_owner\":{\"kind\":\"Sidewalk\",\"owner_index\":11}"));
    assert!(dump.contains("\"matching_face_indices\":[0]"));
    assert!(dump.contains("\"matches_raised_step_owner_pair\":true"));
    assert!(dump.contains("\"visible_from_lower_owner\":true"));
}

#[test]
fn raised_step_face_debug_matches_canonical_step_by_source_identity() {
    let mut piece = empty_node_piece();
    let canonical_segment = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(backend::RoadVec2::new(-1.0, 0.0)),
        NodeArrangementKey::from_point(backend::RoadVec2::new(1.0, 0.0)),
        NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 7),
        NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 11),
    )
    .expect("test step should be non-degenerate");
    piece
        .explicit_vertical_step_segments
        .push(canonical_segment);

    let rendered_end_x = 1.000002;
    piece.owned_regions.push(NodeOwnedRegion {
        kind: RoadSurfaceBandKind::Carriageway,
        owner_index: 7,
        polygon: polygon(vec![
            RoadVec3::new(-1.0, 0.0, -1.0),
            RoadVec3::new(rendered_end_x, 0.0, -1.0),
            RoadVec3::new(rendered_end_x, 0.0, 0.0),
            RoadVec3::new(-1.0, 0.0, 0.0),
        ]),
    });
    piece.owned_regions.push(NodeOwnedRegion {
        kind: RoadSurfaceBandKind::CurbOrShoulder,
        owner_index: 11,
        polygon: polygon(vec![
            RoadVec3::new(-1.0, 0.12, 0.0),
            RoadVec3::new(rendered_end_x, 0.12, 0.0),
            RoadVec3::new(rendered_end_x, 0.12, 1.0),
            RoadVec3::new(-1.0, 0.12, 1.0),
        ]),
    });
    piece.raised_step_face_polygons.push(polygon(vec![
        RoadVec3::new(-1.0, 0.12, 0.0),
        RoadVec3::new(-1.0, 0.0, 0.0),
        RoadVec3::new(rendered_end_x, 0.0, 0.0),
        RoadVec3::new(rendered_end_x, 0.12, 0.0),
    ]));
    piece
        .raised_step_face_sources
        .push(RoadSurfaceVerticalFaceSource::CanonicalStep {
            explicit_vertical_step_index: 0,
            segment: canonical_segment,
        });

    let mut dump = String::new();
    RoadSurfaceSystem::append_raised_step_face_details_debug_literal(&mut dump, &piece);

    assert!(dump.contains("\"canonical_raised_step_count\":1"));
    assert!(dump.contains("\"canonical_raised_step_problem_count\":0"));
    assert!(dump.contains("\"source_kind\":\"canonical_step\""));
    assert!(dump.contains("\"source_explicit_vertical_step_index\":0"));
    assert!(dump.contains("\"matching_canonical_step_indices\":[0]"));
    assert!(dump.contains("\"matching_face_indices\":[0]"));
}

#[test]
fn raised_step_face_debug_matches_canonical_step_by_original_source_index() {
    let mut piece = empty_node_piece();
    let filtered_segment = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(backend::RoadVec2::new(-2.0, 0.0)),
        NodeArrangementKey::from_point(backend::RoadVec2::new(-1.5, 0.0)),
        NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 1),
        NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2),
    )
    .expect("same-height owner segment should still be non-degenerate");
    let canonical_segment = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(backend::RoadVec2::new(-1.0, 0.0)),
        NodeArrangementKey::from_point(backend::RoadVec2::new(1.0, 0.0)),
        NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 7),
        NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 11),
    )
    .expect("test step should be non-degenerate");
    piece.explicit_vertical_step_segments.push(filtered_segment);
    piece
        .explicit_vertical_step_segments
        .push(canonical_segment);
    piece.owned_regions.push(NodeOwnedRegion {
        kind: RoadSurfaceBandKind::Carriageway,
        owner_index: 7,
        polygon: polygon(vec![
            RoadVec3::new(-1.0, 0.0, -1.0),
            RoadVec3::new(1.0, 0.0, -1.0),
            RoadVec3::new(1.0, 0.0, 0.0),
            RoadVec3::new(-1.0, 0.0, 0.0),
        ]),
    });
    piece.owned_regions.push(NodeOwnedRegion {
        kind: RoadSurfaceBandKind::CurbOrShoulder,
        owner_index: 11,
        polygon: polygon(vec![
            RoadVec3::new(-1.0, 0.12, 0.0),
            RoadVec3::new(1.0, 0.12, 0.0),
            RoadVec3::new(1.0, 0.12, 1.0),
            RoadVec3::new(-1.0, 0.12, 1.0),
        ]),
    });
    piece.raised_step_face_polygons.push(polygon(vec![
        RoadVec3::new(-1.0, 0.12, 0.0),
        RoadVec3::new(-1.0, 0.0, 0.0),
        RoadVec3::new(1.0, 0.0, 0.0),
        RoadVec3::new(1.0, 0.12, 0.0),
    ]));
    piece
        .raised_step_face_sources
        .push(RoadSurfaceVerticalFaceSource::CanonicalStep {
            explicit_vertical_step_index: 1,
            segment: canonical_segment,
        });

    let mut dump = String::new();
    RoadSurfaceSystem::append_raised_step_face_details_debug_literal(&mut dump, &piece);

    assert!(dump.contains("\"source_constraint_count\":1"));
    assert!(dump.contains("\"canonical_raised_step_problem_count\":0"));
    assert!(dump.contains("\"problem_count\":0"));
    assert!(dump.contains("\"matching_canonical_step_indices\":[0]"));
    assert!(dump.contains("\"explicit_vertical_step_index\":1"));
    assert!(dump.contains("\"materialization_status\":\"materialized\""));
}

#[test]
fn raised_step_debug_reports_non_exposed_source_constraints_without_failing_final_faces() {
    let mut piece = empty_node_piece();
    let canonical_segment = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(backend::RoadVec2::new(-1.0, 0.0)),
        NodeArrangementKey::from_point(backend::RoadVec2::new(1.0, 0.0)),
        NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 7),
        NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 11),
    )
    .expect("test step should be non-degenerate");
    piece
        .explicit_vertical_step_segments
        .push(canonical_segment);

    let mut dump = String::new();
    RoadSurfaceSystem::append_raised_step_face_details_debug_literal(&mut dump, &piece);

    assert!(dump.contains("\"source_constraint_count\":1"));
    assert!(dump.contains("\"final_required_face_count\":0"));
    assert!(dump.contains("\"non_exposed_source_constraint_count\":1"));
    assert!(dump.contains("\"canonical_raised_step_problem_count\":0"));
    assert!(dump.contains("\"problem_count\":0"));
    assert!(dump.contains("\"materialization_status\":\"not_exposed_after_boolean_ownership\""));
}
