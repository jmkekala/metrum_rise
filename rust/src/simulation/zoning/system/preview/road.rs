// SPDX-License-Identifier: GPL-2.0-only

//! Shared manual and automatic fill anchored to existing parcels, with indexed run spacing.

use super::*;
use crate::simulation::network::graph::Edge;
use crate::simulation::zoning::{MAX_PARCEL_FRONTAGE_M, ZoningParcel};
use std::collections::{HashSet, VecDeque};

struct RoadFillSpan {
    side: i8,
    next: [Option<f32>; 2],
    limits: [f32; 2],
}

impl RoadFillSpan {
    fn new(
        side: i8,
        left: Option<&ZoningParcel>,
        right: Option<&ZoningParcel>,
        length: f32,
        frontage: f32,
        gap: f32,
        range: [f32; 2],
    ) -> Self {
        let min = left.map_or(range[0], |p| {
            p.frontage_center_t() * length + (p.frontage_m() + frontage) * 0.5 + gap
        });
        let max = right.map_or(range[1], |p| {
            p.frontage_center_t() * length - (p.frontage_m() + frontage) * 0.5 - gap
        });
        Self {
            side,
            // Empty rows start at the road start. Bounded rows grow from their existing lots.
            next: [
                (left.is_some() || right.is_none()).then_some(min),
                right.map(|_| max),
            ],
            limits: [max.min(range[1]), min.max(range[0])],
        }
    }
}

impl ZoningSystem {
    /// Projects both road sides, extending from existing parcels with the drag-run spacing solver.
    /// Empty rows start at the road endpoint; gaps between authored parcels fill from both ends.
    /// Existing parcels remain unchanged and all overlap checks use local parcel indices.
    pub fn preview_parcels_on_road(
        &self,
        edge_idx: usize,
        frontage_m: f32,
        depth_m: f32,
        gap_m: f32,
        graph: &RegionGraph,
    ) -> Result<Vec<ParcelGeometry>, ParcelPlacementError> {
        Self::validate_parcel_dimensions(frontage_m, depth_m)?;
        Self::validate_parcel_gap(gap_m)?;
        let edge = buildable_edge(graph, edge_idx, frontage_m)?;
        let range = [frontage_m * 0.5, edge.physical_length - frontage_m * 0.5];
        let anchors =
            self.parcels
                .attached_to_road_span(edge_idx, edge, [0.0, edge.physical_length]);
        let spans = fill_spans(
            &anchors,
            &[1, -1],
            edge.physical_length,
            frontage_m,
            gap_m,
            range,
        );
        let geometries =
            self.project_road_fill(spans, edge_idx, frontage_m, depth_m, gap_m, range, graph);
        self.valid_parcel_run_geometries(geometries, graph)
    }

    /// Uses the road-fill engine within a manual gesture when existing lots anchor that span.
    /// Returning `None` preserves the free-placement phase search on an unanchored drag.
    pub(super) fn preview_anchored_parcel_run(
        &self,
        start_point: Vector2,
        end_point: Vector2,
        frontage_m: f32,
        depth_m: f32,
        gap_m: f32,
        graph: &RegionGraph,
    ) -> Result<Option<Vec<ParcelGeometry>>, ParcelPlacementError> {
        let existing = self.parcel_at(start_point);
        let (edge_idx, side, start_s) = if let Some(p) = existing {
            let edge = buildable_edge(graph, p.edge_idx(), frontage_m)?;
            (
                p.edge_idx(),
                p.side(),
                p.frontage_center_t() * edge.physical_length,
            )
        } else {
            let start =
                parcels::project_buildable_road_point_at(graph, start_point, frontage_m, depth_m)?;
            (start.edge_idx, start.side, start.s_m)
        };
        let edge = buildable_edge(graph, edge_idx, frontage_m)?;
        let end = parcels::project_point_to_edge(graph, edge_idx, end_point)
            .ok_or(ParcelPlacementError::NoRoadAttachment)?;
        let range = [
            start_s.min(end.s_m).max(frontage_m * 0.5),
            start_s
                .max(end.s_m)
                .min(edge.physical_length - frontage_m * 0.5),
        ];
        // Nearby boundary anchors can determine a lot inside the gesture. Query only that
        // interval plus one maximum attachment offset, never all parcels on a long road.
        let margin = (MAX_PARCEL_FRONTAGE_M + frontage_m) * 0.5 + gap_m;
        let mut anchors = self.parcels.attached_to_road_span(
            edge_idx,
            edge,
            [
                (range[0] - margin).max(0.0),
                (range[1] + margin).min(edge.physical_length),
            ],
        );
        anchors.retain(|p| p.side() == side);
        if anchors.is_empty() {
            return Ok(None);
        }
        let spans = fill_spans(
            &anchors,
            &[side],
            edge.physical_length,
            frontage_m,
            gap_m,
            range,
        );
        let geometries =
            self.project_road_fill(spans, edge_idx, frontage_m, depth_m, gap_m, range, graph);
        let mut valid = self.valid_parcel_run_geometries(geometries, graph)?;
        // Existing-parcel extensions retain drag direction; free-start drags retain station order.
        let reverse = existing.is_some() && end.s_m < start_s;
        valid.sort_unstable_by(|a, b| {
            let order = a.frontage_center_t.total_cmp(&b.frontage_center_t);
            if reverse { order.reverse() } else { order }
        });
        Ok(Some(valid))
    }

