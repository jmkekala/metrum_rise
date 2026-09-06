// SPDX-License-Identifier: GPL-2.0-only

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
use super::source_edges::TerrainClipSourceEdgeIndex;

impl RoadSurfaceSystem {
    fn terrain_clip_dust_connector_heights_from_source_edges(
        contour: &NodeOverlayContour,
        segment_index: usize,
        source_edges: &[TerrainClipSourceEdge],
        source_edge_index: &TerrainClipSourceEdgeIndex,
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
        let previous_sources =
            source_edge_index.candidates_for_segment(previous, start, source_edges);
        let next_sources = source_edge_index.candidates_for_segment(end, next, source_edges);
        if let (Some(previous_heights), Some(next_heights)) = (
            Self::terrain_clip_segment_heights_from_source_edges(
                previous,
                start,
                &previous_sources,
            ),
            Self::terrain_clip_segment_heights_from_source_edges(end, next, &next_sources),
        ) {
            Self::validate_terrain_clip_dust_endpoint_height(
                "start",
                start,
                previous_heights.end_y,
                &source_edge_index.candidates_for_segment(start, start, source_edges),
            )?;
            Self::validate_terrain_clip_dust_endpoint_height(
                "end",
                end,
                next_heights.start_y,
                &source_edge_index.candidates_for_segment(end, end, source_edges),
            )?;
            return Ok(Some(TerrainClipSegmentHeights {
                start_y: previous_heights.end_y,
                end_y: next_heights.start_y,
            }));
        }

        let Some(start_y) = Self::terrain_clip_dust_run_vertex_height(
            contour,
            segment_index,
            source_edges,
            source_edge_index,
        )?
        else {
            return Ok(None);
        };
        let Some(end_y) = Self::terrain_clip_dust_run_vertex_height(
            contour,
            (segment_index + 1) % len,
            source_edges,
            source_edge_index,
        )?
        else {
            return Ok(None);
        };
        Ok(Some(TerrainClipSegmentHeights { start_y, end_y }))
    }

    fn terrain_clip_dust_run_vertex_height(
        contour: &NodeOverlayContour,
        vertex_index: usize,
        source_edges: &[TerrainClipSourceEdge],
        source_edge_index: &TerrainClipSourceEdgeIndex,
    ) -> Result<Option<f64>, String> {
        let len = contour.len();
        let point_height = |index: usize| {
            let point = contour[index];
            let sources = source_edge_index.candidates_for_segment(point, point, source_edges);
            Self::terrain_clip_dust_overlay_point_height_from_source_edges(point, &sources)
        };
        if let Some(height) = point_height(vertex_index)? {
            return Ok(Some(height));
        }

        // Only the connected dust run and its two source anchors own this height. Other
        // contour vertices can have legitimate raised curb steps resolved by the cutter's
        // top envelope; applying dust-height equality there rejects unrelated geometry.
        let find_anchor = |backwards: bool| -> Result<Option<(f64, f64)>, String> {
            let mut index = vertex_index;
            let mut distance_m = 0.0;
            for _ in 1..len {
                let next = if backwards {
                    (index + len - 1) % len
                } else {
                    (index + 1) % len
                };
                let edge_index = if backwards { next } else { index };
                if !Self::terrain_clip_connector_is_numeric_dust(contour, edge_index) {
                    return Ok(None);
                }
                distance_m += overlay_segment_length_m(contour[index], contour[next]);
                if let Some(height) = point_height(next)? {
                    return Ok(Some((height, distance_m)));
                }
                index = next;
            }
            Ok(None)
        };
        let Some((previous_y, previous_distance)) = find_anchor(true)? else {
            return Ok(None);
        };
        let Some((next_y, next_distance)) = find_anchor(false)? else {
            return Ok(None);
        };
        let total_distance = previous_distance + next_distance;
        let t = if total_distance > 0.0 {
            previous_distance / total_distance
        } else {
            0.0
        };
        Ok(Some(interpolate_height_f64(previous_y, next_y, t)))
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

    pub(super) fn terrain_clip_dust_connector_points_from_source_edges(
        contour: &NodeOverlayContour,
        segment_index: usize,
        source_edges: &[TerrainClipSourceEdge],
        source_edge_index: &TerrainClipSourceEdgeIndex,
    ) -> TerrainClipDustConnectorRecovery {
        let len = contour.len();
        let heights = match Self::terrain_clip_dust_connector_heights_from_source_edges(
            contour,
            segment_index,
            source_edges,
            source_edge_index,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::{
        RoadSurfaceBandKind, RoadSurfaceEarthworkFaceSource, RoadSurfaceTerrainClipEdgeKind,
        RoadSurfaceVisualNodePieceKind,
    };

    #[test]
    fn dust_run_recovery_ignores_a_remote_curb_step() {
        let mut contour = vec![
            [0.0, 0.0],
            [0.5, 0.0],
            [0.50002, 0.00008],
            [0.49998, 0.00016],
            [0.50001, 0.00024],
            [0.5, 0.00032],
            [1.0, 0.0],
            [1.0, 1.0],
            [0.0, 1.0],
        ];
        let source_edge = |start: [f64; 2], start_y, end: [f64; 2], end_y| TerrainClipSourceEdge {
            start: RoadVec3::new(start[0], start_y, start[1]),
            end: RoadVec3::new(end[0], end_y, end[1]),
            kind: RoadSurfaceTerrainClipEdgeKind::SidewalkOuter,
            source: RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                node_id: 0,
                kind: RoadSurfaceVisualNodePieceKind::Bend,
                owner_kind: RoadSurfaceBandKind::Sidewalk,
                owner_index: 0,
                boundary_source: None,
            },
            source_index: 0,
            edge_index: 0,
        };
        let sources = vec![
            source_edge(contour[0], 20.0, contour[1], 20.0),
            source_edge(contour[5], 20.006, contour[6], 20.006),
            source_edge(contour[6], 20.006, contour[7], 20.006),
            source_edge(contour[7], 20.006, contour[8], 20.0),
            source_edge(contour[8], 20.0, contour[0], 20.0),
        ];
        let mut with_curb_step = sources.clone();
        with_curb_step.push(source_edge(contour[7], 20.126, [1.1, 1.0], 20.126));
        let gap_start = contour[2];
        let gap_end = contour[3];
        // Contour order and cyclic start must not change which run owns the recovery.
        for _ in 0..2 {
            for _ in 0..contour.len() {
                let index = (0..contour.len())
                    .find(|&index| {
                        let pair = [contour[index], contour[(index + 1) % contour.len()]];
                        pair == [gap_start, gap_end] || pair == [gap_end, gap_start]
                    })
                    .unwrap();
                let expected =
                    RoadSurfaceSystem::terrain_clip_dust_connector_heights_from_source_edges(
                        &contour,
                        index,
                        &sources,
                        &TerrainClipSourceEdgeIndex::new(&sources),
                    )
                    .unwrap()
                    .unwrap();
                let actual =
                    RoadSurfaceSystem::terrain_clip_dust_connector_heights_from_source_edges(
                        &contour,
                        index,
                        &with_curb_step,
                        &TerrainClipSourceEdgeIndex::new(&with_curb_step),
                    )
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    [actual.start_y, actual.end_y],
                    [expected.start_y, expected.end_y]
                );
                assert!(actual.start_y >= 20.0 && actual.start_y <= 20.006);
                assert!(actual.end_y >= 20.0 && actual.end_y <= 20.006);
                contour.rotate_left(1);
            }
            contour.reverse();
            with_curb_step.reverse();
        }
    }
}
