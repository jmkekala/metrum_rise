//! Construction pipeline for node-height solutions.

use super::evaluate::*;
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
            apply_junctionn_node_grade_carrier(&mut regions)?;
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

pub(super) fn height_fields_by_source(
    input: &NodeArrangementInput,
    rails: Option<&NodeRailContourSet>,
) -> Result<BTreeMap<NodeSourceBandKey, NodeBandHeightField>, NodeHeightFieldError> {
    let terminal_cap_bands_by_mouth = terminal_cap_bands_by_mouth(input)
        .map_err(|error| NodeHeightFieldError::TerminalCapGeneration { error })?;
    let mut fields = BTreeMap::new();
    for (mouth_index, mouth) in input.mouths.iter().enumerate() {
        for interval in &mouth.band_intervals {
            let field = NodeBandHeightField::from_interval(mouth.order_index, interval)?;
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
        extend_height_fields_with_generated_contours(rails, &mut fields)?;
    }
    Ok(fields)
}

pub(super) fn extend_height_fields_with_generated_contours(
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
