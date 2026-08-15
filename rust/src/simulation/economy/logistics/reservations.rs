//! Reservation views for active inbound, outbound, and border freight jobs.

use std::collections::HashMap;

use crate::simulation::economy::definitions::ResourceRuntimeId;

use super::data::{ShipmentEndpoint, ShipmentStatus, ShipmentSystem};

/// Flattened reservation state derived from active shipments.
pub(super) struct ReservationViews {
    resource_count: usize,
    /// Local-source reserved inventory indexed by building/resource slot.
    pub(super) reserved_outbound: Vec<f32>,
    /// Destination-side expected inventory indexed by building/resource slot.
    pub(super) reserved_inbound: Vec<f32>,
    /// Whether an inbound shipment is already open for a building/resource slot.
    pub(super) has_open_inbound: Vec<bool>,
    /// Active dispatched `OWA` job counts indexed by border node id.
    pub(super) border_active_job_counts: HashMap<u32, usize>,
    /// Queued `OWA` job counts indexed by border node id.
    pub(super) border_queued_job_counts: HashMap<u32, usize>,
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

    pub(super) fn border_active_job_count(&self, border_node: u32) -> usize {
        self.border_active_job_counts
            .get(&border_node)
            .copied()
            .unwrap_or(0)
    }

    pub(super) fn border_queued_job_count(&self, border_node: u32) -> usize {
        self.border_queued_job_counts
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
        status: ShipmentStatus,
    ) {
        if let Some(slot) = self.ensure_slot(destination_idx, resource_runtime_id) {
            self.reserved_inbound[slot] += amount;
            self.has_open_inbound[slot] = true;
        }
        self.record_border_job(border_node, status);
    }

    pub(super) fn record_owa_export(
        &mut self,
        source_idx: usize,
        border_node: u32,
        resource_runtime_id: ResourceRuntimeId,
        amount: f32,
        status: ShipmentStatus,
    ) {
        if let Some(slot) = self.ensure_slot(source_idx, resource_runtime_id) {
            self.reserved_outbound[slot] += amount;
        }
        self.record_border_job(border_node, status);
    }

    fn record_border_job(&mut self, border_node: u32, status: ShipmentStatus) {
        match status {
            ShipmentStatus::Queued => {
                *self
                    .border_queued_job_counts
                    .entry(border_node)
                    .or_insert(0) += 1;
            }
            ShipmentStatus::InTransit => {
                *self
                    .border_active_job_counts
                    .entry(border_node)
                    .or_insert(0) += 1;
            }
            _ => {}
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
            if let ShipmentEndpoint::Building(building_id) = shipment.destination {
                max_building = max_building.max(building_id);
            }
            if let ShipmentEndpoint::Building(building_id) = shipment.source {
                max_building = max_building.max(building_id);
            }
        }

        let slot_count = max_building
            .saturating_add(1)
            .saturating_mul(resource_count.max(1));
        let mut reserved_outbound = vec![0.0; slot_count];
        let mut reserved_inbound = vec![0.0; slot_count];
        let mut has_open_inbound = vec![false; slot_count];
        let mut border_active_job_counts = HashMap::new();
        let mut border_queued_job_counts = HashMap::new();

        for shipment in &self.shipments {
            if shipment.status.reserves_cargo() {
                if let ShipmentEndpoint::Building(destination_building_id) = shipment.destination
                    && let Some(slot) = reservation_slot(
                        destination_building_id,
                        shipment.resource_runtime_id,
                        resource_count,
                    )
                    && slot < reserved_inbound.len()
                {
                    reserved_inbound[slot] += shipment.amount;
                    has_open_inbound[slot] = true;
                }
            }
            if shipment_reserves_source_inventory(shipment) {
                if let ShipmentEndpoint::Building(source_building_id) = shipment.source
                    && let Some(slot) = reservation_slot(
                        source_building_id,
                        shipment.resource_runtime_id,
                        resource_count,
                    )
                    && slot < reserved_outbound.len()
                {
                    reserved_outbound[slot] += shipment.amount;
                }
            }
            if let Some(border_node) = shipment
                .source
                .border_node()
                .or_else(|| shipment.destination.border_node())
            {
                match shipment.status {
                    ShipmentStatus::Queued => {
                        *border_queued_job_counts.entry(border_node).or_insert(0) += 1;
                    }
                    ShipmentStatus::InTransit => {
                        *border_active_job_counts.entry(border_node).or_insert(0) += 1;
                    }
                    _ => {}
                }
            }
        }

        ReservationViews {
            resource_count,
            reserved_outbound,
            reserved_inbound,
            has_open_inbound,
            border_active_job_counts,
            border_queued_job_counts,
        }
    }
}

fn shipment_reserves_source_inventory(shipment: &super::data::Shipment) -> bool {
    matches!(shipment.status, ShipmentStatus::Queued)
        || (shipment.status == ShipmentStatus::InTransit && shipment.carrier_agent_id == usize::MAX)
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
