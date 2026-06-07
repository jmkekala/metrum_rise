//! Reservation views for active inbound, outbound, and border freight jobs.

use std::collections::HashMap;

use crate::simulation::economy::definitions::ResourceRuntimeId;

use super::data::{
    SHIPMENT_DEST_OWA, SHIPMENT_IN_TRANSIT, SHIPMENT_SOURCE_LOCAL, SHIPMENT_SOURCE_OWA,
    ShipmentSystem,
};

/// Flattened reservation state derived from active shipments.
pub(super) struct ReservationViews {
    resource_count: usize,
    /// Local-source reserved inventory indexed by building/resource slot.
    pub(super) reserved_outbound: Vec<f32>,
    /// Destination-side expected inventory indexed by building/resource slot.
    pub(super) reserved_inbound: Vec<f32>,
    /// Whether an inbound shipment is already open for a building/resource slot.
    pub(super) has_open_inbound: Vec<bool>,
    /// Active `OWA` import job counts indexed by border node id.
    pub(super) border_job_counts: HashMap<u32, usize>,
}

impl ReservationViews {
    pub(super) fn reserved_outbound_amount(
        &self,
        building_idx: usize,
        resource_runtime_id: ResourceRuntimeId,
    ) -> f32 {
        self.slot(building_idx, resource_runtime_id)
            .and_then(|slot| self.reserved_outbound.get(slot).copied())
            .unwrap_or(0.0)
    }

    pub(super) fn reserved_inbound_amount(
        &self,
        building_idx: usize,
        resource_runtime_id: ResourceRuntimeId,
    ) -> f32 {
        self.slot(building_idx, resource_runtime_id)
            .and_then(|slot| self.reserved_inbound.get(slot).copied())
            .unwrap_or(0.0)
    }

    pub(super) fn has_open_inbound(
        &self,
        building_idx: usize,
        resource_runtime_id: ResourceRuntimeId,
    ) -> bool {
        self.slot(building_idx, resource_runtime_id)
            .and_then(|slot| self.has_open_inbound.get(slot).copied())
            .unwrap_or(false)
    }

    pub(super) fn border_job_count(&self, border_node: u32) -> usize {
        self.border_job_counts
            .get(&border_node)
            .copied()
            .unwrap_or(0)
    }

    pub(super) fn record_local_shipment(
        &mut self,
        source_idx: usize,
        destination_idx: usize,
        resource_runtime_id: ResourceRuntimeId,
        amount: f32,
    ) {
        if let Some(slot) = self.ensure_slot(source_idx, resource_runtime_id) {
            self.reserved_outbound[slot] += amount;
        }
        if let Some(slot) = self.ensure_slot(destination_idx, resource_runtime_id) {
            self.reserved_inbound[slot] += amount;
            self.has_open_inbound[slot] = true;
        }
    }

    pub(super) fn record_owa_import(
        &mut self,
        destination_idx: usize,
        border_node: u32,
        resource_runtime_id: ResourceRuntimeId,
        amount: f32,
    ) {
        if let Some(slot) = self.ensure_slot(destination_idx, resource_runtime_id) {
            self.reserved_inbound[slot] += amount;
            self.has_open_inbound[slot] = true;
        }
        self.record_border_job(border_node);
    }

    pub(super) fn record_owa_export(
        &mut self,
        source_idx: usize,
        border_node: u32,
        resource_runtime_id: ResourceRuntimeId,
        amount: f32,
    ) {
        if let Some(slot) = self.ensure_slot(source_idx, resource_runtime_id) {
            self.reserved_outbound[slot] += amount;
        }
        self.record_border_job(border_node);
    }

    fn record_border_job(&mut self, border_node: u32) {
        if border_node != u32::MAX {
            *self.border_job_counts.entry(border_node).or_insert(0) += 1;
        }
    }

    fn slot(&self, building_idx: usize, resource_runtime_id: ResourceRuntimeId) -> Option<usize> {
        reservation_slot(building_idx, resource_runtime_id, self.resource_count)
    }

    fn ensure_slot(
        &mut self,
        building_idx: usize,
        resource_runtime_id: ResourceRuntimeId,
    ) -> Option<usize> {
        let slot = self.slot(building_idx, resource_runtime_id)?;
        if slot >= self.reserved_outbound.len() {
            let new_len = slot.saturating_add(1);
            self.reserved_outbound.resize(new_len, 0.0);
            self.reserved_inbound.resize(new_len, 0.0);
            self.has_open_inbound.resize(new_len, false);
        }
        Some(slot)
    }
}

impl ShipmentSystem {
    /// Returns the current local-source outbound reservation slots indexed by building/resource.
    pub(crate) fn reserved_outbound_view(&self, resource_count: usize) -> Vec<f32> {
        self.build_reservation_views(resource_count)
            .reserved_outbound
    }

    /// Computes the flattened reservation slot for a building/resource pair.
    pub(crate) fn reservation_slot_for_building(
        building_idx: usize,
        resource_runtime_id: ResourceRuntimeId,
        resource_count: usize,
    ) -> Option<usize> {
        reservation_slot(building_idx, resource_runtime_id, resource_count)
    }

    pub(super) fn build_reservation_views(&self, resource_count: usize) -> ReservationViews {
        let mut max_building = 0usize;
        for shipment in &self.shipments {
            // Skip the OWA export sentinel; it is not a real building index.
            if shipment.destination_building_id != SHIPMENT_DEST_OWA {
                max_building = max_building.max(shipment.destination_building_id);
            }
            if shipment.source_kind == SHIPMENT_SOURCE_LOCAL {
                max_building = max_building.max(shipment.source_building_id);
            }
        }

        let slot_count = max_building
            .saturating_add(1)
            .saturating_mul(resource_count.max(1));
        let mut reserved_outbound = vec![0.0; slot_count];
        let mut reserved_inbound = vec![0.0; slot_count];
        let mut has_open_inbound = vec![false; slot_count];
        let mut border_job_counts = HashMap::new();

        for shipment in &self.shipments {
            if shipment.status != SHIPMENT_IN_TRANSIT {
                continue;
            }
            if shipment.destination_building_id != SHIPMENT_DEST_OWA
                && let Some(slot) = reservation_slot(
                    shipment.destination_building_id,
                    shipment.resource_runtime_id,
                    resource_count,
                )
                && slot < reserved_inbound.len()
            {
                reserved_inbound[slot] += shipment.amount;
                has_open_inbound[slot] = true;
            }
            if shipment.source_kind == SHIPMENT_SOURCE_LOCAL
                && let Some(slot) = reservation_slot(
                    shipment.source_building_id,
                    shipment.resource_runtime_id,
                    resource_count,
                )
                && slot < reserved_outbound.len()
            {
                reserved_outbound[slot] += shipment.amount;
            }
            if shipment.source_kind == SHIPMENT_SOURCE_OWA
                || shipment.destination_building_id == SHIPMENT_DEST_OWA
            {
                if shipment.source_border_node != u32::MAX {
                    *border_job_counts
                        .entry(shipment.source_border_node)
                        .or_insert(0) += 1;
                }
            }
        }

        ReservationViews {
            resource_count,
            reserved_outbound,
            reserved_inbound,
            has_open_inbound,
            border_job_counts,
        }
    }
}

pub(super) fn reservation_slot(
    building_idx: usize,
    resource_runtime_id: ResourceRuntimeId,
    resource_count: usize,
) -> Option<usize> {
    if resource_runtime_id == 0 || resource_count == 0 {
        return None;
    }
    building_idx
        .checked_mul(resource_count)
        .and_then(|base| base.checked_add(resource_runtime_id as usize - 1))
}
