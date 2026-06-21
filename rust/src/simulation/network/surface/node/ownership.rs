//! Boolean ownership solve for canonical node-arrangement contours.

use super::arrangement::{
    NodeBandHeightFieldId, NodeBandOwner, NodeRegionSeamConstraint, NodeSeamSource,
};
use super::backend::road_vec2_to_overlay_point;
use super::rails::{
    NodeGeneratedContourClaimPriority, NodeGeneratedContourPurpose, NodeRailConstraint,
    NodeRailConstraintKind, NodeRailContourSet,
};
use super::{
    NodeOverlayContour, NodeOverlayShape, NodeOverlayShapes, RoadSurfaceBandKind,
    RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind,
};

mod boundaries;
mod carrier_provenance;
mod contact_semantics;
mod domains;
mod rail_authority;
mod rings;
mod seams;
mod topology_keys;

pub(crate) use rail_authority::NodeSourceCarrierRegistry;

use domains::{
    ResidualKind, asphalt_authority_domains, asphalt_owner_domains, overlay_contour_from_domain,
    overlay_contours_for_domains, overlay_difference, overlay_intersect, overlay_union,
    overlay_union_shape_sets, owned_regions_from_domains, reject_residual,
    sort_boolean_owned_regions, split_non_road_regions,
    validate_non_road_regions_have_explicit_profile_seam_rails,
};

use rings::{canonicalize_owned_region_rings, clean_canonical_owned_region_shapes};

use rail_authority::canonical_points_for_rail_set;

use carrier_provenance::NodeCarrierProvenanceContext;
use seams::{
    ConstraintOverlapMode, materialize_noded_region_seam_constraints, seam_constraints_for_shape,
};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;
use topology_keys::{
    NodeOwnershipPointKey, OwnedRegionEdgeKey, overlay_point_from_key,
    ownership_key_from_overlay_point, ownership_key_from_road_point, ownership_mm_key,
    point_key_lies_on_segment, segment_parameter_key,
};

