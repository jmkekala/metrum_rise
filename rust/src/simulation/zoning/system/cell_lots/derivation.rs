// SPDX-License-Identifier: GPL-2.0-only

//! Local asset-sized lot proposals supported by complete generated road frontages.

use super::ZoningSystem;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::{EdgeClass, TransitType};
use crate::simulation::zoning::cells::{
    CELL_DEPTH, CELL_FRONTAGE_UNITS, CellBounds, CellKey, CellLot, CellLotSize, CellRoadFrontage,
    GridFrame, overlaps_road_corridors,
};
use crate::simulation::zoning::{MAX_PARCEL_FRONTAGE_M, ParcelGeometry, ParcelId, parcels};
use glam::DVec2;
use godot::prelude::Vector2;
use rayon::prelude::*;
use std::cmp::Reverse;

/// Work and stable ids changed by one local derivation transaction.
#[derive(Debug, Default)]
pub(crate) struct CellLotGeneration {
    /// Geometrically supported proposals before sequential cell claim arbitration.
    pub(crate) candidates: usize,
    /// New parcel ids, in deterministic installation order, for demand/undo invalidation.
    pub(crate) created: Vec<ParcelId>,
    /// Empty lots retired after their road, paint, asset support or reservation changed.
    pub(crate) retired: Vec<ParcelId>,
    /// Existing lots whose road reference changed while their footprint and claims stayed fixed.
    pub(crate) reattached: Vec<ParcelId>,
}

struct Proposal {
    seed: CellKey,
    frame: GridFrame,
    frontage: CellRoadFrontage,
    size: CellLotSize,
    lot: CellLot,
    geometry: ParcelGeometry,
}

impl ZoningSystem {
    /// Derives new disjoint lots around a changed painted area using compatible asset sizes.
    /// Existing lots remain fixed. At each canonical frontage address, larger footprints get
    /// first claim; other proposals remain paint when their coverage is already claimed.
    /// Source frontage and all coverage are checked before publishing any parcel.
    pub(crate) fn derive_cell_lots(
        &mut self,
        graph: &RegionGraph,
        bounds: CellBounds,
        sizes: &[CellLotSize],
        blocked: impl Fn(&[DVec2; 4], u16) -> bool + Sync,
    ) -> CellLotGeneration {
        if !bounds.is_valid() {
            return CellLotGeneration::default();
        }
        let cell_m = f64::from(self.config.zone_cell_m);
        let mut sizes: Vec<_> = sizes
            .iter()
            .copied()
            .filter(|size| {
                size.width > 0
                    && size.depth > 0
                    && usize::from(size.depth) <= CELL_DEPTH
                    && f64::from(size.width) * cell_m <= f64::from(MAX_PARCEL_FRONTAGE_M)
                    && size.profile != 0
                    && self.profiles.profile_by_runtime_id(size.profile).is_some()
            })
            .collect();
        sizes.sort_unstable();
        sizes.dedup();
        let (retired, reattached) =
            self.refresh_existing_cell_lots(graph, bounds, &sizes, &blocked);
        let Some(reach) = sizes.iter().map(|s| s.width.max(u16::from(s.depth))).max() else {
            return CellLotGeneration {
                retired,
                reattached,
                ..CellLotGeneration::default()
            };
        };
        // A changed rear cell can enable a lot whose frontage lies outside the edit rectangle.
        // The maximum supported asset extent bounds this dependency; existing lots never move.
        let mut seeds = Vec::new();
        self.cells
            .visit_reserved_in_bounds(bounds.expanded(f64::from(reach) * cell_m), |key| {
                if self.cells.profile(key).is_some_and(|p| p != 0)
                    && self.cells.lot(key) == Some(0)
                    && !self.cells.frontages(key).is_empty()
                {
                    seeds.push(key);
                }
            });
        let mut proposals: Vec<_> = seeds
            .par_iter()
            .flat_map_iter(|&seed| self.cell_lot_proposals(seed, graph, bounds, &sizes, &blocked))
            .collect();
        proposals.sort_unstable_by_key(|p| {
            (
                p.frame,
                p.seed.x,
                p.seed.y,
                p.frontage.boundary,
                Reverse(u32::from(p.size.width) * u32::from(p.size.depth)),
                Reverse(p.size.width),
                Reverse(p.size.depth),
                p.frontage.edge,
                p.frontage.side,
            )
        });
        let mut report = CellLotGeneration {
            candidates: proposals.len(),
            created: Vec::new(),
            retired,
            reattached,
        };
        // Claims depend on earlier proposals, so publication is deliberately ordered. It adds
        // only within this bounded neighborhood; there is no removal/repacking cascade.
        for proposal in proposals {
            if let Some(id) = self.install_prevalidated_cell_lot(
                proposal.lot,
                proposal.frontage.edge,
                proposal.frontage.side,
                proposal.geometry.frontage_center_t,
                proposal.size.profile,
            ) {
                report.created.push(id);
            }
        }
        report
    }

