//! Spline-backed height evaluation for canonical node-owned regions.

use super::arrangement::{
    NodeBandHeightFieldId, NodeBandOwner, NodeRegionSeamConstraint, seam_source_priority,
};
use super::backend::{
    ROAD_OVERLAY_COORDINATE_SCALE, RoadVec2, RoadVec3, overlay_point_to_road,
    quantize_road_vec2_to_overlay_grid,
};
use super::input::{
    NodeArrangementInput, NodeInputBandInterval, NodeInputTerminalEndBand,
    NodeInputTerminalEndBandBoundaryMode,
};
use super::ownership::{NodeBooleanOwnedRegion, NodeBooleanOwnership};
use super::rails::{
    NodeGeneratedContour, NodeGeneratedContourClaimPriority, NodeGeneratedContourKind,
    NodeRailContourSet,
};
use super::{
    NodeOverlayContour, NodeOverlayPoint, NodeOverlayShape, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceVisualNodePieceKind, SurfaceCdt,
};
use spade::{Point2, Triangulation};
use splines::{Interpolation, Key, Spline};
use std::collections::BTreeMap;

const HEIGHT_POINT_KEY_SCALE: f64 = 1000.0;
const HEIGHT_SHARED_KEY_SCALE: f64 = 1000.0;
const HEIGHT_SOURCE_KEY_SCALE: f64 = ROAD_OVERLAY_COORDINATE_SCALE;
const HEIGHT_PARAMETER_KEY_SCALE: f64 = 1_000_000.0;
const HEIGHT_PARAMETER_BOUNDARY_EPS: f64 = 32.0 / ROAD_OVERLAY_COORDINATE_SCALE;
const HEIGHT_PARAMETER_BOUNDARY_DISTANCE_EPS_M: f64 = 0.002;
const HEIGHT_FIELD_MIN_AXIS_LEN2_M2: f64 = 1.0e-12;
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
    DegenerateHeightField {
        mouth_order_index: usize,
        band_index: usize,
        axis: &'static str,
    },
    VertexOutsideHeightField {
        mouth_order_index: usize,
        band_index: usize,
        source_kind: RoadSurfaceBandKind,
        point_x_mm: i64,
        point_z_mm: i64,
        axis: &'static str,
        raw_parameter: f64,
    },
    HeightSampleFailed {
        mouth_order_index: usize,
        band_index: usize,
        axis: &'static str,
        parameter: f64,
    },
    SourceHeightFieldConflict {
        mouth_order_index: usize,
        band_index: usize,
        point_x_mm: i64,
        point_z_mm: i64,
        existing_height_mm: i64,
        incoming_height_mm: i64,
    },
    SharedSourceHeightConflict {
        point_x_mm: i64,
        point_z_mm: i64,
        kind: RoadSurfaceBandKind,
        owner_index: usize,
        constraint_index: Option<usize>,
        existing_height_mm: i64,
        incoming_height_mm: i64,
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

struct NodeBandHeightField {
    id: NodeBandHeightFieldId,
    kind: RoadSurfaceBandKind,
    patches: Vec<NodeBandHeightPatch>,
}

struct NodeBandHeightPatch {
    endpoint_start_xz: RoadVec2,
    endpoint_end_xz: RoadVec2,
    mouth_start_xz: RoadVec2,
    mouth_end_xz: RoadVec2,
    start_height_profile: Spline<f64, f64>,
    end_height_profile: Spline<f64, f64>,
    triangles: Option<Vec<NodeBandHeightTriangle>>,
    contour_edges: Option<Vec<NodeBandHeightEdge>>,
    allow_parametric_fallback: bool,
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

enum NodeHeightPatchEvaluation {
    Inside(f64),
    Outside(NodeHeightFieldError),
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

        for region in &ownership.owned_regions {
            let region = heighted_region(region, &fields)?;
            if !region.shape.is_empty() {
                regions.push(region);
            }
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

    fn from_terminal_end_band(
        mouth_order_index: usize,
        end_band: &NodeInputTerminalEndBand,
    ) -> Self {
        let id = NodeBandHeightFieldId::new(
            mouth_order_index,
            end_band.source_band_index,
            end_band.band_kind,
        );
        Self {
            id,
            kind: end_band.band_kind,
            patches: vec![NodeBandHeightPatch::from_terminal_end_band(end_band)],
        }
    }

    fn extend_with_terminal_end_band(
        &mut self,
        mouth_order_index: usize,
        end_band: &NodeInputTerminalEndBand,
    ) -> Result<(), NodeHeightFieldError> {
        if end_band.band_kind != self.kind {
            return Err(NodeHeightFieldError::SourceBandKindMismatch {
                mouth_order_index,
                band_index: end_band.source_band_index,
                region_kind: self.kind,
                source_kind: end_band.band_kind,
            });
        }
        let extension = Self::from_terminal_end_band_with_base(mouth_order_index, end_band, self)?;
        self.patches.extend(extension.patches);
        Ok(())
    }

    fn extend_with_generated_contour(
        &mut self,
        contour: &NodeGeneratedContour,
    ) -> Result<(), NodeHeightFieldError> {
        let patch = NodeBandHeightPatch::from_generated_contour(contour, self)?;
        self.patches.push(patch);
        Ok(())
    }

    fn from_terminal_end_band_with_base(
        mouth_order_index: usize,
        end_band: &NodeInputTerminalEndBand,
        base: &Self,
    ) -> Result<Self, NodeHeightFieldError> {
        let end_band = reheight_terminal_end_band_from_base(end_band, base)?;
        Ok(Self::from_terminal_end_band(mouth_order_index, &end_band))
    }

    fn evaluate_height(&self, point_xz: RoadVec2) -> Result<f64, NodeHeightFieldError> {
        let mut candidates = Vec::new();
        let mut outside_error = None;
        for patch in &self.patches {
            match patch.evaluate_surface_height(self.id, self.kind, point_xz)? {
                NodeHeightPatchEvaluation::Inside(height_m) => candidates.push(height_m),
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
                    point_x_mm: key.x_mm(),
                    point_z_mm: key.z_mm(),
                    axis: "patch",
                    raw_parameter: f64::NAN,
                }
            }));
        }

        self.agreed_height(point_xz, candidates)
    }

    fn agreed_height(
        &self,
        point_xz: RoadVec2,
        heights_m: Vec<f64>,
    ) -> Result<f64, NodeHeightFieldError> {
        let Some(first_height_m) = heights_m.first().copied() else {
            let key = NodeHeightPointKey::from_point(point_xz);
            return Err(NodeHeightFieldError::VertexOutsideHeightField {
                mouth_order_index: self.id.mouth_order_index(),
                band_index: self.id.band_index(),
                source_kind: self.kind,
                point_x_mm: key.x_mm(),
                point_z_mm: key.z_mm(),
                axis: "patch",
                raw_parameter: f64::NAN,
            });
        };
        let first_height_mm = quantize_m(first_height_m);
        for height_m in heights_m.iter().copied().skip(1) {
            let height_mm = quantize_m(height_m);
            if height_mm != first_height_mm {
                let key = NodeHeightPointKey::from_point(point_xz);
                return Err(NodeHeightFieldError::SourceHeightFieldConflict {
                    mouth_order_index: self.id.mouth_order_index(),
                    band_index: self.id.band_index(),
                    point_x_mm: key.x_mm(),
                    point_z_mm: key.z_mm(),
                    existing_height_mm: first_height_mm,
                    incoming_height_mm: height_mm,
                });
            }
        }
        Ok(first_height_m)
    }
}

