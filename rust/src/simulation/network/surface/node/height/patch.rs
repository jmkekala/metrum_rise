//! Height patch construction and local surface evaluation.

use super::carriers::*;
use super::handoff::{source_handoff_support_heights, source_support_heights};
use super::model::*;
use super::source_edges::height_source_point_key;
use super::triangles::*;
use super::vertices::height_vertex_heights_from_vertices;
use super::*;
use std::collections::BTreeMap;

impl NodeBandHeightPatch {
    pub(super) fn from_interval(
        id: NodeBandHeightFieldId,
        interval: &NodeInputBandInterval,
        source_support_points: Option<&[RoadVec3]>,
    ) -> Result<Self, NodeHeightFieldError> {
        let triangles = interval_height_carrier(id, interval)?;
        let explicit_vertices = interval_height_carrier_vertices(id, interval)?;
        let explicit_vertex_heights = height_vertex_heights_from_vertices(&explicit_vertices)
            .map_err(|error| {
                invalid_source_band_height_carrier_error(
                    id,
                    interval.band_kind,
                    error.diagnostic_reason(),
                )
            })?;
        let source_support_heights = source_support_heights(source_support_points);
        let source_handoff_support_heights =
            source_handoff_support_heights(interval, &source_support_heights);
        let mut contour_edge_support_heights = explicit_vertex_heights.clone();
        contour_edge_support_heights.extend(source_support_heights);
        Ok(Self {
            authority: NodeHeightPatchAuthority::source_interval(),
            explicit_vertex_heights,
            source_handoff_support_heights,
            contour_edge_support_heights,
            triangles: Some(triangles),
        })
    }

    pub(super) fn from_terminal_cap_band(
        id: NodeBandHeightFieldId,
        source_kind: RoadSurfaceBandKind,
        cap_band: &NodeTerminalCapBand,
    ) -> Result<Self, NodeHeightFieldError> {
        let authority = NodeHeightPatchAuthority::terminal_cap();
        let explicit_vertex_heights = height_vertex_heights_from_vertices(&cap_band.contour_world)
            .map_err(|error| NodeHeightFieldError::InvalidHeightCarrierContour {
                mouth_order_index: id.mouth_order_index(),
                band_index: id.band_index(),
                source_kind,
                height_field_id: id,
                authority: authority.source(),
                reason: error.diagnostic_reason(),
            })?;
        let contour_edge_support_heights = explicit_vertex_heights.clone();
        Ok(Self {
            authority,
            explicit_vertex_heights,
            source_handoff_support_heights: BTreeMap::new(),
            contour_edge_support_heights,
            triangles: Some(terminal_cap_band_height_triangles(
                id,
                source_kind,
                authority,
                cap_band,
            )?),
        })
    }

    pub(super) fn from_generated_contour(
        id: NodeBandHeightFieldId,
        source_kind: RoadSurfaceBandKind,
        contour: &NodeGeneratedContour,
    ) -> Result<Self, NodeHeightFieldError> {
        let Some(points_world) = &contour.height_points_world else {
            return Err(NodeHeightFieldError::MissingGeneratedContourHeightPoints {
                mouth_order_index: id.mouth_order_index(),
                band_index: id.band_index(),
                source_kind,
                height_field_id: id,
                purpose: contour.purpose,
                claim_priority: contour.claim_priority,
            });
        };
        Self::from_heighted_contour(
            id,
            source_kind,
            points_world,
            NodeHeightPatchAuthority::generated_contour(contour),
        )
    }

    pub(super) fn from_heighted_contour(
        id: NodeBandHeightFieldId,
        source_kind: RoadSurfaceBandKind,
        points: &[RoadVec3],
        authority: NodeHeightPatchAuthority,
    ) -> Result<Self, NodeHeightFieldError> {
        let explicit_vertex_heights =
            height_vertex_heights_from_vertices(points).map_err(|error| {
                NodeHeightFieldError::InvalidHeightCarrierContour {
                    mouth_order_index: id.mouth_order_index(),
                    band_index: id.band_index(),
                    source_kind,
                    height_field_id: id,
                    authority: authority.source(),
                    reason: error.diagnostic_reason(),
                }
            })?;
        let contour_edge_support_heights = explicit_vertex_heights.clone();
        Ok(Self {
            authority,
            explicit_vertex_heights,
            source_handoff_support_heights: BTreeMap::new(),
            contour_edge_support_heights,
            triangles: Some(height_triangles_from_contour(
                id,
                source_kind,
                authority,
                points,
            )?),
        })
    }

    pub(super) fn source_handoff_height_at(&self, point_xz: RoadVec2) -> Option<f64> {
        if self.authority.role != NodeHeightPatchAuthorityRole::SourceInterval {
            return None;
        }
        self.source_handoff_support_heights
            .get(&height_source_point_key(point_xz))
            .copied()
    }

    pub(super) fn evaluate_surface_height(
        &self,
        id: NodeBandHeightFieldId,
        source_kind: RoadSurfaceBandKind,
        point_xz: RoadVec2,
    ) -> Result<NodeHeightPatchEvaluation, NodeHeightFieldError> {
        if let Some(height_m) = self
            .explicit_vertex_heights
            .get(&height_source_point_key(point_xz))
            .copied()
        {
            return Ok(NodeHeightPatchEvaluation::Inside(height_m));
        }
        if let Some(height_m) = self
            .contour_edge_support_heights
            .get(&height_source_point_key(point_xz))
            .copied()
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

    pub(super) fn evaluate_triangle_surface_height(
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

    pub(super) fn outside_field_error(
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
