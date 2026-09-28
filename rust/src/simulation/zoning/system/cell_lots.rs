// SPDX-License-Identifier: GPL-2.0-only

//! Atomic cell claims shared with the existing parcel allocator and redevelopment lifecycle.

mod derivation;
mod dirty;
pub(crate) use derivation::CellLotGeneration;

use super::ZoningSystem;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::{EdgeClass, TransitType};
use crate::simulation::zoning::cells::{CellLot, overlaps_road_corridors};
use crate::simulation::zoning::{ParcelId, ParcelPlacementError, ZoningParcel, parcels};

/// Local lot state needed to undo demolition without losing erased coverage or its generation.
pub(crate) struct CellLotRemovalUndo {
    parcel: ZoningParcel,
    expected_cell_revision: u64,
}

impl ZoningSystem {
    /// Captures only the cell-derived lot owned by the building being removed.
    pub(crate) fn capture_cell_lot_removal_undo(
        &self,
        parcel_id: u64,
    ) -> Option<CellLotRemovalUndo> {
        let parcel = self.parcel_by_raw_id(parcel_id)?;
        parcel.cell_lot()?;
        Some(CellLotRemovalUndo {
            parcel: parcel.clone(),
            expected_cell_revision: self.cells.revision(),
        })
    }

    /// Seals the local inverse after demolition has released its cell reservations.
    pub(crate) fn seal_cell_lot_removal_undo(&self, undo: &mut CellLotRemovalUndo) {
        undo.expected_cell_revision = self.cells.revision();
    }

    /// Rejects restoration across paint edits, replacement claims or a new overlapping parcel.
    pub(crate) fn can_restore_cell_lot_removal_undo(&self, undo: &CellLotRemovalUndo) -> bool {
        if undo.expected_cell_revision != self.cells.revision() {
            return false;
        }
        let before = &undo.parcel;
        let Some(lot) = before.cell_lot() else {
            return false;
        };
        if let Some(current) = self.parcels.get(before.id()) {
            current.is_available()
                && current.cell_lot() == Some(lot)
                && current.zone_profile_runtime_id() == before.zone_profile_runtime_id()
                && current.corners() == before.corners()
                && current.build_generation() == before.build_generation().wrapping_add(1)
                && lot
                    .cells()
                    .all(|key| self.cells.lot(key) == Some(before.id().raw()))
        } else {
            self.cells.can_restore_lot(lot)
                && !self
                    .parcels
                    .overlaps_existing(&parcels::geometry_for_parcel(before))
        }
    }

    /// Restores the parcel/coverage before the building lifecycle restores its occupant index.
    pub(crate) fn restore_cell_lot_removal_undo(&mut self, undo: CellLotRemovalUndo) {
        debug_assert!(self.can_restore_cell_lot_removal_undo(&undo));
        let mut before = undo.parcel;
        if let Some(parcel) = self.parcels.get_mut(before.id()) {
            parcel.set_build_generation(before.build_generation());
        } else if let Some(lot) = before.cell_lot() {
            before.set_occupied_building(None);
            let restored = self.cells.restore_lot(lot, before.id().raw());
            debug_assert!(restored);
            let restored = self.parcels.restore_local(before);
            debug_assert!(restored);
        }
        self.bump_overlay_revision();
    }

