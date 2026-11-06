// SPDX-License-Identifier: GPL-2.0-only

//! Kerbside parallel bays along a road's parking lanes (`PARK-05`), from its roadbed sections.

use super::Bay;
use crate::assets::AnchorType;
use crate::simulation::buildings::allocator::{
    BuildingAllocator, building_local_xz_basis, main_entrance_anchor,
};
use crate::simulation::network::graph::{Edge, PARKING_LANE_WIDTH_M, RegionGraph};
use crate::simulation::network::roadbed::{BandKind, EdgeEnd, RoadSection, RoadbedSystem};
use godot::prelude::Vector2;

/// Length of one kerbside bay along the kerb; bays are painted end to end.
pub(crate) const KERB_BAY_LENGTH_M: f32 = 6.0;
// No kerbside bay within 5 m of a junction mouth or a crossing, as Finnish traffic rules require.
const KERB_CLEAR_M: f32 = 5.0;
// Room kept either side of a driveway's opening so cars can turn in past parked ones.
const DRIVEWAY_CLEAR_M: f32 = 1.0;
// Opening of a driveway anchor without a width, and of the drive a building without driveway
// anchors has opposite its main entrance.
const DRIVEWAY_WIDTH_M: f32 = 3.0;

/// A stretch of one road side that kerbside bays keep clear of: a driveway, or a lot bay entered
/// straight from the street.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct KerbBlocker {
    pub edge: u32,
    pub right: bool,
    pub s_m: f32,
    pub half_m: f32,
}

/// Bays of the left and the right parking lane. O(sections + bays × log sections + bays ×
/// blockers), at road/site edit boundaries only. Empty for a deleted edge, a side without a
/// parking lane, or a span too short to hold one bay clear of its ends and crossings.
pub(crate) fn kerb_bays(
    edge_idx: usize,
    graph: &RegionGraph,
    roads: &RoadbedSystem,
    blockers: &[KerbBlocker],
) -> [Vec<Bay>; 2] {
    let mut out = [Vec::new(), Vec::new()];
    let Some(edge) = graph.get_edge(edge_idx).filter(|e| !e.deleted) else {
        return out;
    };
    let (Some(sections), Some(span)) = (roads.sections(edge_idx), roads.span_piece(edge_idx))
    else {
        return out;
    };
    let (Some(first), Some(last)) = (sections.first(), sections.last()) else {
        return out;
    };
    let lo = span
        .crossing_strip_m(EdgeEnd::Start)
        .map_or(first.s_m, |(_, high)| high.max(first.s_m))
        + KERB_CLEAR_M;
    let hi = span
        .crossing_strip_m(EdgeEnd::End)
        .map_or(last.s_m, |(low, _)| low.min(last.s_m))
        - KERB_CLEAR_M;
    for (slot, right) in [(0, false), (1, true)] {
        if !(if right {
            edge.parking.right
        } else {
            edge.parking.left
        }) {
            continue;
        }
        let mut free = vec![(lo, hi)];
        for b in blockers.iter().filter(|b| b.right == right) {
            let (a, z) = (
                b.s_m - b.half_m - DRIVEWAY_CLEAR_M,
                b.s_m + b.half_m + DRIVEWAY_CLEAR_M,
            );
            free = free
                .into_iter()
                .flat_map(|(s0, s1)| [(s0, s1.min(a)), (s0.max(z), s1)])
                .filter(|(s0, s1)| s1 > s0)
                .collect();
        }
        free.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        for (s0, s1) in free {
            // Whole bays only, centred in the free stretch.
            let count = ((s1 - s0) / KERB_BAY_LENGTH_M).floor() as usize;
            let start = s0 + 0.5 * (s1 - s0 - count as f32 * KERB_BAY_LENGTH_M);
            for k in 0..count {
                let s = start + (k as f32 + 0.5) * KERB_BAY_LENGTH_M;
                if let Some(bay) = bay_at(edge, edge_idx, sections, s, right, out[slot].len()) {
                    out[slot].push(bay);
                }
            }
        }
    }
    out
}