    fn refresh_existing_cell_lots(
        &mut self,
        graph: &RegionGraph,
        bounds: CellBounds,
        sizes: &[CellLotSize],
        blocked: &(impl Fn(&[DVec2; 4], u16) -> bool + Sync),
    ) -> (Vec<ParcelId>, Vec<ParcelId>) {
        let mut candidates = Vec::new();
        self.parcels.visit_in_bounds(
            Vector2::new(bounds.min.x as f32, bounds.min.y as f32),
            Vector2::new(bounds.max.x as f32, bounds.max.y as f32),
            |parcel| {
                if parcel.cell_lot().is_some() {
                    candidates.push(parcel.id());
                }
            },
        );
        let cell_m = self.config.zone_cell_m;
        let mut plans: Vec<_> = candidates
            .par_iter()
            .filter_map(|&id| {
                let parcel = self.parcels.get(id)?;
                let lot = parcel.cell_lot()?;
                let profile = parcel.zone_profile_runtime_id();
                let occupied = !parcel.is_available();
                let attachment = self
                    .cells
                    .frame(lot.origin().grid)
                    .and_then(|frame| self.cell_lot_road_geometry(lot, frame, graph, occupied));
                if occupied {
                    // Geometry refresh preserves occupied reservations; allocator maintenance
                    // owns road-policy removal and redevelopment. Use complete current frontage;
                    // losing support must not transfer them to a nearby unrelated road.
                    return attachment.map(|(_, geometry)| (id, Some(geometry)));
                }
                let supported_asset = sizes.iter().any(|size| {
                    size.profile == profile
                        && f32::from(size.width) * cell_m <= parcel.frontage_m()
                        && f32::from(size.depth) * cell_m <= parcel.depth_m()
                });
                let invalid = !supported_asset
                    || attachment.is_none()
                    || self.cells.lot_profile(lot) != Some(profile)
                    || self.cells.frame(lot.origin().grid).is_none_or(|frame| {
                        overlaps_road_corridors(graph, &lot.corners(frame))
                            || blocked(&lot.corners(frame), profile)
                    });
                Some((
                    parcel.id(),
                    attachment
                        .map(|(_, geometry)| geometry)
                        .filter(|_| !invalid),
                ))
            })
            .collect();
        plans.sort_unstable_by_key(|(id, _)| *id);
        let mut retired = Vec::new();
        let mut reattached = Vec::new();
        for (id, geometry) in plans {
            if let Some(geometry) = geometry {
                if self.parcels.get(id).is_some_and(|parcel| {
                    parcel.edge_idx() != geometry.edge_idx
                        || parcel.side() != geometry.side
                        || parcel.frontage_center_t() != geometry.frontage_center_t
                }) {
                    self.parcels.replace_geometry(id, geometry);
                    reattached.push(id);
                }
            } else if let Some(lot) = self.parcels.get(id).and_then(|parcel| parcel.cell_lot()) {
                let released = self.cells.release_lot(lot, id.raw());
                debug_assert!(released);
                self.parcels.remove_local(id);
                retired.push(id);
            }
        }
        if !reattached.is_empty() || !retired.is_empty() {
            self.bump_overlay_revision();
        }
        (retired, reattached)
    }

