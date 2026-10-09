// SPDX-License-Identifier: GPL-2.0-only

//! Reservation views for active inbound and outbound freight jobs.

use crate::simulation::economy::definitions::ResourceRuntimeId;

use super::data::{Shipment, ShipmentEndpoint, ShipmentStatus, ShipmentSystem};

/// Flattened reservation state derived from active shipments.
pub(super) struct ReservationViews {
    resource_count: usize,
    /// Local-source reserved inventory indexed by building/resource slot.
    pub(super) reserved_outbound: Vec<f32>,
    /// Destination-side expected inventory indexed by building/resource slot.
    pub(super) reserved_inbound: Vec<f32>,
    /// Whether an inbound shipment is already open for a building/resource slot.
    pub(super) has_open_inbound: Vec<bool>,
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
        resource_runtime_id: ResourceRuntimeId,
        amount: f32,
    ) {
        if let Some(slot) = self.ensure_slot(destination_idx, resource_runtime_id) {
            self.reserved_inbound[slot] += amount;
            self.has_open_inbound[slot] = true;
        }
    }

    pub(super) fn record_owa_export(
        &mut self,
        source_idx: usize,
        resource_runtime_id: ResourceRuntimeId,
        amount: f32,
    ) {
        if let Some(slot) = self.ensure_slot(source_idx, resource_runtime_id) {
            self.reserved_outbound[slot] += amount;
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
        let slot_count = self
            .shipments
            .iter()
            .filter_map(|shipment| source_reservation_slot(shipment, resource_count))
            .max()
            .and_then(|slot| slot.checked_add(1))
            .unwrap_or(0);
        let mut reserved = vec![0.0; slot_count];
        // Preserve shipment order for deterministic floating-point reservation totals.
        for shipment in &self.shipments {
            if let Some(slot) = source_reservation_slot(shipment, resource_count) {
                reserved[slot] += shipment.amount;
            }
        }
        reserved
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
            if let Some(slot) = source_reservation_slot(shipment, resource_count)
                && slot < reserved_outbound.len()
            {
                reserved_outbound[slot] += shipment.amount;
            }
        }

        ReservationViews {
            resource_count,
            reserved_outbound,
            reserved_inbound,
            has_open_inbound,
        }
    }
}

fn source_reservation_slot(shipment: &Shipment, resource_count: usize) -> Option<usize> {
    let reserves_source =
        shipment.status == ShipmentStatus::InTransit && shipment.carrier_agent_id == usize::MAX;
    if !reserves_source {
        return None;
    }
    let ShipmentEndpoint::Building(source) = shipment.source else {
        return None;
    };
    let slot = reservation_slot(source, shipment.resource_runtime_id, resource_count)?;
    // The backing slice must also have a representable one-past-end length.
    slot.checked_add(1)?;
    Some(slot)
}

pub(super) fn reservation_slot(
    building_idx: usize,
    resource_runtime_id: ResourceRuntimeId,
    resource_count: usize,
) -> Option<usize> {
    if resource_runtime_id == 0 || usize::from(resource_runtime_id) > resource_count {
        return None;
    }
    building_idx
        .checked_mul(resource_count)
        .and_then(|base| base.checked_add(resource_runtime_id as usize - 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::economy::logistics::CarrierClass;

    fn reservation_shipment(
        source: usize,
        destination: usize,
        resource: u16,
        amount: f32,
    ) -> Shipment {
        Shipment {
            id: 1,
            resource_runtime_id: resource,
            amount,
            source: ShipmentEndpoint::Building(source),
            destination: ShipmentEndpoint::Building(destination),
            carrier_class: CarrierClass::Truck,
            status: ShipmentStatus::InTransit,
            carrier_agent_id: usize::MAX,
            total_cost: 0.0,
            freight_cost: 0.0,
            eta_hours: 1,
            queued_hours: 0,
        }
    }

    #[test]
    fn unknown_resource_cannot_alias_another_buildings_reservation() {
        let mut shipments = ShipmentSystem::new();
        shipments.shipments = vec![
            reservation_shipment(1, 2, 1, 2.0),
            reservation_shipment(0, 0, 3, 100.0),
        ];
        let all = shipments.build_reservation_views(2);
        assert_eq!(all.reserved_outbound_amount(1, 1), 2.0);
        assert_eq!(all.reserved_inbound_amount(1, 1), 0.0);
        assert!(!all.has_open_inbound(1, 1));
        let outbound = shipments.reserved_outbound_view(2);
        assert_eq!(outbound[2], 2.0);
    }

    #[test]
    fn outbound_projection_matches_full_reservations_across_cargo_states() {
        let mut shipments = ShipmentSystem::new();
        for (idx, (status, carrier)) in [
            (ShipmentStatus::InTransit, usize::MAX),
            (ShipmentStatus::InTransit, 3),
            (ShipmentStatus::Returning, 4),
            (ShipmentStatus::Fulfilled, usize::MAX),
        ]
        .into_iter()
        .enumerate()
        {
            let mut shipment = reservation_shipment(idx, 100, 2, 12.0);
            shipment.status = status;
            shipment.carrier_agent_id = carrier;
            shipments.shipments.push(shipment);
        }
        let full = shipments.build_reservation_views(2);
        let outbound = shipments.reserved_outbound_view(2);
        for building in 0..=100 {
            let slot = reservation_slot(building, 2, 2).unwrap();
            let expected = if building == 0 { 12.0 } else { 0.0 };
            assert_eq!(full.reserved_outbound_amount(building, 2), expected);
            assert_eq!(outbound.get(slot).copied().unwrap_or(0.0), expected);
        }
    }

    #[test]
    #[ignore = "manual matched release timing of source-only freight reservations"]
    fn benchmark_outbound_reservations() {
        use std::{hint::black_box, time::Instant};
        let mut shipments = ShipmentSystem::new();
        shipments.shipments = (0..4_096)
            .map(|idx| reservation_shipment(idx % 256, 65_536 + idx, 1 + (idx % 9) as u16, 12.0))
            .collect();
        let mut samples = [0.0; 11];
        for sample in &mut samples {
            let start = Instant::now();
            for _ in 0..100 {
                black_box(shipments.reserved_outbound_view(black_box(9)));
            }
            *sample = start.elapsed().as_secs_f64() * 1_000.0 / 100.0;
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "outbound_reservations orders=4096 sources=256 high_destination=69631 median_ms={:.3}",
            samples[5]
        );
    }
}