// One bay centred at station `s`, half a parking lane in from the carriageway's edge.
fn bay_at(
    edge: &Edge,
    edge_idx: usize,
    sections: &[RoadSection],
    s: f32,
    right: bool,
    ordinal: usize,
) -> Option<Bay> {
    let i = sections
        .partition_point(|section| section.s_m <= s)
        .clamp(1, sections.len().max(2) - 1);
    let (a, b) = (sections.get(i - 1)?, sections.get(i)?);
    let t = if b.s_m > a.s_m {
        f64::from(((s - a.s_m) / (b.s_m - a.s_m)).clamp(0.0, 1.0))
    } else {
        0.0
    };
    let kerb = |section: &RoadSection| {
        let mut bands = section
            .bands
            .iter()
            .filter(|band| band.kind == BandKind::Carriageway);
        let band = if right {
            bands.next_back()
        } else {
            bands.next()
        }?;
        let half = 0.5 * PARKING_LANE_WIDTH_M;
        let (offset, height) = if right {
            (band.lateral_end_m - half, band.height_end_m)
        } else {
            (band.lateral_start_m + half, band.height_start_m)
        };
        Some((
            section.center_xz + section.lateral_xz * f64::from(offset),
            height,
        ))
    };
    let ((pa, ha), (pb, hb)) = (kerb(a)?, kerb(b)?);
    let centre = pa.lerp(pb, t);
    let tangent = a.tangent_xz.lerp(b.tangent_xz, t).normalize_or_zero();
    let lateral = a.lateral_xz.lerp(b.lateral_xz, t).normalize_or_zero();
    if tangent == glam::DVec2::ZERO || lateral == glam::DVec2::ZERO {
        return None;
    }
    // Cars park in the direction of the traffic lanes beside them; a one-way road's lanes serve
    // both kerbs.
    let along = if right {
        edge.fwd_lanes > 0 || edge.bkw_lanes == 0
    } else {
        edge.bkw_lanes == 0
    };
    let d = if along { tangent } else { -tangent };
    let k = if right { lateral } else { -lateral };
    let c = Vector2::new(centre.x as f32, centre.y as f32);
    let d = Vector2::new(d.x as f32, d.y as f32);
    let k = Vector2::new(k.x as f32, k.y as f32);
    // From beside the bay's rear in the traffic lane, easing in to the kerb.
    let half = 0.5 * KERB_BAY_LENGTH_M;
    let path = [
        c - d * (half + 4.0) - k * 1.75,
        c - d * (half + 1.5) - k * 1.0,
        c - d * 1.0 - k * 0.2,
        c,
    ]
    .map(|p| [p.x, p.y]);
    Some(Bay {
        identity: [
            (1 << 63) | ((edge_idx as u64) << 1) | u64::from(right),
            ordinal as u64,
        ],
        position: [c.x, ha + (hb - ha) * t as f32 + 0.02, c.y],
        path,
        heading: (-d.x).atan2(-d.y),
        width: PARKING_LANE_WIDTH_M,
        length: KERB_BAY_LENGTH_M,
    })
}

