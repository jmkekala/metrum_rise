//! Construction pipeline for node-height solutions.

use super::evaluate::*;
use super::grade::apply_junctionn_height_authority_normalization;
use super::model::*;
use super::seams::*;
use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn build_node_height_solution_from_ownership(
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

    pub(super) fn from_ownership_input_and_rails(
        input: &NodeArrangementInput,
        rails: Option<&NodeRailContourSet>,
        ownership: &NodeBooleanOwnership,
    ) -> Result<Self, NodeHeightFieldError> {
        validate_input_ownership_pair(input, ownership)?;
        let mut fields = height_fields_by_source_for_ownership(input, rails, Some(ownership))?;
        register_owned_region_contour_support(&mut fields, ownership)?;
        let resolved_authority = pre_height_field_completeness_gate(ownership, &fields)?;
        let mut regions = Vec::with_capacity(ownership.owned_regions.len());

        for region in &ownership.owned_regions {
            let region = heighted_region(region, &fields, Some(&resolved_authority))?;
            if !region.shape.is_empty() {
                regions.push(region);
            }
        }
        if ownership.piece_kind == RoadSurfaceVisualNodePieceKind::JunctionN {
            apply_junctionn_height_authority_normalization(&mut regions)?;
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

fn pre_height_field_completeness_gate(
    ownership: &NodeBooleanOwnership,
    fields: &BTreeMap<NodeSourceBandKey, NodeBandHeightField>,
) -> Result<NodeResolvedHeightAuthorityMap, NodeHeightFieldError> {
    NodeResolvedHeightAuthorityMap::from_ownership(ownership, fields)
}

pub(super) fn register_owned_region_contour_support(
    fields: &mut BTreeMap<NodeSourceBandKey, NodeBandHeightField>,
    ownership: &NodeBooleanOwnership,
) -> Result<(), NodeHeightFieldError> {
    for region in &ownership.owned_regions {
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
            .get_mut(&key)
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
        for point_xz in pre_height_completeness_points(region) {
            field.register_contour_edge_support(region.owner, region.claim_priority, point_xz);
            field.register_owned_region_source_handoff(
                region.owner,
                region.claim_priority,
                point_xz,
            );
        }
    }
    Ok(())
}

pub(super) fn validate_input_ownership_pair(
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

#[cfg(test)]
pub(super) fn height_fields_by_source(
    input: &NodeArrangementInput,
    rails: Option<&NodeRailContourSet>,
) -> Result<BTreeMap<NodeSourceBandKey, NodeBandHeightField>, NodeHeightFieldError> {
    height_fields_by_source_for_ownership(input, rails, None)
}

fn height_fields_by_source_for_ownership(
    input: &NodeArrangementInput,
    rails: Option<&NodeRailContourSet>,
    ownership: Option<&NodeBooleanOwnership>,
) -> Result<BTreeMap<NodeSourceBandKey, NodeBandHeightField>, NodeHeightFieldError> {
    let terminal_cap_bands_by_mouth = terminal_cap_bands_by_mouth(input)
        .map_err(|error| NodeHeightFieldError::TerminalCapGeneration { error })?;
    let rail_height_carrier_points = rails
        .map(|rails| rails.height_carrier_points_for_ownership(ownership))
        .transpose()
        .map_err(rail_generation_error_to_height_error)?;
    let mut fields = BTreeMap::new();
    for (mouth_index, mouth) in input.mouths.iter().enumerate() {
        for interval in &mouth.band_intervals {
            let key = NodeSourceBandKey {
                mouth_order_index: mouth.order_index,
                band_index: interval.band_index,
            };
            let source_support_points = rail_height_carrier_points.as_ref().and_then(|points| {
                points
                    .get(&(interval.band_kind, mouth.order_index, interval.band_index))
                    .map(Vec::as_slice)
            });
            let resolved_interval = rails
                .and_then(|rails| {
                    rails.height_carrier_paths_by_source.get(&(
                        interval.band_kind,
                        mouth.order_index,
                        interval.band_index,
                    ))
                })
                .map(|paths| {
                    let mut interval = interval.clone();
                    interval.start_path_world = paths.start_path_world.clone();
                    interval.end_path_world = paths.end_path_world.clone();
                    interval
                });
            let interval = resolved_interval.as_ref().unwrap_or(interval);
            let field = NodeBandHeightField::from_interval(
                mouth.order_index,
                interval,
                source_support_points,
            )?;
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
            let field = NodeBandHeightField::from_terminal_cap_band(mouth.order_index, cap_band)?;
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
        extend_height_fields_with_generated_contours(rails, &mut fields, ownership)?;
    }
    Ok(fields)
}

fn rail_generation_error_to_height_error(error: NodeRailGenerationError) -> NodeHeightFieldError {
    match error {
        NodeRailGenerationError::ConflictingHeightCarrierPoint {
            kind,
            mouth_order_index,
            band_index,
            point_x_key,
            point_z_key,
            existing_height_mm,
            incoming_height_mm,
        } => {
            let id = NodeBandHeightFieldId::new(mouth_order_index, band_index, kind);
            NodeHeightFieldError::SourceHeightFieldConflict {
                mouth_order_index,
                band_index,
                source_kind: kind,
                height_field_id: id,
                owner: None,
                existing_authority: NodeHeightAuthoritySource::SourceInterval,
                incoming_authority: NodeHeightAuthoritySource::SourceInterval,
                point_x_mm: SurfaceXzKey::from_raw_keys(point_x_key, point_z_key).x_mm(),
                point_z_mm: SurfaceXzKey::from_raw_keys(point_x_key, point_z_key).z_mm(),
                existing_height_mm,
                incoming_height_mm,
            }
        }
        NodeRailGenerationError::MissingCarrierProvenanceHeight {
            kind,
            mouth_order_index,
            band_index,
            point_x_key,
            point_z_key,
            source_segment_id,
        } => {
            let key = SurfaceXzKey::from_raw_keys(point_x_key, point_z_key);
            NodeHeightFieldError::MissingCarrierProvenanceHeight {
                mouth_order_index,
                band_index,
                source_kind: kind,
                height_field_id: NodeBandHeightFieldId::new(mouth_order_index, band_index, kind),
                point_x_key,
                point_z_key,
                point_x_mm: key.x_mm(),
                point_z_mm: key.z_mm(),
                source_segment_id,
            }
        }
        NodeRailGenerationError::InvalidHeightCarrier { reason, .. } => {
            NodeHeightFieldError::RailHeightCarrierGeneration { reason }
        }
        NodeRailGenerationError::SideJoinGeneration { error } => {
            NodeHeightFieldError::RailHeightCarrierGeneration {
                reason: error.reason,
            }
        }
        NodeRailGenerationError::TerminalCapGeneration { error } => {
            NodeHeightFieldError::TerminalCapGeneration { error }
        }
        NodeRailGenerationError::DegenerateConstraint { .. } => {
            NodeHeightFieldError::RailHeightCarrierGeneration {
                reason: "degenerate_constraint",
            }
        }
        NodeRailGenerationError::DegenerateContour { .. } => {
            NodeHeightFieldError::RailHeightCarrierGeneration {
                reason: "degenerate_contour",
            }
        }
        NodeRailGenerationError::EmptyInput { .. } => {
            NodeHeightFieldError::RailHeightCarrierGeneration {
                reason: "empty_input",
            }
        }
        NodeRailGenerationError::NonCanonicalGeneratedContactEndpoint { .. } => {
            NodeHeightFieldError::RailHeightCarrierGeneration {
                reason: "noncanonical_generated_contact_endpoint",
            }
        }
    }
}

pub(super) fn extend_height_fields_with_generated_contours(
    rails: &NodeRailContourSet,
    fields: &mut BTreeMap<NodeSourceBandKey, NodeBandHeightField>,
    ownership: Option<&NodeBooleanOwnership>,
) -> Result<(), NodeHeightFieldError> {
    for contour in &rails.contours {
        let NodeGeneratedContourKind::Band { kind } = contour.kind else {
            continue;
        };
        let Some(band_index) = contour.source_band_index else {
            if generated_contour_requires_height_field(contour) {
                return Err(
                    NodeHeightFieldError::GeneratedContourMissingSourceBandIndex {
                        mouth_order_index: contour.source_mouth_order_index,
                        source_kind: kind,
                        purpose: contour.purpose,
                        claim_priority: contour.claim_priority,
                        owner: contour.owner,
                    },
                );
            }
            continue;
        };
        let key = NodeSourceBandKey {
            mouth_order_index: contour.source_mouth_order_index,
            band_index,
        };
        let Some(field) = fields.get_mut(&key) else {
            if generated_contour_requires_height_field(contour) {
                return Err(NodeHeightFieldError::GeneratedContourMissingSourceBand {
                    mouth_order_index: key.mouth_order_index,
                    band_index: key.band_index,
                    source_kind: kind,
                    purpose: contour.purpose,
                    claim_priority: contour.claim_priority,
                    owner: contour.owner,
                });
            }
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
        if let Err(error) = field.extend_with_generated_contour(contour) {
            if matches!(
                error,
                NodeHeightFieldError::InvalidHeightCarrierContour { .. }
                    | NodeHeightFieldError::MissingGeneratedContourHeightPoints { .. }
            ) && generated_contour_is_superseded_by_post_boolean_region(contour, ownership)
            {
                continue;
            }
            return Err(error);
        }
    }
    Ok(())
}

fn generated_contour_requires_height_field(contour: &NodeGeneratedContour) -> bool {
    contour.owner.is_some() || contour.height_points_world.is_some()
}

fn generated_contour_is_superseded_by_post_boolean_region(
    contour: &NodeGeneratedContour,
    ownership: Option<&NodeBooleanOwnership>,
) -> bool {
    let (Some(ownership), NodeGeneratedContourKind::Band { kind }, Some(owner), band_index) = (
        ownership,
        contour.kind,
        contour.owner,
        contour.source_band_index,
    ) else {
        return false;
    };
    ownership.owned_regions.iter().any(|region| {
        region.kind == kind
            && region.owner == owner
            && region.claim_priority == contour.claim_priority
            && region.source_mouth_order_index == contour.source_mouth_order_index
            && region.source_band_index == band_index
            && !region.shape.is_empty()
    })
}