    fn project_road_fill(
        &self,
        mut spans: Vec<RoadFillSpan>,
        edge_idx: usize,
        frontage_m: f32,
        depth_m: f32,
        gap_m: f32,
        range: [f32; 2],
        graph: &RegionGraph,
    ) -> Vec<ParcelGeometry> {
        let length = graph.edge(edge_idx).physical_length;
        let mut active = VecDeque::with_capacity(spans.len() * 2);
        for (index, span) in spans.iter().enumerate() {
            for direction in 0..2 {
                if span.next[direction].is_some() {
                    active.push_back((index, direction));
                }
            }
        }
        let spacing = frontage_m + gap_m;
        let mut projected = parcels::ParcelStore::default();
        let mut projected_seen = HashSet::new();
        let mut existing_seen = HashSet::new();
        // Each active front performs one spacing step per turn. Retire exhausted fronts so a
        // long empty stretch never repeatedly scans the many completed spans of a zoned road.
        while let Some((index, direction_index)) = active.pop_front() {
            let span = &mut spans[index];
            let Some(center_s) = span.next[direction_index].take() else {
                continue;
            };
            let direction = if direction_index == 0 { 1.0 } else { -1.0 };
            let Some((station, geometry)) = parcels::next_non_overlapping_run_geometry_with(
                graph,
                edge_idx,
                span.side,
                center_s,
                span.limits[direction_index],
                direction,
                length,
                frontage_m,
                depth_m,
                |geometry| {
                    projected_seen.clear();
                    existing_seen.clear();
                    projected.overlaps_existing_with_scratch(geometry, &mut projected_seen)
                        || self
                            .parcels
                            .overlaps_existing_with_scratch(geometry, &mut existing_seen)
                },
            ) else {
                continue;
            };
            projected.insert_new(geometry, 0);
            let next = station + direction * spacing;
            span.next[direction_index] = Some(next);
            // The opposite front must stop here instead of searching through the row already
            // produced from this end. Any remainder stays between fronts, not beside anchors.
            span.limits[1 - direction_index] = next;
            active.push_back((index, direction_index));
        }
        projected
            .parcels()
            .iter()
            .filter(|p| {
                let station = p.frontage_center_t() * length;
                station >= range[0] - 0.001 && station <= range[1] + 0.001
            })
            .map(parcels::geometry_for_parcel)
            .collect()
    }
}

fn fill_spans(
    anchors: &[&ZoningParcel],
    sides: &[i8],
    length: f32,
    frontage: f32,
    gap: f32,
    range: [f32; 2],
) -> Vec<RoadFillSpan> {
    let mut spans = Vec::with_capacity(anchors.len() + sides.len());
    for &side in sides {
        let mut previous = None;
        for &anchor in anchors.iter().filter(|p| p.side() == side) {
            spans.push(RoadFillSpan::new(
                side,
                previous,
                Some(anchor),
                length,
                frontage,
                gap,
                range,
            ));
            previous = Some(anchor);
        }
        spans.push(RoadFillSpan::new(
            side, previous, None, length, frontage, gap, range,
        ));
    }
    spans
}

fn buildable_edge(
    graph: &RegionGraph,
    edge_idx: usize,
    frontage: f32,
) -> Result<&Edge, ParcelPlacementError> {
    let edge = graph
        .edges()
        .get(edge_idx)
        .ok_or(ParcelPlacementError::NoRoadAttachment)?;
    if edge.deleted
        || edge.no_building_spawn
        || edge.primary_type != crate::simulation::network::types::TransitType::Road
        || edge.physical_geometry.len() < 2
        || !edge.physical_length.is_finite()
        || edge.physical_length <= frontage
    {
        return Err(ParcelPlacementError::NoRoadAttachment);
    }
    Ok(edge)
}
