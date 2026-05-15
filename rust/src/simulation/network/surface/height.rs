//! Explicit height-carrier evaluation for canonical node-owned regions.

use super::arrangement::{
    NodeBandHeightFieldId, NodeBandOwner, NodeRegionSeamConstraint, seam_source_priority,
};
use super::backend::{
    ROAD_OVERLAY_COORDINATE_SCALE, RoadVec2, RoadVec3, overlay_point_to_road,
    quantize_road_vec2_to_overlay_grid,
};
use super::input::{NodeArrangementInput, NodeInputBandInterval};
use super::ownership::{NodeBooleanOwnedRegion, NodeBooleanOwnership};
use super::rails::{
    NodeGeneratedContour, NodeGeneratedContourClaimPriority, NodeGeneratedContourKind,
    NodeGeneratedContourPurpose, NodeRailContourSet,
};
use super::terminal::{
    NodeTerminalCapBand, TerminalCapGenerationError, terminal_cap_bands_by_mouth,
};
use super::{
    NodeOverlayContour, NodeOverlayPoint, NodeOverlayShape, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceVisualNodePieceKind, SurfaceCdt,
};
use spade::{Point2, Triangulation};
use std::collections::BTreeMap;

const HEIGHT_POINT_KEY_SCALE: f64 = 1000.0;
const HEIGHT_SHARED_KEY_SCALE: f64 = 1000.0;
const HEIGHT_SOURCE_KEY_SCALE: f64 = ROAD_OVERLAY_COORDINATE_SCALE;
const HEIGHT_SOURCE_EDGE_NEIGHBOR_UNITS: i128 = 8192;
const SAME_MATERIAL_SHARED_EDGE_HEIGHT_CANONICAL_EPS_M: f64 = 0.01;
const EXPLICIT_MATERIAL_SEAM_HEIGHT_CANONICAL_EPS_M: f64 = 0.01;
type NodeHeightedContour = Vec<NodeHeightedVertex>;
type NodeHeightedShape = Vec<NodeHeightedContour>;
type NodeHeightSourcePointKey = (i64, i64);

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeHeightSolution {
    pub(crate) node_id: u32,
    pub(crate) piece_kind: RoadSurfaceVisualNodePieceKind,
    pub(crate) regions: Vec<NodeHeightedRegion>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeHeightedRegion {
    pub(crate) kind: RoadSurfaceBandKind,
    pub(crate) owner: NodeBandOwner,
    pub(crate) height_field_id: NodeBandHeightFieldId,
    pub(crate) shape: NodeHeightedShape,
    pub(crate) area_m2: f32,
    pub(crate) seam_constraints: Vec<NodeRegionSeamConstraint>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeHeightedVertex {
    pub(crate) point_xz: RoadVec2,
    pub(crate) height_m: f64,
    pub(crate) height_field_id: NodeBandHeightFieldId,
    pub(crate) height_authority: Option<NodeHeightAuthoritySource>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeHeightFieldError {
    InputOwnershipMismatch {
        input_node_id: u32,
        ownership_node_id: u32,
        input_piece_kind: RoadSurfaceVisualNodePieceKind,
        ownership_piece_kind: RoadSurfaceVisualNodePieceKind,
    },
    DuplicateSourceBand {
        mouth_order_index: usize,
        band_index: usize,
    },
    MissingRegionBandIndex {
        mouth_order_index: usize,
        kind: RoadSurfaceBandKind,
    },
    MissingSourceBand {
        mouth_order_index: usize,
        band_index: usize,
    },
    SourceBandKindMismatch {
        mouth_order_index: usize,
        band_index: usize,
        region_kind: RoadSurfaceBandKind,
        source_kind: RoadSurfaceBandKind,
    },
    VertexOutsideHeightField {
        mouth_order_index: usize,
        band_index: usize,
        source_kind: RoadSurfaceBandKind,
        height_field_id: NodeBandHeightFieldId,
        owner: Option<NodeBandOwner>,
        point_x_mm: i64,
        point_z_mm: i64,
        axis: &'static str,
        raw_parameter: f64,
    },
    SourceHeightFieldConflict {
        mouth_order_index: usize,
        band_index: usize,
        source_kind: RoadSurfaceBandKind,
        height_field_id: NodeBandHeightFieldId,
        owner: Option<NodeBandOwner>,
        existing_authority: NodeHeightAuthoritySource,
        incoming_authority: NodeHeightAuthoritySource,
        point_x_mm: i64,
        point_z_mm: i64,
        existing_height_mm: i64,
        incoming_height_mm: i64,
    },
    SharedSourceHeightConflict {
        point_x_mm: i64,
        point_z_mm: i64,
        kind: RoadSurfaceBandKind,
        owner: NodeBandOwner,
        opposite_owner: Option<NodeBandOwner>,
        height_field_id: Option<NodeBandHeightFieldId>,
        incoming_owner: NodeBandOwner,
        incoming_height_field_id: Option<NodeBandHeightFieldId>,
        constraint_index: Option<usize>,
        existing_authority: Option<NodeHeightAuthoritySource>,
        incoming_authority: Option<NodeHeightAuthoritySource>,
        existing_height_mm: i64,
        incoming_height_mm: i64,
    },
    TerminalCapGeneration {
        error: TerminalCapGenerationError,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NodeHeightAuthoritySource {
    SourceInterval,
    TerminalCap,
    GeneratedContour {
        purpose: NodeGeneratedContourPurpose,
        claim_priority: NodeGeneratedContourClaimPriority,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeHeightPointKey {
    x_key: i64,
    z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeSourceBandKey {
    mouth_order_index: usize,
    band_index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeHeightVertexContextKey {
    point: NodeHeightPointKey,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ExplicitSeamHeightKey {
    point: NodeHeightPointKey,
    constraint_index: usize,
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
    start: NodeHeightPointKey,
    end: NodeHeightPointKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeResolvedHeightAuthorityKey {
    point: NodeHeightPointKey,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    claim_priority: NodeGeneratedContourClaimPriority,
}

struct NodeResolvedHeightAuthorityMap {
    heights_by_key: BTreeMap<NodeResolvedHeightAuthorityKey, NodeResolvedHeightAuthority>,
}

#[derive(Clone, Copy, Debug)]
struct NodeResolvedHeightAuthority {
    point_xz: RoadVec2,
    height_m: f64,
    authority: NodeHeightAuthoritySource,
}

struct NodeBandHeightField {
    id: NodeBandHeightFieldId,
    kind: RoadSurfaceBandKind,
    patches: Vec<NodeBandHeightPatch>,
}

struct NodeBandHeightPatch {
    authority: NodeHeightPatchAuthority,
    triangles: Option<Vec<NodeBandHeightTriangle>>,
    contour_edges: Option<Vec<NodeBandHeightEdge>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NodeHeightPatchAuthority {
    owner: Option<NodeBandOwner>,
    role: NodeHeightPatchAuthorityRole,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NodeHeightPatchAuthorityRole {
    SourceInterval,
    TerminalCap,
    GeneratedContour {
        purpose: NodeGeneratedContourPurpose,
        claim_priority: NodeGeneratedContourClaimPriority,
    },
}

struct NodeBandHeightTriangle {
    a_xz: RoadVec2,
    b_xz: RoadVec2,
    c_xz: RoadVec2,
    a_height_m: f64,
    b_height_m: f64,
    c_height_m: f64,
}

struct NodeBandHeightEdge {
    start_xz: RoadVec2,
    end_xz: RoadVec2,
    start_height_m: f64,
    end_height_m: f64,
}

#[derive(Clone, Copy, Debug)]
struct NodeAuthorizedHeightCandidate {
    authority_rank: u8,
    authority: NodeHeightAuthoritySource,
    height_m: f64,
}

#[derive(Clone, Copy, Debug)]
struct NodeEvaluatedHeight {
    height_m: f64,
    authority: NodeHeightAuthoritySource,
}

enum NodeHeightPatchEvaluation {
    Inside(f64),
    Outside(NodeHeightFieldError),
}

impl NodeHeightPatchAuthority {
    fn source_interval() -> Self {
        Self {
            owner: None,
            role: NodeHeightPatchAuthorityRole::SourceInterval,
        }
    }

    fn terminal_cap() -> Self {
        Self {
            owner: None,
            role: NodeHeightPatchAuthorityRole::TerminalCap,
        }
    }

    fn generated_contour(contour: &NodeGeneratedContour) -> Self {
        Self {
            owner: contour.owner,
            role: NodeHeightPatchAuthorityRole::GeneratedContour {
                purpose: contour.purpose,
                claim_priority: contour.claim_priority,
            },
        }
    }

    fn rank_for_owned_region(
        self,
        owner: NodeBandOwner,
        claim_priority: NodeGeneratedContourClaimPriority,
    ) -> Option<u8> {
        if let Some(authority_owner) = self.owner
            && authority_owner != owner
        {
            return None;
        }
        Some(match self.role {
            NodeHeightPatchAuthorityRole::SourceInterval => 1,
            NodeHeightPatchAuthorityRole::TerminalCap => 2,
            NodeHeightPatchAuthorityRole::GeneratedContour {
                claim_priority: authority_claim_priority,
                ..
            } => {
                if authority_claim_priority != claim_priority {
                    return None;
                }
                3
            }
        })
    }

    fn source(self) -> NodeHeightAuthoritySource {
        match self.role {
            NodeHeightPatchAuthorityRole::SourceInterval => {
                NodeHeightAuthoritySource::SourceInterval
            }
            NodeHeightPatchAuthorityRole::TerminalCap => NodeHeightAuthoritySource::TerminalCap,
            NodeHeightPatchAuthorityRole::GeneratedContour {
                purpose,
                claim_priority,
            } => NodeHeightAuthoritySource::GeneratedContour {
                purpose,
                claim_priority,
            },
        }
    }
}

impl RoadSurfaceSystem {
    pub(super) fn build_node_height_solution_from_ownership(
        input: &NodeArrangementInput,
        rails: &NodeRailContourSet,
        ownership: &NodeBooleanOwnership,
    ) -> Result<NodeHeightSolution, NodeHeightFieldError> {
        NodeHeightSolution::from_ownership_input_and_rails(input, Some(rails), ownership)
    }
}

impl NodeHeightSolution {
    #[cfg(test)]
    pub(crate) fn from_ownership_and_input(
        input: &NodeArrangementInput,
        ownership: &NodeBooleanOwnership,
    ) -> Result<Self, NodeHeightFieldError> {
        Self::from_ownership_input_and_rails(input, None, ownership)
    }

    fn from_ownership_input_and_rails(
        input: &NodeArrangementInput,
        rails: Option<&NodeRailContourSet>,
        ownership: &NodeBooleanOwnership,
    ) -> Result<Self, NodeHeightFieldError> {
        validate_input_ownership_pair(input, ownership)?;
        let fields = height_fields_by_source(input, rails)?;
        let mut regions = Vec::with_capacity(ownership.owned_regions.len());
        let resolved_authority = (ownership.piece_kind
            == RoadSurfaceVisualNodePieceKind::JunctionN)
            .then(|| NodeResolvedHeightAuthorityMap::from_ownership(ownership, &fields))
            .transpose()?;

        for region in &ownership.owned_regions {
            let region = heighted_region(region, &fields, resolved_authority.as_ref())?;
            if !region.shape.is_empty() {
                regions.push(region);
            }
        }
        if ownership.piece_kind == RoadSurfaceVisualNodePieceKind::JunctionN {
            apply_junctionn_same_owner_height_field_vertex_unification(&mut regions);
            apply_junctionn_same_material_shared_edge_height_tiebreak(&mut regions);
            apply_junctionn_same_material_vertex_height_tiebreak(&mut regions);
            apply_junctionn_explicit_material_seam_height_unification(&mut regions);
        }
        validate_explicit_material_seam_heights(&regions)?;
        validate_shared_source_height_agreement(&regions)?;

        Ok(Self {
            node_id: ownership.node_id,
            piece_kind: ownership.piece_kind,
            regions,
        })
    }
}

impl NodeBandHeightField {
    fn from_interval(mouth_order_index: usize, interval: &NodeInputBandInterval) -> Self {
        let id =
            NodeBandHeightFieldId::new(mouth_order_index, interval.band_index, interval.band_kind);
        Self {
            id,
            kind: interval.band_kind,
            patches: vec![NodeBandHeightPatch::from_interval(interval)],
        }
    }

    fn from_terminal_cap_band(mouth_order_index: usize, cap_band: &NodeTerminalCapBand) -> Self {
        let id = NodeBandHeightFieldId::new(
            mouth_order_index,
            cap_band.source_band_index,
            cap_band.band_kind,
        );
        Self {
            id,
            kind: cap_band.band_kind,
            patches: vec![NodeBandHeightPatch::from_terminal_cap_band(cap_band)],
        }
    }

    fn extend_with_terminal_cap_band(
        &mut self,
        mouth_order_index: usize,
        cap_band: &NodeTerminalCapBand,
    ) -> Result<(), NodeHeightFieldError> {
        if cap_band.band_kind != self.kind {
            return Err(NodeHeightFieldError::SourceBandKindMismatch {
                mouth_order_index,
                band_index: cap_band.source_band_index,
                region_kind: self.kind,
                source_kind: cap_band.band_kind,
            });
        }
        self.patches
            .push(NodeBandHeightPatch::from_terminal_cap_band(cap_band));
        Ok(())
    }

    fn extend_with_generated_contour(
        &mut self,
        contour: &NodeGeneratedContour,
        allow_missing_height_points_backfill: bool,
    ) -> Result<(), NodeHeightFieldError> {
        if let Some(patch) = NodeBandHeightPatch::from_generated_contour(
            contour,
            self,
            allow_missing_height_points_backfill,
        )? {
            self.patches.push(patch);
        }
        Ok(())
    }

    fn evaluate_height(&self, point_xz: RoadVec2) -> Result<f64, NodeHeightFieldError> {
        let mut candidates = Vec::new();
        let mut outside_error = None;
        for patch in &self.patches {
            match patch.evaluate_surface_height(self.id, self.kind, point_xz)? {
                NodeHeightPatchEvaluation::Inside(height_m) => {
                    candidates.push(NodeAuthorizedHeightCandidate {
                        authority_rank: 0,
                        authority: patch.authority.source(),
                        height_m,
                    });
                }
                NodeHeightPatchEvaluation::Outside(error) => {
                    if outside_error.is_none() {
                        outside_error = Some(error);
                    }
                }
            }
        }
        if candidates.is_empty() {
            return Err(outside_error.unwrap_or_else(|| {
                let key = NodeHeightPointKey::from_point(point_xz);
                NodeHeightFieldError::VertexOutsideHeightField {
                    mouth_order_index: self.id.mouth_order_index(),
                    band_index: self.id.band_index(),
                    source_kind: self.kind,
                    height_field_id: self.id,
                    owner: None,
                    point_x_mm: key.x_mm(),
                    point_z_mm: key.z_mm(),
                    axis: "patch",
                    raw_parameter: f64::NAN,
                }
            }));
        }

        self.agreed_height(point_xz, None, candidates)
            .map(|height| height.height_m)
    }

    fn evaluate_authorized_height(
        &self,
        owner: NodeBandOwner,
        claim_priority: NodeGeneratedContourClaimPriority,
        point_xz: RoadVec2,
    ) -> Result<NodeEvaluatedHeight, NodeHeightFieldError> {
        let mut candidates = Vec::new();
        let mut outside_error = None;
        for patch in &self.patches {
            let Some(authority_rank) = patch.authority.rank_for_owned_region(owner, claim_priority)
            else {
                continue;
            };
            if let Some(height_m) = patch.source_handoff_height_at(point_xz) {
                candidates.push(NodeAuthorizedHeightCandidate {
                    authority_rank: 4,
                    authority: patch.authority.source(),
                    height_m,
                });
                continue;
            }
            match patch.evaluate_surface_height(self.id, self.kind, point_xz)? {
                NodeHeightPatchEvaluation::Inside(height_m) => {
                    candidates.push(NodeAuthorizedHeightCandidate {
                        authority_rank,
                        authority: patch.authority.source(),
                        height_m,
                    });
                }
                NodeHeightPatchEvaluation::Outside(error) => {
                    if outside_error.is_none() {
                        outside_error = Some(owner_scoped_outside_height_error(error, owner));
                    }
                }
            }
        }
        if candidates.is_empty() {
            if let Some(error) = outside_error {
                return Err(error);
            }
            let key = NodeHeightPointKey::from_point(point_xz);
            return Err(NodeHeightFieldError::VertexOutsideHeightField {
                mouth_order_index: self.id.mouth_order_index(),
                band_index: self.id.band_index(),
                source_kind: self.kind,
                height_field_id: self.id,
                owner: Some(owner),
                point_x_mm: key.x_mm(),
                point_z_mm: key.z_mm(),
                axis: "canonical_authority",
                raw_parameter: f64::NAN,
            });
        }

        let best_rank = candidates
            .iter()
            .map(|candidate| candidate.authority_rank)
            .max()
            .expect("non-empty candidate set has a maximum rank");
        let heights_m = candidates
            .into_iter()
            .filter(|candidate| candidate.authority_rank == best_rank)
            .collect();
        self.agreed_height(point_xz, Some(owner), heights_m)
    }

    fn agreed_height(
        &self,
        point_xz: RoadVec2,
        owner: Option<NodeBandOwner>,
        candidates: Vec<NodeAuthorizedHeightCandidate>,
    ) -> Result<NodeEvaluatedHeight, NodeHeightFieldError> {
        let Some(first_candidate) = candidates.first().copied() else {
            let key = NodeHeightPointKey::from_point(point_xz);
            return Err(NodeHeightFieldError::VertexOutsideHeightField {
                mouth_order_index: self.id.mouth_order_index(),
                band_index: self.id.band_index(),
                source_kind: self.kind,
                height_field_id: self.id,
                owner,
                point_x_mm: key.x_mm(),
                point_z_mm: key.z_mm(),
                axis: "patch",
                raw_parameter: f64::NAN,
            });
        };
        let first_height_mm = quantize_m(first_candidate.height_m);
        for candidate in candidates.iter().copied().skip(1) {
            let height_mm = quantize_m(candidate.height_m);
            if height_mm != first_height_mm {
                let key = NodeHeightPointKey::from_point(point_xz);
                return Err(NodeHeightFieldError::SourceHeightFieldConflict {
                    mouth_order_index: self.id.mouth_order_index(),
                    band_index: self.id.band_index(),
                    source_kind: self.kind,
                    height_field_id: self.id,
                    owner,
                    existing_authority: first_candidate.authority,
                    incoming_authority: candidate.authority,
                    point_x_mm: key.x_mm(),
                    point_z_mm: key.z_mm(),
                    existing_height_mm: first_height_mm,
                    incoming_height_mm: height_mm,
                });
            }
        }
        Ok(NodeEvaluatedHeight {
            height_m: first_candidate.height_m,
            authority: first_candidate.authority,
        })
    }
}

fn owner_scoped_outside_height_error(
    error: NodeHeightFieldError,
    owner: NodeBandOwner,
) -> NodeHeightFieldError {
    match error {
        NodeHeightFieldError::VertexOutsideHeightField {
            mouth_order_index,
            band_index,
            source_kind,
            height_field_id,
            owner: error_owner,
            point_x_mm,
            point_z_mm,
            axis,
            raw_parameter,
        } => NodeHeightFieldError::VertexOutsideHeightField {
            mouth_order_index,
            band_index,
            source_kind,
            height_field_id,
            owner: error_owner.or(Some(owner)),
            point_x_mm,
            point_z_mm,
            axis,
            raw_parameter,
        },
        other => other,
    }
}

impl NodeBandHeightPatch {
    fn from_interval(interval: &NodeInputBandInterval) -> Self {
        Self {
            authority: NodeHeightPatchAuthority::source_interval(),
            triangles: Some(interval_height_triangles(interval)),
            contour_edges: Some(interval_height_edges(interval)),
        }
    }

    fn from_terminal_cap_band(cap_band: &NodeTerminalCapBand) -> Self {
        Self {
            authority: NodeHeightPatchAuthority::terminal_cap(),
            triangles: Some(terminal_cap_band_height_triangles(cap_band)),
            contour_edges: Some(terminal_cap_band_height_edges(cap_band)),
        }
    }

    fn from_generated_contour(
        contour: &NodeGeneratedContour,
        base: &NodeBandHeightField,
        allow_missing_height_points_backfill: bool,
    ) -> Result<Option<Self>, NodeHeightFieldError> {
        if let Some(points_world) = &contour.height_points_world {
            return Ok(Some(Self::from_heighted_contour(
                points_world,
                NodeHeightPatchAuthority::generated_contour(contour),
            )));
        }
        if !allow_missing_height_points_backfill {
            return Ok(None);
        }
        let mut points = Vec::with_capacity(contour.points_xz.len());
        for point_xz in &contour.points_xz {
            let point_xz = quantize_road_vec2_to_overlay_grid(*point_xz);
            let height_m = base.evaluate_height(point_xz)?;
            points.push(RoadVec3::new(point_xz.x, height_m, point_xz.y));
        }
        Ok(Some(Self::from_heighted_contour(
            &points,
            NodeHeightPatchAuthority::generated_contour(contour),
        )))
    }

    fn from_heighted_contour(points: &[RoadVec3], authority: NodeHeightPatchAuthority) -> Self {
        Self {
            authority,
            triangles: Some(height_triangles_from_contour(points)),
            contour_edges: Some(height_edges_from_vertices(points)),
        }
    }

    fn source_handoff_height_at(&self, point_xz: RoadVec2) -> Option<f64> {
        if self.authority.role != NodeHeightPatchAuthorityRole::SourceInterval {
            return None;
        }
        self.contour_edges
            .as_ref()
            .and_then(|edges| terminal_edge_height_at(point_xz, edges))
    }

    fn evaluate_surface_height(
        &self,
        id: NodeBandHeightFieldId,
        source_kind: RoadSurfaceBandKind,
        point_xz: RoadVec2,
    ) -> Result<NodeHeightPatchEvaluation, NodeHeightFieldError> {
        if let Some(edges) = &self.contour_edges
            && let Some(height_m) = terminal_edge_height_at(point_xz, edges)
        {
            return Ok(NodeHeightPatchEvaluation::Inside(height_m));
        }
        let mut triangle_outside_error = None;
        if let Some(triangles) = &self.triangles {
            match self.evaluate_triangle_surface_height(id, source_kind, point_xz, triangles)? {
                NodeHeightPatchEvaluation::Inside(height_m) => {
                    return Ok(NodeHeightPatchEvaluation::Inside(height_m));
                }
                NodeHeightPatchEvaluation::Outside(error) => {
                    triangle_outside_error = Some(error);
                }
            }
        }
        Ok(NodeHeightPatchEvaluation::Outside(
            triangle_outside_error.unwrap_or_else(|| {
                self.outside_field_error(id, source_kind, point_xz, "height_carrier", f64::NAN)
            }),
        ))
    }

    fn evaluate_triangle_surface_height(
        &self,
        id: NodeBandHeightFieldId,
        source_kind: RoadSurfaceBandKind,
        point_xz: RoadVec2,
        triangles: &[NodeBandHeightTriangle],
    ) -> Result<NodeHeightPatchEvaluation, NodeHeightFieldError> {
        let mut candidates = Vec::new();
        for triangle in triangles {
            if let Some(height_m) = triangle.height_at(point_xz) {
                candidates.push(height_m);
            }
        }
        if candidates.is_empty() {
            return Ok(NodeHeightPatchEvaluation::Outside(
                self.outside_field_error(id, source_kind, point_xz, "triangle", f64::NAN),
            ));
        }
        let first_height_m = candidates[0];
        let first_height_mm = quantize_m(first_height_m);
        for height_m in candidates.iter().copied().skip(1) {
            let height_mm = quantize_m(height_m);
            if height_mm != first_height_mm {
                let key = NodeHeightPointKey::from_point(point_xz);
                return Err(NodeHeightFieldError::SourceHeightFieldConflict {
                    mouth_order_index: id.mouth_order_index(),
                    band_index: id.band_index(),
                    source_kind,
                    height_field_id: id,
                    owner: self.authority.owner,
                    existing_authority: self.authority.source(),
                    incoming_authority: self.authority.source(),
                    point_x_mm: key.x_mm(),
                    point_z_mm: key.z_mm(),
                    existing_height_mm: first_height_mm,
                    incoming_height_mm: height_mm,
                });
            }
        }
        Ok(NodeHeightPatchEvaluation::Inside(first_height_m))
    }

    fn outside_field_error(
        &self,
        id: NodeBandHeightFieldId,
        source_kind: RoadSurfaceBandKind,
        point_xz: RoadVec2,
        axis: &'static str,
        raw_parameter: f64,
    ) -> NodeHeightFieldError {
        let key = NodeHeightPointKey::from_point(point_xz);
        NodeHeightFieldError::VertexOutsideHeightField {
            mouth_order_index: id.mouth_order_index(),
            band_index: id.band_index(),
            source_kind,
            height_field_id: id,
            owner: self.authority.owner,
            point_x_mm: key.x_mm(),
            point_z_mm: key.z_mm(),
            axis,
            raw_parameter,
        }
    }
}

impl NodeBandHeightTriangle {
    fn height_at(&self, point_xz: RoadVec2) -> Option<f64> {
        let a = height_source_point_key(self.a_xz);
        let b = height_source_point_key(self.b_xz);
        let c = height_source_point_key(self.c_xz);
        let p = height_source_point_key(point_xz);
        let area = height_triangle_area2(a, b, c);
        if area == 0 {
            return None;
        }
        let abp = height_triangle_area2(a, b, p);
        let bcp = height_triangle_area2(b, c, p);
        let cap = height_triangle_area2(c, a, p);
        let has_negative = abp < 0 || bcp < 0 || cap < 0;
        let has_positive = abp > 0 || bcp > 0 || cap > 0;
        if has_negative && has_positive {
            return None;
        }
        let area_f = area as f64;
        let wa = bcp as f64 / area_f;
        let wb = cap as f64 / area_f;
        let wc = abp as f64 / area_f;
        Some(self.a_height_m * wa + self.b_height_m * wb + self.c_height_m * wc)
    }
}

fn validate_input_ownership_pair(
    input: &NodeArrangementInput,
    ownership: &NodeBooleanOwnership,
) -> Result<(), NodeHeightFieldError> {
    if input.node_id == ownership.node_id && input.piece_kind == ownership.piece_kind {
        return Ok(());
    }

    Err(NodeHeightFieldError::InputOwnershipMismatch {
        input_node_id: input.node_id,
        ownership_node_id: ownership.node_id,
        input_piece_kind: input.piece_kind,
        ownership_piece_kind: ownership.piece_kind,
    })
}

fn height_fields_by_source(
    input: &NodeArrangementInput,
    rails: Option<&NodeRailContourSet>,
) -> Result<BTreeMap<NodeSourceBandKey, NodeBandHeightField>, NodeHeightFieldError> {
    let terminal_cap_bands_by_mouth = terminal_cap_bands_by_mouth(input)
        .map_err(|error| NodeHeightFieldError::TerminalCapGeneration { error })?;
    let mut fields = BTreeMap::new();
    for (mouth_index, mouth) in input.mouths.iter().enumerate() {
        for interval in &mouth.band_intervals {
            let field = NodeBandHeightField::from_interval(mouth.order_index, interval);
            let key = NodeSourceBandKey {
                mouth_order_index: mouth.order_index,
                band_index: interval.band_index,
            };
            if fields.insert(key, field).is_some() {
                return Err(NodeHeightFieldError::DuplicateSourceBand {
                    mouth_order_index: mouth.order_index,
                    band_index: interval.band_index,
                });
            }
        }
        let terminal_cap_bands = terminal_cap_bands_by_mouth
            .get(mouth_index)
            .map_or(&[] as &[NodeTerminalCapBand], Vec::as_slice);
        for cap_band in terminal_cap_bands {
            let field = NodeBandHeightField::from_terminal_cap_band(mouth.order_index, cap_band);
            let key = NodeSourceBandKey {
                mouth_order_index: mouth.order_index,
                band_index: cap_band.source_band_index,
            };
            if let Some(existing) = fields.get_mut(&key) {
                existing.extend_with_terminal_cap_band(mouth.order_index, cap_band)?;
            } else {
                fields.insert(key, field);
            }
        }
    }
    if let Some(rails) = rails {
        extend_height_fields_with_generated_contours(input.piece_kind, rails, &mut fields)?;
    }
    Ok(fields)
}

fn extend_height_fields_with_generated_contours(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    rails: &NodeRailContourSet,
    fields: &mut BTreeMap<NodeSourceBandKey, NodeBandHeightField>,
) -> Result<(), NodeHeightFieldError> {
    let allow_missing_height_points_backfill =
        piece_kind != RoadSurfaceVisualNodePieceKind::JunctionN;
    for contour in &rails.contours {
        let NodeGeneratedContourKind::Band { kind } = contour.kind else {
            continue;
        };
        let Some(band_index) = contour.source_band_index else {
            continue;
        };
        let key = NodeSourceBandKey {
            mouth_order_index: contour.source_mouth_order_index,
            band_index,
        };
        let Some(field) = fields.get_mut(&key) else {
            continue;
        };
        if field.kind != kind {
            return Err(NodeHeightFieldError::SourceBandKindMismatch {
                mouth_order_index: key.mouth_order_index,
                band_index: key.band_index,
                region_kind: kind,
                source_kind: field.kind,
            });
        }
        field.extend_with_generated_contour(contour, allow_missing_height_points_backfill)?;
    }
    Ok(())
}

impl NodeResolvedHeightAuthorityMap {
    fn from_ownership(
        ownership: &NodeBooleanOwnership,
        fields: &BTreeMap<NodeSourceBandKey, NodeBandHeightField>,
    ) -> Result<Self, NodeHeightFieldError> {
        let mut map = Self {
            heights_by_key: BTreeMap::new(),
        };
        for region in &ownership.owned_regions {
            let field = height_field_for_region(region, fields)?;
            for point in region
                .shape
                .iter()
                .flat_map(|contour| contour.iter().copied())
            {
                let point_xz = quantize_road_vec2_to_overlay_grid(overlay_point_to_road(point));
                let height = field.evaluate_authorized_height(
                    region.owner,
                    region.claim_priority,
                    point_xz,
                )?;
                map.insert(
                    region.owner,
                    field.id,
                    region.claim_priority,
                    point_xz,
                    height,
                    region.kind,
                )?;
            }
        }
        Ok(map)
    }

    fn insert(
        &mut self,
        owner: NodeBandOwner,
        height_field_id: NodeBandHeightFieldId,
        claim_priority: NodeGeneratedContourClaimPriority,
        point_xz: RoadVec2,
        height: NodeEvaluatedHeight,
        kind: RoadSurfaceBandKind,
    ) -> Result<(), NodeHeightFieldError> {
        let point = NodeHeightPointKey::from_point(point_xz);
        let key = NodeResolvedHeightAuthorityKey {
            point,
            owner,
            height_field_id,
            claim_priority,
        };
        let height_mm = quantize_m(height.height_m);
        if let Some(existing) = self.heights_by_key.get(&key) {
            let existing_height_mm = quantize_m(existing.height_m);
            if existing_height_mm != height_mm {
                return Err(NodeHeightFieldError::SharedSourceHeightConflict {
                    point_x_mm: point.x_mm(),
                    point_z_mm: point.z_mm(),
                    kind,
                    owner,
                    opposite_owner: None,
                    height_field_id: Some(height_field_id),
                    incoming_owner: owner,
                    incoming_height_field_id: Some(height_field_id),
                    constraint_index: None,
                    existing_authority: Some(existing.authority),
                    incoming_authority: Some(height.authority),
                    existing_height_mm,
                    incoming_height_mm: height_mm,
                });
            }
            return Ok(());
        }
        self.heights_by_key.insert(
            key,
            NodeResolvedHeightAuthority {
                point_xz,
                height_m: height.height_m,
                authority: height.authority,
            },
        );
        Ok(())
    }

    fn height_for_vertex(
        &self,
        owner: NodeBandOwner,
        height_field_id: NodeBandHeightFieldId,
        claim_priority: NodeGeneratedContourClaimPriority,
        point: NodeOverlayPoint,
    ) -> Option<NodeResolvedHeightAuthority> {
        let point_xz = quantize_road_vec2_to_overlay_grid(overlay_point_to_road(point));
        let key = NodeResolvedHeightAuthorityKey {
            point: NodeHeightPointKey::from_point(point_xz),
            owner,
            height_field_id,
            claim_priority,
        };
        self.heights_by_key.get(&key).copied()
    }
}

fn heighted_region(
    region: &NodeBooleanOwnedRegion,
    fields: &BTreeMap<NodeSourceBandKey, NodeBandHeightField>,
    resolved_authority: Option<&NodeResolvedHeightAuthorityMap>,
) -> Result<NodeHeightedRegion, NodeHeightFieldError> {
    let field = height_field_for_region(region, fields)?;
    let shape = heighted_shape(&region.shape, region, field, resolved_authority)?;

    Ok(NodeHeightedRegion {
        kind: region.kind,
        owner: region.owner,
        height_field_id: field.id,
        shape,
        area_m2: region.area_m2,
        seam_constraints: region.seam_constraints.clone(),
    })
}

fn height_field_for_region<'a>(
    region: &NodeBooleanOwnedRegion,
    fields: &'a BTreeMap<NodeSourceBandKey, NodeBandHeightField>,
) -> Result<&'a NodeBandHeightField, NodeHeightFieldError> {
    let band_index =
        region
            .source_band_index
            .ok_or(NodeHeightFieldError::MissingRegionBandIndex {
                mouth_order_index: region.source_mouth_order_index,
                kind: region.kind,
            })?;
    let key = NodeSourceBandKey {
        mouth_order_index: region.source_mouth_order_index,
        band_index,
    };
    let field = fields
        .get(&key)
        .ok_or(NodeHeightFieldError::MissingSourceBand {
            mouth_order_index: key.mouth_order_index,
            band_index: key.band_index,
        })?;
    if field.kind != region.kind {
        return Err(NodeHeightFieldError::SourceBandKindMismatch {
            mouth_order_index: key.mouth_order_index,
            band_index: key.band_index,
            region_kind: region.kind,
            source_kind: field.kind,
        });
    }
    Ok(field)
}

fn heighted_shape(
    shape: &NodeOverlayShape,
    region: &NodeBooleanOwnedRegion,
    field: &NodeBandHeightField,
    resolved_authority: Option<&NodeResolvedHeightAuthorityMap>,
) -> Result<NodeHeightedShape, NodeHeightFieldError> {
    let mut heighted = Vec::with_capacity(shape.len());
    for contour in shape {
        let contour = heighted_contour(contour, region, field, resolved_authority)?;
        if contour.len() >= 3 {
            heighted.push(contour);
        }
    }
    Ok(heighted)
}

fn heighted_contour(
    contour: &NodeOverlayContour,
    region: &NodeBooleanOwnedRegion,
    field: &NodeBandHeightField,
    resolved_authority: Option<&NodeResolvedHeightAuthorityMap>,
) -> Result<NodeHeightedContour, NodeHeightFieldError> {
    contour
        .iter()
        .copied()
        .map(|point| heighted_vertex(point, region, field, resolved_authority))
        .collect()
}

fn heighted_vertex(
    point: NodeOverlayPoint,
    region: &NodeBooleanOwnedRegion,
    field: &NodeBandHeightField,
    resolved_authority: Option<&NodeResolvedHeightAuthorityMap>,
) -> Result<NodeHeightedVertex, NodeHeightFieldError> {
    let (point_xz, height_m) = if let Some(resolved_authority) = resolved_authority {
        let Some(authority) = resolved_authority.height_for_vertex(
            region.owner,
            field.id,
            region.claim_priority,
            point,
        ) else {
            let point_xz = quantize_road_vec2_to_overlay_grid(overlay_point_to_road(point));
            let key = NodeHeightPointKey::from_point(point_xz);
            return Err(NodeHeightFieldError::VertexOutsideHeightField {
                mouth_order_index: field.id.mouth_order_index(),
                band_index: field.id.band_index(),
                source_kind: field.kind,
                height_field_id: field.id,
                owner: Some(region.owner),
                point_x_mm: key.x_mm(),
                point_z_mm: key.z_mm(),
                axis: "canonical_authority",
                raw_parameter: f64::NAN,
            });
        };
        (authority.point_xz, authority.height_m)
    } else {
        let point_xz = quantize_road_vec2_to_overlay_grid(overlay_point_to_road(point));
        (point_xz, field.evaluate_height(point_xz)?)
    };
    Ok(NodeHeightedVertex {
        point_xz,
        height_m,
        height_field_id: field.id,
        height_authority: resolved_authority.and_then(|authority_map| {
            authority_map
                .height_for_vertex(region.owner, field.id, region.claim_priority, point)
                .map(|authority| authority.authority)
        }),
    })
}

#[derive(Clone, Copy)]
struct SameMaterialVertexHeightCandidate {
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    height_m: f64,
    height_authority: Option<NodeHeightAuthoritySource>,
    has_explicit_shared_material_seam: bool,
}

fn apply_junctionn_same_owner_height_field_vertex_unification(regions: &mut [NodeHeightedRegion]) {
    let mut heights_by_key =
        BTreeMap::<NodeHeightVertexContextKey, SameMaterialVertexHeightCandidate>::new();
    let mut distinct_heights_by_key = BTreeMap::<NodeHeightVertexContextKey, Vec<i64>>::new();

    for region in regions.iter() {
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            let key = NodeHeightVertexContextKey {
                point: NodeHeightPointKey::from_point(vertex.point_xz),
                owner: region.owner,
                height_field_id: vertex.height_field_id,
            };
            heights_by_key
                .entry(key.clone())
                .and_modify(|selected| {
                    let candidate = SameMaterialVertexHeightCandidate {
                        owner: region.owner,
                        height_field_id: vertex.height_field_id,
                        height_m: vertex.height_m,
                        height_authority: vertex.height_authority,
                        has_explicit_shared_material_seam: vertex_has_explicit_shared_material_seam(
                            vertex,
                            &region.seam_constraints,
                        ),
                    };
                    if same_material_vertex_height_candidate_key(candidate)
                        < same_material_vertex_height_candidate_key(*selected)
                    {
                        *selected = candidate;
                    }
                })
                .or_insert(SameMaterialVertexHeightCandidate {
                    owner: region.owner,
                    height_field_id: vertex.height_field_id,
                    height_m: vertex.height_m,
                    height_authority: vertex.height_authority,
                    has_explicit_shared_material_seam: vertex_has_explicit_shared_material_seam(
                        vertex,
                        &region.seam_constraints,
                    ),
                });
            let heights = distinct_heights_by_key.entry(key).or_default();
            let height_mm = quantize_m(vertex.height_m);
            if !heights.contains(&height_mm) {
                heights.push(height_mm);
            }
        }
    }

    for region in regions {
        for vertex in region
            .shape
            .iter_mut()
            .flat_map(|contour| contour.iter_mut())
        {
            let key = NodeHeightVertexContextKey {
                point: NodeHeightPointKey::from_point(vertex.point_xz),
                owner: region.owner,
                height_field_id: vertex.height_field_id,
            };
            if distinct_heights_by_key
                .get(&key)
                .is_none_or(|heights| heights.len() < 2)
            {
                continue;
            }
            if let Some(selected) = heights_by_key.get(&key) {
                vertex.height_m = selected.height_m;
            }
        }
    }
}

fn apply_junctionn_same_material_shared_edge_height_tiebreak(regions: &mut [NodeHeightedRegion]) {
    let mut candidates_by_edge =
        BTreeMap::<SameMaterialSharedEdgeKey, Vec<SameMaterialSharedEdgeCandidate>>::new();

    for region in regions.iter() {
        for contour in &region.shape {
            if contour.len() < 2 {
                continue;
            }
            for index in 0..contour.len() {
                let start = &contour[index];
                let end = &contour[(index + 1) % contour.len()];
                let Some((key, candidate)) = SameMaterialSharedEdgeCandidate::new(
                    region.kind,
                    region.owner,
                    &region.seam_constraints,
                    start,
                    end,
                ) else {
                    continue;
                };
                let candidates = candidates_by_edge.entry(key).or_default();
                if !candidates.contains(&candidate) {
                    candidates.push(candidate);
                }
            }
        }
    }

    let mut selected_by_vertex =
        BTreeMap::<SameMaterialSharedVertexKey, SameMaterialVertexHeightCandidate>::new();
    let mut affected_contexts_by_vertex =
        BTreeMap::<SameMaterialSharedVertexKey, Vec<SameMaterialSharedVertexContext>>::new();

    for (edge, candidates) in candidates_by_edge {
        if candidates.len() < 2 {
            continue;
        }
        if !same_material_shared_edge_candidates_are_canonical_drift(&candidates) {
            continue;
        }
        for endpoint in [edge.start, edge.end] {
            let selected = candidates
                .iter()
                .copied()
                .map(|candidate| candidate.endpoint_candidate(endpoint))
                .min_by_key(|candidate| same_material_vertex_height_candidate_key(*candidate))
                .expect("shared edge with candidates has an endpoint candidate");
            let vertex_key = SameMaterialSharedVertexKey {
                kind: edge.kind,
                point: endpoint,
            };
            selected_by_vertex
                .entry(vertex_key)
                .and_modify(|existing| {
                    if same_material_vertex_height_candidate_key(selected)
                        < same_material_vertex_height_candidate_key(*existing)
                    {
                        *existing = selected;
                    }
                })
                .or_insert(selected);
            let contexts = affected_contexts_by_vertex.entry(vertex_key).or_default();
            for candidate in &candidates {
                let context = SameMaterialSharedVertexContext {
                    owner: candidate.owner,
                    height_field_id: candidate.height_field_id,
                };
                if !contexts.contains(&context) {
                    contexts.push(context);
                }
            }
        }
    }

    for region in regions {
        for vertex in region
            .shape
            .iter_mut()
            .flat_map(|contour| contour.iter_mut())
        {
            let key = SameMaterialSharedVertexKey {
                kind: region.kind,
                point: NodeHeightPointKey::from_point(vertex.point_xz),
            };
            let Some(contexts) = affected_contexts_by_vertex.get(&key) else {
                continue;
            };
            let context = SameMaterialSharedVertexContext {
                owner: region.owner,
                height_field_id: vertex.height_field_id,
            };
            if !contexts.contains(&context) {
                continue;
            }
            if let Some(selected) = selected_by_vertex.get(&key) {
                vertex.height_m = selected.height_m;
            }
        }
    }
}

fn same_material_shared_edge_candidates_are_canonical_drift(
    candidates: &[SameMaterialSharedEdgeCandidate],
) -> bool {
    let mut start_min = f64::INFINITY;
    let mut start_max = f64::NEG_INFINITY;
    let mut end_min = f64::INFINITY;
    let mut end_max = f64::NEG_INFINITY;
    for candidate in candidates {
        start_min = start_min.min(candidate.start_height_m);
        start_max = start_max.max(candidate.start_height_m);
        end_min = end_min.min(candidate.end_height_m);
        end_max = end_max.max(candidate.end_height_m);
    }
    start_max - start_min <= SAME_MATERIAL_SHARED_EDGE_HEIGHT_CANONICAL_EPS_M
        && end_max - end_min <= SAME_MATERIAL_SHARED_EDGE_HEIGHT_CANONICAL_EPS_M
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct SameMaterialSharedEdgeKey {
    kind: RoadSurfaceBandKind,
    start: NodeHeightPointKey,
    end: NodeHeightPointKey,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SameMaterialSharedEdgeCandidate {
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    start: NodeHeightPointKey,
    start_height_m: f64,
    start_height_authority: Option<NodeHeightAuthoritySource>,
    start_has_explicit_shared_material_seam: bool,
    end: NodeHeightPointKey,
    end_height_m: f64,
    end_height_authority: Option<NodeHeightAuthoritySource>,
    end_has_explicit_shared_material_seam: bool,
}

impl SameMaterialSharedEdgeCandidate {
    fn new(
        kind: RoadSurfaceBandKind,
        owner: NodeBandOwner,
        seam_constraints: &[NodeRegionSeamConstraint],
        start: &NodeHeightedVertex,
        end: &NodeHeightedVertex,
    ) -> Option<(SameMaterialSharedEdgeKey, Self)> {
        let mut start_key = NodeHeightPointKey::from_point(start.point_xz);
        let mut end_key = NodeHeightPointKey::from_point(end.point_xz);
        if start_key == end_key {
            return None;
        }
        let mut start_height_m = start.height_m;
        let mut end_height_m = end.height_m;
        let mut start_height_authority = start.height_authority;
        let mut end_height_authority = end.height_authority;
        let mut start_has_explicit_shared_material_seam =
            vertex_has_explicit_shared_material_seam(start, seam_constraints);
        let mut end_has_explicit_shared_material_seam =
            vertex_has_explicit_shared_material_seam(end, seam_constraints);
        if end_key < start_key {
            std::mem::swap(&mut start_key, &mut end_key);
            std::mem::swap(&mut start_height_m, &mut end_height_m);
            std::mem::swap(&mut start_height_authority, &mut end_height_authority);
            std::mem::swap(
                &mut start_has_explicit_shared_material_seam,
                &mut end_has_explicit_shared_material_seam,
            );
        }
        Some((
            SameMaterialSharedEdgeKey {
                kind,
                start: start_key,
                end: end_key,
            },
            Self {
                owner,
                height_field_id: start.height_field_id,
                start: start_key,
                start_height_m,
                start_height_authority,
                start_has_explicit_shared_material_seam,
                end: end_key,
                end_height_m,
                end_height_authority,
                end_has_explicit_shared_material_seam,
            },
        ))
    }

    fn endpoint_candidate(self, point: NodeHeightPointKey) -> SameMaterialVertexHeightCandidate {
        let (height_m, height_authority, has_explicit_shared_material_seam) = if point == self.start
        {
            (
                self.start_height_m,
                self.start_height_authority,
                self.start_has_explicit_shared_material_seam,
            )
        } else {
            debug_assert_eq!(point, self.end);
            (
                self.end_height_m,
                self.end_height_authority,
                self.end_has_explicit_shared_material_seam,
            )
        };
        SameMaterialVertexHeightCandidate {
            owner: self.owner,
            height_field_id: self.height_field_id,
            height_m,
            height_authority,
            has_explicit_shared_material_seam,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct SameMaterialSharedVertexKey {
    kind: RoadSurfaceBandKind,
    point: NodeHeightPointKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SameMaterialSharedVertexContext {
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
}

fn apply_junctionn_same_material_vertex_height_tiebreak(regions: &mut [NodeHeightedRegion]) {
    let mut contexts_by_key =
        BTreeMap::<SameMaterialVertexHeightTieKey, Vec<SameMaterialVertexHeightContext>>::new();
    let mut selected_by_key =
        BTreeMap::<SameMaterialVertexHeightTieKey, SameMaterialVertexHeightCandidate>::new();

    for region in regions.iter() {
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            let key = same_material_vertex_height_tie_key_from_parts(
                region.kind,
                &region.seam_constraints,
                vertex,
            );
            let candidate = SameMaterialVertexHeightCandidate {
                owner: region.owner,
                height_field_id: vertex.height_field_id,
                height_m: vertex.height_m,
                height_authority: vertex.height_authority,
                has_explicit_shared_material_seam: vertex_has_explicit_shared_material_seam(
                    vertex,
                    &region.seam_constraints,
                ),
            };
            let contexts = contexts_by_key.entry(key.clone()).or_default();
            let context = SameMaterialVertexHeightContext::from_candidate(candidate);
            if !contexts.contains(&context) {
                contexts.push(context);
                contexts.sort_unstable();
            }
            selected_by_key
                .entry(key)
                .and_modify(|selected| {
                    if same_material_vertex_height_candidate_key(candidate)
                        < same_material_vertex_height_candidate_key(*selected)
                    {
                        *selected = candidate;
                    }
                })
                .or_insert(candidate);
        }
    }

    for region in regions {
        let kind = region.kind;
        let seam_constraints = &region.seam_constraints;
        for vertex in region
            .shape
            .iter_mut()
            .flat_map(|contour| contour.iter_mut())
        {
            let key =
                same_material_vertex_height_tie_key_from_parts(kind, seam_constraints, vertex);
            if contexts_by_key
                .get(&key)
                .is_none_or(|contexts| contexts.len() < 2)
            {
                continue;
            }
            if let Some(selected) = selected_by_key.get(&key) {
                vertex.height_m = selected.height_m;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct SameMaterialVertexHeightContext {
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    height_mm: i64,
}

impl SameMaterialVertexHeightContext {
    fn from_candidate(candidate: SameMaterialVertexHeightCandidate) -> Self {
        Self {
            owner: candidate.owner,
            height_field_id: candidate.height_field_id,
            height_mm: quantize_m(candidate.height_m),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct SameMaterialVertexHeightTieKey {
    kind: RoadSurfaceBandKind,
    point: NodeHeightPointKey,
    explicit_seams: Vec<ExplicitSeamHeightKey>,
}

fn same_material_vertex_height_tie_key_from_parts(
    kind: RoadSurfaceBandKind,
    seam_constraints: &[NodeRegionSeamConstraint],
    vertex: &NodeHeightedVertex,
) -> SameMaterialVertexHeightTieKey {
    let point = NodeHeightPointKey::from_point(vertex.point_xz);
    let mut explicit_seams =
        material_height_constraints_for_vertex(vertex.point_xz, seam_constraints)
            .into_iter()
            .map(|constraint| ExplicitSeamHeightKey::new(point, constraint))
            .collect::<Vec<_>>();
    explicit_seams.sort_unstable();
    explicit_seams.dedup();
    SameMaterialVertexHeightTieKey {
        kind,
        point,
        explicit_seams,
    }
}

fn same_material_vertex_height_candidate_key(
    candidate: SameMaterialVertexHeightCandidate,
) -> (bool, bool, usize, usize, usize) {
    (
        !candidate.has_explicit_shared_material_seam,
        candidate.height_authority != Some(NodeHeightAuthoritySource::SourceInterval),
        candidate.height_field_id.mouth_order_index(),
        candidate.height_field_id.band_index(),
        candidate.owner.owner_index(),
    )
}

fn apply_junctionn_explicit_material_seam_height_unification(
    regions: &mut [NodeHeightedRegion],
) {
    let mut ranges_by_key =
        BTreeMap::<ExplicitSeamHeightKey, (f64, f64, SameMaterialVertexHeightCandidate)>::new();

    for region in regions.iter() {
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            for constraint in
                material_height_constraints_for_vertex(vertex.point_xz, &region.seam_constraints)
            {
                if !constraint.is_material_transition {
                    continue;
                }
                let point = NodeHeightPointKey::from_point(vertex.point_xz);
                let key = ExplicitSeamHeightKey::new(point, constraint);
                let candidate = SameMaterialVertexHeightCandidate {
                    owner: region.owner,
                    height_field_id: vertex.height_field_id,
                    height_m: vertex.height_m,
                    height_authority: vertex.height_authority,
                    has_explicit_shared_material_seam: true,
                };
                ranges_by_key
                    .entry(key)
                    .and_modify(|(min_height, max_height, selected)| {
                        *min_height = min_height.min(vertex.height_m);
                        *max_height = max_height.max(vertex.height_m);
                        if same_material_vertex_height_candidate_key(candidate)
                            < same_material_vertex_height_candidate_key(*selected)
                        {
                            *selected = candidate;
                        }
                    })
                    .or_insert((vertex.height_m, vertex.height_m, candidate));
            }
        }
    }

    let selected_by_key = ranges_by_key
        .into_iter()
        .filter_map(|(key, (min_height, max_height, selected))| {
            (max_height - min_height <= EXPLICIT_MATERIAL_SEAM_HEIGHT_CANONICAL_EPS_M)
                .then_some((key, selected.height_m))
        })
        .collect::<BTreeMap<_, _>>();

    if selected_by_key.is_empty() {
        return;
    }

    for region in regions {
        for vertex in region
            .shape
            .iter_mut()
            .flat_map(|contour| contour.iter_mut())
        {
            let constraints =
                material_height_constraints_for_vertex(vertex.point_xz, &region.seam_constraints);
            for constraint in constraints {
                if !constraint.is_material_transition {
                    continue;
                }
                let point = NodeHeightPointKey::from_point(vertex.point_xz);
                let key = ExplicitSeamHeightKey::new(point, constraint);
                if let Some(height_m) = selected_by_key.get(&key) {
                    vertex.height_m = *height_m;
                    break;
                }
            }
        }
    }
}

fn vertex_has_explicit_shared_material_seam(
    vertex: &NodeHeightedVertex,
    seam_constraints: &[NodeRegionSeamConstraint],
) -> bool {
    material_height_constraints_for_vertex(vertex.point_xz, seam_constraints)
        .into_iter()
        .any(|constraint| constraint.is_material_transition)
}

fn validate_explicit_material_seam_heights(
    regions: &[NodeHeightedRegion],
) -> Result<(), NodeHeightFieldError> {
    let mut shared_heights = BTreeMap::<ExplicitSeamHeightKey, ExplicitSeamHeight>::new();
    for region in regions.iter() {
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            for constraint in
                material_height_constraints_for_vertex(vertex.point_xz, &region.seam_constraints)
            {
                let point = NodeHeightPointKey::from_point(vertex.point_xz);
                let key = ExplicitSeamHeightKey::new(point, constraint);
                let height_mm = quantize_m(vertex.height_m);
                let incoming = ExplicitSeamHeight {
                    height_mm,
                    owner: region.owner,
                    height_field_id: vertex.height_field_id,
                };
                if let Some(existing) = shared_heights.insert(key, incoming) {
                    if existing.height_mm != height_mm {
                        let (owner, opposite_owner) = canonical_explicit_seam_owner_pair(
                            constraint.owner,
                            constraint.opposite_owner,
                        );
                        return Err(NodeHeightFieldError::SharedSourceHeightConflict {
                            point_x_mm: point.x_mm(),
                            point_z_mm: point.z_mm(),
                            kind: region.kind,
                            owner: owner.unwrap_or(existing.owner),
                            opposite_owner,
                            height_field_id: Some(existing.height_field_id),
                            incoming_owner: region.owner,
                            incoming_height_field_id: Some(vertex.height_field_id),
                            constraint_index: Some(constraint.constraint_index),
                            existing_authority: None,
                            incoming_authority: None,
                            existing_height_mm: existing.height_mm,
                            incoming_height_mm: height_mm,
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct ExplicitSeamHeight {
    height_mm: i64,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
}

impl ExplicitSeamHeightKey {
    fn new(point: NodeHeightPointKey, constraint: &NodeRegionSeamConstraint) -> Self {
        let (owner, opposite_owner) =
            canonical_explicit_seam_owner_pair(constraint.owner, constraint.opposite_owner);
        let start = NodeHeightPointKey::from_point(constraint.start_xz);
        let end = NodeHeightPointKey::from_point(constraint.end_xz);
        let (start, end) = if end < start {
            (end, start)
        } else {
            (start, end)
        };
        Self {
            point,
            constraint_index: constraint.constraint_index,
            owner,
            opposite_owner,
            start,
            end,
        }
    }
}

fn canonical_explicit_seam_owner_pair(
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
) -> (Option<NodeBandOwner>, Option<NodeBandOwner>) {
    match (owner, opposite_owner) {
        (Some(owner), Some(opposite_owner)) if opposite_owner < owner => {
            (Some(opposite_owner), Some(owner))
        }
        pair => pair,
    }
}

fn material_height_constraints_for_vertex<'a>(
    point_xz: RoadVec2,
    constraints: &'a [NodeRegionSeamConstraint],
) -> Vec<&'a NodeRegionSeamConstraint> {
    let mut matches = constraints
        .iter()
        .filter(|constraint| constraint.constrains_shared_height)
        .filter(|constraint| {
            point_lies_on_height_segment(point_xz, constraint.start_xz, constraint.end_xz)
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|constraint| {
        (
            !constraint.is_material_transition,
            seam_source_priority(&constraint.seam_source),
            constraint.constraint_index,
            NodeHeightPointKey::from_point(constraint.start_xz),
            NodeHeightPointKey::from_point(constraint.end_xz),
        )
    });
    matches.dedup_by_key(|constraint| {
        let point = NodeHeightPointKey::from_point(point_xz);
        ExplicitSeamHeightKey::new(point, constraint)
    });
    matches
}

fn point_lies_on_height_segment(point: RoadVec2, start: RoadVec2, end: RoadVec2) -> bool {
    let point = NodeHeightPointKey::from_point(point);
    let start = NodeHeightPointKey::from_point(start);
    let end = NodeHeightPointKey::from_point(end);
    if point == start || point == end {
        return true;
    }
    if start == end {
        return false;
    }
    let dx = i128::from(end.x_key - start.x_key);
    let dz = i128::from(end.z_key - start.z_key);
    let px = i128::from(point.x_key - start.x_key);
    let pz = i128::from(point.z_key - start.z_key);
    if px * dz - pz * dx != 0 {
        return false;
    }
    let inside_x = if start.x_key == end.x_key {
        point.x_key == start.x_key
    } else {
        point.x_key > start.x_key.min(end.x_key) && point.x_key < start.x_key.max(end.x_key)
    };
    let inside_z = if start.z_key == end.z_key {
        point.z_key == start.z_key
    } else {
        point.z_key > start.z_key.min(end.z_key) && point.z_key < start.z_key.max(end.z_key)
    };
    inside_x && inside_z
}

fn validate_shared_source_height_agreement(
    regions: &[NodeHeightedRegion],
) -> Result<(), NodeHeightFieldError> {
    let mut heights = BTreeMap::<NodeHeightVertexContextKey, SharedSourceHeight>::new();
    for region in regions {
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            let point = NodeHeightPointKey::from_point(vertex.point_xz);
            let key = NodeHeightVertexContextKey {
                point,
                owner: region.owner,
                height_field_id: vertex.height_field_id,
            };
            let height_mm = quantize_m(vertex.height_m);
            let incoming = SharedSourceHeight {
                height_mm,
                owner: region.owner,
                height_field_id: vertex.height_field_id,
            };
            if let Some(existing) = heights.insert(key, incoming)
                && existing.height_mm != height_mm
            {
                return Err(NodeHeightFieldError::SharedSourceHeightConflict {
                    point_x_mm: point.x_mm(),
                    point_z_mm: point.z_mm(),
                    kind: region.kind,
                    owner: existing.owner,
                    opposite_owner: None,
                    height_field_id: Some(existing.height_field_id),
                    incoming_owner: region.owner,
                    incoming_height_field_id: Some(vertex.height_field_id),
                    constraint_index: None,
                    existing_authority: None,
                    incoming_authority: None,
                    existing_height_mm: existing.height_mm,
                    incoming_height_mm: height_mm,
                });
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct SharedSourceHeight {
    height_mm: i64,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
}

fn quantize_source_height_m(value_m: f64) -> f64 {
    (value_m * HEIGHT_SOURCE_KEY_SCALE).round() / HEIGHT_SOURCE_KEY_SCALE
}

fn interval_height_triangles(interval: &NodeInputBandInterval) -> Vec<NodeBandHeightTriangle> {
    if let Some(triangles) =
        path_band_height_triangles(&interval.start_path_world, &interval.end_path_world)
    {
        return triangles;
    }
    height_triangles_from_vertices(&[
        interval.mouth_start_world,
        interval.mouth_end_world,
        interval.endpoint_end_world,
        interval.endpoint_start_world,
    ])
}

fn interval_height_edges(interval: &NodeInputBandInterval) -> Vec<NodeBandHeightEdge> {
    if let Some(edges) =
        path_band_height_edges(&interval.start_path_world, &interval.end_path_world)
    {
        return edges;
    }
    height_edges_from_vertices(&[
        interval.mouth_start_world,
        interval.mouth_end_world,
        interval.endpoint_end_world,
        interval.endpoint_start_world,
    ])
}

fn path_band_height_triangles(
    start_path_world: &[RoadVec3],
    end_path_world: &[RoadVec3],
) -> Option<Vec<NodeBandHeightTriangle>> {
    if start_path_world.len() != end_path_world.len() || start_path_world.len() < 2 {
        return None;
    }

    let mut triangles = Vec::with_capacity((start_path_world.len() - 1) * 2);
    for index in 0..start_path_world.len() - 1 {
        let start_current = start_path_world[index];
        let start_next = start_path_world[index + 1];
        let end_next = end_path_world[index + 1];
        let end_current = end_path_world[index];
        push_height_triangle(&mut triangles, start_current, start_next, end_next);
        push_height_triangle(&mut triangles, start_current, end_next, end_current);
    }
    (!triangles.is_empty()).then_some(triangles)
}

fn path_band_height_edges(
    start_path_world: &[RoadVec3],
    end_path_world: &[RoadVec3],
) -> Option<Vec<NodeBandHeightEdge>> {
    if start_path_world.len() != end_path_world.len() || start_path_world.len() < 2 {
        return None;
    }

    let mut contour = Vec::with_capacity(start_path_world.len() + end_path_world.len());
    contour.extend_from_slice(start_path_world);
    contour.extend(end_path_world.iter().rev().copied());
    let edges = height_edges_from_vertices(&contour);
    (!edges.is_empty()).then_some(edges)
}

fn terminal_cap_band_height_triangles(
    cap_band: &NodeTerminalCapBand,
) -> Vec<NodeBandHeightTriangle> {
    if let Some(triangles) = terminal_material_band_height_triangles(&cap_band.contour_world) {
        return triangles;
    }

    height_triangles_from_contour(&cap_band.contour_world)
}

fn terminal_material_band_height_triangles(
    points: &[RoadVec3],
) -> Option<Vec<NodeBandHeightTriangle>> {
    if points.len() < 4 || points.len() % 2 != 0 {
        return None;
    }

    let rail_point_count = points.len() / 2;
    let mut triangles = Vec::with_capacity((rail_point_count - 1) * 2);
    for index in 0..rail_point_count - 1 {
        let inner_start = points[index];
        let inner_end = points[index + 1];
        let outer_end = points[points.len() - 2 - index];
        let outer_start = points[points.len() - 1 - index];
        push_height_triangle(&mut triangles, inner_start, inner_end, outer_end);
        push_height_triangle(&mut triangles, inner_start, outer_end, outer_start);
    }

    Some(triangles)
}

fn push_height_triangle(
    triangles: &mut Vec<NodeBandHeightTriangle>,
    a_world: RoadVec3,
    b_world: RoadVec3,
    c_world: RoadVec3,
) {
    let a_xz = quantize_road_vec2_to_overlay_grid(xz(a_world));
    let b_xz = quantize_road_vec2_to_overlay_grid(xz(b_world));
    let c_xz = quantize_road_vec2_to_overlay_grid(xz(c_world));
    if height_triangle_area2(
        height_source_point_key(a_xz),
        height_source_point_key(b_xz),
        height_source_point_key(c_xz),
    ) == 0
    {
        return;
    }
    triangles.push(NodeBandHeightTriangle {
        a_xz,
        b_xz,
        c_xz,
        a_height_m: quantize_source_height_m(a_world.y),
        b_height_m: quantize_source_height_m(b_world.y),
        c_height_m: quantize_source_height_m(c_world.y),
    });
}

fn terminal_cap_band_height_edges(cap_band: &NodeTerminalCapBand) -> Vec<NodeBandHeightEdge> {
    height_edges_from_vertices(&cap_band.contour_world)
}

fn height_triangles_from_vertices(points: &[RoadVec3]) -> Vec<NodeBandHeightTriangle> {
    let vertices = canonical_height_vertices(points);
    fan_height_triangles_from_vertices(&vertices)
}

fn height_triangles_from_contour(points: &[RoadVec3]) -> Vec<NodeBandHeightTriangle> {
    constrained_height_triangles_from_vertices(points)
        .unwrap_or_else(|| height_triangles_from_vertices(points))
}

fn constrained_height_triangles_from_vertices(
    points: &[RoadVec3],
) -> Option<Vec<NodeBandHeightTriangle>> {
    let vertices = canonical_height_vertices(points);
    if vertices.len() < 3 {
        return None;
    }
    if vertices.len() == 3 {
        let triangles = fan_height_triangles_from_vertices(&vertices);
        return (!triangles.is_empty()).then_some(triangles);
    }

    let spade_vertices = vertices
        .iter()
        .map(|(point_xz, _)| Point2::new(point_xz.x, point_xz.y))
        .collect::<Vec<_>>();
    let constraints = (0..vertices.len())
        .map(|index| [index, (index + 1) % vertices.len()])
        .collect::<Vec<_>>();
    let mut invalid_constraints = 0usize;
    let cdt = SurfaceCdt::try_bulk_load_cdt(spade_vertices, constraints, |_| {
        invalid_constraints += 1;
    })
    .ok()?;
    if invalid_constraints > 0 {
        return None;
    }

    let mut triangles = Vec::new();
    for face in cdt.inner_faces() {
        let [a, b, c] = face.vertices();
        let indices = [a.fix().index(), b.fix().index(), c.fix().index()];
        let centroid =
            (vertices[indices[0]].0 + vertices[indices[1]].0 + vertices[indices[2]].0) / 3.0;
        if !height_polygon_contains_point_xz(&vertices, centroid) {
            continue;
        }
        push_height_triangle_from_vertices(
            &mut triangles,
            vertices[indices[0]],
            vertices[indices[1]],
            vertices[indices[2]],
        );
    }
    triangles.sort_by(|a, b| height_triangle_sort_key(a).cmp(&height_triangle_sort_key(b)));
    triangles.dedup_by_key(|triangle| height_triangle_sort_key(triangle));
    (!triangles.is_empty()).then_some(triangles)
}

fn canonical_height_vertices(points: &[RoadVec3]) -> Vec<(RoadVec2, f64)> {
    let mut vertices = Vec::with_capacity(points.len());
    for point in points {
        let point_xz = quantize_road_vec2_to_overlay_grid(xz(*point));
        let key = height_source_point_key(point_xz);
        if vertices
            .last()
            .is_some_and(|(last_xz, _)| height_source_point_key(*last_xz) == key)
        {
            continue;
        }
        vertices.push((point_xz, quantize_source_height_m(point.y)));
    }
    if vertices.len() > 1
        && height_source_point_key(vertices[0].0)
            == height_source_point_key(vertices.last().expect("height vertices are non-empty").0)
    {
        vertices.pop();
    }
    vertices
}

fn fan_height_triangles_from_vertices(vertices: &[(RoadVec2, f64)]) -> Vec<NodeBandHeightTriangle> {
    let mut triangles = Vec::new();
    if vertices.len() < 3 {
        return triangles;
    }
    let (a_xz, a_height_m) = vertices[0];
    for index in 1..vertices.len() - 1 {
        let (b_xz, b_height_m) = vertices[index];
        let (c_xz, c_height_m) = vertices[index + 1];
        if height_triangle_area2(
            height_source_point_key(a_xz),
            height_source_point_key(b_xz),
            height_source_point_key(c_xz),
        ) == 0
        {
            continue;
        }
        triangles.push(NodeBandHeightTriangle {
            a_xz,
            b_xz,
            c_xz,
            a_height_m,
            b_height_m,
            c_height_m,
        });
    }
    triangles
}

fn push_height_triangle_from_vertices(
    triangles: &mut Vec<NodeBandHeightTriangle>,
    a: (RoadVec2, f64),
    b: (RoadVec2, f64),
    c: (RoadVec2, f64),
) {
    if height_triangle_area2(
        height_source_point_key(a.0),
        height_source_point_key(b.0),
        height_source_point_key(c.0),
    ) == 0
    {
        return;
    }
    triangles.push(NodeBandHeightTriangle {
        a_xz: a.0,
        b_xz: b.0,
        c_xz: c.0,
        a_height_m: a.1,
        b_height_m: b.1,
        c_height_m: c.1,
    });
}

fn height_triangle_sort_key(triangle: &NodeBandHeightTriangle) -> [NodeHeightSourcePointKey; 3] {
    let mut keys = [
        height_source_point_key(triangle.a_xz),
        height_source_point_key(triangle.b_xz),
        height_source_point_key(triangle.c_xz),
    ];
    keys.sort();
    keys
}

fn height_polygon_contains_point_xz(vertices: &[(RoadVec2, f64)], point: RoadVec2) -> bool {
    if vertices.len() < 3 {
        return false;
    }
    let point_key = height_source_point_key(point);
    for index in 0..vertices.len() {
        let start = vertices[index].0;
        let end = vertices[(index + 1) % vertices.len()].0;
        if height_source_point_key_lies_on_segment(
            point_key,
            height_source_point_key(start),
            height_source_point_key(end),
        ) {
            return true;
        }
    }

    let mut inside = false;
    for index in 0..vertices.len() {
        let start = vertices[index].0;
        let end = vertices[(index + 1) % vertices.len()].0;
        if (start.y > point.y) != (end.y > point.y) {
            let edge_x_at_point_z =
                (end.x - start.x) * (point.y - start.y) / (end.y - start.y) + start.x;
            if point.x < edge_x_at_point_z {
                inside = !inside;
            }
        }
    }
    inside
}

fn height_edges_from_vertices(points: &[RoadVec3]) -> Vec<NodeBandHeightEdge> {
    let mut vertices = Vec::with_capacity(points.len());
    for point in points {
        let point_xz = quantize_road_vec2_to_overlay_grid(xz(*point));
        let key = height_source_point_key(point_xz);
        if vertices
            .last()
            .is_some_and(|(last_xz, _)| height_source_point_key(*last_xz) == key)
        {
            continue;
        }
        vertices.push((point_xz, quantize_source_height_m(point.y)));
    }
    if vertices.len() > 1
        && height_source_point_key(vertices[0].0)
            == height_source_point_key(vertices.last().expect("height vertices are non-empty").0)
    {
        vertices.pop();
    }
    if vertices.len() < 2 {
        return Vec::new();
    }

    let mut edges = Vec::with_capacity(vertices.len());
    for index in 0..vertices.len() {
        let (start_xz, start_height_m) = vertices[index];
        let (end_xz, end_height_m) = vertices[(index + 1) % vertices.len()];
        if height_source_point_key(start_xz) == height_source_point_key(end_xz) {
            continue;
        }
        edges.push(NodeBandHeightEdge {
            start_xz,
            end_xz,
            start_height_m,
            end_height_m,
        });
    }
    edges
}

fn terminal_edge_height_at(point_xz: RoadVec2, edges: &[NodeBandHeightEdge]) -> Option<f64> {
    let point = height_source_point_key(point_xz);
    for edge in edges {
        let start = height_source_point_key(edge.start_xz);
        let end = height_source_point_key(edge.end_xz);
        if height_source_point_key_lies_on_segment(point, start, end)
            || height_source_point_key_quantization_cell_intersects_segment(point, start, end)
        {
            let dx = end.0 - start.0;
            let dz = end.1 - start.1;
            let denominator = if dx.abs() >= dz.abs() { dx } else { dz };
            if denominator == 0 {
                continue;
            }
            let numerator = if dx.abs() >= dz.abs() {
                point.0 - start.0
            } else {
                point.1 - start.1
            };
            let t = numerator as f64 / denominator as f64;
            if !(0.0..=1.0).contains(&t) {
                continue;
            }
            return Some(edge.start_height_m + (edge.end_height_m - edge.start_height_m) * t);
        }

        let point = NodeHeightPointKey::from_point(point_xz);
        let start = NodeHeightPointKey::from_point(edge.start_xz);
        let end = NodeHeightPointKey::from_point(edge.end_xz);
        if !height_point_key_lies_on_segment(point, start, end) {
            continue;
        }
        let dx = end.x_key - start.x_key;
        let dz = end.z_key - start.z_key;
        let denominator = if dx.abs() >= dz.abs() { dx } else { dz };
        if denominator == 0 {
            continue;
        }
        let numerator = if dx.abs() >= dz.abs() {
            point.x_key - start.x_key
        } else {
            point.z_key - start.z_key
        };
        let t = numerator as f64 / denominator as f64;
        return Some(edge.start_height_m + (edge.end_height_m - edge.start_height_m) * t);
    }
    None
}

fn height_source_point_key(point: RoadVec2) -> NodeHeightSourcePointKey {
    (
        (point.x * HEIGHT_SOURCE_KEY_SCALE).round() as i64,
        (point.y * HEIGHT_SOURCE_KEY_SCALE).round() as i64,
    )
}

fn height_triangle_area2(
    a: NodeHeightSourcePointKey,
    b: NodeHeightSourcePointKey,
    c: NodeHeightSourcePointKey,
) -> i128 {
    let ab_x = i128::from(b.0 - a.0);
    let ab_z = i128::from(b.1 - a.1);
    let ac_x = i128::from(c.0 - a.0);
    let ac_z = i128::from(c.1 - a.1);
    ab_x * ac_z - ab_z * ac_x
}

fn height_source_point_key_lies_on_segment(
    point: NodeHeightSourcePointKey,
    start: NodeHeightSourcePointKey,
    end: NodeHeightSourcePointKey,
) -> bool {
    height_triangle_area2(start, end, point) == 0
        && point.0 >= start.0.min(end.0)
        && point.0 <= start.0.max(end.0)
        && point.1 >= start.1.min(end.1)
        && point.1 <= start.1.max(end.1)
}

fn height_source_point_key_quantization_cell_intersects_segment(
    point: NodeHeightSourcePointKey,
    start: NodeHeightSourcePointKey,
    end: NodeHeightSourcePointKey,
) -> bool {
    if start == end {
        return false;
    }
    let neighbor_radius_x2 = HEIGHT_SOURCE_EDGE_NEIGHBOR_UNITS * 2;
    let min_x2 = i128::from(point.0) * 2 - neighbor_radius_x2;
    let max_x2 = i128::from(point.0) * 2 + neighbor_radius_x2;
    let min_z2 = i128::from(point.1) * 2 - neighbor_radius_x2;
    let max_z2 = i128::from(point.1) * 2 + neighbor_radius_x2;
    let segment_start = (i128::from(start.0) * 2, i128::from(start.1) * 2);
    let segment_end = (i128::from(end.0) * 2, i128::from(end.1) * 2);
    if height_doubled_point_inside_axis_aligned_box(segment_start, min_x2, max_x2, min_z2, max_z2)
        || height_doubled_point_inside_axis_aligned_box(segment_end, min_x2, max_x2, min_z2, max_z2)
    {
        return true;
    }
    let lower_left = (min_x2, min_z2);
    let lower_right = (max_x2, min_z2);
    let upper_right = (max_x2, max_z2);
    let upper_left = (min_x2, max_z2);
    [
        (lower_left, lower_right),
        (lower_right, upper_right),
        (upper_right, upper_left),
        (upper_left, lower_left),
    ]
    .into_iter()
    .any(|(edge_start, edge_end)| {
        height_doubled_segments_intersect(segment_start, segment_end, edge_start, edge_end)
    })
}

fn height_doubled_point_inside_axis_aligned_box(
    point: (i128, i128),
    min_x: i128,
    max_x: i128,
    min_z: i128,
    max_z: i128,
) -> bool {
    point.0 >= min_x && point.0 <= max_x && point.1 >= min_z && point.1 <= max_z
}

fn height_doubled_segments_intersect(
    a: (i128, i128),
    b: (i128, i128),
    c: (i128, i128),
    d: (i128, i128),
) -> bool {
    let ab_c = height_doubled_triangle_area2(a, b, c);
    let ab_d = height_doubled_triangle_area2(a, b, d);
    let cd_a = height_doubled_triangle_area2(c, d, a);
    let cd_b = height_doubled_triangle_area2(c, d, b);
    if ab_c == 0 && height_doubled_point_on_segment(c, a, b) {
        return true;
    }
    if ab_d == 0 && height_doubled_point_on_segment(d, a, b) {
        return true;
    }
    if cd_a == 0 && height_doubled_point_on_segment(a, c, d) {
        return true;
    }
    if cd_b == 0 && height_doubled_point_on_segment(b, c, d) {
        return true;
    }
    (ab_c > 0) != (ab_d > 0) && (cd_a > 0) != (cd_b > 0)
}

fn height_doubled_triangle_area2(a: (i128, i128), b: (i128, i128), c: (i128, i128)) -> i128 {
    let ab_x = b.0 - a.0;
    let ab_z = b.1 - a.1;
    let ac_x = c.0 - a.0;
    let ac_z = c.1 - a.1;
    ab_x * ac_z - ab_z * ac_x
}

fn height_doubled_point_on_segment(
    point: (i128, i128),
    start: (i128, i128),
    end: (i128, i128),
) -> bool {
    point.0 >= start.0.min(end.0)
        && point.0 <= start.0.max(end.0)
        && point.1 >= start.1.min(end.1)
        && point.1 <= start.1.max(end.1)
}

fn height_point_key_lies_on_segment(
    point: NodeHeightPointKey,
    start: NodeHeightPointKey,
    end: NodeHeightPointKey,
) -> bool {
    let point = (point.x_key, point.z_key);
    let start = (start.x_key, start.z_key);
    let end = (end.x_key, end.z_key);
    height_triangle_area2(start, end, point) == 0
        && point.0 >= start.0.min(end.0)
        && point.0 <= start.0.max(end.0)
        && point.1 >= start.1.min(end.1)
        && point.1 <= start.1.max(end.1)
}

fn xz(point: RoadVec3) -> RoadVec2 {
    RoadVec2::new(point.x, point.z)
}

fn quantize_m(value: f64) -> i64 {
    (value * HEIGHT_SHARED_KEY_SCALE).round() as i64
}

impl NodeHeightPointKey {
    fn from_point(point: RoadVec2) -> Self {
        Self {
            x_key: (point.x * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
            z_key: (point.y * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
        }
    }

    fn x_mm(self) -> i64 {
        ((self.x_key as f64 / ROAD_OVERLAY_COORDINATE_SCALE) * HEIGHT_POINT_KEY_SCALE).round()
            as i64
    }

    fn z_mm(self) -> i64 {
        ((self.z_key as f64 / ROAD_OVERLAY_COORDINATE_SCALE) * HEIGHT_POINT_KEY_SCALE).round()
            as i64
    }
}

#[cfg(test)]
mod tests {
    use super::super::arrangement::NodeSeamSource;
    use super::*;
    use crate::simulation::network::surface::input::NodeInputMouth;
    use crate::simulation::network::surface::ownership::{
        NodeBooleanOwnership, NodeOwnedRegionArrangement,
    };
    use crate::simulation::network::surface::rails::NodeRailContourSet;
    use crate::simulation::network::surface::terminal::{
        TerminalCapBandProvenance, TerminalCapBandRole,
    };
    use crate::simulation::network::surface::{
        IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, OrderedIncidentPieceMouth,
    };
    use godot::prelude::{Vector2, Vector3};

    fn band(kind: RoadSurfaceBandKind, start: Vector3, end: Vector3) -> IncidentMouthBand {
        IncidentMouthBand {
            kind,
            start_point_world: start,
            end_point_world: end,
        }
    }

    fn profile(x: f32, base_height: f32) -> IncidentMouthProfile {
        let boundary_points_world = vec![
            Vector3::new(x, base_height, -4.0),
            Vector3::new(x, base_height + 0.1, -2.0),
            Vector3::new(x, base_height + 0.2, 0.0),
            Vector3::new(x, base_height + 0.3, 2.0),
            Vector3::new(x, base_height + 0.4, 4.0),
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

    fn solved_input() -> NodeArrangementInput {
        let mouth = OrderedIncidentPieceMouth {
            profile: profile(10.0, 4.0),
            endpoint_profile: profile(0.0, 2.0),
            boundary_paths_world: Vec::new(),
            band_start_paths_world: Vec::new(),
            band_end_paths_world: Vec::new(),
            uses_sampled_band_domain_paths: false,
            direction_angle_ccw: 0.0,
            direction_xz: Vector2::RIGHT,
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

    fn solved_ownership(input: &NodeArrangementInput) -> NodeBooleanOwnership {
        let rails = NodeRailContourSet::from_input(input).expect("test input should produce rails");
        NodeBooleanOwnership::from_rails(&rails).expect("test rails should produce ownership")
    }

    #[test]
    fn evaluates_owned_region_vertices_from_band_height_fields() {
        let input = solved_input();
        let ownership = solved_ownership(&input);
        let solution = NodeHeightSolution::from_ownership_and_input(&input, &ownership)
            .expect("valid ownership should height every canonical vertex");

        assert_eq!(solution.node_id, 42);
        assert_eq!(
            solution.piece_kind,
            RoadSurfaceVisualNodePieceKind::JunctionN
        );
        assert_eq!(solution.regions.len(), ownership.owned_regions.len());

        let carriageway = solution
            .regions
            .iter()
            .find(|region| region.kind == RoadSurfaceBandKind::Carriageway)
            .expect("test input has a carriageway band");
        assert!(has_vertex_height(carriageway, 0.0, 0.0, 2.2));
        assert!(has_vertex_height(carriageway, 10.0, 2.0, 4.3));
    }

    #[test]
    fn rejects_missing_source_band() {
        let input = conflicting_manual_input();
        let owned_regions = vec![manual_region(RoadSurfaceBandKind::Carriageway, 99, 2.0)];
        let ownership = NodeBooleanOwnership {
            node_id: 77,
            piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
            footprint_shapes: Vec::new(),
            asphalt_shapes: Vec::new(),
            non_road_shapes: Vec::new(),
            owned_region_arrangement: NodeOwnedRegionArrangement::from_owned_regions(
                77,
                RoadSurfaceVisualNodePieceKind::Bend,
                &owned_regions,
                &Vec::new(),
                &[],
            ),
            owned_regions,
        };

        assert_eq!(
            NodeHeightSolution::from_ownership_and_input(&input, &ownership),
            Err(NodeHeightFieldError::MissingSourceBand {
                mouth_order_index: 0,
                band_index: 99,
            })
        );
    }

    #[test]
    fn rejects_owned_region_vertex_outside_explicit_height_carrier() {
        let input = conflicting_manual_input();
        let mut region = manual_region(RoadSurfaceBandKind::Carriageway, 0, 2.0);
        region.shape = vec![vec![[0.0, 0.0], [10.0, 0.0], [11.0, 1.0], [0.0, 2.0]]];
        let owned_regions = vec![region];
        let ownership = NodeBooleanOwnership {
            node_id: 77,
            piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
            footprint_shapes: Vec::new(),
            asphalt_shapes: Vec::new(),
            non_road_shapes: Vec::new(),
            owned_region_arrangement: NodeOwnedRegionArrangement::from_owned_regions(
                77,
                RoadSurfaceVisualNodePieceKind::Bend,
                &owned_regions,
                &Vec::new(),
                &[],
            ),
            owned_regions,
        };

        assert!(matches!(
            NodeHeightSolution::from_ownership_and_input(&input, &ownership),
            Err(NodeHeightFieldError::VertexOutsideHeightField {
                mouth_order_index: 0,
                band_index: 0,
                source_kind: RoadSurfaceBandKind::Carriageway,
                ..
            })
        ));
    }

    #[test]
    fn junctionn_canonical_height_authority_rejects_vertex_outside_explicit_carrier() {
        let mut input = conflicting_manual_input();
        input.piece_kind = RoadSurfaceVisualNodePieceKind::JunctionN;
        let mut region = manual_region(RoadSurfaceBandKind::Carriageway, 0, 2.0);
        region.shape = vec![vec![[0.0, 0.0], [10.0, 0.0], [11.0, 1.0], [0.0, 2.0]]];
        let owned_regions = vec![region];
        let ownership = NodeBooleanOwnership {
            node_id: 77,
            piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            footprint_shapes: Vec::new(),
            asphalt_shapes: Vec::new(),
            non_road_shapes: Vec::new(),
            owned_region_arrangement: NodeOwnedRegionArrangement::from_owned_regions(
                77,
                RoadSurfaceVisualNodePieceKind::JunctionN,
                &owned_regions,
                &Vec::new(),
                &[],
            ),
            owned_regions,
        };

        assert!(matches!(
            NodeHeightSolution::from_ownership_and_input(&input, &ownership),
            Err(NodeHeightFieldError::VertexOutsideHeightField { .. })
        ));
    }

    #[test]
    fn junctionn_canonical_height_authority_prefers_owner_generated_carrier_over_base_interval() {
        let mut field = NodeBandHeightField::from_interval(
            0,
            &manual_interval(0, RoadSurfaceBandKind::Carriageway, 0.0, 0.0),
        );
        let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        field
            .patches
            .push(NodeBandHeightPatch::from_heighted_contour(
                &[
                    RoadVec3::new(0.0, 1.0, 0.0),
                    RoadVec3::new(10.0, 1.0, 0.0),
                    RoadVec3::new(10.0, 1.0, 2.0),
                    RoadVec3::new(0.0, 1.0, 2.0),
                ],
                NodeHeightPatchAuthority {
                    owner: Some(owner),
                    role: NodeHeightPatchAuthorityRole::GeneratedContour {
                        purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
                        claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
                    },
                },
            ));

        assert!(matches!(
            field.evaluate_height(RoadVec2::new(5.0, 1.0)),
            Err(NodeHeightFieldError::SourceHeightFieldConflict { .. })
        ));
        let height = field
            .evaluate_authorized_height(
                owner,
                NodeGeneratedContourClaimPriority::SideJoin,
                RoadVec2::new(5.0, 1.0),
            )
            .expect("owner-generated carrier is explicit height authority for JunctionN");
        assert!((height.height_m - 1.0).abs() <= 1.0e-6);
        assert_eq!(
            height.authority,
            NodeHeightAuthoritySource::GeneratedContour {
                purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
                claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
            }
        );
    }

    #[test]
    fn junctionn_canonical_height_authority_scopes_generated_carriers_to_owned_region_claim() {
        let mut field = NodeBandHeightField::from_interval(
            0,
            &manual_interval(0, RoadSurfaceBandKind::CurbOrShoulder, 0.0, 0.0),
        );
        let owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 0);
        for (height_m, purpose, claim_priority) in [
            (
                1.0,
                NodeGeneratedContourPurpose::NonRoadBand,
                NodeGeneratedContourClaimPriority::MouthBand,
            ),
            (
                2.0,
                NodeGeneratedContourPurpose::JunctionSideJoin,
                NodeGeneratedContourClaimPriority::SideJoin,
            ),
        ] {
            field
                .patches
                .push(NodeBandHeightPatch::from_heighted_contour(
                    &[
                        RoadVec3::new(0.0, height_m, 0.0),
                        RoadVec3::new(10.0, height_m, 0.0),
                        RoadVec3::new(10.0, height_m, 2.0),
                        RoadVec3::new(0.0, height_m, 2.0),
                    ],
                    NodeHeightPatchAuthority {
                        owner: Some(owner),
                        role: NodeHeightPatchAuthorityRole::GeneratedContour {
                            purpose,
                            claim_priority,
                        },
                    },
                ));
        }

        let mouth_height = field
            .evaluate_authorized_height(
                owner,
                NodeGeneratedContourClaimPriority::MouthBand,
                RoadVec2::new(5.0, 1.0),
            )
            .expect("mouth-owned region should use mouth-band generated carrier");
        assert_eq!(
            mouth_height.authority,
            NodeHeightAuthoritySource::GeneratedContour {
                purpose: NodeGeneratedContourPurpose::NonRoadBand,
                claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
            }
        );
        assert!((mouth_height.height_m - 1.0).abs() <= 1.0e-6);

        let side_join_height = field
            .evaluate_authorized_height(
                owner,
                NodeGeneratedContourClaimPriority::SideJoin,
                RoadVec2::new(5.0, 1.0),
            )
            .expect("side-join-owned region should use side-join generated carrier");
        assert_eq!(
            side_join_height.authority,
            NodeHeightAuthoritySource::GeneratedContour {
                purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
                claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
            }
        );
        assert!((side_join_height.height_m - 2.0).abs() <= 1.0e-6);
    }

    #[test]
    fn side_join_height_authority_reuses_source_rail_at_handoff_vertices() {
        let mut field = NodeBandHeightField::from_interval(
            0,
            &manual_interval(0, RoadSurfaceBandKind::Sidewalk, 0.0, 1.0),
        );
        let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 0);
        field
            .patches
            .push(NodeBandHeightPatch::from_heighted_contour(
                &[
                    RoadVec3::new(0.0, 2.0, 0.0),
                    RoadVec3::new(10.0, 2.0, 0.0),
                    RoadVec3::new(10.0, 2.0, 2.0),
                    RoadVec3::new(0.0, 2.0, 2.0),
                ],
                NodeHeightPatchAuthority {
                    owner: Some(owner),
                    role: NodeHeightPatchAuthorityRole::GeneratedContour {
                        purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
                        claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
                    },
                },
            ));

        let handoff_height = field
            .evaluate_authorized_height(
                owner,
                NodeGeneratedContourClaimPriority::SideJoin,
                RoadVec2::new(5.0, 0.0),
            )
            .expect("side-join vertex on source rail should reuse source rail height");
        assert_eq!(
            handoff_height.authority,
            NodeHeightAuthoritySource::SourceInterval
        );
        assert!((handoff_height.height_m - 0.5).abs() <= 1.0e-6);

        let drifted_handoff_height = field
            .evaluate_authorized_height(
                owner,
                NodeGeneratedContourClaimPriority::SideJoin,
                RoadVec2::new(5.0, 0.00005),
            )
            .expect("overlay-drifted side-join vertex on source rail should still reuse the rail");
        assert_eq!(
            drifted_handoff_height.authority,
            NodeHeightAuthoritySource::SourceInterval
        );
        assert!((drifted_handoff_height.height_m - 0.5).abs() <= 1.0e-6);

        let interior_height = field
            .evaluate_authorized_height(
                owner,
                NodeGeneratedContourClaimPriority::SideJoin,
                RoadVec2::new(5.0, 1.0),
            )
            .expect("side-join interior should still use generated contour authority");
        assert_eq!(
            interior_height.authority,
            NodeHeightAuthoritySource::GeneratedContour {
                purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
                claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
            }
        );
        assert!((interior_height.height_m - 2.0).abs() <= 1.0e-6);
    }

    #[test]
    fn junctionn_canonical_height_authority_rejects_conflicting_owner_generated_carriers() {
        let mut field = NodeBandHeightField::from_interval(
            0,
            &manual_interval(0, RoadSurfaceBandKind::Carriageway, 0.0, 0.0),
        );
        let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let authority = NodeHeightPatchAuthority {
            owner: Some(owner),
            role: NodeHeightPatchAuthorityRole::GeneratedContour {
                purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
                claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
            },
        };
        for height_m in [1.0, 2.0] {
            field
                .patches
                .push(NodeBandHeightPatch::from_heighted_contour(
                    &[
                        RoadVec3::new(0.0, height_m, 0.0),
                        RoadVec3::new(10.0, height_m, 0.0),
                        RoadVec3::new(10.0, height_m, 2.0),
                        RoadVec3::new(0.0, height_m, 2.0),
                    ],
                    authority,
                ));
        }

        assert!(matches!(
            field.evaluate_authorized_height(
                owner,
                NodeGeneratedContourClaimPriority::SideJoin,
                RoadVec2::new(5.0, 1.0)
            ),
            Err(NodeHeightFieldError::SourceHeightFieldConflict { .. })
        ));
    }

    #[test]
    fn height_solution_has_no_post_overlay_height_repair_path() {
        let source = include_str!("height.rs");
        for forbidden in [
            concat!("heighted_shape_with_", "canonical_contour_insertions"),
            concat!("heighted_contour_with_", "canonical_insertions"),
            concat!("fill_canonical_contour_", "height_insertions"),
            concat!("reheight_terminal_", "cap_band_from_base"),
            concat!("reheight_point_", "from_base"),
            concat!("from_terminal_cap_band_", "with_base"),
            concat!("evaluate_region_", "scoped_height"),
            concat!("bounded_region_", "scoped_edge_height"),
            concat!("region_scoped_", "carrier"),
        ] {
            assert!(
                !source.contains(forbidden),
                "canonical arrangement vertices must be inside their explicit height carrier, not repaired by `{forbidden}`"
            );
        }
    }

    fn terminal_cap_band_for_height_test(
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

    #[test]
    fn terminal_material_band_height_field_keeps_curb_cap_inner_rail_raised() {
        let inner_start = RoadVec3::new(0.0, 0.12, -1.0);
        let inner_center = RoadVec3::new(0.0, 0.12, 0.0);
        let inner_end = RoadVec3::new(0.0, 0.12, 1.0);
        let outer_start = RoadVec3::new(0.15, 0.12, -1.0);
        let outer_center = RoadVec3::new(0.15, 0.12, 0.0);
        let outer_end = RoadVec3::new(0.15, 0.12, 1.0);
        let cap_band = NodeTerminalCapBand {
            source_band_index: 0,
            band_kind: RoadSurfaceBandKind::CurbOrShoulder,
            provenance: TerminalCapBandProvenance {
                layer_index: 0,
                role: TerminalCapBandRole::EndBand,
                left_source_band_index: 0,
                right_source_band_index: 0,
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
        };
        let patch = NodeBandHeightPatch::from_terminal_cap_band(&cap_band);
        let height = match patch
            .evaluate_surface_height(
                NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::CurbOrShoulder),
                RoadSurfaceBandKind::CurbOrShoulder,
                RoadVec2::new(0.0, 0.0),
            )
            .expect("center vertex should be evaluable")
        {
            NodeHeightPatchEvaluation::Inside(height) => height,
            NodeHeightPatchEvaluation::Outside(error) => {
                panic!("center vertex should be inside terminal material band: {error:?}")
            }
        };

        assert!(
            (height - 0.12).abs() <= 1.0e-6,
            "terminal curb cap inner rail must stay raised across the carriageway split"
        );
    }

    #[test]
    fn terminal_cap_height_field_extends_with_explicit_cap_patches_only() {
        let first_cap = terminal_cap_band_for_height_test(0.0, 0.12, TerminalCapBandRole::EndBand);
        let second_cap =
            terminal_cap_band_for_height_test(1.0, 0.32, TerminalCapBandRole::RightSide);
        let mut field = NodeBandHeightField::from_terminal_cap_band(0, &first_cap);

        field
            .extend_with_terminal_cap_band(0, &second_cap)
            .expect("same terminal source may carry multiple explicit cap patches");

        let second_height = field
            .evaluate_height(RoadVec2::new(1.0, 0.0))
            .expect("second terminal cap patch should be an explicit carrier");
        assert!((second_height - 0.32).abs() <= 1.0e-6);
        assert!(matches!(
            field.evaluate_height(RoadVec2::new(0.5, 0.0)),
            Err(NodeHeightFieldError::VertexOutsideHeightField { .. })
        ));
    }

    #[test]
    fn shared_xz_vertices_keep_distinct_owner_source_heights() {
        let regions = vec![
            manual_heighted_region(
                RoadSurfaceBandKind::CurbOrShoulder,
                0,
                0.0,
                vec![manual_heighted_vertex(0.0, 0.0, 0.0)],
            ),
            manual_heighted_region(
                RoadSurfaceBandKind::Sidewalk,
                1,
                0.25,
                vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
            ),
        ];

        validate_shared_source_height_agreement(&regions)
            .expect("different owner/source contexts are explicit seams, not height repairs");

        assert_eq!(regions[0].shape[0][0].height_m, 0.0);
        assert_eq!(regions[1].shape[0][0].height_m, 0.25);
    }

    #[test]
    fn shared_xz_vertices_without_explicit_seam_are_not_height_constrained() {
        let regions = vec![
            manual_heighted_region(
                RoadSurfaceBandKind::CurbOrShoulder,
                0,
                0.0,
                vec![manual_heighted_vertex(0.0, 0.0, 0.0)],
            ),
            manual_heighted_region(
                RoadSurfaceBandKind::Sidewalk,
                1,
                0.25,
                vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
            ),
        ];

        validate_explicit_material_seam_heights(&regions)
            .expect("missing explicit seam must not trigger coincident-XZ height repair");

        assert_eq!(regions[0].shape[0][0].height_m, 0.0);
        assert_eq!(regions[1].shape[0][0].height_m, 0.25);
    }

    #[test]
    fn junctionn_same_material_shared_vertices_use_deterministic_sample_owner_height() {
        let mut regions = vec![
            manual_heighted_region(
                RoadSurfaceBandKind::Carriageway,
                9,
                0.0,
                vec![manual_heighted_vertex(-1.0, 0.0, 2.0)],
            ),
            manual_heighted_region(
                RoadSurfaceBandKind::Carriageway,
                14,
                0.0,
                vec![manual_heighted_vertex(-1.0, 0.0, 1.0)],
            ),
            manual_heighted_region(
                RoadSurfaceBandKind::CurbOrShoulder,
                1,
                0.0,
                vec![manual_heighted_vertex(-1.0, 0.0, 0.25)],
            ),
        ];

        apply_junctionn_same_material_vertex_height_tiebreak(&mut regions);

        assert_eq!(regions[0].shape[0][0].height_m, 2.0);
        assert_eq!(
            regions[1].shape[0][0].height_m, 2.0,
            "same-material carriageway samples should use the deterministic lower source key"
        );
        assert_eq!(
            regions[2].shape[0][0].height_m, 0.25,
            "different materials must not be pulled into the same-material tie-break"
        );
    }

    #[test]
    fn junctionn_same_material_tiebreak_preserves_explicit_cross_material_seam_height() {
        let sidewalk_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 0);
        let other_sidewalk_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
        let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 5);
        let seam = manual_owned_pair_seam_constraint(77, curb_owner, sidewalk_owner, true);
        let mut regions = vec![
            manual_heighted_region_with_seams(
                RoadSurfaceBandKind::Sidewalk,
                sidewalk_owner.owner_index(),
                0.0,
                vec![manual_heighted_vertex(0.0, 0.0, 1.0)],
                vec![seam.clone()],
            ),
            manual_heighted_region(
                RoadSurfaceBandKind::Sidewalk,
                other_sidewalk_owner.owner_index(),
                0.0,
                vec![manual_heighted_vertex(0.0, 0.0, 2.0)],
            ),
            manual_heighted_region_with_seams(
                RoadSurfaceBandKind::CurbOrShoulder,
                curb_owner.owner_index(),
                0.0,
                vec![manual_heighted_vertex(0.0, 0.0, 1.0)],
                vec![seam],
            ),
        ];

        apply_junctionn_same_material_vertex_height_tiebreak(&mut regions);

        assert_eq!(
            regions[0].shape[0][0].height_m, 1.0,
            "explicit curb/sidewalk seam containment must outrank same-material tie-breaks"
        );
        assert_eq!(regions[1].shape[0][0].height_m, 2.0);
        assert_eq!(regions[2].shape[0][0].height_m, 1.0);
        validate_explicit_material_seam_heights(&regions)
            .expect("preserved seam heights should still validate");
    }

    #[test]
    fn explicit_curb_sidewalk_seam_rejects_shared_height_disagreement() {
        let seam = manual_seam_constraint(
            12,
            NodeSeamSource::RaisedStepContact { owner_index: 0 },
            true,
            true,
        );
        let regions = vec![
            manual_heighted_region_with_seams(
                RoadSurfaceBandKind::CurbOrShoulder,
                0,
                0.0,
                vec![manual_heighted_vertex(0.0, 0.0, 0.0)],
                vec![seam.clone()],
            ),
            manual_heighted_region_with_seams(
                RoadSurfaceBandKind::Sidewalk,
                1,
                0.25,
                vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
                vec![seam],
            ),
        ];

        assert!(matches!(
            validate_explicit_material_seam_heights(&regions),
            Err(NodeHeightFieldError::SharedSourceHeightConflict { .. })
        ));
    }

    #[test]
    fn explicit_curb_sidewalk_seam_accepts_matching_shared_height() {
        let seam = manual_seam_constraint(
            12,
            NodeSeamSource::RaisedStepContact { owner_index: 0 },
            true,
            true,
        );
        let regions = vec![
            manual_heighted_region_with_seams(
                RoadSurfaceBandKind::CurbOrShoulder,
                0,
                0.0,
                vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
                vec![seam.clone()],
            ),
            manual_heighted_region_with_seams(
                RoadSurfaceBandKind::Sidewalk,
                1,
                0.25,
                vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
                vec![seam],
            ),
        ];

        validate_explicit_material_seam_heights(&regions)
            .expect("explicit seam authority may only accept already matching heights");
    }

    #[test]
    fn same_source_constraint_index_keeps_distinct_owner_pair_height_contexts() {
        let first_pair = manual_owned_pair_seam_constraint(
            12,
            NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 0),
            NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1),
            true,
        );
        let second_pair = manual_owned_pair_seam_constraint(
            12,
            NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 2),
            NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 3),
            true,
        );
        let regions = vec![
            manual_heighted_region_with_seams(
                RoadSurfaceBandKind::CurbOrShoulder,
                0,
                0.0,
                vec![manual_heighted_vertex(0.0, 0.0, 1.0)],
                vec![first_pair.clone()],
            ),
            manual_heighted_region_with_seams(
                RoadSurfaceBandKind::Sidewalk,
                1,
                0.25,
                vec![manual_heighted_vertex(0.0, 0.0, 1.0)],
                vec![first_pair],
            ),
            manual_heighted_region_with_seams(
                RoadSurfaceBandKind::CurbOrShoulder,
                2,
                0.0,
                vec![manual_heighted_vertex(0.0, 0.0, 2.0)],
                vec![second_pair.clone()],
            ),
            manual_heighted_region_with_seams(
                RoadSurfaceBandKind::Sidewalk,
                3,
                0.25,
                vec![manual_heighted_vertex(0.0, 0.0, 2.0)],
                vec![second_pair],
            ),
        ];

        validate_explicit_material_seam_heights(&regions)
            .expect("same source rail index may materialize distinct final owner-pair seams");
    }

    #[test]
    fn asphalt_curb_seams_allow_explicit_vertical_height_step() {
        let seam = manual_seam_constraint(
            3,
            NodeSeamSource::RaisedStepContact { owner_index: 0 },
            false,
            true,
        );
        let regions = vec![
            manual_heighted_region_with_seams(
                RoadSurfaceBandKind::Carriageway,
                0,
                0.0,
                vec![manual_heighted_vertex(0.0, 0.0, 0.0)],
                vec![seam.clone()],
            ),
            manual_heighted_region_with_seams(
                RoadSurfaceBandKind::CurbOrShoulder,
                1,
                0.25,
                vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
                vec![seam],
            ),
        ];

        validate_explicit_material_seam_heights(&regions)
            .expect("asphalt / curb contact is a vertical material step, not shared-height repair");
    }

    #[test]
    fn shared_xz_vertices_reject_same_source_height_conflict() {
        let regions = vec![
            manual_heighted_region(
                RoadSurfaceBandKind::Sidewalk,
                0,
                0.0,
                vec![manual_heighted_vertex(0.0, 0.0, 0.0)],
            ),
            manual_heighted_region(
                RoadSurfaceBandKind::Sidewalk,
                0,
                0.25,
                vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
            ),
        ];

        assert!(matches!(
            validate_shared_source_height_agreement(&regions),
            Err(NodeHeightFieldError::SharedSourceHeightConflict { .. })
        ));
    }

    fn has_vertex_height(
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

    fn conflicting_manual_input() -> NodeArrangementInput {
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
                uses_sampled_band_domain_paths: false,
            }],
        }
    }

    fn manual_interval(
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

    fn manual_region(
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

    fn manual_heighted_region(
        kind: RoadSurfaceBandKind,
        owner_index: usize,
        area_m2: f32,
        contour: NodeHeightedContour,
    ) -> NodeHeightedRegion {
        manual_heighted_region_with_seams(kind, owner_index, area_m2, contour, Vec::new())
    }

    fn manual_heighted_region_with_seams(
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

    fn manual_seam_constraint(
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

    fn manual_owned_pair_seam_constraint(
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

    fn manual_heighted_vertex(x: f64, z: f64, height_m: f64) -> NodeHeightedVertex {
        NodeHeightedVertex {
            point_xz: RoadVec2::new(x, z),
            height_m,
            height_field_id: NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Sidewalk),
            height_authority: None,
        }
    }
}