fn reheight_terminal_end_band_from_base(
    end_band: &NodeInputTerminalEndBand,
    base: &NodeBandHeightField,
) -> Result<NodeInputTerminalEndBand, NodeHeightFieldError> {
    let mut end_band = end_band.clone();
    end_band.inner_start_world = reheight_point_from_base(end_band.inner_start_world, base)?;
    end_band.inner_end_world = reheight_point_from_base(end_band.inner_end_world, base)?;
    end_band.outer_start_world = reheight_point_from_base(end_band.outer_start_world, base)?;
    end_band.outer_end_world = reheight_point_from_base(end_band.outer_end_world, base)?;
    for point in &mut end_band.contour_world {
        *point = reheight_point_from_base(*point, base)?;
    }
    Ok(end_band)
}

fn reheight_point_from_base(
    point: RoadVec3,
    base: &NodeBandHeightField,
) -> Result<RoadVec3, NodeHeightFieldError> {
    let point_xz = quantize_road_vec2_to_overlay_grid(xz(point));
    match base.evaluate_height(point_xz) {
        Ok(height_m) => Ok(RoadVec3::new(point.x, height_m, point.z)),
        Err(NodeHeightFieldError::VertexOutsideHeightField { .. }) => Ok(point),
        Err(error) => Err(error),
    }
}

