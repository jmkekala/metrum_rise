//! Tests for node boolean ownership.

use super::rail_authority::{
    NodeRailCanonicalPointSet, NodeRailSourceSegmentAuthority, canonical_points_by_mm_key_by_owner,
    constraint_authority_owners, insert_open_source_segments,
    validate_owned_region_vertices_against_source_authority,
};
use super::rings::{
    canonicalize_final_owned_region_boundary_edges,
    canonicalize_owned_region_rings_with_rail_point_set,
};
use super::seams::{
    ConstraintOverlapMode, canonicalize_seam_constraints, owned_shape_is_discardable_numeric_dust,
};
use super::topology_keys::{
    NodeOwnershipPointKey, OwnedRegionEdgeKey, overlay_point_from_key,
    ownership_key_from_overlay_point, ownership_key_from_road_point,
};
use super::*;
use crate::simulation::network::surface::backend::{RoadVec2, road_points_to_polyline};
use crate::simulation::network::surface::input::NodeArrangementInput;
use crate::simulation::network::surface::rails::{
    NodeGeneratedContour, NodeGeneratedContourKind, NodeGeneratedContourPurpose,
    NodeRailConstraint, NodeRailConstraintKind, NodeRailContourSet,
};
use crate::simulation::network::surface::validation::NodeValidationReport;
use crate::simulation::network::surface::{
    IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, NodeOverlayContour,
    OrderedIncidentPieceMouth,
};
use godot::prelude::{Vector2, Vector3};
use std::collections::{BTreeMap, BTreeSet};

fn band(kind: RoadSurfaceBandKind, start: Vector3, end: Vector3) -> IncidentMouthBand {
    IncidentMouthBand {
        kind,
        start_point_world: start,
        end_point_world: end,
    }
}

fn profile(x: f32) -> IncidentMouthProfile {
    let boundary_points_world = vec![
        Vector3::new(x, 4.0, -4.0),
        Vector3::new(x, 4.1, -2.0),
        Vector3::new(x, 4.2, 0.0),
        Vector3::new(x, 4.3, 2.0),
        Vector3::new(x, 4.4, 4.0),
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

fn contour_set() -> NodeRailContourSet {
    let mouth = OrderedIncidentPieceMouth {
        profile: profile(10.0),
        endpoint_profile: profile(0.0),
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
    NodeRailContourSet::from_input(&input).expect("test input should produce contours")
}

fn test_rail_canonical_points_from_constraints(
    rail_constraints: &[NodeRailConstraint],
) -> NodeRailCanonicalPointSet {
    let mut all_points = rail_constraints
        .iter()
        .flat_map(|constraint| constraint.points_xz.iter().copied())
        .map(ownership_key_from_road_point)
        .collect::<Vec<_>>();
    all_points.sort_unstable();
    all_points.dedup();

    let mut points_by_owner = BTreeMap::<NodeBandOwner, Vec<NodeOwnershipPointKey>>::new();
    let mut segments_by_owner = BTreeMap::<NodeBandOwner, Vec<OwnedRegionEdgeKey>>::new();
    let mut source_segments_by_owner =
        BTreeMap::<NodeBandOwner, Vec<NodeRailSourceSegmentAuthority>>::new();
    for constraint in rail_constraints {
        let path = constraint
            .points_xz
            .iter()
            .copied()
            .map(ownership_key_from_road_point)
            .collect::<Vec<_>>();
        for owner in constraint_authority_owners(constraint) {
            points_by_owner
                .entry(owner)
                .or_default()
                .extend(path.iter().copied());
            insert_open_source_segments(&mut segments_by_owner, owner, &path);
            if let Some(source_band_index) = constraint.source_band_index {
                for segment in path.windows(2) {
                    if segment[0] == segment[1] {
                        continue;
                    }
                    source_segments_by_owner.entry(owner).or_default().push(
                        NodeRailSourceSegmentAuthority {
                            owner,
                            source: (
                                owner.kind(),
                                constraint.source_mouth_order_index,
                                source_band_index,
                            ),
                            segment: OwnedRegionEdgeKey::new(segment[0], segment[1]),
                        },
                    );
                }
            }
        }
    }
    for points in points_by_owner.values_mut() {
        points.sort_unstable();
        points.dedup();
    }
    for segments in segments_by_owner.values_mut() {
        segments.sort_unstable();
        segments.dedup();
    }
    for segments in source_segments_by_owner.values_mut() {
        segments.sort_unstable();
        segments.dedup();
    }
    let canonical_points_by_mm_key_by_owner = canonical_points_by_mm_key_by_owner(&points_by_owner);
    NodeRailCanonicalPointSet {
        all_points,
        points_by_owner,
        segments_by_owner,
        source_segments_by_owner,
        canonical_points_by_mm_key_by_owner,
        height_points_by_source: BTreeMap::new(),
        paths_by_owner: BTreeMap::new(),
    }
}

mod arrangement_diagnostics;
mod carrier_provenance;
mod domains;
mod rail_authority;
mod ring_noding;
mod seam_materialization;

fn test_owned_region(
    kind: RoadSurfaceBandKind,
    owner: NodeBandOwner,
    contour: NodeOverlayContour,
) -> NodeBooleanOwnedRegion {
    NodeBooleanOwnedRegion {
        kind,
        owner,
        claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
        source_mouth_order_index: owner.owner_index(),
        source_band_index: Some(owner.owner_index()),
        shape: vec![contour],
        area_m2: 1.0,
        seam_constraints: Vec::new(),
    }
}
