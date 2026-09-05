// SPDX-License-Identifier: GPL-2.0-only

//! Same-height source identity checks for boundary height candidates.

use super::*;

pub(super) fn node_footprint_height_candidates_share_source_identity(
    a: NodeFootprintBoundaryHeightCandidate,
    b: NodeFootprintBoundaryHeightCandidate,
) -> bool {
    a.height_mm == b.height_mm
        && node_footprint_direct_vertices_share_source_identity(a.source, b.source)
}

pub(super) fn reject_same_owner_same_height_source_conflicts(
    key: arrangement::NodeArrangementKey,
    candidates: &[NodeFootprintBoundaryHeightCandidate],
) -> Result<(), NodeBoundaryExportError> {
    for (left_index, left) in candidates.iter().copied().enumerate() {
        for right in candidates.iter().copied().skip(left_index + 1) {
            if left.height_mm != right.height_mm {
                continue;
            }
            if left.source.owner_kind != right.source.owner_kind
                || left.source.owner_index != right.source.owner_index
            {
                continue;
            }
            if !node_footprint_direct_vertices_share_boundary_point_authority(
                ArrangementBoundaryPointKey {
                    x_key: key.x_key(),
                    z_key: key.z_key(),
                    y_mm: left.height_mm,
                },
                left.source,
                right.source,
            ) {
                return Err(ambiguous_footprint_boundary_point_source_error(
                    ArrangementBoundaryPointKey {
                        x_key: key.x_key(),
                        z_key: key.z_key(),
                        y_mm: left.height_mm,
                    },
                    left.source,
                    right.source,
                ));
            }
        }
    }
    Ok(())
}
