//! Carrier-provenance closure tests for post-boolean owned vertices.

use super::*;
use crate::simulation::network::surface::backend::RoadVec3;
use crate::simulation::network::surface::rails::NodeRailHeightCarrierPaths;

#[test]
fn carrier_provenance_closure_records_source_segment_projection() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let source = (RoadSurfaceBandKind::Carriageway, 0, 2);
    let point = ownership_key_from_road_point(RoadVec2::new(5.0, 0.0));
    let region = region_for_source(owner, source, vec![[0.0, 0.0], [5.0, 0.0], [0.0, 2.0]]);
    let rails = rails_with_source_surface(source);
    let rail_points = NodeRailCanonicalPointSet {
        all_points: Vec::new(),
        points_by_owner: BTreeMap::new(),
        segments_by_owner: BTreeMap::new(),
        source_segments_by_owner: BTreeMap::new(),
        canonical_points_by_mm_key_by_owner: BTreeMap::new(),
        height_points_by_source: BTreeMap::new(),
        paths_by_owner: BTreeMap::new(),
    };

    let closure = NodeCarrierProvenanceClosure::from_owned_regions(&[region], &rails, &rail_points)
        .expect("source path segment should explicitly authorize the boolean vertex");

    assert!(closure.records.iter().any(|record| {
        record.point.raw_tuple() == point
            && matches!(
                record.origin,
                NodeCarrierProvenanceOrigin::SourceSegment { .. }
            )
    }));
}

#[test]
fn carrier_provenance_closure_records_generated_carrier_vertex() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let source = (RoadSurfaceBandKind::Carriageway, 0, 2);
    let point = ownership_key_from_road_point(RoadVec2::new(5.0, 5.0));
    let region = region_for_source(owner, source, vec![[0.0, 0.0], [5.0, 5.0], [0.0, 10.0]]);
    let rails = rails_with_generated_carrier_vertex(owner, source);
    let rail_points = NodeRailCanonicalPointSet {
        all_points: Vec::new(),
        points_by_owner: BTreeMap::new(),
        segments_by_owner: BTreeMap::new(),
        source_segments_by_owner: BTreeMap::new(),
        canonical_points_by_mm_key_by_owner: BTreeMap::new(),
        height_points_by_source: BTreeMap::new(),
        paths_by_owner: BTreeMap::new(),
    };

    let closure = NodeCarrierProvenanceClosure::from_owned_regions(&[region], &rails, &rail_points)
        .expect("source surface should explicitly authorize the interior boolean vertex");

    assert!(closure.records.iter().any(|record| {
        record.point.raw_tuple() == point
            && matches!(
                record.origin,
                NodeCarrierProvenanceOrigin::GeneratedCarrierVertex { .. }
            )
    }));
}

#[test]
fn carrier_provenance_closure_rejects_unemitted_source_surface_interior() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let source = (RoadSurfaceBandKind::Carriageway, 0, 2);
    let region = region_for_source(owner, source, vec![[0.0, 0.0], [5.0, 5.0], [0.0, 10.0]]);
    let rails = rails_with_source_surface(source);
    let rail_points = NodeRailCanonicalPointSet {
        all_points: Vec::new(),
        points_by_owner: BTreeMap::new(),
        segments_by_owner: BTreeMap::new(),
        source_segments_by_owner: BTreeMap::new(),
        canonical_points_by_mm_key_by_owner: BTreeMap::new(),
        height_points_by_source: BTreeMap::new(),
        paths_by_owner: BTreeMap::new(),
    };

    let error = NodeCarrierProvenanceClosure::from_owned_regions(&[region], &rails, &rail_points)
        .expect_err("interior source-surface containment alone is not carrier provenance");

    assert!(matches!(
        error,
        NodeBooleanOwnershipError::MissingCarrierProvenance { .. }
    ));
}

fn rails_with_source_surface(source: (RoadSurfaceBandKind, usize, usize)) -> NodeRailContourSet {
    let start_path_world = vec![
        RoadVec3::new(0.0, 10.0, 0.0),
        RoadVec3::new(10.0, 20.0, 0.0),
    ];
    let end_path_world = vec![
        RoadVec3::new(0.0, 10.0, 10.0),
        RoadVec3::new(10.0, 20.0, 10.0),
    ];
    let mut height_carrier_paths_by_source = BTreeMap::new();
    height_carrier_paths_by_source.insert(
        source,
        NodeRailHeightCarrierPaths {
            start_path_world: start_path_world.clone(),
            end_path_world: end_path_world.clone(),
        },
    );
    NodeRailContourSet {
        node_id: 42,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        contours: Vec::new(),
        constraints: Vec::new(),
        height_carrier_paths_by_source,
        height_carrier_points_by_source: BTreeMap::from([(
            source,
            start_path_world
                .into_iter()
                .chain(end_path_world)
                .collect::<Vec<_>>(),
        )]),
    }
}

fn rails_with_generated_carrier_vertex(
    owner: NodeBandOwner,
    source: (RoadSurfaceBandKind, usize, usize),
) -> NodeRailContourSet {
    let mut rails = rails_with_source_surface(source);
    let contour_points = vec![
        RoadVec2::new(0.0, 0.0),
        RoadVec2::new(5.0, 5.0),
        RoadVec2::new(0.0, 10.0),
    ];
    rails.contours.push(NodeGeneratedContour {
        kind: NodeGeneratedContourKind::Band { kind: source.0 },
        purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
        source_mouth_order_index: source.1,
        source_band_index: Some(source.2),
        owner: Some(owner),
        claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        points_xz: contour_points.clone(),
        height_points_world: Some(vec![
            RoadVec3::new(0.0, 10.0, 0.0),
            RoadVec3::new(5.0, 15.0, 5.0),
            RoadVec3::new(0.0, 20.0, 10.0),
        ]),
        backend_polyline: road_points_to_polyline(contour_points.clone(), true),
    });
    rails
}

fn region_for_source(
    owner: NodeBandOwner,
    source: (RoadSurfaceBandKind, usize, usize),
    contour: NodeOverlayContour,
) -> NodeBooleanOwnedRegion {
    NodeBooleanOwnedRegion {
        kind: source.0,
        owner,
        claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
        source_mouth_order_index: source.1,
        source_band_index: Some(source.2),
        shape: vec![contour],
        area_m2: 1.0,
        seam_constraints: Vec::new(),
    }
}
