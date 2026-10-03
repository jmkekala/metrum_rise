// SPDX-License-Identifier: GPL-2.0-only

//! Span owned-region resolution and render bucket routing.

use super::super::{RoadSurfaceSection, RoadSurfaceSystem};
use super::{
    RoadSurfaceSpanBandOwner, RoadSurfaceSpanOwnedRegion, RoadSurfaceSpanRegionRole,
    SPAN_REGION_MIN_BAND_WIDTH_M, SpanQuad, SpanResolvedRegionSet,
};
use crate::simulation::network::types::EdgeClass;

impl RoadSurfaceSystem {
    pub(super) fn resolve_span_regions_for_ranges(
        &self,
        sections: &[RoadSurfaceSection],
        ranges: &[(usize, usize)],
        edge_class: EdgeClass,
    ) -> Option<SpanResolvedRegionSet> {
        let mut resolved = SpanResolvedRegionSet::default();

        for &(start_index, end_index) in ranges {
            if end_index <= start_index {
                continue;
            }
            for (segment_offset, pair) in sections[start_index..=end_index].windows(2).enumerate() {
                let start_section_index = start_index + segment_offset;
                let end_section_index = start_section_index + 1;
                if pair[0].bands.len() != pair[1].bands.len() {
                    return None;
                }
                for (band_index, (band_a, band_b)) in
                    pair[0].bands.iter().zip(&pair[1].bands).enumerate()
                {
                    if band_a.kind != band_b.kind {
                        return None;
                    }
                    let width_a = (band_a.lateral_end_m - band_a.lateral_start_m).abs();
                    let width_b = (band_b.lateral_end_m - band_b.lateral_start_m).abs();
                    if width_a <= SPAN_REGION_MIN_BAND_WIDTH_M
                        && width_b <= SPAN_REGION_MIN_BAND_WIDTH_M
                    {
                        continue;
                    }

                    let region = RoadSurfaceSpanOwnedRegion {
                        edge_idx: pair[0].edge_idx,
                        owner: RoadSurfaceSpanBandOwner {
                            source_band_index: band_index,
                            kind: band_a.kind,
                        },
                        role: RoadSurfaceSpanRegionRole::from_band_pair(band_a.kind, band_b.kind),
                        start_section_index,
                        end_section_index,
                        start_s_m: pair[0].s_m,
                        end_s_m: pair[1].s_m,
                    };
                    // Regions keep only their section span; the quad is rebuilt on read.
                    if SpanQuad::from_corners(region.corners(sections)).is_some() {
                        resolved.regions.push(region);
                    }
                }
                resolved.raised_step_constraints.extend(
                    Self::span_raised_step_constraints_for_resolved_segment(
                        pair,
                        start_section_index,
                        end_section_index,
                    ),
                );
            }
        }

        let (outer_boundary_loops, terrain_clip_boundary_loops) =
            Self::build_span_boundary_loops_from_regions(sections, &resolved.regions, edge_class)
                .map_err(|error| {
                    crate::debug_log!(
                        "road",
                        "span_boundary_failed edge={:?} error={:?}",
                        sections.first().map(|s| s.edge_idx),
                        error
                    );
                })
                .ok()?;
        resolved.outer_boundary_loops = outer_boundary_loops;
        resolved.terrain_clip_boundary_loops = terrain_clip_boundary_loops;
        Some(resolved)
    }

    pub(super) fn sort_span_owned_regions(
        sections: &[RoadSurfaceSection],
        regions: &mut [RoadSurfaceSpanOwnedRegion],
    ) {
        regions.sort_by(|a, b| {
            a.edge_idx
                .cmp(&b.edge_idx)
                .then(a.start_section_index.cmp(&b.start_section_index))
                .then(a.end_section_index.cmp(&b.end_section_index))
                .then(a.start_s_m.total_cmp(&b.start_s_m))
                .then(a.end_s_m.total_cmp(&b.end_s_m))
                .then(a.role.sort_key().cmp(&b.role.sort_key()))
                .then(a.owner.sort_key().cmp(&b.owner.sort_key()))
                .then_with(|| {
                    Self::visual_points_ordering(
                        a.quad(sections).points(),
                        b.quad(sections).points(),
                    )
                })
        });
    }
}