    /// Restores a cell lot's exact footprint and claim, including mixed paint during grace.
    pub(crate) fn restore_saved_cell_lot(
        &mut self,
        id: ParcelId,
        lot: CellLot,
        edge_idx: usize,
        side: i8,
        frontage_t: f32,
        frontage_m: f32,
        depth_m: f32,
        profile: u16,
        graph: &RegionGraph,
    ) -> Result<(), ParcelPlacementError> {
        self.validate_profile_id(profile)?;
        if id.is_none()
            || self.parcels.get(id).is_some()
            || !self.cells.can_restore_lot(lot)
            || self.cells.lot_profile(lot).unwrap_or(0) != profile
        {
            return Err(ParcelPlacementError::OverlapsExistingParcel);
        }
        if edge_idx >= graph.edge_count()
            || !matches!(side, -1 | 1)
            || !frontage_t.is_finite()
            || !(0.0..=1.0).contains(&frontage_t)
        {
            return Err(ParcelPlacementError::NoRoadAttachment);
        }
        let edge = graph.edge(edge_idx);
        // Growth eligibility may change while occupied lots retain their saved claims.
        // The allocator and derivation recheck no-build policy before any new construction.
        if edge.deleted
            || edge.primary_type != TransitType::Road
            || edge.class != EdgeClass::Standard
            || edge.physical_geometry.len() < 2
            || edge.physical_length <= 0.0
        {
            return Err(ParcelPlacementError::NoRoadAttachment);
        }
        let frame = self
            .cells
            .frame(lot.origin().grid)
            .ok_or(ParcelPlacementError::NoRoadAttachment)?;
        let geometry = lot.geometry(frame, edge_idx, side, frontage_t);
        if geometry.frontage_m != frontage_m || geometry.depth_m != depth_m {
            return Err(ParcelPlacementError::InvalidDimensions);
        }
        if !parcels::geometry_inside_world(&geometry, self.config.width_m, self.config.height_m) {
            return Err(ParcelPlacementError::OutsideWorld);
        }
        if self.parcels.overlaps_existing(&geometry) {
            return Err(ParcelPlacementError::OverlapsExistingParcel);
        }
        if overlaps_road_corridors(graph, &lot.corners(frame)) {
            return Err(ParcelPlacementError::OverlapsRoad);
        }
        self.parcels.insert_loaded(id, geometry, profile);
        if let Some(parcel) = self.parcels.get_mut(id) {
            parcel.set_cell_lot(lot);
        }
        let restored = self.cells.restore_lot(lot, id.raw());
        debug_assert!(restored);
        self.bump_overlay_revision();
        Ok(())
    }

    /// Installs a derived lot after the caller has validated road access and external sites.
    /// Paint, ownership, world bounds and parcel conflicts are rechecked here before any write.
    /// The rectangle is always reconstructed from its grid, independently of road tangents.
    pub(crate) fn install_prevalidated_cell_lot(
        &mut self,
        lot: CellLot,
        edge_idx: usize,
        side: i8,
        frontage_center_t: f32,
        profile: u16,
    ) -> Option<ParcelId> {
        if self.validate_profile_id(profile).is_err()
            || !self.cells.can_claim_lot(lot, profile)
            || !matches!(side, -1 | 1)
            || !frontage_center_t.is_finite()
            || !(0.0..=1.0).contains(&frontage_center_t)
        {
            return None;
        }
        let frame = self.cells.frame(lot.origin().grid)?;
        let geometry = lot.geometry(frame, edge_idx, side, frontage_center_t);
        if !parcels::geometry_inside_world(&geometry, self.config.width_m, self.config.height_m)
            || self.parcels.overlaps_existing(&geometry)
        {
            return None;
        }
        let id = self.parcels.insert_new(geometry, profile);
        if !self.cells.claim_lot(lot, id.raw(), profile) {
            self.parcels.remove_local(id);
            return None;
        }
        if let Some(parcel) = self.parcels.get_mut(id) {
            parcel.set_cell_lot(lot);
        }
        self.bump_overlay_revision();
        Some(id)
    }

    /// Releases incompatible paint and queues road revalidation after the occupant is cleared.
    pub(super) fn release_invalid_empty_cell_lot(&mut self, id: ParcelId) {
        let Some(parcel) = self.parcels.get(id) else {
            return;
        };
        let Some(lot) = parcel.cell_lot() else {
            return;
        };
        if !parcel.is_available() {
            return;
        }
        // A road may have disappeared while the occupied coverage was pinned. Even intact
        // paint needs a local frontage refresh once the building releases its footprint.
        if let Some(frame) = self.cells.frame(lot.origin().grid) {
            self.mark_cell_lots_dirty(crate::simulation::zoning::cells::CellBounds::from_points(
                lot.corners(frame),
            ));
        }
        if self.cells.lot_profile(lot).is_none() && self.cells.release_lot(lot, id.raw()) {
            self.parcels.remove_local(id);
            self.bump_overlay_revision();
        }
    }
}
