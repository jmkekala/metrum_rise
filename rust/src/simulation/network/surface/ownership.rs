//! Boolean ownership solve for canonical node-arrangement contours.

use super::arrangement::{NodeBandOwner, NodeRegionSeamConstraint, NodeSeamSource};
use super::backend::RoadVec2;
use super::band_semantics::{
    raised_step_kinds_can_contact, raised_step_requires_exact_constraint_span,
};
use super::keys::{SurfaceXzKey, SurfaceXzSegmentKey};
use super::rails::{NodeGeneratedContourClaimPriority, NodeRailContourSet};
use super::segments::{
    key_collinear_with_overlay_grid_segment, key_collinear_with_segment, raw_tuple_key,
    raw_tuple_key_lies_exactly_on_segment, raw_tuple_key_lies_on_segment,
    raw_tuple_segment_parameter_key,
};
use super::{
    NodeOverlayPoint, NodeOverlayShape, NodeOverlayShapes, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceVisualNodePieceKind,
};
use std::collections::{BTreeMap, BTreeSet};

mod boundaries;
mod domains;
mod rings;
mod seams;

use domains::{
    ResidualKind, asphalt_authority_domains, asphalt_owner_domains, overlay_contour_from_domain,
    overlay_contours_for_domains, overlay_difference, overlay_intersect, overlay_union,
    owned_regions_from_domains, reject_residual, sort_boolean_owned_regions,
    split_non_road_regions, validate_non_road_regions_have_explicit_profile_seam_rails,
};

use rings::{
    canonical_points_for_rail_set, canonicalize_owned_region_rings,
    clean_canonical_owned_region_shapes,
};

use seams::{materialize_noded_region_seam_constraints, seam_constraints_for_shape};

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
}

struct NodeRailCanonicalPointSet {
    all_points: Vec<NodeOwnershipPointKey>,
    points_by_owner: BTreeMap<NodeBandOwner, Vec<NodeOwnershipPointKey>>,
    segments_by_owner: BTreeMap<NodeBandOwner, Vec<OwnedRegionEdgeKey>>,
    canonical_points_by_mm_key_by_owner:
        BTreeMap<NodeBandOwner, BTreeMap<NodeOwnershipPointKey, BTreeSet<NodeOwnershipPointKey>>>,
    height_points_by_source:
        BTreeMap<(RoadSurfaceBandKind, usize, usize), Vec<NodeOwnershipPointKey>>,
    paths_by_owner: BTreeMap<NodeBandOwner, Vec<Vec<NodeOwnershipPointKey>>>,
}

impl RoadSurfaceSystem {
    pub(super) fn build_node_boolean_ownership_from_rails(
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

        let allow_grid_bounded_constraint_overlap =
            rails.piece_kind == RoadSurfaceVisualNodePieceKind::Terminal;
        let mut owned_regions = Vec::new();
        let asphalt_owner_domains = asphalt_owner_domains(rails);
        let asphalt_result = owned_regions_from_domains(
            &asphalt_shapes,
            &asphalt_owner_domains,
            &rails.constraints,
            ResidualKind::Asphalt,
            allow_grid_bounded_constraint_overlap,
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
            allow_grid_bounded_constraint_overlap,
        )?;
        for region in &mut owned_regions {
            region.seam_constraints = seam_constraints_for_shape(
                &region.shape,
                region.owner,
                &rails.constraints,
                allow_grid_bounded_constraint_overlap,
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

impl NodeOwnedRegionArrangementKey {
    #[cfg(test)]
    pub(crate) fn from_point(point: RoadVec2) -> Self {
        Self::from_ownership_key(ownership_key_from_road_point(point))
    }

    fn from_ownership_key(point: NodeOwnershipPointKey) -> Self {
        Self {
            x_key: point.0,
            z_key: point.1,
        }
    }

    pub(crate) fn x_mm(self) -> i64 {
        ownership_coordinate_key_to_mm(self.x_key)
    }

    pub(crate) fn z_mm(self) -> i64 {
        ownership_coordinate_key_to_mm(self.z_key)
    }
}

fn canonical_source_indices(sources: impl IntoIterator<Item = usize>) -> Vec<usize> {
    let mut sources = sources.into_iter().collect::<Vec<_>>();
    sources.sort_unstable();
    sources.dedup();
    sources
}

fn segment_parameter_key(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    point: NodeOwnershipPointKey,
) -> i128 {
    raw_tuple_segment_parameter_key(start, end, point)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct OwnedRegionEdgeKey {
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
}

impl OwnedRegionEdgeKey {
    fn new(a: NodeOwnershipPointKey, b: NodeOwnershipPointKey) -> Self {
        let segment = SurfaceXzSegmentKey::new(
            SurfaceXzKey::from_raw_tuple(a),
            SurfaceXzKey::from_raw_tuple(b),
        );
        Self {
            start: segment.start().raw_tuple(),
            end: segment.end().raw_tuple(),
        }
    }
}

fn road_point_from_key(point: NodeOwnershipPointKey) -> RoadVec2 {
    SurfaceXzKey::from_raw_tuple(point).to_road_xz()
}

fn overlay_point_from_key(point: NodeOwnershipPointKey) -> NodeOverlayPoint {
    let point = SurfaceXzKey::from_raw_tuple(point).to_road_xz();
    [point.x, point.y]
}

fn ownership_mm_key(point: NodeOwnershipPointKey) -> NodeOwnershipPointKey {
    (
        ownership_coordinate_key_to_mm(point.0),
        ownership_coordinate_key_to_mm(point.1),
    )
}

fn point_key_collinear_with_edge(
    point: NodeOwnershipPointKey,
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
) -> bool {
    key_collinear_with_segment(
        raw_tuple_key(point),
        raw_tuple_key(edge_start),
        raw_tuple_key(edge_end),
    )
}

fn point_key_collinear_with_edge_on_overlay_grid(
    point: NodeOwnershipPointKey,
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
) -> bool {
    key_collinear_with_overlay_grid_segment(
        raw_tuple_key(point),
        raw_tuple_key(edge_start),
        raw_tuple_key(edge_end),
    )
}

fn point_key_lies_on_segment(
    point: NodeOwnershipPointKey,
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
) -> bool {
    raw_tuple_key_lies_on_segment(point, start, end)
}

fn point_key_lies_exactly_on_segment(
    point: NodeOwnershipPointKey,
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
) -> bool {
    raw_tuple_key_lies_exactly_on_segment(point, start, end)
}

pub(crate) type NodeOwnershipPointKey = (i64, i64);

fn ownership_coordinate_key_to_mm(value: i64) -> i64 {
    SurfaceXzKey::coordinate_key_to_mm(value)
}

fn ownership_key_from_overlay_point(point: NodeOverlayPoint) -> NodeOwnershipPointKey {
    SurfaceXzKey::from_overlay_point(point).raw_tuple()
}

fn ownership_key_from_road_point(point: RoadVec2) -> NodeOwnershipPointKey {
    SurfaceXzKey::from_road_xz(point).raw_tuple()
}

fn owners_form_raised_step_contact(owner: NodeBandOwner, opposite_owner: NodeBandOwner) -> bool {
    raised_step_kinds_can_contact(owner.kind(), opposite_owner.kind())
}

fn raised_step_contact_requires_exact_constraint_span(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    raised_step_requires_exact_constraint_span(owner.kind(), opposite_owner.kind())
}

fn raised_step_contact_constrains_shared_height(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    owners_form_raised_step_contact(owner, opposite_owner)
        && !raised_step_contact_requires_exact_constraint_span(owner, opposite_owner)
}

#[cfg(test)]
mod tests;
