// SPDX-License-Identifier: GPL-2.0-only

//! Source terrain patch variant export helpers.

use super::super::*;

impl SimulationNode {
    fn set_mask_bytes(
        dict: &mut VarDictionary,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        mask_bytes: &[u8],
    ) {
        let bytes = if mask_bytes.is_empty() {
            PackedByteArray::from(patch.shading_mask_bytes().as_slice())
        } else {
            PackedByteArray::from(mask_bytes)
        };
        dict.set("mask_bytes", bytes);
    }

    pub(in crate::nodes::simulation_node) fn terrain_patch_metadata_dict(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
    ) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("patch_x", i64::try_from(patch.patch_x).unwrap_or(0));
        dict.set("patch_z", i64::try_from(patch.patch_z).unwrap_or(0));
        dict.set("sample_width", i64::try_from(patch.sample_width).unwrap_or(0));
        dict.set("sample_height", i64::try_from(patch.sample_height).unwrap_or(0));
        dict.set("texture_width", i64::try_from(patch.texture_width).unwrap_or(0));
        dict.set("texture_height", i64::try_from(patch.texture_height).unwrap_or(0));
        dict.set("inner_offset_x", i64::try_from(patch.inner_offset_x).unwrap_or(0));
        dict.set("inner_offset_z", i64::try_from(patch.inner_offset_z).unwrap_or(0));
        dict.set("world_origin_x", f64::from(patch.world_origin_x));
        dict.set("world_origin_z", f64::from(patch.world_origin_z));
        dict.set("world_size_x", f64::from(patch.world_size_x));
        dict.set("world_size_z", f64::from(patch.world_size_z));
        dict
    }

    pub(in crate::nodes::simulation_node) fn f32_bytes_vec(values: &[f32]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(values.len().saturating_mul(std::mem::size_of::<f32>()));
        for value in values {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
        bytes
    }

    pub(in crate::nodes::simulation_node) fn terrain_patch_payload_dict(
        payload: &TerrainPatchPayload,
    ) -> VarDictionary {
        let TerrainPatchPayloadData::Regular {
            patch,
            height_bytes,
        } = &payload.data;
        let mut dict = Self::terrain_patch_metadata_dict(patch);
        dict.set("height_bytes", PackedByteArray::from(height_bytes.as_slice()));
        Self::set_mask_bytes(&mut dict, patch, &payload.mask_bytes);
        dict.set("render_step_mm", i64::from(payload.key.render_step_mm));
        dict.set(
            "surface_generation",
            i64::try_from(payload.surface_generation).unwrap_or(i64::MAX),
        );
        dict
    }
}
