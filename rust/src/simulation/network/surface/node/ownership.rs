//! Boolean ownership solve for canonical node-arrangement contours.

use super::arrangement::{NodeBandOwner, NodeRegionSeamConstraint, NodeSeamSource};
use super::rails::{NodeGeneratedContourClaimPriority, NodeRailContourSet};
use super::{
    NodeOverlayShape, NodeOverlayShapes, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceVisualNodePieceKind,
};

mod boundaries;
mod contact_semantics;
mod domains;
mod rail_authority;
mod rings;
mod seams;
mod topology_keys;

use domains::{
    ResidualKind, asphalt_authority_domains, asphalt_owner_domains, overlay_contour_from_domain,
    overlay_contours_for_domains, overlay_difference, overlay_intersect, overlay_union,
    owned_regions_from_domains, reject_residual, sort_boolean_owned_regions,
    split_non_road_regions, validate_non_road_regions_have_explicit_profile_seam_rails,
};

use rings::{canonicalize_owned_region_rings, clean_canonical_owned_region_shapes};

use rail_authority::canonical_points_for_rail_set;

use seams::{
    ConstraintOverlapMode, materialize_noded_region_seam_constraints, seam_constraints_for_shape,
};
use topology_keys::NodeOwnershipPointKey;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeBooleanOwnership {
    pub(crate) node_id: u32,
    pub(crate) piece_kind: RoadSurfaceVisualNodePieceKind,
    pub(crate) footprint_shapes: NodeOverlayShapes,
    pub(crate) asphalt_shapes: NodeOverlayShapes,
    pub(crate) non_road_shapes: NodeOverlayShapes,
    pub(crate) owned_regions: Vec<NodeBooleanOwnedRegion>,
    pub(crate) owned_region_arrangement: NodeOwnedRegionArrangement,
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
    NonCanonicalOwnedRegionVertex {
        owner: NodeBandOwner,
        point_x_key: i64,
        point_z_key: i64,
        canonical_x_key: i64,
        canonical_z_key: i64,
    },
    AmbiguousCanonicalOwnedRegionVertex {
        owner: NodeBandOwner,
        point_x_key: i64,
        point_z_key: i64,
        candidates: Vec<NodeOwnershipPointKey>,
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

        let footprint_contours =
            overlay_contours_for_domains(rails, |contour| contour.contributes_to_footprint());
        let mut footprint_shapes = overlay_union(&footprint_contours, "footprint_union")?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut footprint_shapes);
        if footprint_shapes.is_empty() {
            return Err(NodeBooleanOwnershipError::EmptyFootprint {
                node_id: rails.node_id,
            });
        }

        let asphalt_authority_domains = asphalt_authority_domains(rails);
        let asphalt_contours = asphalt_authority_domains
            .iter()
            .map(|domain| overlay_contour_from_domain(domain))
            .collect::<Vec<_>>();
        let asphalt_raw_shapes = overlay_union(&asphalt_contours, "asphalt_union")?;
        let mut asphalt_shapes = overlay_intersect(
            &asphalt_raw_shapes,
            &footprint_shapes,
            "asphalt_clip_to_footprint",
        )?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut asphalt_shapes);

        let mut non_road_shapes =
            overlay_difference(&footprint_shapes, &asphalt_shapes, "non_road_difference")?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut non_road_shapes);

        let constraint_overlap_mode = ConstraintOverlapMode::for_piece_kind(rails.piece_kind);
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

        sort_boolean_owned_regions(&mut owned_regions);
        canonicalize_owned_region_rings(&mut owned_regions, &footprint_shapes);
        let rail_canonical_points = canonical_points_for_rail_set(rails);
        clean_canonical_owned_region_shapes(
            &mut owned_regions,
            &footprint_shapes,
            &rails.constraints,
            &rail_canonical_points,
            constraint_overlap_mode,
        )?;
        for region in &mut owned_regions {
            region.seam_constraints = seam_constraints_for_shape(
                &region.shape,
                region.owner,
                &rails.constraints,
                constraint_overlap_mode,
            );
        }
        materialize_noded_region_seam_constraints(
            &mut owned_regions,
            &footprint_shapes,
            &rails.constraints,
            rails.piece_kind,
        );
        validate_non_road_regions_have_explicit_profile_seam_rails(
            &owned_regions,
            &rails.constraints,
        )?;
        let owned_region_arrangement = NodeOwnedRegionArrangement::from_owned_regions(
            rails.node_id,
            rails.piece_kind,
            &owned_regions,
            &footprint_shapes,
            &rails.constraints,
        );
        Ok(Self {
            node_id: rails.node_id,
            piece_kind: rails.piece_kind,
            footprint_shapes,
            asphalt_shapes,
            non_road_shapes,
            owned_regions,
            owned_region_arrangement,
        })
    }
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
