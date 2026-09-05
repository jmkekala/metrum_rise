// SPDX-License-Identifier: GPL-2.0-only

//! Missing footprint boundary height rejection.

use super::*;

impl NodeFootprintBoundaryExportSources {
    #[cfg(test)]
    pub(in crate::simulation::network::surface) fn reject_missing_footprint_boundary_heights(
        &self,
        vertices: &[(arrangement::NodeArrangementKey, Option<i64>)],
    ) -> Result<(), NodeBoundaryExportError> {
        if let Some((key, _)) = vertices.iter().find(|(_, height_mm)| height_mm.is_none()) {
            return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight {
                x_key: key.x_key(),
                z_key: key.z_key(),
            });
        }
        Ok(())
    }
}
