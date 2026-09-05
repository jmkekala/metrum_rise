// SPDX-License-Identifier: GPL-2.0-only

//! Region vertex height evaluation against authorized fields.

use super::super::keys::SurfaceSegmentParameter;
use super::model::*;
use super::*;
use std::collections::BTreeSet;

const SOURCE_ENDPOINT_DUST_KEYS: i64 = 2;

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
    pub(super) fn from_ownership_with_points(
        ownership: &NodeBooleanOwnership,
        fields: &BTreeMap<NodeSourceBandKey, NodeBandHeightField>,
        completeness_points: &[Vec<RoadVec2>],
    ) -> Result<Self, NodeHeightFieldError> {
        debug_assert_eq!(ownership.owned_regions.len(), completeness_points.len());
        let provenance_by_key = height_carrier_provenance_by_key(ownership);
        let mut map = Self {
            heights_by_key: BTreeMap::new(),
            raw_heights_by_key: BTreeMap::new(),
            claim_keys_by_context: BTreeMap::new(),
            canonical_key_by_context: BTreeMap::new(),
        };
        for (region, region_points) in ownership.owned_regions.iter().zip(completeness_points) {
            let field = height_field_for_region(region, fields)?;
            for &point_xz in region_points {
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
                    provenance_by_key
                        .get(&resolved_height_authority_key(
                            region.owner,
                            field.id,
                            region.claim_priority,
                            point_xz,
                        ))
                        .copied(),
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
        source_provenance: Option<NodeHeightCarrierProvenanceKey>,
    ) -> Result<(), NodeHeightFieldError> {
        let key = resolved_height_authority_key(owner, height_field_id, claim_priority, point_xz);
        let point = key.point;
        let context = NodeHeightVertexContextKey {
            point,
            owner,
            height_field_id,
        };
        let incoming = NodeResolvedHeightAuthority {
            point_xz,
            height_m: height.height_m,
            authority: height.authority,
            source_provenance,
        };
        let height_mm = quantize_m(height.height_m);
        if let Some(existing) = self.raw_heights_by_key.get(&key) {
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
        } else {
            self.raw_heights_by_key.insert(key, incoming);
        }
        self.claim_keys_by_context
            .entry(context.clone())
            .or_default()
            .insert(key);
        let selected_key = match self.canonical_key_by_context.get(&context).copied() {
            Some(existing_key) if existing_key.claim_priority <= claim_priority => existing_key,
            _ => key,
        };
        self.canonical_key_by_context
            .insert(context.clone(), selected_key);
        let selected = *self
            .raw_heights_by_key
            .get(&selected_key)
            .expect("selected canonical height authority must be inserted");
        if let Some(claim_keys) = self.claim_keys_by_context.get(&context) {
            for claim_key in claim_keys {
                self.heights_by_key.insert(*claim_key, selected);
            }
        }
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

fn resolved_height_authority_key(
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    claim_priority: NodeGeneratedContourClaimPriority,
    point_xz: RoadVec2,
) -> NodeResolvedHeightAuthorityKey {
    NodeResolvedHeightAuthorityKey {
        point: NodeHeightPointKey::from_point(point_xz),
        owner,
        height_field_id,
        claim_priority,
    }
}

fn height_carrier_provenance_by_key(
    ownership: &NodeBooleanOwnership,
) -> BTreeMap<NodeResolvedHeightAuthorityKey, NodeHeightCarrierProvenanceKey> {
    let mut candidates_by_key =
        BTreeMap::<NodeResolvedHeightAuthorityKey, Vec<NodeHeightCarrierProvenanceKey>>::new();
    for record in &ownership.carrier_provenance.records {
        let (x_key, z_key) = record.point.raw_tuple();
        let key = NodeResolvedHeightAuthorityKey {
            point: NodeHeightPointKey { x_key, z_key },
            owner: record.owner,
            height_field_id: record.height_field_id,
            claim_priority: record.claim_priority,
        };
        candidates_by_key
            .entry(key)
            .or_default()
            .push(NodeHeightCarrierProvenanceKey::from_record(*record));
    }

    candidates_by_key
        .into_iter()
        .filter_map(|(key, mut candidates)| {
            candidates.sort_unstable();
            candidates.dedup();
            let [candidate] = candidates.as_slice() else {
                return None;
            };
            Some((key, *candidate))
        })
        .collect()
}

pub(super) fn pre_height_completeness_points(region: &NodeBooleanOwnedRegion) -> Vec<RoadVec2> {
    let mut points_by_key = Vec::new();
    for point in region
        .shape
        .iter()
        .flat_map(|contour| contour.iter().copied())
    {
        let point_xz = quantize_road_vec2_to_overlay_grid(overlay_point_to_road(point));
        points_by_key.push((NodeHeightPointKey::from_point(point_xz), point_xz));
    }
    for constraint in &region.seam_constraints {
        for point_xz in [constraint.start_xz, constraint.end_xz] {
            let point_xz = quantize_road_vec2_to_overlay_grid(point_xz);
            points_by_key.push((NodeHeightPointKey::from_point(point_xz), point_xz));
        }
    }
    points_by_key.sort_unstable_by_key(|(key, _)| *key);
    points_by_key.dedup_by_key(|(key, _)| *key);
    points_by_key
        .into_iter()
        .map(|(_, point_xz)| point_xz)
        .collect()
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
                point_x_key: key.x_key,
                point_z_key: key.z_key,
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
            kind: region.kind,
            owner: Some(region.owner),
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
    let (point_xz, height_m, height_authority, source_provenance) =
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
                authority.source_provenance,
            )
        } else {
            let point_xz = quantize_road_vec2_to_overlay_grid(overlay_point_to_road(point));
            (point_xz, field.evaluate_height(point_xz)?, None, None)
        };
    Ok(NodeHeightedVertex {
        point_xz,
        height_m,
        height_field_id: field.id,
        height_authority,
        source_provenance,
        grade_authority: Some(NodeGradeVertexAuthority::new_with_source_provenance(
            point_xz,
            height_m,
            region.owner,
            field.id,
            NodeGradeCarrierDecision::SourceCarrier {
                authority: height_authority,
            },
            source_provenance,
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
    let contour = canonicalize_contour_source_endpoint_dust(contour, &source_keys);
    let mut output = Vec::with_capacity(contour.len());
    let mut insertions = Vec::new();
    for index in 0..contour.len() {
        let start = contour[index];
        let end = contour[(index + 1) % contour.len()];
        output.push(start);
        append_source_authorized_points_for_contour_edge(
            &source_keys,
            start,
            end,
            &mut insertions,
            &mut output,
        );
    }
    output
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

fn append_source_authorized_points_for_contour_edge(
    source_keys: &BTreeSet<SurfaceXzKey>,
    start: NodeOverlayPoint,
    end: NodeOverlayPoint,
    insertions: &mut Vec<(SurfaceSegmentParameter, SurfaceXzKey)>,
    output: &mut NodeOverlayContour,
) {
    let start_key = SurfaceXzKey::from_overlay_point(start);
    let end_key = SurfaceXzKey::from_overlay_point(end);
    if start_key == end_key {
        return;
    }
    insertions.clear();
    let min_x = start_key.x_key().min(end_key.x_key());
    let max_x = start_key.x_key().max(end_key.x_key());
    let min_z = start_key.z_key().min(end_key.z_key());
    let max_z = start_key.z_key().max(end_key.z_key());
    let range_start = SurfaceXzKey::from_raw_keys(min_x, i64::MIN);
    let range_end = SurfaceXzKey::from_raw_keys(max_x, i64::MAX);
    for &point_key in source_keys.range(range_start..=range_end) {
        if point_key.z_key() < min_z || point_key.z_key() > max_z {
            continue;
        }
        if point_key == start_key || point_key == end_key {
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
    insertions.sort_by_key(|(parameter, key)| (*parameter, *key));
    output.extend(insertions.iter().map(|(_, key)| {
        let point = key.to_road_xz();
        [point.x, point.y]
    }));
}

fn canonicalize_contour_source_endpoint_dust(
    contour: &NodeOverlayContour,
    source_keys: &BTreeSet<SurfaceXzKey>,
) -> NodeOverlayContour {
    contour
        .iter()
        .copied()
        .map(|point| {
            let key = SurfaceXzKey::from_overlay_point(point);
            if source_keys.contains(&key) {
                return point;
            }
            let (x, z) = key.raw_tuple();
            let range_start =
                SurfaceXzKey::from_raw_keys(x.saturating_sub(SOURCE_ENDPOINT_DUST_KEYS), i64::MIN);
            let range_end =
                SurfaceXzKey::from_raw_keys(x.saturating_add(SOURCE_ENDPOINT_DUST_KEYS), i64::MAX);
            let mut candidate = None;
            for &source_key in source_keys.range(range_start..=range_end) {
                let (_, source_z) = source_key.raw_tuple();
                if (source_z - z).abs() > SOURCE_ENDPOINT_DUST_KEYS {
                    continue;
                }
                if candidate.replace(source_key).is_some() {
                    return point;
                }
            }
            let Some(source_key) = candidate else {
                return point;
            };
            let point = source_key.to_road_xz();
            [point.x, point.y]
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_source_vertex_uses_canonical_claim_priority_for_all_claim_lookups() {
        let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 5);
        let field_id = NodeBandHeightFieldId::new(0, 5, RoadSurfaceBandKind::Sidewalk);
        let point_xz = RoadVec2::new(572.636, 4.995);
        let mut map = NodeResolvedHeightAuthorityMap {
            heights_by_key: BTreeMap::new(),
            raw_heights_by_key: BTreeMap::new(),
            claim_keys_by_context: BTreeMap::new(),
            canonical_key_by_context: BTreeMap::new(),
        };
        map.insert(
            owner,
            field_id,
            NodeGeneratedContourClaimPriority::MouthBand,
            point_xz,
            NodeEvaluatedHeight {
                height_m: 170.092,
                authority: NodeHeightAuthoritySource::GeneratedContour {
                    purpose: NodeGeneratedContourPurpose::NonRoadBand,
                    claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
                },
            },
            RoadSurfaceBandKind::Sidewalk,
            None,
        )
        .expect("mouth-band authority inserts");
        map.insert(
            owner,
            field_id,
            NodeGeneratedContourClaimPriority::SideJoin,
            point_xz,
            NodeEvaluatedHeight {
                height_m: 170.046,
                authority: NodeHeightAuthoritySource::GeneratedContour {
                    purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
                    claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
                },
            },
            RoadSurfaceBandKind::Sidewalk,
            None,
        )
        .expect("side-join authority supersedes shared source vertex");

        let mouth_lookup = map
            .height_for_vertex(
                owner,
                field_id,
                NodeGeneratedContourClaimPriority::MouthBand,
                [point_xz.x, point_xz.y],
            )
            .expect("mouth-band lookup is populated");
        let side_join_lookup = map
            .height_for_vertex(
                owner,
                field_id,
                NodeGeneratedContourClaimPriority::SideJoin,
                [point_xz.x, point_xz.y],
            )
            .expect("side-join lookup is populated");
        assert_eq!(quantize_m(mouth_lookup.height_m), 170_046);
        assert_eq!(mouth_lookup.authority, side_join_lookup.authority);
    }
}