    fn cell_lot_proposals(
        &self,
        seed: CellKey,
        graph: &RegionGraph,
        bounds: CellBounds,
        sizes: &[CellLotSize],
        blocked: &(impl Fn(&[DVec2; 4], u16) -> bool + Sync),
    ) -> Vec<Proposal> {
        let Some(frame) = self.cells.frame(seed.grid) else {
            return Vec::new();
        };
        let Some(profile) = self.cells.profile(seed) else {
            return Vec::new();
        };
        let first = sizes.partition_point(|size| size.profile < profile);
        let end = sizes.partition_point(|size| size.profile <= profile);
        let mut proposals = Vec::new();
        let seed_frontages = self.cells.frontages(seed);
        for (index, &frontage) in seed_frontages.iter().enumerate() {
            // A boundary can have multiple interval suppliers after a split. It still
            // proposes each asset size once; the centre interval chooses its attachment.
            if seed_frontages[..index]
                .iter()
                .any(|other| other.boundary == frontage.boundary)
            {
                continue;
            }
            for &size in &sizes[first..end] {
                let Some(lot) =
                    CellLot::from_frontage(seed, size.width, size.depth, frontage.boundary)
                else {
                    continue;
                };
                if !self.cells.can_claim_lot(lot, profile) {
                    continue;
                }
                let Some((frontage, geometry)) =
                    self.cell_lot_road_geometry(lot, frame, graph, false)
                else {
                    continue;
                };
                let corners = lot.corners(frame);
                let lot_bounds = CellBounds::from_points(corners);
                if !lot_bounds.intersects(bounds) {
                    continue;
                }
                if overlaps_road_corridors(graph, &corners) || blocked(&corners, profile) {
                    continue;
                }
                proposals.push(Proposal {
                    seed,
                    frame,
                    frontage,
                    size,
                    lot,
                    geometry,
                });
            }
        }
        proposals
    }

    fn cell_lot_road_geometry(
        &self,
        lot: CellLot,
        frame: GridFrame,
        graph: &RegionGraph,
        occupied: bool,
    ) -> Option<(CellRoadFrontage, ParcelGeometry)> {
        let frontage = self.cell_lot_frontage(lot, graph, occupied)?;
        let mut geometry = lot.geometry(frame, frontage.edge, frontage.side, 0.0);
        let projection =
            parcels::project_point_to_edge(graph, frontage.edge, geometry.front_center)?;
        if projection.side != frontage.side {
            return None;
        }
        geometry.frontage_center_t = projection.s_m / projection.edge_len_m;
        Some((frontage, geometry))
    }

    fn cell_lot_frontage(
        &self,
        lot: CellLot,
        graph: &RegionGraph,
        occupied: bool,
    ) -> Option<CellRoadFrontage> {
        let eligible = |link: &&CellRoadFrontage| {
            link.boundary == lot.frontage()
                && graph.get_edge(link.edge).is_some_and(|edge| {
                    !edge.deleted
                        && (occupied || !edge.no_building_spawn)
                        && edge.primary_type == TransitType::Road
                        && edge.class == EdgeClass::Standard
                })
        };
        let mut width = 0;
        for key in lot.frontage_cells() {
            let mut end = 0;
            // Store order is by boundary then interval start. Removing an ineligible road
            // cannot leave a falsely complete cell at a split, even before cache regeneration.
            for link in self.cells.frontages(key).iter().filter(&eligible) {
                if link.start > end {
                    return None;
                }
                end = end.max(link.end);
            }
            if end != CELL_FRONTAGE_UNITS {
                return None;
            }
            width += 1;
        }
        let centre = lot.frontage_cells().nth(width / 2)?;
        let position = if width % 2 == 0 {
            0
        } else {
            CELL_FRONTAGE_UNITS / 2
        };
        self.cells
            .frontages(centre)
            .iter()
            .filter(&eligible)
            .filter(|link| link.start <= position && link.end >= position)
            .min_by_key(|link| (link.edge, link.side))
            .copied()
    }
}
