//! Height patch construction and local surface evaluation.

use super::carriers::*;
use super::handoff::{source_handoff_support_heights, source_support_heights};
use super::model::*;
use super::source_edges::height_source_point_key;
use super::triangles::*;
use super::vertices::{
    closed_height_contour_edges_from_vertices, height_vertex_heights_from_vertices,
};
use super::*;
use crate::simulation::network::surface::keys::SURFACE_MM_PER_M;
use crate::simulation::network::surface::segments::interpolate_height_i64;
use std::collections::BTreeMap;

const HEIGHT_CONTOUR_ENDPOINT_DUST_KEYS: i64 = 2;

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
        let source_support_heights =
            source_support_heights(id, interval.band_kind, source_support_points)?;
        let source_handoff_support_heights =
            source_handoff_support_heights(interval, &source_support_heights);
        let mut contour_edge_support_heights = explicit_vertex_heights.clone();
        contour_edge_support_heights.extend(source_support_heights);
        let contour_edges = interval_height_contour_edges(id, interval)?;
        Ok(Self {
            authority: NodeHeightPatchAuthority::source_interval(),
            explicit_vertex_heights,
            source_handoff_support_heights,
            contour_edge_support_heights,
            contour_edges,
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
        let contour_edges = closed_height_contour_edges_from_vertices(&cap_band.contour_world)
            .map_err(|error| NodeHeightFieldError::InvalidHeightCarrierContour {
                mouth_order_index: id.mouth_order_index(),
                band_index: id.band_index(),
                source_kind,
                height_field_id: id,
                authority: authority.source(),
                reason: error.diagnostic_reason(),
            })?;
        Ok(Self {
            authority,
            explicit_vertex_heights,
            source_handoff_support_heights: BTreeMap::new(),
            contour_edge_support_heights,
            contour_edges,
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

    pub(super) fn from_generated_contour_edge_support(
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
        let authority = NodeHeightPatchAuthority::generated_contour(contour);
        let explicit_vertex_heights =
            height_vertex_heights_from_vertices(points_world).map_err(|error| {
                NodeHeightFieldError::InvalidHeightCarrierContour {
                    mouth_order_index: id.mouth_order_index(),
                    band_index: id.band_index(),
                    source_kind,
                    height_field_id: id,
                    authority: authority.source(),
                    reason: error.diagnostic_reason(),
                }
            })?;
        let contour_edges =
            closed_height_contour_edges_from_vertices(points_world).map_err(|error| {
                NodeHeightFieldError::InvalidHeightCarrierContour {
                    mouth_order_index: id.mouth_order_index(),
                    band_index: id.band_index(),
                    source_kind,
                    height_field_id: id,
                    authority: authority.source(),
                    reason: error.diagnostic_reason(),
                }
            })?;
        Ok(Self {
            authority,
            explicit_vertex_heights: explicit_vertex_heights.clone(),
            source_handoff_support_heights: BTreeMap::new(),
            contour_edge_support_heights: explicit_vertex_heights,
            contour_edges,
            triangles: None,
        })
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
        let contour_edges = closed_height_contour_edges_from_vertices(points).map_err(|error| {
            NodeHeightFieldError::InvalidHeightCarrierContour {
                mouth_order_index: id.mouth_order_index(),
                band_index: id.band_index(),
                source_kind,
                height_field_id: id,
                authority: authority.source(),
                reason: error.diagnostic_reason(),
            }
        })?;
        Ok(Self {
            authority,
            explicit_vertex_heights,
            source_handoff_support_heights: BTreeMap::new(),
            contour_edge_support_heights,
            contour_edges,
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
        let point = height_source_point_key(point_xz);
        self.explicit_vertex_heights
            .get(&point)
            .or_else(|| self.source_handoff_support_heights.get(&point))
            .copied()
            .or_else(|| self.height_on_contour_edge_at(point_xz))
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
        if let Some(height_m) = self.height_on_contour_edge_at(point_xz) {
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

    pub(super) fn height_on_contour_edge_at(&self, point_xz: RoadVec2) -> Option<f64> {
        let point = height_source_point_key(point_xz);
        if let Some(height_m) = self.height_at_near_contour_endpoint(point) {
            return Some(height_m);
        }
        let point_key = SurfaceXzKey::from_raw_tuple(point);
        let mut accepted_height_mm = None;
        for edge in &self.contour_edges {
            let Some(parameter) = point_key.overlay_segment_parameter(
                SurfaceXzKey::from_raw_tuple(edge.start),
                SurfaceXzKey::from_raw_tuple(edge.end),
            ) else {
                continue;
            };
            let height_mm =
                interpolate_height_i64(edge.start_height_mm, edge.end_height_mm, parameter);
            match accepted_height_mm {
                Some(accepted_height_mm) if accepted_height_mm != height_mm => return None,
                Some(_) => {}
                None => accepted_height_mm = Some(height_mm),
            }
        }
        accepted_height_mm.map(|height_mm| height_mm as f64 / SURFACE_MM_PER_M)
    }

    fn height_at_near_contour_endpoint(&self, point: NodeHeightSourcePointKey) -> Option<f64> {
        let mut accepted = None;
        for edge in &self.contour_edges {
            for (endpoint, height_mm) in [
                (edge.start, edge.start_height_mm),
                (edge.end, edge.end_height_mm),
            ] {
                if !height_endpoint_keys_match_with_dust(point, endpoint) {
                    continue;
                }
                match accepted {
                    Some((accepted_endpoint, accepted_height_mm))
                        if accepted_endpoint != endpoint || accepted_height_mm != height_mm =>
                    {
                        return None;
                    }
                    Some(_) => {}
                    None => accepted = Some((endpoint, height_mm)),
                }
            }
        }
        accepted.map(|(_, height_mm)| height_mm as f64 / SURFACE_MM_PER_M)
    }

    pub(super) fn evaluate_triangle_surface_height(
        &self,
        id: NodeBandHeightFieldId,
        source_kind: RoadSurfaceBandKind,
        point_xz: RoadVec2,
        triangles: &[NodeBandHeightTriangle],
    ) -> Result<NodeHeightPatchEvaluation, NodeHeightFieldError> {
        let mut candidate = None;
        for triangle in triangles {
            if let Some(height_m) = triangle.height_at(point_xz) {
                let height_mm = quantize_m(height_m);
                if let Some((first_height_m, first_height_mm)) = candidate {
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
                    candidate = Some((first_height_m, first_height_mm));
                } else {
                    candidate = Some((height_m, height_mm));
                }
            }
        }
        let Some((first_height_m, _)) = candidate else {
            return Ok(NodeHeightPatchEvaluation::Outside(
                self.outside_field_error(id, source_kind, point_xz, "triangle", f64::NAN),
            ));
        };
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

fn height_endpoint_keys_match_with_dust(
    point: NodeHeightSourcePointKey,
    endpoint: NodeHeightSourcePointKey,
) -> bool {
    (point.0 - endpoint.0).abs() <= HEIGHT_CONTOUR_ENDPOINT_DUST_KEYS
        && (point.1 - endpoint.1).abs() <= HEIGHT_CONTOUR_ENDPOINT_DUST_KEYS
}