impl NodeBandHeightPatch {
    fn from_interval(interval: &NodeInputBandInterval) -> Self {
        let endpoint_start_xz =
            quantize_road_vec2_to_overlay_grid(xz(interval.endpoint_start_world));
        let endpoint_end_xz = quantize_road_vec2_to_overlay_grid(xz(interval.endpoint_end_world));
        let mouth_start_xz = quantize_road_vec2_to_overlay_grid(xz(interval.mouth_start_world));
        let mouth_end_xz = quantize_road_vec2_to_overlay_grid(xz(interval.mouth_end_world));

        Self {
            endpoint_start_xz,
            endpoint_end_xz,
            mouth_start_xz,
            mouth_end_xz,
            start_height_profile: linear_height_profile(
                interval.endpoint_start_world.y,
                interval.mouth_start_world.y,
            ),
            end_height_profile: linear_height_profile(
                interval.endpoint_end_world.y,
                interval.mouth_end_world.y,
            ),
            triangles: Some(interval_height_triangles(interval)),
            contour_edges: Some(interval_height_edges(interval)),
            allow_parametric_fallback: true,
        }
    }

    fn from_terminal_end_band(end_band: &NodeInputTerminalEndBand) -> Self {
        let endpoint_start_xz = quantize_road_vec2_to_overlay_grid(xz(end_band.inner_start_world));
        let endpoint_end_xz = quantize_road_vec2_to_overlay_grid(xz(end_band.inner_end_world));
        let mouth_start_xz = quantize_road_vec2_to_overlay_grid(xz(end_band.outer_start_world));
        let mouth_end_xz = quantize_road_vec2_to_overlay_grid(xz(end_band.outer_end_world));

        Self {
            endpoint_start_xz,
            endpoint_end_xz,
            mouth_start_xz,
            mouth_end_xz,
            start_height_profile: linear_height_profile(
                end_band.inner_start_world.y,
                end_band.outer_start_world.y,
            ),
            end_height_profile: linear_height_profile(
                end_band.inner_end_world.y,
                end_band.outer_end_world.y,
            ),
            triangles: Some(terminal_end_band_height_triangles(end_band)),
            contour_edges: Some(terminal_end_band_height_edges(end_band)),
            allow_parametric_fallback: end_band.boundary_mode
                == NodeInputTerminalEndBandBoundaryMode::TerminalMaterialBand,
        }
    }

    fn from_generated_contour(
        contour: &NodeGeneratedContour,
        base: &NodeBandHeightField,
    ) -> Result<Self, NodeHeightFieldError> {
        let mut points = Vec::with_capacity(contour.points_xz.len());
        for point_xz in &contour.points_xz {
            let point_xz = quantize_road_vec2_to_overlay_grid(*point_xz);
            let height_m = base.evaluate_height(point_xz)?;
            points.push(RoadVec3::new(point_xz.x, height_m, point_xz.y));
        }
        Ok(Self::from_heighted_contour(&points))
    }