/// Stretches of the building's frontage its driveways and street-entered lot bays need clear.
/// A building without driveway anchors keeps a drive clear opposite its main entrance, where
/// its deliveries and drop-offs pull in. O(anchors + bays), allocation bounded by them.
pub(crate) fn kerb_blockers(
    allocator: &BuildingAllocator,
    index: usize,
    graph: &RegionGraph,
    lot_bays: &[(u32, Bay)],
) -> Vec<KerbBlocker> {
    let Some(building) = allocator.buildings.get(index) else {
        return Vec::new();
    };
    let Some(edge) = graph.get_edge(building.edge_idx).filter(|e| !e.deleted) else {
        return Vec::new();
    };
    let Some(asset) = allocator.registry.get(&building.asset_id) else {
        return Vec::new();
    };
    let (bx, bz) = building_local_xz_basis(
        building.facing_dir,
        asset.manifest.building_frontage_forward(),
    );
    let center = Vector2::new(building.center_x, building.center_y);
    let blocker = |point: Vector2, half_m: f32| KerbBlocker {
        edge: building.edge_idx as u32,
        right: building.side == -1,
        s_m: BuildingAllocator::project_point_to_polyline_s(&edge.physical_geometry, point),
        half_m,
    };
    let anchors = &asset.manifest.anchors;
    let at = |a: &crate::assets::Anchor| center + bx * a.position[0] + bz * a.position[2];
    let mut out: Vec<_> = anchors
        .iter()
        .filter(|a| a.anchor_type == AnchorType::Driveway)
        .map(|a| blocker(at(a), 0.5 * a.width_m.unwrap_or(DRIVEWAY_WIDTH_M)))
        .collect();
    if out.is_empty() {
        out.extend(main_entrance_anchor(anchors).map(|a| blocker(at(a), 0.5 * DRIVEWAY_WIDTH_M)));
    }
    out.extend(lot_bays.iter().map(|(_, bay)| {
        blocker(
            Vector2::new(bay.path[0][0], bay.path[0][1]),
            0.5 * bay.width,
        )
    }));
    out
}

impl super::ParkingSystem {
    /// Replaces one building's kerb blockers and records the edges whose kerbside bays they
    /// move. O(old + new blockers + edge blocker lists), at site/road edit boundaries only.
    pub(crate) fn set_kerb_blockers(
        &mut self,
        building: u64,
        blockers: Vec<KerbBlocker>,
        touched: &mut std::collections::BTreeSet<usize>,
    ) {
        let old = self.kerb_blockers.remove(&building).unwrap_or_default();
        if old == blockers {
            if !blockers.is_empty() {
                self.kerb_blockers.insert(building, blockers);
            }
            return;
        }
        for edge in old.iter().map(|b| b.edge) {
            touched.insert(edge as usize);
            if let Some(list) = self.edge_blockers.get_mut(&edge) {
                list.retain(|&h| h != building);
                if list.is_empty() {
                    self.edge_blockers.remove(&edge);
                }
            }
        }
        for edge in blockers.iter().map(|b| b.edge) {
            touched.insert(edge as usize);
            let list = self.edge_blockers.entry(edge).or_default();
            if let Err(at) = list.binary_search(&building) {
                list.insert(at, building);
            }
        }
        if !blockers.is_empty() {
            self.kerb_blockers.insert(building, blockers);
        }
    }

    /// Every blocker on one edge, in building-handle order. O(blockers on the edge).
    pub(crate) fn edge_kerb_blockers(&self, edge: usize) -> Vec<KerbBlocker> {
        let edge = edge as u32;
        self.edge_blockers
            .get(&edge)
            .into_iter()
            .flatten()
            .filter_map(|b| self.kerb_blockers.get(b))
            .flatten()
            .filter(|b| b.edge == edge)
            .copied()
            .collect()
    }

    /// Reconciles both parking lanes of one edge, keeping parked cars whose bay did not move.
    pub(crate) fn replace_kerb(&mut self, edge: usize, bays: &[Vec<Bay>; 2]) {
        for (right, bays) in [(false, &bays[0]), (true, &bays[1])] {
            let source = super::PlaceSource::Kerb {
                edge: edge as u32,
                right,
            };
            self.replace_source(source, bays);
        }
    }

    /// Positions of one edge's kerbside bays, for local candidate invalidation. O(its bays).
    pub(crate) fn kerb_points(&self, edge: usize) -> Vec<[f32; 2]> {
        [false, true]
            .into_iter()
            .flat_map(|right| {
                self.sources.get(&super::PlaceSource::Kerb {
                    edge: edge as u32,
                    right,
                })
            })
            .flatten()
            .filter_map(|&h| self.place_handles.resolve(h))
            .map(|p| {
                let pos = self.places.position[p];
                [pos[0], pos[2]]
            })
            .collect()
    }
}
