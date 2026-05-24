//! Region vertex height evaluation against authorized fields.

use super::super::keys::SurfaceSegmentParameter;
use super::model::*;
use super::*;
use std::collections::BTreeSet;

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

pub(super) fn pre_height_completeness_points(region: &NodeBooleanOwnedRegion) -> Vec<RoadVec2> {
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
    let area_m2 = heighted_shape_area_m2(&shape);

    Ok(NodeHeightedRegion {
        kind: region.kind,
        owner: region.owner,
        height_field_id: field.id,
        shape,
        area_m2,
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
    source_authorized_contour_points(contour, region)
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

fn source_authorized_contour_points(
    contour: &NodeOverlayContour,
    region: &NodeBooleanOwnedRegion,
) -> NodeOverlayContour {
    if contour.len() < 2 || region.seam_constraints.is_empty() {
        return contour.clone();
    }
    let source_keys = material_transition_constraint_point_keys(region);
    let protected_original_keys = original_contour_vertex_keys(contour);
    let mut output = Vec::with_capacity(contour.len());
    for index in 0..contour.len() {
        let start = contour[index];
        let end = contour[(index + 1) % contour.len()];
        output.push(start);
        output.extend(source_authorized_points_for_contour_edge(
            region, start, end,
        ));
    }
    remove_subbudget_non_source_contour_points(output, &source_keys, &protected_original_keys)
}

fn material_transition_constraint_point_keys(
    region: &NodeBooleanOwnedRegion,
) -> BTreeSet<SurfaceXzKey> {
    region
        .seam_constraints
        .iter()
        .filter(|constraint| constraint.is_material_transition)
        .flat_map(|constraint| [constraint.start_xz, constraint.end_xz])
        .map(|point_xz| SurfaceXzKey::from_road_xz(quantize_road_vec2_to_overlay_grid(point_xz)))
        .collect()
}

fn original_contour_vertex_keys(contour: &NodeOverlayContour) -> BTreeSet<SurfaceXzKey> {
    contour
        .iter()
        .copied()
        .map(SurfaceXzKey::from_overlay_point)
        .collect()
}

fn source_authorized_points_for_contour_edge(
    region: &NodeBooleanOwnedRegion,
    start: NodeOverlayPoint,
    end: NodeOverlayPoint,
) -> Vec<NodeOverlayPoint> {
    let start_key = SurfaceXzKey::from_overlay_point(start);
    let end_key = SurfaceXzKey::from_overlay_point(end);
    if start_key == end_key {
        return Vec::new();
    }
    let mut insertions = Vec::<(SurfaceSegmentParameter, SurfaceXzKey)>::new();
    for constraint in &region.seam_constraints {
        if !constraint.is_material_transition {
            continue;
        }
        for point_xz in [constraint.start_xz, constraint.end_xz] {
            let point_key =
                SurfaceXzKey::from_road_xz(quantize_road_vec2_to_overlay_grid(point_xz));
            if point_key == start_key
                || point_key == end_key
                || insertions.iter().any(|(_, key)| *key == point_key)
            {
                continue;
            }
            let Some(parameter) = point_key.overlay_segment_parameter(start_key, end_key) else {
                continue;
            };
            if parameter <= SurfaceSegmentParameter::zero()
                || parameter >= SurfaceSegmentParameter::one()
            {
                continue;
            }
            insertions.push((parameter, point_key));
        }
    }
    insertions.sort_by_key(|(parameter, key)| (*parameter, *key));
    insertions
        .into_iter()
        .map(|(_, key)| {
            let point = key.to_road_xz();
            [point.x, point.y]
        })
        .collect()
}

fn remove_subbudget_non_source_contour_points(
    mut points: NodeOverlayContour,
    source_keys: &BTreeSet<SurfaceXzKey>,
    protected_original_keys: &BTreeSet<SurfaceXzKey>,
) -> NodeOverlayContour {
    loop {
        if points.len() < 3 {
            return points;
        }
        let mut removed = false;
        for index in 0..points.len() {
            let previous = if index == 0 {
                points.len() - 1
            } else {
                index - 1
            };
            let next = (index + 1) % points.len();
            let current_key = SurfaceXzKey::from_overlay_point(points[index]);
            let previous_key = SurfaceXzKey::from_overlay_point(points[previous]);
            let next_key = SurfaceXzKey::from_overlay_point(points[next]);
            if source_keys.contains(&current_key)
                || protected_original_keys.contains(&current_key)
                || (!source_keys.contains(&previous_key) && !source_keys.contains(&next_key))
            {
                continue;
            }
            let local_points = [points[previous], points[index], points[next]];
            if local_triangle_area_m2(local_points)
                > local_overlay_numeric_area_budget_m2(local_points)
            {
                continue;
            }
            points.remove(index);
            removed = true;
            break;
        }
        if !removed {
            return points;
        }
    }
}

fn local_triangle_area_m2(points: [NodeOverlayPoint; 3]) -> f32 {
    (((points[0][0] * points[1][1] - points[1][0] * points[0][1])
        + (points[1][0] * points[2][1] - points[2][0] * points[1][1])
        + (points[2][0] * points[0][1] - points[0][0] * points[2][1]))
        * 0.5)
        .abs() as f32
}

fn local_overlay_numeric_area_budget_m2(points: [NodeOverlayPoint; 3]) -> f32 {
    let perimeter_m = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(start, end)| {
            let dx = start[0] - end[0];
            let dz = start[1] - end[1];
            (dx * dx + dz * dz).sqrt() as f32
        })
        .sum::<f32>();
    RoadSurfaceSystem::overlay_numeric_area_budget_m2(perimeter_m, points.len())
}

fn heighted_shape_area_m2(shape: &NodeHeightedShape) -> f32 {
    let Some((outer, holes)) = shape.split_first() else {
        return 0.0;
    };
    let holes_area = holes
        .iter()
        .map(|hole| heighted_contour_area_m2(hole).abs())
        .sum::<f64>();
    (heighted_contour_area_m2(outer).abs() - holes_area).max(0.0) as f32
}

fn heighted_contour_area_m2(contour: &NodeHeightedContour) -> f64 {
    if contour.len() < 3 {
        return 0.0;
    }
    let mut signed_area = 0.0;
    for index in 0..contour.len() {
        let current = contour[index].point_xz;
        let next = contour[(index + 1) % contour.len()].point_xz;
        signed_area += current.x * next.y - next.x * current.y;
    }
    signed_area * 0.5
}
