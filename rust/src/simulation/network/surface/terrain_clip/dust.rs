//! Numeric-dust terrain-clip connector recovery.

use super::super::backend::RoadVec3;
use super::super::{
    NODE_OVERLAY_NUMERIC_DUST_WIDTH_M, NodeOverlayContour, RoadSurfaceSystem,
    keys::SurfaceHeightMmKey,
};
use super::geometry::{
    contour_area_delta_after_removing_vertex, interpolate_height_f64, overlay_segment_length_m,
};
use super::heights::TERRAIN_CLIP_DUST_HEIGHT_TIE_TOLERANCE_MM;
use super::model::{
    TerrainClipDustConnectorRecovery, TerrainClipSegmentHeights, TerrainClipSourceEdge,
};

impl RoadSurfaceSystem {
    fn terrain_clip_dust_connector_heights_from_source_edges(
        contour: &NodeOverlayContour,
        segment_index: usize,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Result<Option<TerrainClipSegmentHeights>, String> {
        let len = contour.len();
        if len < 3 {
            return Ok(None);
        }

        let start = contour[segment_index];
        let end = contour[(segment_index + 1) % len];
        if !Self::terrain_clip_connector_is_numeric_dust(contour, segment_index) {
            return Ok(None);
        }

        let previous = contour[(segment_index + len - 1) % len];
        let next = contour[(segment_index + 2) % len];
        if let (Some(previous_heights), Some(next_heights)) = (
            Self::terrain_clip_segment_heights_from_source_edges(previous, start, source_edges),
            Self::terrain_clip_segment_heights_from_source_edges(end, next, source_edges),
        ) {
            Self::validate_terrain_clip_dust_endpoint_height(
                "start",
                start,
                previous_heights.end_y,
                source_edges,
            )?;
            Self::validate_terrain_clip_dust_endpoint_height(
                "end",
                end,
                next_heights.start_y,
                source_edges,
            )?;
            return Ok(Some(TerrainClipSegmentHeights {
                start_y: previous_heights.end_y,
                end_y: next_heights.start_y,
            }));
        }

        let heights =
            Self::terrain_clip_contour_vertex_heights_from_source_edges(contour, source_edges)?;
        let Some(heights) = heights else {
            return Ok(None);
        };
        Ok(Some(TerrainClipSegmentHeights {
            start_y: heights[segment_index],
            end_y: heights[(segment_index + 1) % len],
        }))
    }

    fn terrain_clip_contour_vertex_heights_from_source_edges(
        contour: &NodeOverlayContour,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Result<Option<Vec<f64>>, String> {
        let len = contour.len();
        if len < 3 {
            return Ok(None);
        }

        let mut heights = contour
            .iter()
            .copied()
            .map(|point| {
                Self::terrain_clip_dust_overlay_point_height_from_source_edges(point, source_edges)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let Some(anchor) = heights.iter().position(Option::is_some) else {
            return Ok(None);
        };
        let mut offset = 1usize;
        while offset < len {
            let index = (anchor + offset) % len;
            if heights[index].is_some() {
                offset += 1;
                continue;
            }

            let run_start_offset = offset;
            while offset < len && heights[(anchor + offset) % len].is_none() {
                offset += 1;
            }
            let prev_index = (anchor + run_start_offset - 1) % len;
            let next_index = (anchor + offset) % len;
            if Self::interpolate_terrain_clip_dust_run_heights(
                contour,
                &mut heights,
                prev_index,
                next_index,
            )
            .is_none()
            {
                return Ok(None);
            }
        }

        Ok(heights.into_iter().collect())
    }

    fn validate_terrain_clip_dust_endpoint_height(
        label: &'static str,
        point: super::super::NodeOverlayPoint,
        height: f64,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Result<(), String> {
        let Some(source_height) =
            Self::terrain_clip_dust_overlay_point_height_from_source_edges(point, source_edges)?
        else {
            return Ok(());
        };
        if Self::terrain_clip_dust_heights_equal(source_height, height) {
            Ok(())
        } else {
            Err(format!(
                "dust_connector_{label}_height_disagrees source={source_height:.6} recovered={height:.6}"
            ))
        }
    }

    fn interpolate_terrain_clip_dust_run_heights(
        contour: &NodeOverlayContour,
        heights: &mut [Option<f64>],
        prev_index: usize,
        next_index: usize,
    ) -> Option<()> {
        let start_y = heights[prev_index]?;
        let end_y = heights[next_index]?;
        let mut total_length_m = 0.0f64;
        let mut edge_index = prev_index;
        while edge_index != next_index {
            if !Self::terrain_clip_connector_is_numeric_dust(contour, edge_index) {
                return None;
            }
            let next = (edge_index + 1) % contour.len();
            total_length_m += overlay_segment_length_m(contour[edge_index], contour[next]);
            edge_index = next;
        }

        let mut distance_m = 0.0f64;
        edge_index = prev_index;
        while edge_index != next_index {
            let next = (edge_index + 1) % contour.len();
            distance_m += overlay_segment_length_m(contour[edge_index], contour[next]);
            if heights[next].is_none() {
                let t = if total_length_m > 0.0 {
                    distance_m / total_length_m
                } else {
                    0.0
                };
                heights[next] = Some(interpolate_height_f64(start_y, end_y, t));
            }
            edge_index = next;
        }

        Some(())
    }

    pub(super) fn terrain_clip_dust_connector_points_from_source_edges(
        contour: &NodeOverlayContour,
        segment_index: usize,
        source_edges: &[TerrainClipSourceEdge],
    ) -> TerrainClipDustConnectorRecovery {
        let len = contour.len();
        let heights = match Self::terrain_clip_dust_connector_heights_from_source_edges(
            contour,
            segment_index,
            source_edges,
        ) {
            Ok(Some(heights)) => heights,
            Ok(None) => return TerrainClipDustConnectorRecovery::Missing,
            Err(context) => return TerrainClipDustConnectorRecovery::Ambiguous(context),
        };
        let start = contour[segment_index];
        let end = contour[(segment_index + 1) % len];
        TerrainClipDustConnectorRecovery::Covered(vec![
            RoadVec3::new(start[0], heights.start_y, start[1]),
            RoadVec3::new(end[0], heights.end_y, end[1]),
        ])
    }

    fn terrain_clip_connector_is_numeric_dust(
        contour: &NodeOverlayContour,
        segment_index: usize,
    ) -> bool {
        let len = contour.len();
        if len < 4 {
            return false;
        }

        let start = contour[segment_index];
        let end = contour[(segment_index + 1) % len];
        let connector_length_squared_m2 =
            (start[0] - end[0]) * (start[0] - end[0]) + (start[1] - end[1]) * (start[1] - end[1]);
        if connector_length_squared_m2
            <= f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M)
                * f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M)
        {
            return true;
        }
        let budget_m2 = f64::from(Self::overlay_numeric_area_budget_m2(
            Self::overlay_contour_perimeter_m(contour),
            contour.len(),
        ));
        if connector_length_squared_m2 > budget_m2 {
            return false;
        }

        let area_m2 = Self::overlay_contour_area_f64(contour).abs();
        let remove_start_delta = contour_area_delta_after_removing_vertex(contour, segment_index)
            .map(|area| (area - area_m2).abs());
        let remove_end_delta =
            contour_area_delta_after_removing_vertex(contour, (segment_index + 1) % len)
                .map(|area| (area - area_m2).abs());
        remove_start_delta
            .into_iter()
            .chain(remove_end_delta)
            .any(|delta| delta <= budget_m2)
    }

    fn terrain_clip_dust_heights_equal(a: f64, b: f64) -> bool {
        let a_mm = SurfaceHeightMmKey::from_m_f64(a).as_i64();
        let b_mm = SurfaceHeightMmKey::from_m_f64(b).as_i64();
        a_mm.abs_diff(b_mm) <= TERRAIN_CLIP_DUST_HEIGHT_TIE_TOLERANCE_MM
    }
}
