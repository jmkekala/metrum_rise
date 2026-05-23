//! Region vertex height evaluation against authorized fields.

use super::model::*;
use super::*;

pub(super) fn owner_scoped_outside_height_error(
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

impl NodeResolvedHeightAuthorityMap {
    pub(super) fn from_ownership(
        ownership: &NodeBooleanOwnership,
        fields: &BTreeMap<NodeSourceBandKey, NodeBandHeightField>,
    ) -> Result<Self, NodeHeightFieldError> {
        let mut map = Self {
            heights_by_key: BTreeMap::new(),
        };
        for region in &ownership.owned_regions {
            let field = height_field_for_region(region, fields)?;
            for point_xz in pre_height_completeness_points(region) {
                let height = field
                    .evaluate_authorized_height(region.owner, region.claim_priority, point_xz)
                    .map_err(|error| {
                        missing_owned_region_carrier_support_error(
                            error,
                            region.owner,
                            field,
                            point_xz,
                        )
                    })?;
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

    pub(super) fn insert(
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

    pub(super) fn height_for_vertex(
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

fn pre_height_completeness_points(region: &NodeBooleanOwnedRegion) -> Vec<RoadVec2> {
    let mut points_by_key = BTreeMap::new();
    for point in region
        .shape
        .iter()
        .flat_map(|contour| contour.iter().copied())
    {
        let point_xz = quantize_road_vec2_to_overlay_grid(overlay_point_to_road(point));
        points_by_key.insert(NodeHeightPointKey::from_point(point_xz), point_xz);
    }
    for constraint in &region.seam_constraints {
        for point_xz in [constraint.start_xz, constraint.end_xz] {
            let point_xz = quantize_road_vec2_to_overlay_grid(point_xz);
            points_by_key.insert(NodeHeightPointKey::from_point(point_xz), point_xz);
        }
    }
    points_by_key.into_values().collect()
}

fn missing_owned_region_carrier_support_error(
    error: NodeHeightFieldError,
    owner: NodeBandOwner,
    field: &NodeBandHeightField,
    point_xz: RoadVec2,
) -> NodeHeightFieldError {
    match error {
        NodeHeightFieldError::VertexOutsideHeightField { .. } => {
            let key = NodeHeightPointKey::from_point(point_xz);
            NodeHeightFieldError::MissingOwnedRegionCarrierSupport {
                mouth_order_index: field.id.mouth_order_index(),
                band_index: field.id.band_index(),
                source_kind: field.kind,
                height_field_id: field.id,
                owner,
                point_x_mm: key.x_mm(),
                point_z_mm: key.z_mm(),
            }
        }
        other => other,
    }
}

pub(super) fn heighted_region(
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

pub(super) fn height_field_for_region<'a>(
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

pub(super) fn heighted_shape(
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

pub(super) fn heighted_contour(
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

pub(super) fn heighted_vertex(
    point: NodeOverlayPoint,
    region: &NodeBooleanOwnedRegion,
    field: &NodeBandHeightField,
    resolved_authority: Option<&NodeResolvedHeightAuthorityMap>,
) -> Result<NodeHeightedVertex, NodeHeightFieldError> {
    let (point_xz, height_m, height_authority) =
        if let Some(resolved_authority) = resolved_authority {
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
            (
                authority.point_xz,
                authority.height_m,
                Some(authority.authority),
            )
        } else {
            let point_xz = quantize_road_vec2_to_overlay_grid(overlay_point_to_road(point));
            (point_xz, field.evaluate_height(point_xz)?, None)
        };
    Ok(NodeHeightedVertex {
        point_xz,
        height_m,
        height_field_id: field.id,
        height_authority,
        grade_authority: Some(NodeGradeVertexAuthority::new(
            point_xz,
            height_m,
            region.owner,
            field.id,
            NodeGradeCarrierDecision::SourceCarrier {
                authority: height_authority,
            },
        )),
    })
}