fn elapsed_profile_ms(start: Option<Instant>) -> f64 {
    start
        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeBooleanOwnership {
    pub(crate) node_id: u32,
    pub(crate) piece_kind: RoadSurfaceVisualNodePieceKind,
    pub(crate) footprint_shapes: NodeOverlayShapes,
    pub(crate) asphalt_shapes: NodeOverlayShapes,
    pub(crate) non_road_shapes: NodeOverlayShapes,
    pub(crate) owned_regions: Vec<NodeBooleanOwnedRegion>,
    pub(crate) owned_region_arrangement: NodeOwnedRegionArrangement,
    pub(crate) carrier_provenance: NodeCarrierProvenanceClosure,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeOwnedRegionArrangement {
    node_id: u32,
    piece_kind: RoadSurfaceVisualNodePieceKind,
    region_count: usize,
    edges: Vec<NodeOwnedRegionArrangementEdge>,
    diagnostics: Vec<NodeOwnedRegionArrangementDiagnostic>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeOwnedRegionArrangementEdge {
    pub(crate) region_index: usize,
    pub(crate) owner: NodeBandOwner,
    pub(crate) opposite_owner: Option<NodeBandOwner>,
    pub(crate) start: NodeOwnedRegionArrangementKey,
    pub(crate) end: NodeOwnedRegionArrangementKey,
    pub(crate) seam_source: NodeSeamSource,
    pub(crate) source_constraint_indices: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NodeOwnedRegionArrangementDiagnostic {
    MissingSeamConstraint {
        region_index: usize,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
        start: NodeOwnedRegionArrangementKey,
        end: NodeOwnedRegionArrangementKey,
    },
    UnmaterializedRaisedStepAuthority {
        region_index: usize,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
        start: NodeOwnedRegionArrangementKey,
        end: NodeOwnedRegionArrangementKey,
        source_constraint_indices: Vec<usize>,
    },
    AmbiguousSeamConstraint {
        region_index: usize,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
        start: NodeOwnedRegionArrangementKey,
        end: NodeOwnedRegionArrangementKey,
    },
    AmbiguousOwnedBoundaryEdge {
        region_index: usize,
        owner: NodeBandOwner,
        opposite_owners: Vec<NodeBandOwner>,
        start: NodeOwnedRegionArrangementKey,
        end: NodeOwnedRegionArrangementKey,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct NodeOwnedRegionArrangementKey {
    x_key: i64,
    z_key: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeBooleanOwnedRegion {
    pub(crate) kind: RoadSurfaceBandKind,
    pub(crate) owner: NodeBandOwner,
    pub(crate) claim_priority: NodeGeneratedContourClaimPriority,
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: Option<usize>,
    pub(crate) shape: NodeOverlayShape,
    pub(crate) area_m2: f32,
    pub(crate) seam_constraints: Vec<NodeRegionSeamConstraint>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeCarrierProvenanceClosure {
    pub(crate) records: Vec<NodeCarrierProvenanceRecord>,
}

impl NodeCarrierProvenanceClosure {
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self {
            records: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct NodeCarrierProvenanceRecord {
    pub(crate) owner: NodeBandOwner,
    pub(crate) source_kind: RoadSurfaceBandKind,
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: usize,
    pub(crate) height_field_id: NodeBandHeightFieldId,
    pub(crate) claim_priority: NodeGeneratedContourClaimPriority,
    pub(crate) point: NodeOwnedRegionArrangementKey,
    pub(crate) origin: NodeCarrierProvenanceOrigin,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum NodeCarrierProvenanceOrigin {
    SourceVertex,
    SourceSegment {
        source_segment_id: NodeSourceCarrierSegmentId,
        canonical_point: NodeOwnedRegionArrangementKey,
        segment_start: NodeOwnedRegionArrangementKey,
        segment_end: NodeOwnedRegionArrangementKey,
        distance_key_units_sq: i64,
        dust_budget_key_units: i64,
    },
    SourceIntersection {
        peer_count: usize,
    },
    GeneratedCarrierVertex {
        contour_index: usize,
        purpose: NodeGeneratedContourPurpose,
        claim_priority: NodeGeneratedContourClaimPriority,
    },
    GeneratedCarrierSurface {
        contour_index: usize,
        purpose: NodeGeneratedContourPurpose,
        claim_priority: NodeGeneratedContourClaimPriority,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct NodeSourceCarrierSegmentId {
    pub(crate) owner: NodeBandOwner,
    pub(crate) source_kind: RoadSurfaceBandKind,
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: usize,
    pub(crate) segment_start: NodeOwnedRegionArrangementKey,
    pub(crate) segment_end: NodeOwnedRegionArrangementKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct NodeSourceSegmentAuthorizationCandidate {
    pub(crate) source_segment_id: NodeSourceCarrierSegmentId,
    pub(crate) source_kind: RoadSurfaceBandKind,
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: usize,
    pub(crate) canonical_point: NodeOwnershipPointKey,
    pub(crate) segment_start: NodeOwnershipPointKey,
    pub(crate) segment_end: NodeOwnershipPointKey,
    pub(crate) distance_key_units_sq: i64,
    pub(crate) dust_budget_key_units: i64,
}

fn source_carrier_segment_id(
    owner: NodeBandOwner,
    source: (RoadSurfaceBandKind, usize, usize),
    segment: OwnedRegionEdgeKey,
) -> NodeSourceCarrierSegmentId {
    NodeSourceCarrierSegmentId {
        owner,
        source_kind: source.0,
        source_mouth_order_index: source.1,
        source_band_index: source.2,
        segment_start: NodeOwnedRegionArrangementKey::from_ownership_key(segment.start),
        segment_end: NodeOwnedRegionArrangementKey::from_ownership_key(segment.end),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeBooleanOwnershipError {
    EmptyContourSet {
        node_id: u32,
    },
    EmptyFootprint {
        node_id: u32,
    },
    MissingBandOwner {
        mouth_order_index: usize,
        band_index: Option<usize>,
    },
    BooleanOperationFailed {
        stage: &'static str,
    },
    UnownedAsphaltResidual {
        shape_count: usize,
        area_m2: f32,
    },
    UnownedBandResidual {
        kind: RoadSurfaceBandKind,
        shape_count: usize,
        area_m2: f32,
    },
    UnownedNonRoadResidual {
        shape_count: usize,
        area_m2: f32,
    },
    AmbiguousCanonicalOwnedRegionVertex {
        owner: NodeBandOwner,
        point_x_key: i64,
        point_z_key: i64,
        candidates: Vec<NodeOwnershipPointKey>,
    },
    AmbiguousSourceSegmentAuthorizedOwnedRegionVertex {
        owner: NodeBandOwner,
        point_x_key: i64,
        point_z_key: i64,
        source_kind: RoadSurfaceBandKind,
        source_mouth_order_index: usize,
        source_band_index: usize,
        candidates: Vec<NodeSourceSegmentAuthorizationCandidate>,
    },
    MissingCarrierProvenance {
        owner: NodeBandOwner,
        point_x_key: i64,
        point_z_key: i64,
        source_kind: RoadSurfaceBandKind,
        source_mouth_order_index: usize,
        source_band_index: usize,
        height_field_id: NodeBandHeightFieldId,
    },
}

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn build_node_boolean_ownership_from_rails(
        rails: &NodeRailContourSet,
    ) -> Result<NodeBooleanOwnership, NodeBooleanOwnershipError> {
        NodeBooleanOwnership::from_rails(rails)
    }
}

impl NodeBooleanOwnership {
    pub(crate) fn from_rails(
        rails: &NodeRailContourSet,
    ) -> Result<Self, NodeBooleanOwnershipError> {
        if rails.contours.is_empty() {
            return Err(NodeBooleanOwnershipError::EmptyContourSet {
                node_id: rails.node_id,
            });
        }

        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let footprint_start = road_debug.then(Instant::now);
        let footprint_contours =
            overlay_contours_for_domains(rails, |contour| contour.contributes_to_footprint());
        let mut footprint_shapes = overlay_union(&footprint_contours, "footprint_union")?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut footprint_shapes);
        if footprint_shapes.is_empty() {
            return Err(NodeBooleanOwnershipError::EmptyFootprint {
                node_id: rails.node_id,
            });
        }
        let footprint_ms = elapsed_profile_ms(footprint_start);

        let constraint_overlap_mode = ConstraintOverlapMode::for_piece_kind(rails.piece_kind);
        let material_domain_start = road_debug.then(Instant::now);
        let asphalt_authority_domains = asphalt_authority_domains(rails);
        let asphalt_contours = asphalt_authority_domains
            .iter()
            .map(|domain| overlay_contour_from_domain(domain))
            .collect::<Vec<_>>();
        let asphalt_untrimmed_shapes = overlay_union(&asphalt_contours, "asphalt_union")?;
        let mut asphalt_raw_shapes = asphalt_raw_shapes_from_authority_domains(
            rails,
            &asphalt_authority_domains,
            &asphalt_untrimmed_shapes,
        )?;
        let footprint_corner_trim =
            footprint_corner_trim_shapes_for_piece(rails, &asphalt_untrimmed_shapes)?;
        if !footprint_corner_trim.shapes.is_empty() {
            footprint_shapes = overlay_difference(
                &footprint_shapes,
                &footprint_corner_trim.shapes,
                "footprint_corner_trim_difference",
            )?;
            RoadSurfaceSystem::sort_overlay_shapes(&mut footprint_shapes);
            if footprint_shapes.is_empty() {
                return Err(NodeBooleanOwnershipError::EmptyFootprint {
                    node_id: rails.node_id,
                });
            }
        }
        let asphalt_blocker_contours = asphalt_blocker_contours_for_material_priority(rails);
        if !asphalt_blocker_contours.is_empty() {
            let asphalt_blocker_shapes = overlay_union(
                &asphalt_blocker_contours,
                "asphalt_material_priority_blocker_union",
            )?;
            asphalt_raw_shapes = overlay_difference(
                &asphalt_raw_shapes,
                &asphalt_blocker_shapes,
                "asphalt_material_priority_blocker_difference",
            )?;
        }
        let mut asphalt_shapes = overlay_intersect(
            &asphalt_raw_shapes,
            &footprint_shapes,
            "asphalt_clip_to_footprint",
        )?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut asphalt_shapes);

        let mut non_road_shapes =
            overlay_difference(&footprint_shapes, &asphalt_shapes, "non_road_difference")?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut non_road_shapes);
        let material_domain_ms = elapsed_profile_ms(material_domain_start);
        let region_claim_start = road_debug.then(Instant::now);
        let mut owned_regions = Vec::new();
        let asphalt_owner_domains = asphalt_owner_domains(rails);
        let asphalt_result = owned_regions_from_domains(
            &asphalt_shapes,
            &asphalt_owner_domains,
            &rails.constraints,
            ResidualKind::Asphalt,
            constraint_overlap_mode,
        )?;
        owned_regions.extend(asphalt_result.regions);

        let non_road_result = split_non_road_regions(&non_road_shapes, rails)?;
        owned_regions.extend(non_road_result.regions);
        let non_road_residual = overlay_difference(
            &non_road_shapes,
            &non_road_result.claimed_shapes,
            "non_road_residual",
        )?;
        reject_residual(non_road_residual, ResidualKind::NonRoad)?;
        discard_unanchored_bend_asphalt_mouth_band_regions(rails, &mut owned_regions)?;
        let region_claim_ms = elapsed_profile_ms(region_claim_start);

        let canonical_cleanup_start = road_debug.then(Instant::now);
        sort_boolean_owned_regions(&mut owned_regions);
        canonicalize_owned_region_rings(&mut owned_regions, &footprint_shapes);
        let rail_canonical_points = canonical_points_for_rail_set(rails);
        clean_canonical_owned_region_shapes(
            &mut owned_regions,
            &footprint_shapes,
            &rails.constraints,
            &rail_canonical_points,
            constraint_overlap_mode,
            rails.piece_kind,
        )?;
        footprint_shapes =
            final_footprint_shapes_from_owned_regions(rails.node_id, &owned_regions)?;
        canonicalize_footprint_shapes_with_final_points(&mut footprint_shapes, &owned_regions);
        rebuild_owned_region_seam_constraints(
            &mut owned_regions,
            &footprint_shapes,
            &rails.constraints,
            constraint_overlap_mode,
            rails.piece_kind,
        );
        let mut owned_region_arrangement = NodeOwnedRegionArrangement::from_owned_regions(
            rails.node_id,
            rails.piece_kind,
            &owned_regions,
            &footprint_shapes,
            &rails.constraints,
        );
        let base_canonical_cleanup_ms = elapsed_profile_ms(canonical_cleanup_start);
        let sliver_promotion_start = road_debug.then(Instant::now);
        if promote_source_authorized_asphalt_adjacent_sidewalk_slivers(
            &mut owned_regions,
            &owned_region_arrangement,
            &rails.constraints,
        ) {
            footprint_shapes =
                final_footprint_shapes_from_owned_regions(rails.node_id, &owned_regions)?;
            canonicalize_footprint_shapes_with_final_points(&mut footprint_shapes, &owned_regions);
            rebuild_owned_region_seam_constraints(
                &mut owned_regions,
                &footprint_shapes,
                &rails.constraints,
                constraint_overlap_mode,
                rails.piece_kind,
            );
            owned_region_arrangement = NodeOwnedRegionArrangement::from_owned_regions(
                rails.node_id,
                rails.piece_kind,
                &owned_regions,
                &footprint_shapes,
                &rails.constraints,
            );
        }
        let sliver_promotion_ms = elapsed_profile_ms(sliver_promotion_start);
        let mut final_boundary_vertices_stable = false;
        let final_boundary_start = road_debug.then(Instant::now);
        let carrier_context = NodeCarrierProvenanceContext::new(rails, &rail_canonical_points);
        for _ in 0..8 {
            if !materialize_final_boundary_vertices_for_height(
                rails.node_id,
                &mut owned_regions,
                &mut footprint_shapes,
                &mut owned_region_arrangement,
                &carrier_context,
                &rails.constraints,
                constraint_overlap_mode,
                rails.piece_kind,
            )? {
                final_boundary_vertices_stable = true;
                break;
            }
        }
        if !final_boundary_vertices_stable {
            return Err(NodeBooleanOwnershipError::BooleanOperationFailed {
                stage: "final_boundary_vertex_materialization",
            });
        }
        let final_boundary_ms = elapsed_profile_ms(final_boundary_start);
        let dust_cleanup_start = road_debug.then(Instant::now);
        if discard_unprovenanced_numeric_dust_regions(&mut owned_regions, &carrier_context)? {
            footprint_shapes =
                final_footprint_shapes_from_owned_regions(rails.node_id, &owned_regions)?;
            canonicalize_footprint_shapes_with_final_points(&mut footprint_shapes, &owned_regions);
            rebuild_owned_region_seam_constraints(
                &mut owned_regions,
                &footprint_shapes,
                &rails.constraints,
                constraint_overlap_mode,
                rails.piece_kind,
            );
            owned_region_arrangement = NodeOwnedRegionArrangement::from_owned_regions(
                rails.node_id,
                rails.piece_kind,
                &owned_regions,
                &footprint_shapes,
                &rails.constraints,
            );
        }
        let dust_cleanup_ms = elapsed_profile_ms(dust_cleanup_start);
        let validation_start = road_debug.then(Instant::now);
        validate_non_road_regions_have_explicit_profile_seam_rails(
            &owned_regions,
            &rails.constraints,
        )?;
        let validation_ms = elapsed_profile_ms(validation_start);
        let carrier_provenance_start = road_debug.then(Instant::now);
        let carrier_provenance = NodeCarrierProvenanceClosure::from_owned_regions(
            &owned_regions,
            rails,
            &rail_canonical_points,
        )?;
        let carrier_provenance_ms = elapsed_profile_ms(carrier_provenance_start);
        asphalt_shapes = owned_region_shapes_matching(&owned_regions, |region| {
            region.kind == RoadSurfaceBandKind::Carriageway
        })?;
        non_road_shapes = owned_region_shapes_matching(&owned_regions, |region| {
            region.kind != RoadSurfaceBandKind::Carriageway
        })?;
        if road_debug {
            let total_ms = elapsed_profile_ms(total_start);
            if total_ms >= 20.0 {
                crate::debug_log!(
                    "road",
                    "node_ownership_detail node={} kind={:?} contours={} constraints={} footprint_shapes={} asphalt_shapes={} non_road_shapes={} owned_regions={} arrangement_edges={} diagnostics={} footprint_ms={:.3} material_domain_ms={:.3} region_claim_ms={:.3} base_canonical_cleanup_ms={:.3} sliver_promotion_ms={:.3} final_boundary_ms={:.3} dust_cleanup_ms={:.3} validation_ms={:.3} carrier_provenance_ms={:.3} total_ms={:.3}",
                    rails.node_id,
                    rails.piece_kind,
                    rails.contours.len(),
                    rails.constraints.len(),
                    footprint_shapes.len(),
                    asphalt_shapes.len(),
                    non_road_shapes.len(),
                    owned_regions.len(),
                    owned_region_arrangement.edges.len(),
                    owned_region_arrangement.diagnostics.len(),
                    footprint_ms,
                    material_domain_ms,
                    region_claim_ms,
                    base_canonical_cleanup_ms,
                    sliver_promotion_ms,
                    final_boundary_ms,
                    dust_cleanup_ms,
                    validation_ms,
                    carrier_provenance_ms,
                    total_ms
                );
            }
        }
        Ok(Self {
            node_id: rails.node_id,
            piece_kind: rails.piece_kind,
            footprint_shapes,
            asphalt_shapes,
            non_road_shapes,
            owned_regions,
            owned_region_arrangement,
            carrier_provenance,
        })
    }
}

fn promote_source_authorized_asphalt_adjacent_sidewalk_slivers(
    owned_regions: &mut [NodeBooleanOwnedRegion],
    arrangement: &NodeOwnedRegionArrangement,
    rail_constraints: &[NodeRailConstraint],
) -> bool {
    let curb_region_sources = curb_region_sources_by_owner(owned_regions);
    let mut promotions = BTreeMap::<usize, (NodeBandOwner, usize, Option<usize>)>::new();
    for diagnostic in arrangement.diagnostics() {
        let NodeOwnedRegionArrangementDiagnostic::MissingSeamConstraint {
            region_index,
            owner,
            opposite_owner,
            start,
            end,
        } = diagnostic
        else {
            continue;
        };
        if owner.kind() != RoadSurfaceBandKind::Sidewalk
            || opposite_owner.kind() != RoadSurfaceBandKind::Carriageway
        {
            continue;
        }
        let Some(region) = owned_regions.get(*region_index) else {
            continue;
        };
        let Some(curb_source) = source_authorized_curb_owner_for_sidewalk_asphalt_sliver(
            region,
            *owner,
            *start,
            *end,
            rail_constraints,
        ) else {
            continue;
        };
        match promotions.get(region_index).copied() {
            Some(existing) if existing != curb_source => {
                promotions.remove(region_index);
            }
            Some(_) => {}
            None => {
                promotions.insert(*region_index, curb_source);
            }
        }
    }

    let mut changed = false;
    for (region_index, (curb_owner, source_mouth_order_index, source_band_index)) in promotions {
        let Some(region) = owned_regions.get_mut(region_index) else {
            continue;
        };
        let (source_mouth_order_index, source_band_index) = if source_band_index.is_some() {
            (source_mouth_order_index, source_band_index)
        } else if let Some(source) = curb_region_sources.get(&curb_owner).copied() {
            source
        } else {
            continue;
        };
        region.kind = RoadSurfaceBandKind::CurbOrShoulder;
        region.owner = curb_owner;
        region.source_mouth_order_index = source_mouth_order_index;
        region.source_band_index = source_band_index;
        changed = true;
    }
    changed
}

fn curb_region_sources_by_owner(
    owned_regions: &[NodeBooleanOwnedRegion],
) -> BTreeMap<NodeBandOwner, (usize, Option<usize>)> {
    let mut sources = BTreeMap::new();
    for region in owned_regions {
        if region.owner.kind() != RoadSurfaceBandKind::CurbOrShoulder
            || region.source_band_index.is_none()
        {
            continue;
        }
        sources
            .entry(region.owner)
            .or_insert((region.source_mouth_order_index, region.source_band_index));
    }
    sources
}

fn source_authorized_curb_owner_for_sidewalk_asphalt_sliver(
    region: &NodeBooleanOwnedRegion,
    sidewalk_owner: NodeBandOwner,
    start: NodeOwnedRegionArrangementKey,
    end: NodeOwnedRegionArrangementKey,
    rail_constraints: &[NodeRailConstraint],
) -> Option<(NodeBandOwner, usize, Option<usize>)> {
    let edge_start = (start.x_key, start.z_key);
    let edge_end = (end.x_key, end.z_key);
    let mut candidates = region
        .seam_constraints
        .iter()
        .filter(|constraint| constraint.is_material_transition)
        .filter(|constraint| {
            matches!(
                constraint.seam_source,
                NodeSeamSource::RaisedStepContact { .. }
            )
        })
        .filter_map(|constraint| {
            let curb_owner = raised_step_curb_owner_for_sidewalk_constraint(
                constraint.owner,
                constraint.opposite_owner,
                sidewalk_owner,
            )?;
            let constraint_start = ownership_key_from_road_point(constraint.start_xz);
            let constraint_end = ownership_key_from_road_point(constraint.end_xz);
            let touches_missing_edge =
                point_key_lies_on_segment(edge_start, constraint_start, constraint_end)
                    || point_key_lies_on_segment(edge_end, constraint_start, constraint_end)
                    || point_key_lies_on_segment(constraint_start, edge_start, edge_end)
                    || point_key_lies_on_segment(constraint_end, edge_start, edge_end);
            if !touches_missing_edge {
                return None;
            }
            let source = rail_constraints
                .iter()
                .find(|rail_constraint| {
                    rail_constraint.constraint_index == constraint.constraint_index
                        && rail_constraint.kind == NodeRailConstraintKind::RaisedStepContact
                        && [rail_constraint.owner, rail_constraint.opposite_owner]
                            .contains(&Some(curb_owner))
                })
                .map(|rail_constraint| {
                    (
                        rail_constraint.source_mouth_order_index,
                        rail_constraint.source_band_index,
                    )
                })
                .unwrap_or((region.source_mouth_order_index, None));
            Some((curb_owner, source.0, source.1))
        })
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    candidates.dedup();
    (candidates.len() == 1).then(|| candidates[0])
}

fn raised_step_curb_owner_for_sidewalk_constraint(
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
    sidewalk_owner: NodeBandOwner,
) -> Option<NodeBandOwner> {
    match (owner, opposite_owner) {
        (Some(left), Some(right))
            if left == sidewalk_owner && right.kind() == RoadSurfaceBandKind::CurbOrShoulder =>
        {
            Some(right)
        }
        (Some(left), Some(right))
            if right == sidewalk_owner && left.kind() == RoadSurfaceBandKind::CurbOrShoulder =>
        {
            Some(left)
        }
        _ => None,
    }
}

fn discard_unprovenanced_numeric_dust_regions(
    owned_regions: &mut Vec<NodeBooleanOwnedRegion>,
    carrier_context: &NodeCarrierProvenanceContext<'_>,
) -> Result<bool, NodeBooleanOwnershipError> {
    let mut changed = false;
    let mut retained = Vec::with_capacity(owned_regions.len());
    for region in owned_regions.drain(..) {
        // A sub-cap backend sliver that cannot prove a carrier is residual overlay dust, not a
        // renderable top surface. Larger unsupported regions still fail in carrier closure below.
        if region.area_m2 <= crate::simulation::network::surface::NODE_OVERLAY_NUMERIC_AREA_CAP_M2
            && carrier_context.region_has_missing_provenance(&region)?
        {
            changed = true;
            continue;
        }
        retained.push(region);
    }
    *owned_regions = retained;
    Ok(changed)
}

fn materialize_final_boundary_vertices_for_height(
    node_id: u32,
    owned_regions: &mut Vec<NodeBooleanOwnedRegion>,
    footprint_shapes: &mut NodeOverlayShapes,
    owned_region_arrangement: &mut NodeOwnedRegionArrangement,
    carrier_context: &NodeCarrierProvenanceContext<'_>,
    rail_constraints: &[NodeRailConstraint],
    constraint_overlap_mode: ConstraintOverlapMode,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> Result<bool, NodeBooleanOwnershipError> {
    let previous_region_points = owned_region_point_keys(owned_regions);
    let regions_changed = materialize_final_footprint_vertices_in_owned_regions(
        owned_regions,
        footprint_shapes,
        owned_region_arrangement,
        carrier_context,
    )?;
    if !regions_changed {
        return Ok(false);
    }
    *footprint_shapes = final_footprint_shapes_from_owned_regions(node_id, owned_regions)?;
    canonicalize_footprint_shapes_with_final_points(footprint_shapes, owned_regions);
    rebuild_owned_region_seam_constraints(
        owned_regions,
        footprint_shapes,
        rail_constraints,
        constraint_overlap_mode,
        piece_kind,
    );
    *owned_region_arrangement = NodeOwnedRegionArrangement::from_owned_regions(
        node_id,
        piece_kind,
        owned_regions,
        footprint_shapes,
        rail_constraints,
    );
    Ok(owned_region_point_keys(owned_regions) != previous_region_points)
}

fn owned_region_point_keys(owned_regions: &[NodeBooleanOwnedRegion]) -> Vec<NodeOwnershipPointKey> {
    let mut points = owned_regions
        .iter()
        .flat_map(|region| region.shape.iter())
        .flat_map(|contour| contour.iter().copied())
        .map(ownership_key_from_overlay_point)
        .collect::<Vec<_>>();
    points.sort_unstable();
    points.dedup();
    points
}

fn protected_bend_corner_trim_shapes(
    rails: &NodeRailContourSet,
) -> Result<NodeOverlayShapes, NodeBooleanOwnershipError> {
    let mut protected_trim_shapes = Vec::new();
    for trim in &rails.corner_trims {
        let trim_contour = overlay_contour_for_corner_trim(trim);
        let mut trim_shapes = overlay_union(&[trim_contour], "footprint_corner_trim_single_union")?;
        let protected_contours = bend_side_join_protection_contours_for_corner_trim(rails, trim);
        if !protected_contours.is_empty() {
            let protected_shapes = overlay_union(
                &protected_contours,
                "footprint_corner_trim_side_join_protection_union",
            )?;
            trim_shapes = overlay_difference(
                &trim_shapes,
                &protected_shapes,
                "footprint_corner_trim_side_join_protection_difference",
            )?;
        }
        protected_trim_shapes = overlay_union_shape_sets(
            &protected_trim_shapes,
            &trim_shapes,
            "footprint_corner_trim_protected_union",
        )?;
    }
    Ok(protected_trim_shapes)
}

struct FootprintCornerTrimShapes {
    shapes: NodeOverlayShapes,
}

fn footprint_corner_trim_shapes_for_piece(
    rails: &NodeRailContourSet,
    asphalt_untrimmed_shapes: &NodeOverlayShapes,
) -> Result<FootprintCornerTrimShapes, NodeBooleanOwnershipError> {
    let shapes = match rails.piece_kind {
        RoadSurfaceVisualNodePieceKind::Bend => {
            let protected_shapes = protected_bend_corner_trim_shapes(rails)?;
            if protected_shapes.is_empty() {
                Vec::new()
            } else {
                overlay_difference(
                    &protected_shapes,
                    asphalt_untrimmed_shapes,
                    "footprint_corner_trim_clip_asphalt",
                )?
            }
        }
        RoadSurfaceVisualNodePieceKind::JunctionN => junction_exterior_corner_trim_shapes(rails)?,
        RoadSurfaceVisualNodePieceKind::Terminal => Vec::new(),
    };
    Ok(FootprintCornerTrimShapes { shapes })
}

fn junction_exterior_corner_trim_shapes(
    rails: &NodeRailContourSet,
) -> Result<NodeOverlayShapes, NodeBooleanOwnershipError> {
    let mut trim_shapes = Vec::new();
    for trim in rails
        .corner_trims
        .iter()
        .filter(|trim| corner_trim_source_is_exterior_gap(rails, trim.source_mouth_order_index))
    {
        let trim_contour = overlay_contour_for_corner_trim(trim);
        let single_shapes = overlay_union(&[trim_contour], "junction_exterior_corner_trim_union")?;
        trim_shapes = overlay_union_shape_sets(
            &trim_shapes,
            &single_shapes,
            "junction_exterior_corner_trim_shape_union",
        )?;
    }
    Ok(trim_shapes)
}

fn corner_trim_source_is_exterior_gap(
    rails: &NodeRailContourSet,
    source_mouth_order_index: usize,
) -> bool {
    rails.side_join_gaps.iter().any(|gap| {
        gap.from_mouth_order_index == source_mouth_order_index
            && gap.role == super::joins::NodeInputSideJoinGapRole::Exterior
    })
}

fn overlay_contour_for_corner_trim(
    trim: &super::rails::NodeGeneratedCornerTrim,
) -> NodeOverlayContour {
    trim.points_xz
        .iter()
        .copied()
        .map(road_vec2_to_overlay_point)
        .collect()
}

fn bend_side_join_protection_contours_for_corner_trim(
    rails: &NodeRailContourSet,
    trim: &super::rails::NodeGeneratedCornerTrim,
) -> Vec<NodeOverlayContour> {
    rails
        .contours
        .iter()
        .filter(|contour| {
            contour.purpose == NodeGeneratedContourPurpose::BendSideJoin
                && contour.source_mouth_order_index == trim.source_mouth_order_index
        })
        .map(overlay_contour_from_domain)
        .collect()
}

fn asphalt_raw_shapes_from_authority_domains(
    rails: &NodeRailContourSet,
    asphalt_authority_domains: &[&super::rails::NodeGeneratedContour],
    asphalt_untrimmed_shapes: &NodeOverlayShapes,
) -> Result<NodeOverlayShapes, NodeBooleanOwnershipError> {
    let Some(side_join_purpose) = asphalt_side_join_purpose_for_trim(rails.piece_kind) else {
        return Ok(asphalt_untrimmed_shapes.clone());
    };

    let mut side_join_contours = Vec::new();
    let mut other_contours = Vec::new();
    for domain in asphalt_authority_domains {
        let contour = overlay_contour_from_domain(domain);
        if domain.purpose == side_join_purpose {
            side_join_contours.push(contour);
        } else {
            other_contours.push(contour);
        }
    }

    if side_join_contours.is_empty() {
        return Ok(asphalt_untrimmed_shapes.clone());
    }

    let mut side_join_shapes = overlay_union(&side_join_contours, "asphalt_side_join_union")?;
    let blocker_contours = non_road_side_join_contours_for_asphalt_trim(rails);
    if !blocker_contours.is_empty() {
        let blocker_shapes =
            overlay_union(&blocker_contours, "asphalt_side_join_non_road_trim_union")?;
        side_join_shapes = overlay_difference(
            &side_join_shapes,
            &blocker_shapes,
            "asphalt_side_join_non_road_trim_difference",
        )?;
    }

    let other_shapes = if other_contours.is_empty() {
        Vec::new()
    } else {
        overlay_union(&other_contours, "asphalt_non_side_join_union")?
    };
    overlay_union_shape_sets(
        &other_shapes,
        &side_join_shapes,
        "asphalt_side_join_reunion",
    )
}

fn asphalt_side_join_purpose_for_trim(
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> Option<NodeGeneratedContourPurpose> {
    match piece_kind {
        RoadSurfaceVisualNodePieceKind::Bend => Some(NodeGeneratedContourPurpose::BendSideJoin),
        RoadSurfaceVisualNodePieceKind::JunctionN => {
            Some(NodeGeneratedContourPurpose::JunctionSideJoin)
        }
        RoadSurfaceVisualNodePieceKind::Terminal => None,
    }
}

fn asphalt_blocker_contours_for_material_priority(
    rails: &NodeRailContourSet,
) -> Vec<NodeOverlayContour> {
    match rails.piece_kind {
        RoadSurfaceVisualNodePieceKind::Bend => non_road_side_join_contours_for_asphalt_trim(rails),
        RoadSurfaceVisualNodePieceKind::JunctionN => {
            non_road_side_join_contours_for_asphalt_trim(rails)
        }
        RoadSurfaceVisualNodePieceKind::Terminal => Vec::new(),
    }
}

fn discard_unanchored_bend_asphalt_mouth_band_regions(
    rails: &NodeRailContourSet,
    owned_regions: &mut Vec<NodeBooleanOwnedRegion>,
) -> Result<bool, NodeBooleanOwnershipError> {
    if rails.piece_kind != RoadSurfaceVisualNodePieceKind::Bend {
        return Ok(false);
    }

    let anchor_shapes = bend_asphalt_side_join_anchor_shapes(rails)?;
    if anchor_shapes.is_empty() {
        return Ok(false);
    }
    let asphalt_component_shapes = owned_region_shapes_matching(owned_regions, |region| {
        region.kind == RoadSurfaceBandKind::Carriageway
    })?;
    let mut unanchored_component_shapes = Vec::new();
    for shape in asphalt_component_shapes {
        if !overlay_shape_intersects_anchor(&shape, &anchor_shapes)? {
            unanchored_component_shapes.push(shape);
        }
    }
    if unanchored_component_shapes.is_empty() {
        return Ok(false);
    }
    RoadSurfaceSystem::sort_overlay_shapes(&mut unanchored_component_shapes);

    let before_count = owned_regions.len();
    let mut retained_regions = Vec::with_capacity(owned_regions.len());
    for region in owned_regions.drain(..) {
        if region.kind != RoadSurfaceBandKind::Carriageway
            || region.claim_priority != NodeGeneratedContourClaimPriority::MouthBand
            || !overlay_shape_intersects_anchor(&region.shape, &unanchored_component_shapes)?
        {
            retained_regions.push(region);
        }
    }
    let removed = retained_regions.len() != before_count;
    *owned_regions = retained_regions;
    Ok(removed)
}

fn bend_asphalt_side_join_anchor_shapes(
    rails: &NodeRailContourSet,
) -> Result<NodeOverlayShapes, NodeBooleanOwnershipError> {
    let anchor_contours = rails
        .contours
        .iter()
        .filter(|contour| {
            contour.contributes_to_asphalt()
                && contour.purpose == NodeGeneratedContourPurpose::BendSideJoin
        })
        .map(overlay_contour_from_domain)
        .collect::<Vec<_>>();
    if anchor_contours.is_empty() {
        return Ok(Vec::new());
    }

    let mut anchor_shapes = overlay_union(&anchor_contours, "bend_asphalt_anchor_union")?;
    let blocker_contours = non_road_side_join_contours_for_asphalt_trim(rails);
    if !blocker_contours.is_empty() {
        let blocker_shapes =
            overlay_union(&blocker_contours, "bend_asphalt_anchor_non_road_trim_union")?;
        anchor_shapes = overlay_difference(
            &anchor_shapes,
            &blocker_shapes,
            "bend_asphalt_anchor_non_road_trim_difference",
        )?;
    }
    RoadSurfaceSystem::sort_overlay_shapes(&mut anchor_shapes);
    Ok(anchor_shapes)
}

fn overlay_shape_intersects_anchor(
    shape: &NodeOverlayShape,
    anchor_shapes: &NodeOverlayShapes,
) -> Result<bool, NodeBooleanOwnershipError> {
    let shape_set = vec![shape.clone()];
    let overlap = overlay_intersect(
        &shape_set,
        anchor_shapes,
        "bend_asphalt_anchor_intersection",
    )?;
    let overlap_area_m2 = overlap
        .iter()
        .map(RoadSurfaceSystem::overlay_shape_area_m2)
        .sum::<f32>();
    Ok(overlap_area_m2 > RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(&overlap))
}

fn owned_region_shapes_matching(
    owned_regions: &[NodeBooleanOwnedRegion],
    predicate: impl Fn(&NodeBooleanOwnedRegion) -> bool,
) -> Result<NodeOverlayShapes, NodeBooleanOwnershipError> {
    let contours = owned_regions
        .iter()
        .filter(|region| predicate(region))
        .flat_map(|region| region.shape.iter().cloned())
        .collect::<Vec<_>>();
    if contours.is_empty() {
        return Ok(Vec::new());
    }
    overlay_union(&contours, "owned_region_material_shape_union")
}

fn non_road_side_join_contours_for_asphalt_trim(
    rails: &NodeRailContourSet,
) -> Vec<NodeOverlayContour> {
    rails
        .contours
        .iter()
        .filter(|contour| {
            contour.contributes_to_non_road_band()
                && matches!(
                    contour.purpose,
                    NodeGeneratedContourPurpose::BendSideJoin
                        | NodeGeneratedContourPurpose::JunctionSideJoin
                )
        })
        .map(overlay_contour_from_domain)
        .collect()
}

fn final_footprint_shapes_from_owned_regions(
    node_id: u32,
    owned_regions: &[NodeBooleanOwnedRegion],
) -> Result<NodeOverlayShapes, NodeBooleanOwnershipError> {
    let contours = owned_regions
        .iter()
        .flat_map(|region| region.shape.iter().cloned())
        .collect::<Vec<_>>();
    if contours.is_empty() {
        return Err(NodeBooleanOwnershipError::EmptyFootprint { node_id });
    }
    overlay_union(&contours, "final_owned_footprint_union")
}

fn rebuild_owned_region_seam_constraints(
    owned_regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
    rail_constraints: &[NodeRailConstraint],
    constraint_overlap_mode: ConstraintOverlapMode,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) {
    for region in owned_regions.iter_mut() {
        region.seam_constraints = seam_constraints_for_shape(
            &region.shape,
            region.owner,
            rail_constraints,
            constraint_overlap_mode,
        );
    }
    materialize_noded_region_seam_constraints(
        owned_regions,
        footprint_shapes,
        rail_constraints,
        piece_kind,
    );
}

fn materialize_final_footprint_vertices_in_owned_regions(
    owned_regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
    _arrangement: &NodeOwnedRegionArrangement,
    carrier_context: &NodeCarrierProvenanceContext<'_>,
) -> Result<bool, NodeBooleanOwnershipError> {
    let final_footprint_points = final_footprint_point_keys(footprint_shapes);
    let final_footprint_edges = final_footprint_edge_keys(footprint_shapes);
    if final_footprint_points.is_empty() {
        return Ok(false);
    }
    let mut changed = false;
    for region in owned_regions.iter_mut() {
        let region_owner = region.owner;
        let region_kind = region.kind;
        let region_source_mouth_order_index = region.source_mouth_order_index;
        let region_source_band_index = region.source_band_index;
        let region_claim_priority = region.claim_priority;
        for contour in &mut region.shape {
            let materialized = materialized_contour_with_final_footprint_points(
                contour,
                region_owner,
                region_kind,
                region_source_mouth_order_index,
                region_source_band_index,
                region_claim_priority,
                &final_footprint_edges,
                carrier_context,
            )?;
            if contour_point_keys(&materialized) == contour_point_keys(contour) {
                continue;
            }
            *contour = materialized;
            changed = true;
        }
    }
    Ok(changed)
}

fn final_footprint_point_keys(footprint_shapes: &NodeOverlayShapes) -> Vec<NodeOwnershipPointKey> {
    let mut points = footprint_shapes
        .iter()
        .flat_map(|shape| shape.iter())
        .flat_map(|contour| contour.iter().copied())
        .map(ownership_key_from_overlay_point)
        .collect::<Vec<_>>();
    points.sort_unstable();
    points.dedup();
    points
}

fn final_footprint_edge_keys(
    footprint_shapes: &NodeOverlayShapes,
) -> Vec<(NodeOwnershipPointKey, NodeOwnershipPointKey)> {
    let mut edges = Vec::new();
    for contour in footprint_shapes
        .iter()
        .flat_map(|shape| shape.iter())
        .filter(|contour| contour.len() >= 2)
    {
        for index in 0..contour.len() {
            let start = ownership_key_from_overlay_point(contour[index]);
            let end = ownership_key_from_overlay_point(contour[(index + 1) % contour.len()]);
            if start != end {
                edges.push((start, end));
            }
        }
    }
    edges
}

fn materialized_contour_with_final_footprint_points(
    contour: &NodeOverlayContour,
    region_owner: NodeBandOwner,
    region_kind: RoadSurfaceBandKind,
    region_source_mouth_order_index: usize,
    region_source_band_index: Option<usize>,
    region_claim_priority: NodeGeneratedContourClaimPriority,
    final_footprint_edges: &[(NodeOwnershipPointKey, NodeOwnershipPointKey)],
    carrier_context: &NodeCarrierProvenanceContext<'_>,
) -> Result<NodeOverlayContour, NodeBooleanOwnershipError> {
    if contour.len() < 2 {
        return Ok(contour.clone());
    }
    let mut materialized = Vec::with_capacity(contour.len());
    for edge_index in 0..contour.len() {
        let start = ownership_key_from_overlay_point(contour[edge_index]);
        let end = ownership_key_from_overlay_point(contour[(edge_index + 1) % contour.len()]);
        if start == end {
            continue;
        }
        let mut edge_points = supported_final_footprint_boundary_points_for_edge(
            start,
            end,
            final_footprint_edges,
            region_owner,
            region_kind,
            region_source_mouth_order_index,
            region_source_band_index,
            region_claim_priority,
            carrier_context,
        )?;
        if edge_points.is_empty() {
            materialized.push(start);
            continue;
        }
        edge_points.sort_by_key(|point| segment_parameter_key(start, end, *point));
        edge_points.dedup();
        materialized.push(start);
        materialized.extend(edge_points);
    }
    dedup_materialized_contour(&mut materialized);
    Ok(materialized
        .into_iter()
        .map(overlay_point_from_key)
        .collect())
}

fn supported_final_footprint_boundary_points_for_edge(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    final_footprint_edges: &[(NodeOwnershipPointKey, NodeOwnershipPointKey)],
    region_owner: NodeBandOwner,
    region_kind: RoadSurfaceBandKind,
    region_source_mouth_order_index: usize,
    region_source_band_index: Option<usize>,
    region_claim_priority: NodeGeneratedContourClaimPriority,
    carrier_context: &NodeCarrierProvenanceContext<'_>,
) -> Result<Vec<NodeOwnershipPointKey>, NodeBooleanOwnershipError> {
    let mut points = Vec::new();
    for (edge_start, edge_end) in final_footprint_edges {
        if !footprint_edges_overlap(start, end, *edge_start, *edge_end) {
            continue;
        }
        points.extend(
            [*edge_start, *edge_end]
                .into_iter()
                .filter(|point| *point != start && *point != end)
                .filter(|point| point_key_lies_on_segment(*point, start, end)),
        );
    }
    points.sort_by_key(|point| segment_parameter_key(start, end, *point));
    points.dedup();
    let mut supported = Vec::with_capacity(points.len());
    for point in points {
        if carrier_context
            .origin_for_owned_source_point(
                region_owner,
                region_kind,
                region_source_mouth_order_index,
                region_source_band_index,
                region_claim_priority,
                point,
            )?
            .is_some()
        {
            supported.push(point);
        }
    }
    Ok(supported)
}

fn footprint_edges_overlap(
    left_start: NodeOwnershipPointKey,
    left_end: NodeOwnershipPointKey,
    right_start: NodeOwnershipPointKey,
    right_end: NodeOwnershipPointKey,
) -> bool {
    point_key_lies_on_segment(left_start, right_start, right_end)
        || point_key_lies_on_segment(left_end, right_start, right_end)
        || point_key_lies_on_segment(right_start, left_start, left_end)
        || point_key_lies_on_segment(right_end, left_start, left_end)
}

fn dedup_materialized_contour(points: &mut Vec<NodeOwnershipPointKey>) {
    points.dedup_by(|a, b| ownership_mm_key(*a) == ownership_mm_key(*b));
    if points.len() >= 2 && points.first().copied() == points.last().copied() {
        points.pop();
    }
    if points.len() >= 2
        && ownership_mm_key(points[0])
            == ownership_mm_key(*points.last().expect("materialized contour has last point"))
    {
        points.pop();
    }
}

fn contour_point_keys(contour: &NodeOverlayContour) -> Vec<NodeOwnershipPointKey> {
    contour
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect()
}

fn canonicalize_footprint_shapes_with_final_points(
    footprint_shapes: &mut NodeOverlayShapes,
    owned_regions: &[NodeBooleanOwnedRegion],
) {
    let mut canonical_points_by_mm = BTreeMap::<NodeOwnershipPointKey, Vec<_>>::new();
    for point in owned_regions
        .iter()
        .flat_map(|region| region.shape.iter())
        .flat_map(|contour| contour.iter().copied())
        .map(ownership_key_from_overlay_point)
    {
        canonical_points_by_mm
            .entry(ownership_mm_key(point))
            .or_default()
            .push(point);
    }
    for candidates in canonical_points_by_mm.values_mut() {
        candidates.sort_unstable();
        candidates.dedup();
    }
    let owned_boundary_edges = owned_region_boundary_edge_keys(owned_regions);

    for shape in &mut *footprint_shapes {
        for contour in shape {
            let contour_keys = contour_point_keys(contour);
            let mut canonical_contour = Vec::with_capacity(contour_keys.len());
            for index in 0..contour_keys.len() {
                let key = contour_keys[index];
                if let Some(canonical) = canonical_footprint_point(key, &canonical_points_by_mm) {
                    canonical_contour.push(canonical);
                    continue;
                }
                let previous = contour_keys[if index == 0 {
                    contour_keys.len() - 1
                } else {
                    index - 1
                }];
                let next = contour_keys[(index + 1) % contour_keys.len()];
                if let Some(path) = canonical_footprint_point_path(
                    key,
                    previous,
                    next,
                    &canonical_points_by_mm,
                    &owned_boundary_edges,
                ) {
                    canonical_contour.extend(path);
                } else {
                    canonical_contour.push(key);
                }
            }
            dedup_canonical_footprint_contour(&mut canonical_contour);
            *contour = canonical_contour
                .into_iter()
                .map(overlay_point_from_key)
                .collect();
        }
    }
    RoadSurfaceSystem::sort_overlay_shapes(footprint_shapes);
}

fn owned_region_boundary_edge_keys(
    owned_regions: &[NodeBooleanOwnedRegion],
) -> BTreeSet<OwnedRegionEdgeKey> {
    let mut edges = BTreeSet::new();
    for contour in owned_regions
        .iter()
        .flat_map(|region| region.shape.iter())
        .filter(|contour| contour.len() >= 2)
    {
        for index in 0..contour.len() {
            let start = ownership_key_from_overlay_point(contour[index]);
            let end = ownership_key_from_overlay_point(contour[(index + 1) % contour.len()]);
            if start != end {
                edges.insert(OwnedRegionEdgeKey::new(start, end));
            }
        }
    }
    edges
}

fn canonical_footprint_point_path(
    point: NodeOwnershipPointKey,
    previous: NodeOwnershipPointKey,
    next: NodeOwnershipPointKey,
    canonical_points_by_mm: &BTreeMap<NodeOwnershipPointKey, Vec<NodeOwnershipPointKey>>,
    owned_boundary_edges: &BTreeSet<OwnedRegionEdgeKey>,
) -> Option<Vec<NodeOwnershipPointKey>> {
    let candidates = canonical_points_by_mm.get(&ownership_mm_key(point))?;
    if candidates.len() < 2 {
        return None;
    }
    let previous = canonical_footprint_point(previous, canonical_points_by_mm).unwrap_or(previous);
    let next = canonical_footprint_point(next, canonical_points_by_mm).unwrap_or(next);
    let mut nodes = candidates.clone();
    nodes.push(previous);
    nodes.push(next);
    nodes.sort_unstable();
    nodes.dedup();

    let mut paths = Vec::new();
    let mut path = vec![previous];
    collect_owned_boundary_paths(
        previous,
        next,
        &nodes,
        owned_boundary_edges,
        &mut path,
        &mut paths,
    );
    let min_len = paths.iter().map(Vec::len).min()?;
    let mut shortest = paths
        .into_iter()
        .filter(|path| path.len() == min_len)
        .collect::<Vec<_>>();
    shortest.sort_unstable();
    shortest.dedup();
    if shortest.len() != 1 {
        return supported_canonical_footprint_cluster(
            candidates,
            previous,
            next,
            owned_boundary_edges,
        );
    }
    let mut path = shortest.pop().expect("unique shortest path is present");
    if path.len() < 3 {
        return None;
    }
    path.remove(0);
    path.pop();
    if path.is_empty() || path.iter().all(|candidate| candidates.contains(candidate)) {
        Some(path)
    } else {
        supported_canonical_footprint_cluster(candidates, previous, next, owned_boundary_edges)
    }
}

fn supported_canonical_footprint_cluster(
    candidates: &[NodeOwnershipPointKey],
    previous: NodeOwnershipPointKey,
    next: NodeOwnershipPointKey,
    owned_boundary_edges: &BTreeSet<OwnedRegionEdgeKey>,
) -> Option<Vec<NodeOwnershipPointKey>> {
    if candidates.len() < 2 {
        return None;
    }
    let mut support_nodes = candidates.to_vec();
    support_nodes.push(previous);
    support_nodes.push(next);
    support_nodes.sort_unstable();
    support_nodes.dedup();
    if candidates.iter().copied().all(|candidate| {
        support_nodes.iter().copied().any(|peer| {
            peer != candidate
                && owned_boundary_edges.contains(&OwnedRegionEdgeKey::new(candidate, peer))
        })
    }) {
        Some(candidates.to_vec())
    } else {
        None
    }
}

fn collect_owned_boundary_paths(
    current: NodeOwnershipPointKey,
    target: NodeOwnershipPointKey,
    nodes: &[NodeOwnershipPointKey],
    owned_boundary_edges: &BTreeSet<OwnedRegionEdgeKey>,
    path: &mut Vec<NodeOwnershipPointKey>,
    paths: &mut Vec<Vec<NodeOwnershipPointKey>>,
) {
    if current == target {
        paths.push(path.clone());
        return;
    }
    if path.len() > nodes.len() {
        return;
    }
    for next in nodes.iter().copied() {
        if path.contains(&next)
            || !owned_boundary_edges.contains(&OwnedRegionEdgeKey::new(current, next))
        {
            continue;
        }
        path.push(next);
        collect_owned_boundary_paths(next, target, nodes, owned_boundary_edges, path, paths);
        path.pop();
    }
}

fn dedup_canonical_footprint_contour(points: &mut Vec<NodeOwnershipPointKey>) {
    points.dedup();
    if points.len() >= 2 && points.first().copied() == points.last().copied() {
        points.pop();
    }
}

fn canonical_footprint_point(
    point: NodeOwnershipPointKey,
    canonical_points_by_mm: &BTreeMap<NodeOwnershipPointKey, Vec<NodeOwnershipPointKey>>,
) -> Option<NodeOwnershipPointKey> {
    let candidates = canonical_points_by_mm.get(&ownership_mm_key(point))?;
    if candidates.binary_search(&point).is_ok() {
        return Some(point);
    }
    if candidates.len() == 1 {
        Some(candidates[0])
    } else {
        nearest_unique_canonical_footprint_point(point, candidates)
    }
}

fn nearest_unique_canonical_footprint_point(
    point: NodeOwnershipPointKey,
    candidates: &[NodeOwnershipPointKey],
) -> Option<NodeOwnershipPointKey> {
    let mut best = None;
    for candidate in candidates.iter().copied() {
        let distance_sq = ownership_key_distance_sq(point, candidate);
        match best {
            None => best = Some((distance_sq, candidate)),
            Some((best_distance_sq, _)) if distance_sq < best_distance_sq => {
                best = Some((distance_sq, candidate));
            }
            Some((best_distance_sq, _)) if distance_sq == best_distance_sq => return None,
            Some(_) => {}
        }
    }
    best.map(|(_, candidate)| candidate)
}

fn ownership_key_distance_sq(left: NodeOwnershipPointKey, right: NodeOwnershipPointKey) -> i128 {
    let dx = i128::from(left.0 - right.0);
    let dz = i128::from(left.1 - right.1);
    dx * dx + dz * dz
}

impl NodeOwnedRegionArrangement {
    pub(crate) fn node_id(&self) -> u32 {
        self.node_id
    }

    pub(crate) fn piece_kind(&self) -> RoadSurfaceVisualNodePieceKind {
        self.piece_kind
    }

    pub(crate) fn region_count(&self) -> usize {
        self.region_count
    }

    pub(crate) fn edges(&self) -> &[NodeOwnedRegionArrangementEdge] {
        &self.edges
    }

    pub(crate) fn diagnostics(&self) -> &[NodeOwnedRegionArrangementDiagnostic] {
        &self.diagnostics
    }
}

#[cfg(test)]
mod tests;