    fn from_heighted_contour(points: &[RoadVec3]) -> Self {
        let (edge_start, edge_end) = nondegenerate_contour_height_edge(points);
        Self {
            endpoint_start_xz: quantize_road_vec2_to_overlay_grid(xz(edge_start)),
            endpoint_end_xz: quantize_road_vec2_to_overlay_grid(xz(edge_end)),
            mouth_start_xz: quantize_road_vec2_to_overlay_grid(xz(edge_start)),
            mouth_end_xz: quantize_road_vec2_to_overlay_grid(xz(edge_end)),
            start_height_profile: linear_height_profile(edge_start.y, edge_start.y),
            end_height_profile: linear_height_profile(edge_end.y, edge_end.y),
            triangles: Some(height_triangles_from_contour(points)),
            contour_edges: Some(height_edges_from_vertices(points)),
            allow_parametric_fallback: false,
        }
    }

    fn evaluate_surface_height(
        &self,
        id: NodeBandHeightFieldId,
        source_kind: RoadSurfaceBandKind,
        point_xz: RoadVec2,
    ) -> Result<NodeHeightPatchEvaluation, NodeHeightFieldError> {
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
        if let Some(edges) = &self.contour_edges
            && let Some(height_m) = terminal_edge_height_at(point_xz, edges)
        {
            return Ok(NodeHeightPatchEvaluation::Inside(height_m));
        }
        if self.triangles.is_some() && !self.allow_parametric_fallback {
            return Ok(NodeHeightPatchEvaluation::Outside(
                triangle_outside_error.unwrap_or_else(|| {
                    self.outside_field_error(
                        id,
                        source_kind,
                        point_xz,
                        "terminal_contour",
                        f64::NAN,
                    )
                }),
            ));
        }

        let endpoint_center = midpoint(self.endpoint_start_xz, self.endpoint_end_xz);
        let mouth_center = midpoint(self.mouth_start_xz, self.mouth_end_xz);
        let longitudinal_axis = mouth_center - endpoint_center;
        let longitudinal_len2 = longitudinal_axis.length_squared();
        if longitudinal_len2 <= HEIGHT_FIELD_MIN_AXIS_LEN2_M2 {
            return Err(NodeHeightFieldError::DegenerateHeightField {
                mouth_order_index: id.mouth_order_index(),
                band_index: id.band_index(),
                axis: "longitudinal",
            });
        }
        let longitudinal_len_m = longitudinal_len2.sqrt();

        let raw_t = (point_xz - endpoint_center).dot(longitudinal_axis) / longitudinal_len2;
        let Some(t) = canonical_unit_parameter(raw_t, longitudinal_len_m) else {
            return Ok(NodeHeightPatchEvaluation::Outside(
                self.outside_field_error(id, source_kind, point_xz, "longitudinal", raw_t),
            ));
        };

        let start_xz = interpolate(self.endpoint_start_xz, self.mouth_start_xz, t);
        let end_xz = interpolate(self.endpoint_end_xz, self.mouth_end_xz, t);
        let lateral_axis = end_xz - start_xz;
        let lateral_len2 = lateral_axis.length_squared();
        if lateral_len2 <= HEIGHT_FIELD_MIN_AXIS_LEN2_M2 {
            return Err(NodeHeightFieldError::DegenerateHeightField {
                mouth_order_index: id.mouth_order_index(),
                band_index: id.band_index(),
                axis: "lateral",
            });
        }
        let lateral_len_m = lateral_len2.sqrt();

        let raw_u = (point_xz - start_xz).dot(lateral_axis) / lateral_len2;
        let Some(u) = canonical_unit_parameter(raw_u, lateral_len_m) else {
            return Ok(NodeHeightPatchEvaluation::Outside(
                self.outside_field_error(id, source_kind, point_xz, "lateral", raw_u),
            ));
        };
        let start_height = self.start_height_profile.clamped_sample(t).ok_or(
            NodeHeightFieldError::HeightSampleFailed {
                mouth_order_index: id.mouth_order_index(),
                band_index: id.band_index(),
                axis: "start",
                parameter: t,
            },
        )?;
        let end_height = self.end_height_profile.clamped_sample(t).ok_or(
            NodeHeightFieldError::HeightSampleFailed {
                mouth_order_index: id.mouth_order_index(),
                band_index: id.band_index(),
                axis: "end",
                parameter: t,
            },
        )?;

        Ok(NodeHeightPatchEvaluation::Inside(
            start_height + (end_height - start_height) * u,
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
    let mut fields = BTreeMap::new();
    for mouth in &input.mouths {
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
        for end_band in &mouth.terminal_end_bands {
            let field = NodeBandHeightField::from_terminal_end_band(mouth.order_index, end_band);
            let key = NodeSourceBandKey {
                mouth_order_index: mouth.order_index,
                band_index: end_band.source_band_index,
            };
            if let Some(existing) = fields.get_mut(&key) {
                existing.extend_with_terminal_end_band(mouth.order_index, end_band)?;
            } else {
                fields.insert(key, field);
            }
        }
    }
    if let Some(rails) = rails {
        extend_height_fields_with_generated_contours(rails, &mut fields)?;
    }
    Ok(fields)
}

fn extend_height_fields_with_generated_contours(
    rails: &NodeRailContourSet,
    fields: &mut BTreeMap<NodeSourceBandKey, NodeBandHeightField>,
) -> Result<(), NodeHeightFieldError> {
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
        field.extend_with_generated_contour(contour)?;
    }
    Ok(())
}

fn heighted_region(
    region: &NodeBooleanOwnedRegion,
    fields: &BTreeMap<NodeSourceBandKey, NodeBandHeightField>,
) -> Result<NodeHeightedRegion, NodeHeightFieldError> {
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
    let shape = match heighted_shape(&region.shape, field) {
        Ok(shape) => shape,
        Err(error)
            if matches!(
                region.claim_priority,
                NodeGeneratedContourClaimPriority::SideJoin
                    | NodeGeneratedContourClaimPriority::JoinOrCap
            ) =>
        {
            heighted_shape_with_canonical_contour_insertions(&region.shape, field, error)?
        }
        Err(error) => return Err(error),
    };

    Ok(NodeHeightedRegion {
        kind: region.kind,
        owner: region.owner,
        height_field_id: field.id,
        shape,
        area_m2: region.area_m2,
        seam_constraints: region.seam_constraints.clone(),
    })
}

fn heighted_shape(
    shape: &NodeOverlayShape,
    field: &NodeBandHeightField,
) -> Result<NodeHeightedShape, NodeHeightFieldError> {
    let mut heighted = Vec::with_capacity(shape.len());
    for contour in shape {
        let contour = heighted_contour(contour, field)?;
        if contour.len() >= 3 {
            heighted.push(contour);
        }
    }
    Ok(heighted)
}

fn heighted_shape_with_canonical_contour_insertions(
    shape: &NodeOverlayShape,
    field: &NodeBandHeightField,
    original_error: NodeHeightFieldError,
) -> Result<NodeHeightedShape, NodeHeightFieldError> {
    let mut heighted = Vec::with_capacity(shape.len());
    for contour in shape {
        let contour =
            heighted_contour_with_canonical_insertions(contour, field, original_error.clone())?;
        if contour.len() >= 3 {
            heighted.push(contour);
        }
    }
    Ok(heighted)
}

fn heighted_contour(
    contour: &NodeOverlayContour,
    field: &NodeBandHeightField,
) -> Result<NodeHeightedContour, NodeHeightFieldError> {
    contour
        .iter()
        .copied()
        .map(|point| heighted_vertex(point, field))
        .collect()
}

fn heighted_contour_with_canonical_insertions(
    contour: &NodeOverlayContour,
    field: &NodeBandHeightField,
    original_error: NodeHeightFieldError,
) -> Result<NodeHeightedContour, NodeHeightFieldError> {
    let mut vertices = Vec::with_capacity(contour.len());
    let mut solved_indices = Vec::new();
    let mut first_outside_error = None;

    for (index, point) in contour.iter().copied().enumerate() {
        let point_xz = quantize_road_vec2_to_overlay_grid(overlay_point_to_road(point));
        match field.evaluate_height(point_xz) {
            Ok(height_m) => {
                vertices.push((point_xz, Some(quantize_source_height_m(height_m))));
                solved_indices.push(index);
            }
            Err(error @ NodeHeightFieldError::VertexOutsideHeightField { .. }) => {
                first_outside_error.get_or_insert(error);
                vertices.push((point_xz, None));
            }
            Err(error) => return Err(error),
        }
    }

    if first_outside_error.is_none() {
        return Ok(vertices
            .into_iter()
            .map(|(point_xz, height_m)| NodeHeightedVertex {
                point_xz,
                height_m: height_m.expect("directly solved contour vertex has a height"),
                height_field_id: field.id,
            })
            .collect());
    }
    if solved_indices.len() < 2 {
        return Err(first_outside_error.unwrap_or(original_error));
    }

    fill_canonical_contour_height_insertions(
        &mut vertices,
        first_outside_error
            .clone()
            .unwrap_or(original_error.clone()),
    )?;

    vertices
        .into_iter()
        .map(|(point_xz, height_m)| {
            let height_m = height_m.ok_or_else(|| {
                first_outside_error
                    .clone()
                    .unwrap_or(original_error.clone())
            })?;
            Ok(NodeHeightedVertex {
                point_xz,
                height_m,
                height_field_id: field.id,
            })
        })
        .collect()
}

fn fill_canonical_contour_height_insertions(
    vertices: &mut [(RoadVec2, Option<f64>)],
    outside_error: NodeHeightFieldError,
) -> Result<(), NodeHeightFieldError> {
    let Some(first_solved_index) = vertices.iter().position(|(_, height_m)| height_m.is_some())
    else {
        return Err(outside_error);
    };
    if vertices
        .iter()
        .filter(|(_, height_m)| height_m.is_some())
        .count()
        < 2
    {
        return Err(outside_error);
    }

    let mut ordered_indices = Vec::with_capacity(vertices.len() + 1);
    ordered_indices.extend(first_solved_index..vertices.len());
    ordered_indices.extend(0..=first_solved_index);

    let mut start_pos = 0;
    while start_pos + 1 < ordered_indices.len() {
        let start_index = ordered_indices[start_pos];
        let Some(start_height_m) = vertices[start_index].1 else {
            return Err(outside_error);
        };

        let Some(end_pos) = (start_pos + 1..ordered_indices.len())
            .find(|pos| vertices[ordered_indices[*pos]].1.is_some())
        else {
            return Err(outside_error);
        };
        if end_pos == start_pos + 1 {
            start_pos = end_pos;
            continue;
        }

        let end_index = ordered_indices[end_pos];
        let Some(end_height_m) = vertices[end_index].1 else {
            return Err(outside_error);
        };
        let mut cumulative_lengths = Vec::with_capacity(end_pos - start_pos + 1);
        cumulative_lengths.push(0.0);
        let mut total_length_m = 0.0;
        for pair_pos in start_pos..end_pos {
            let a = vertices[ordered_indices[pair_pos]].0;
            let b = vertices[ordered_indices[pair_pos + 1]].0;
            total_length_m += (b - a).length();
            cumulative_lengths.push(total_length_m);
        }
        if total_length_m <= HEIGHT_FIELD_MIN_AXIS_LEN2_M2.sqrt() {
            return Err(outside_error);
        }
        for run_offset in 1..cumulative_lengths.len() - 1 {
            let index = ordered_indices[start_pos + run_offset];
            let t = cumulative_lengths[run_offset] / total_length_m;
            let height_m = start_height_m + (end_height_m - start_height_m) * t;
            vertices[index].1 = Some(quantize_source_height_m(height_m));
        }
        start_pos = end_pos;
    }
    Ok(())
}

fn heighted_vertex(
    point: NodeOverlayPoint,
    field: &NodeBandHeightField,
) -> Result<NodeHeightedVertex, NodeHeightFieldError> {
    let point_xz = quantize_road_vec2_to_overlay_grid(overlay_point_to_road(point));
    Ok(NodeHeightedVertex {
        point_xz,
        height_m: field.evaluate_height(point_xz)?,
        height_field_id: field.id,
    })
}

fn validate_explicit_material_seam_heights(
    regions: &[NodeHeightedRegion],
) -> Result<(), NodeHeightFieldError> {
    let mut shared_heights = BTreeMap::<(NodeHeightPointKey, usize), ExplicitSeamHeight>::new();
    for region in regions.iter() {
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            for constraint in
                material_height_constraints_for_vertex(vertex.point_xz, &region.seam_constraints)
            {
                let point = NodeHeightPointKey::from_point(vertex.point_xz);
                let key = (point, constraint.constraint_index);
                let height_mm = quantize_m(vertex.height_m);
                if let Some(existing) = shared_heights.insert(key, ExplicitSeamHeight { height_mm })
                {
                    if existing.height_mm != height_mm {
                        return Err(NodeHeightFieldError::SharedSourceHeightConflict {
                            point_x_mm: point.x_mm(),
                            point_z_mm: point.z_mm(),
                            kind: region.kind,
                            owner_index: region.owner.owner_index(),
                            constraint_index: Some(constraint.constraint_index),
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
        )
    });
    matches.dedup_by_key(|constraint| constraint.constraint_index);
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
    let mut heights = BTreeMap::<NodeHeightVertexContextKey, i64>::new();
    for region in regions {
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            let point = NodeHeightPointKey::from_point(vertex.point_xz);
            let key = NodeHeightVertexContextKey {
                point,
                owner: region.owner,
                height_field_id: vertex.height_field_id,
            };
            let height_mm = quantize_m(vertex.height_m);
            if let Some(existing_height_mm) = heights.insert(key, height_mm)
                && existing_height_mm != height_mm
            {
                return Err(NodeHeightFieldError::SharedSourceHeightConflict {
                    point_x_mm: point.x_mm(),
                    point_z_mm: point.z_mm(),
                    kind: region.kind,
                    owner_index: region.owner.owner_index(),
                    constraint_index: None,
                    existing_height_mm,
                    incoming_height_mm: height_mm,
                });
            }
        }
    }
    Ok(())
}

fn linear_height_profile(endpoint_height_m: f64, mouth_height_m: f64) -> Spline<f64, f64> {
    Spline::from_vec(vec![
        Key::new(
            0.0,
            quantize_source_height_m(endpoint_height_m),
            Interpolation::Linear,
        ),
        Key::new(
            1.0,
            quantize_source_height_m(mouth_height_m),
            Interpolation::Linear,
        ),
    ])
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

fn terminal_end_band_height_triangles(
    end_band: &NodeInputTerminalEndBand,
) -> Vec<NodeBandHeightTriangle> {
    if end_band.boundary_mode == NodeInputTerminalEndBandBoundaryMode::TerminalMaterialBand
        && let Some(triangles) = terminal_material_band_height_triangles(&end_band.contour_world)
    {
        return triangles;
    }

    let mut triangles = height_triangles_from_contour(&end_band.contour_world);
    if end_band.boundary_mode == NodeInputTerminalEndBandBoundaryMode::SameOwnerOuterCap
        && end_band.contour_world.len() >= 3
    {
        let a_world = end_band.contour_world[0];
        let b_world = end_band.contour_world[1];
        let c_world = (end_band.outer_start_world + end_band.outer_end_world) * 0.5;
        let a_xz = quantize_road_vec2_to_overlay_grid(xz(a_world));
        let b_xz = quantize_road_vec2_to_overlay_grid(xz(b_world));
        let c_xz = quantize_road_vec2_to_overlay_grid(xz(c_world));
        if height_triangle_area2(
            height_source_point_key(a_xz),
            height_source_point_key(b_xz),
            height_source_point_key(c_xz),
        ) != 0
        {
            triangles.push(NodeBandHeightTriangle {
                a_xz,
                b_xz,
                c_xz,
                a_height_m: quantize_source_height_m(a_world.y),
                b_height_m: quantize_source_height_m(b_world.y),
                c_height_m: quantize_source_height_m(c_world.y),
            });
        }
    }
    triangles
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

fn terminal_end_band_height_edges(end_band: &NodeInputTerminalEndBand) -> Vec<NodeBandHeightEdge> {
    height_edges_from_vertices(&end_band.contour_world)
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
        if height_source_point_key_lies_on_segment(point, start, end) {
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

fn canonical_unit_parameter(raw_parameter: f64, axis_length_m: f64) -> Option<f64> {
    if !raw_parameter.is_finite() {
        return None;
    }

    let parameter =
        (raw_parameter * HEIGHT_PARAMETER_KEY_SCALE).round() / HEIGHT_PARAMETER_KEY_SCALE;
    let boundary_eps = if axis_length_m > f64::EPSILON {
        HEIGHT_PARAMETER_BOUNDARY_EPS.max(HEIGHT_PARAMETER_BOUNDARY_DISTANCE_EPS_M / axis_length_m)
    } else {
        HEIGHT_PARAMETER_BOUNDARY_EPS
    };
    (-boundary_eps..=1.0 + boundary_eps)
        .contains(&parameter)
        .then(|| {
            let parameter = parameter.clamp(0.0, 1.0);
            if parameter <= HEIGHT_PARAMETER_BOUNDARY_EPS {
                0.0
            } else if 1.0 - parameter <= HEIGHT_PARAMETER_BOUNDARY_EPS {
                1.0
            } else {
                parameter
            }
        })
}

fn nondegenerate_contour_height_edge(points: &[RoadVec3]) -> (RoadVec3, RoadVec3) {
    let first = points.first().copied().unwrap_or(RoadVec3::ZERO);
    for point in points.iter().copied().skip(1) {
        if xz(point).distance_squared(xz(first)) > HEIGHT_FIELD_MIN_AXIS_LEN2_M2 {
            return (first, point);
        }
    }
    (
        first,
        RoadVec3::new(
            first.x + HEIGHT_FIELD_MIN_AXIS_LEN2_M2.sqrt(),
            first.y,
            first.z,
        ),
    )
}

fn xz(point: RoadVec3) -> RoadVec2 {
    RoadVec2::new(point.x, point.z)
}

fn midpoint(start: RoadVec2, end: RoadVec2) -> RoadVec2 {
    (start + end) * 0.5
}

fn interpolate(start: RoadVec2, end: RoadVec2, t: f64) -> RoadVec2 {
    start + (end - start) * t
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
    fn terminal_material_band_height_field_keeps_curb_cap_inner_rail_raised() {
        let inner_start = RoadVec3::new(0.0, 0.12, -1.0);
        let inner_center = RoadVec3::new(0.0, 0.12, 0.0);
        let inner_end = RoadVec3::new(0.0, 0.12, 1.0);
        let outer_start = RoadVec3::new(0.15, 0.12, -1.0);
        let outer_center = RoadVec3::new(0.15, 0.12, 0.0);
        let outer_end = RoadVec3::new(0.15, 0.12, 1.0);
        let end_band = NodeInputTerminalEndBand {
            source_band_index: 0,
            band_kind: RoadSurfaceBandKind::CurbOrShoulder,
            boundary_mode: NodeInputTerminalEndBandBoundaryMode::TerminalMaterialBand,
            inner_start_world: inner_start,
            inner_end_world: inner_end,
            outer_start_world: outer_start,
            outer_end_world: outer_end,
            contour_world: vec![
                inner_start,
                inner_center,
                inner_end,
                outer_end,
                outer_center,
                outer_start,
            ],
        };
        let patch = NodeBandHeightPatch::from_terminal_end_band(&end_band);
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
    fn explicit_curb_sidewalk_seam_rejects_shared_height_disagreement() {
        let seam = manual_seam_constraint(
            12,
            NodeSeamSource::CurbSidewalkContact { owner_index: 0 },
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
            NodeSeamSource::CurbSidewalkContact { owner_index: 0 },
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
    fn asphalt_curb_seams_allow_explicit_vertical_height_step() {
        let seam = manual_seam_constraint(
            3,
            NodeSeamSource::AsphaltCurbContact { owner_index: 0 },
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
                terminal_end_bands: Vec::new(),
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

    fn manual_heighted_vertex(x: f64, z: f64, height_m: f64) -> NodeHeightedVertex {
        NodeHeightedVertex {
            point_xz: RoadVec2::new(x, z),
            height_m,
            height_field_id: NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Sidewalk),
        }
    }
}
