//! Explicit source-handoff and contour-support height keys.

use super::model::*;
use super::seams::quantize_source_height_m;
use super::source_edges::height_source_point_key;
use super::*;
use std::collections::{BTreeMap, BTreeSet};

impl NodeBandHeightField {
    pub(super) fn register_contour_edge_support(
        &mut self,
        owner: NodeBandOwner,
        claim_priority: NodeGeneratedContourClaimPriority,
        point_xz: RoadVec2,
    ) {
        let point = height_source_point_key(point_xz);
        for patch in &mut self.patches {
            if patch
                .authority
                .rank_for_owned_region(owner, claim_priority)
                .is_some()
                && let Some(height_m) = patch.explicit_vertex_heights.get(&point).copied()
            {
                patch.contour_edge_support_heights.insert(point, height_m);
            }
        }
    }

    pub(super) fn register_owned_region_source_handoff(
        &mut self,
        owner: NodeBandOwner,
        claim_priority: NodeGeneratedContourClaimPriority,
        point_xz: RoadVec2,
    ) {
        if self
            .patches
            .iter()
            .any(|patch| patch.source_handoff_height_at(point_xz).is_some())
        {
            self.source_handoff_keys
                .insert(NodeAuthorizedSourceHandoffKey {
                    owner,
                    claim_priority,
                    point: height_source_point_key(point_xz),
                });
        }
    }

    pub(super) fn register_generated_contour_source_handoffs(
        &mut self,
        contour: &NodeGeneratedContour,
    ) -> Result<(), NodeHeightFieldError> {
        let (Some(owner), Some(points_world)) =
            (contour.owner, contour.height_points_world.as_ref())
        else {
            return Ok(());
        };
        for point in points_world {
            let point_xz = quantize_road_vec2_to_overlay_grid(xz(*point));
            let Some(source_height_m) = self.source_interval_height_at(point_xz)? else {
                continue;
            };
            let source_height_key = SurfaceHeightMmKey::from_m_f64(source_height_m);
            let contour_height_key = SurfaceHeightMmKey::from_m_f64(point.y);
            if source_height_key != contour_height_key {
                continue;
            }
            self.source_handoff_keys
                .insert(NodeAuthorizedSourceHandoffKey {
                    owner,
                    claim_priority: contour.claim_priority,
                    point: height_source_point_key(point_xz),
                });
        }
        Ok(())
    }

    fn source_interval_height_at(
        &self,
        point_xz: RoadVec2,
    ) -> Result<Option<f64>, NodeHeightFieldError> {
        for patch in &self.patches {
            match patch.source_handoff_height_at(point_xz) {
                Some(height_m) => return Ok(Some(height_m)),
                None => continue,
            }
        }
        Ok(None)
    }

    pub(super) fn source_handoff_authorized(
        &self,
        owner: NodeBandOwner,
        claim_priority: NodeGeneratedContourClaimPriority,
        point_xz: RoadVec2,
    ) -> bool {
        self.source_handoff_keys
            .contains(&NodeAuthorizedSourceHandoffKey {
                owner,
                claim_priority,
                point: height_source_point_key(point_xz),
            })
    }
}

pub(super) fn source_handoff_support_heights(
    interval: &NodeInputBandInterval,
    source_support_heights: &BTreeMap<NodeHeightSourcePointKey, f64>,
) -> BTreeMap<NodeHeightSourcePointKey, f64> {
    let base_keys = interval_declared_source_point_keys(interval);
    let mut support_keys = BTreeMap::new();
    for (&point_key, &height_m) in source_support_heights {
        if base_keys.contains(&point_key) {
            continue;
        }
        support_keys.insert(point_key, height_m);
    }
    support_keys
}

pub(super) fn source_support_heights(
    id: NodeBandHeightFieldId,
    source_kind: RoadSurfaceBandKind,
    source_support_points: Option<&[RoadVec3]>,
) -> Result<BTreeMap<NodeHeightSourcePointKey, f64>, NodeHeightFieldError> {
    let mut support_heights =
        BTreeMap::<NodeHeightSourcePointKey, (SurfaceHeightMmKey, f64)>::new();
    for point in source_support_points.unwrap_or(&[]) {
        let point_xz = quantize_road_vec2_to_overlay_grid(xz(*point));
        let point_key = height_source_point_key(point_xz);
        let height_key = SurfaceHeightMmKey::from_m_f64(point.y);
        let height_m = quantize_source_height_m(point.y);
        match support_heights.get_mut(&point_key) {
            Some((existing_height_key, _)) if *existing_height_key == height_key => {}
            Some((existing_height_key, _)) => {
                let key = NodeHeightPointKey::from_point(point_xz);
                return Err(NodeHeightFieldError::SourceHeightFieldConflict {
                    mouth_order_index: id.mouth_order_index(),
                    band_index: id.band_index(),
                    source_kind,
                    height_field_id: id,
                    owner: None,
                    existing_authority: NodeHeightAuthoritySource::SourceInterval,
                    incoming_authority: NodeHeightAuthoritySource::SourceInterval,
                    point_x_mm: key.x_mm(),
                    point_z_mm: key.z_mm(),
                    existing_height_mm: existing_height_key.as_i64(),
                    incoming_height_mm: height_key.as_i64(),
                });
            }
            None => {
                support_heights.insert(point_key, (height_key, height_m));
            }
        }
    }
    Ok(support_heights
        .into_iter()
        .map(|(point_key, (_, height_m))| (point_key, height_m))
        .collect())
}

fn interval_declared_source_point_keys(
    interval: &NodeInputBandInterval,
) -> BTreeSet<NodeHeightSourcePointKey> {
    if interval.start_path_world.is_empty() && interval.end_path_world.is_empty() {
        return [
            interval.mouth_start_world,
            interval.mouth_end_world,
            interval.endpoint_end_world,
            interval.endpoint_start_world,
        ]
        .into_iter()
        .map(|point| height_source_point_key(quantize_road_vec2_to_overlay_grid(xz(point))))
        .collect();
    }

    interval
        .start_path_world
        .iter()
        .chain(interval.end_path_world.iter())
        .map(|point| height_source_point_key(quantize_road_vec2_to_overlay_grid(xz(*point))))
        .collect()
}
