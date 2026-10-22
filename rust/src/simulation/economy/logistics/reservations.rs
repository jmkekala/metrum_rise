// SPDX-License-Identifier: GPL-2.0-only

//! Reservation views for active inbound and outbound freight jobs.
//!
//! The views are maintained state (`SIM-02-I`): each change of a shipment's reserving state
//! moves its amount in O(1), so no pass rebuilds them from every shipment. Each slot sums
//! integer units of 2^-30 kg, each amount converted once, so adds and removes in any order give
//! the same sum as a fresh build, which a load makes. Test builds compare the views with a fresh
//! build at the start of every logistics hour and at the end of each test driver's hour.

use crate::simulation::economy::definitions::ResourceRuntimeId;

use super::data::{Shipment, ShipmentEndpoint, ShipmentStatus, ShipmentSystem};

// Reservation units per kilogram, 2^30. Every `f32` of at least 2^-7 kg is a whole number of
// units, so a shipment amount converts exactly (batches are tens of kilograms); a smaller one
// rounds to the nearest unit, the same way every time. A sum of exact amounts converts back
// exactly whenever its `f32` would have been exact, as for batch multiples below 2^24 kg.
const RESERVATION_UNITS_PER_KG: f64 = (1u64 << 30) as f64;

// `amount` in whole reservation units. Products by a power of two are exact in `f64`.
fn reservation_units(amount: f32) -> u64 {
    (f64::from(amount) * RESERVATION_UNITS_PER_KG).round() as u64
}

// The `f32` nearest to `units` reservation units.
fn reservation_kg(units: u64) -> f32 {
    (units as f64 / RESERVATION_UNITS_PER_KG) as f32
}

/// Flattened reservation state of the open shipments, kept in step as they change.
#[derive(Clone, Debug)]
pub(super) struct ReservationViews {
    resource_count: usize,
    // Local-source reserved inventory indexed by building/resource slot, in reservation units.
    reserved_outbound: Vec<u64>,
    // Destination-side expected inventory indexed by building/resource slot, in reservation units.
    reserved_inbound: Vec<u64>,
    // Reservations summed in each slot.
    outbound_open: Vec<u32>,
    inbound_open: Vec<u32>,
}

impl ReservationViews {
    /// Views of `shipments`, summed in shipment order. O(shipments + reserved slots).
    pub(super) fn from_shipments(shipments: &[Shipment], resource_count: usize) -> Self {
        let mut max_building = 0usize;
        for shipment in shipments {
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
        let mut views = Self {
            resource_count,
            reserved_outbound: vec![0; slot_count],
            reserved_inbound: vec![0; slot_count],
            outbound_open: vec![0; slot_count],
            inbound_open: vec![0; slot_count],
        };
        for shipment in shipments {
            views.add_shipment(shipment);
        }
        views
    }

    pub(super) fn reserved_outbound_amount(
        &self,
        building_idx: usize,
        resource_runtime_id: ResourceRuntimeId,
    ) -> f32 {
        self.slot(building_idx, resource_runtime_id)
            .and_then(|slot| self.reserved_outbound.get(slot).copied())
            .map_or(0.0, reservation_kg)
    }

    pub(super) fn reserved_inbound_amount(
        &self,
        building_idx: usize,
        resource_runtime_id: ResourceRuntimeId,
    ) -> f32 {
        self.slot(building_idx, resource_runtime_id)
            .and_then(|slot| self.reserved_inbound.get(slot).copied())
            .map_or(0.0, reservation_kg)
    }

    pub(super) fn has_open_inbound(
        &self,
        building_idx: usize,
        resource_runtime_id: ResourceRuntimeId,
    ) -> bool {
        self.slot(building_idx, resource_runtime_id)
            .and_then(|slot| self.inbound_open.get(slot).copied())
            .is_some_and(|open| open > 0)
    }

    pub(super) fn record_local_shipment(
        &mut self,
        source_idx: usize,
        destination_idx: usize,
        resource_runtime_id: ResourceRuntimeId,
        amount: f32,
    ) {
        let units = reservation_units(amount);
        self.reserve(Side::Outbound, source_idx, resource_runtime_id, units);
        self.reserve(Side::Inbound, destination_idx, resource_runtime_id, units);
    }

    pub(super) fn record_owa_import(
        &mut self,
        destination_idx: usize,
        resource_runtime_id: ResourceRuntimeId,
        amount: f32,
    ) {
        let units = reservation_units(amount);
        self.reserve(Side::Inbound, destination_idx, resource_runtime_id, units);
    }

    pub(super) fn record_owa_export(
        &mut self,
        source_idx: usize,
        resource_runtime_id: ResourceRuntimeId,
        amount: f32,
    ) {
        let units = reservation_units(amount);
        self.reserve(Side::Outbound, source_idx, resource_runtime_id, units);
    }

    pub(super) fn add_shipment(&mut self, shipment: &Shipment) {
        let (outbound, inbound) = self.shipment_slots(shipment);
        self.apply(outbound, inbound, shipment.amount, true);
    }

    pub(super) fn remove_shipment(&mut self, shipment: &Shipment) {
        let (outbound, inbound) = self.shipment_slots(shipment);
        self.apply(outbound, inbound, shipment.amount, false);
    }

    /// Moves every reservation of building `from` onto building `to` after a remap. O(resources).
    fn move_building(&mut self, from: usize, to: usize) {
        for resource in 1..=self.resource_count {
            let resource = resource as ResourceRuntimeId;
            let Some(from_slot) = self.slot(from, resource) else {
                continue;
            };
            for side in [Side::Outbound, Side::Inbound] {
                let (amounts, open) = self.side(side);
                let (units, count) = match (amounts.get(from_slot), open.get(from_slot)) {
                    (Some(&units), Some(&count)) if count > 0 => (units, count),
                    _ => continue,
                };
                amounts[from_slot] = 0;
                open[from_slot] = 0;
                let Some(to_slot) = self.ensure_slot(to, resource) else {
                    continue;
                };
                let (amounts, open) = self.side(side);
                amounts[to_slot] += units;
                open[to_slot] += count;
            }
        }
    }

    // Outbound slot while the cargo still sits at a source building, inbound slot while it is on
    // its way to a destination building.
    fn shipment_slots(&self, shipment: &Shipment) -> (Option<usize>, Option<usize>) {
        let outbound = source_reservation_slot(shipment, self.resource_count);
        let inbound = match shipment.destination {
            ShipmentEndpoint::Building(destination) if shipment.status.reserves_cargo() => {
                self.slot(destination, shipment.resource_runtime_id)
            }
            _ => None,
        };
        (outbound, inbound)
    }

    fn apply(&mut self, outbound: Option<usize>, inbound: Option<usize>, amount: f32, add: bool) {
        let units = reservation_units(amount);
        for (side, slot) in [(Side::Outbound, outbound), (Side::Inbound, inbound)] {
            let Some(slot) = slot else {
                continue;
            };
            if add {
                self.reserve_slot(side, slot, units);
            } else {
                self.release_slot(side, slot, units);
            }
        }
    }

    fn reserve(
        &mut self,
        side: Side,
        building_idx: usize,
        resource_runtime_id: ResourceRuntimeId,
        units: u64,
    ) {
        if let Some(slot) = self.slot(building_idx, resource_runtime_id) {
            self.reserve_slot(side, slot, units);
        }
    }

    fn reserve_slot(&mut self, side: Side, slot: usize, units: u64) {
        self.grow_to(slot);
        let (amounts, open) = self.side(side);
        amounts[slot] += units;
        open[slot] += 1;
    }

    // Integer sums make the release exact: the slot returns to what it held without this
    // reservation, in any order.
    fn release_slot(&mut self, side: Side, slot: usize, units: u64) {
        let (amounts, open) = self.side(side);
        debug_assert!(open.get(slot).is_some_and(|&count| count > 0));
        debug_assert!(amounts.get(slot).is_some_and(|&held| held >= units));
        let Some(count) = open.get_mut(slot).filter(|count| **count > 0) else {
            return;
        };
        *count -= 1;
        amounts[slot] = amounts[slot].saturating_sub(units);
    }

    fn side(&mut self, side: Side) -> (&mut Vec<u64>, &mut Vec<u32>) {
        match side {
            Side::Outbound => (&mut self.reserved_outbound, &mut self.outbound_open),
            Side::Inbound => (&mut self.reserved_inbound, &mut self.inbound_open),
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
        self.grow_to(slot);
        Some(slot)
    }

    fn grow_to(&mut self, slot: usize) {
        if slot >= self.reserved_outbound.len() {
            let new_len = slot.saturating_add(1);
            self.reserved_outbound.resize(new_len, 0);
            self.reserved_inbound.resize(new_len, 0);
            self.outbound_open.resize(new_len, 0);
            self.inbound_open.resize(new_len, 0);
        }
    }

    // Same unit sums and counts in every slot, a missing slot counting as empty.
    #[cfg(test)]
    pub(super) fn matches(&self, other: &Self) -> bool {
        let len = self
            .reserved_outbound
            .len()
            .max(other.reserved_outbound.len());
        let slot = |views: &Self, slot: usize| {
            (
                views.reserved_outbound.get(slot).copied().unwrap_or(0),
                views.reserved_inbound.get(slot).copied().unwrap_or(0),
                views.outbound_open.get(slot).copied().unwrap_or(0),
                views.inbound_open.get(slot).copied().unwrap_or(0),
            )
        };
        self.resource_count == other.resource_count
            && (0..len).all(|index| slot(self, index) == slot(other, index))
    }
}

#[derive(Clone, Copy)]
enum Side {
    Outbound,
    Inbound,
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

    /// Fresh views of every shipment; the maintained views must always equal them.
    #[cfg(test)]
    pub(super) fn build_reservation_views(&self, resource_count: usize) -> ReservationViews {
        ReservationViews::from_shipments(&self.shipments, resource_count)
    }

    /// Moves the maintained views out for a planning pass, rebuilding them from every shipment
    /// when a change left them unknown (loading, clearing, an undo remap). O(1), or
    /// O(shipments + slots) after such a change.
    pub(super) fn take_reservation_views(&mut self, resource_count: usize) -> ReservationViews {
        match self.reservations.take() {
            Some(views) if views.resource_count == resource_count => views,
            _ => ReservationViews::from_shipments(&self.shipments, resource_count),
        }
    }

    /// Applies `change` to shipment `idx` and moves its reservations to match its new state.
    /// O(1).
    pub(super) fn update_shipment(&mut self, idx: usize, change: impl FnOnce(&mut Shipment)) {
        let shipment = &mut self.shipments[idx];
        let Some(views) = &mut self.reservations else {
            change(shipment);
            return;
        };
        let before = views.shipment_slots(shipment);
        change(shipment);
        let after = views.shipment_slots(shipment);
        let amount = shipment.amount;
        let released = |slot: Option<usize>, kept: Option<usize>| slot.filter(|_| slot != kept);
        views.apply(
            released(before.0, after.0),
            released(before.1, after.1),
            amount,
            false,
        );
        views.apply(
            released(after.0, before.0),
            released(after.1, before.1),
            amount,
            true,
        );
    }

    /// Drops removed shipment `shipment`'s reservations. O(1).
    pub(super) fn forget_shipment_reservations(
        views: &mut Option<ReservationViews>,
        shipment: &Shipment,
    ) {
        if let Some(views) = views {
            views.remove_shipment(shipment);
        }
    }

    /// Follows a building remap: a set of moves onto buildings that are not moved themselves
    /// moves their reservations in O(moves × resources); any other remap rebuilds them later.
    pub(super) fn remap_reservations(&mut self, mapping: &std::collections::HashMap<usize, usize>) {
        if mapping.values().any(|to| mapping.contains_key(to)) {
            self.reservations = None;
            return;
        }
        let Some(views) = &mut self.reservations else {
            return;
        };
        // No target is moved itself, so the moves touch disjoint rows in any order.
        for (&from, &to) in mapping {
            views.move_building(from, to);
        }
    }

    /// Panics when the maintained views differ from a fresh build of the shipments.
    #[cfg(test)]
    pub(super) fn assert_reservations_current(&self) {
        if let Some(views) = &self.reservations {
            assert!(
                views.matches(&self.build_reservation_views(views.resource_count)),
                "maintained freight reservations differ from the shipments"
            );
        }
    }
}

// The views of `views`, rebuilt from `shipments` when unknown or keyed to another catalog.
pub(super) fn current_views<'a>(
    views: &'a mut Option<ReservationViews>,
    shipments: &[Shipment],
    resource_count: usize,
) -> &'a mut ReservationViews {
    if views
        .as_ref()
        .is_some_and(|views| views.resource_count != resource_count)
    {
        *views = None;
    }
    views.get_or_insert_with(|| ReservationViews::from_shipments(shipments, resource_count))
}
fn source_reservation_slot(shipment: &Shipment, resource_count: usize) -> Option<usize> {
    let reserves_source = shipment.status == ShipmentStatus::InTransit && !shipment.cargo_loaded;
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
            carrier: crate::simulation::economy::agents::AgentHandle::NONE,
            total_cost: 0.0,
            freight_cost: 0.0,
            eta_hours: 1,
            leg_started_hour: 0,
            trip_event_slot: 0,
            cargo_loaded: false,
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
            (
                ShipmentStatus::InTransit,
                crate::simulation::economy::agents::AgentHandle::NONE,
            ),
            (
                ShipmentStatus::InTransit,
                crate::simulation::economy::agents::AgentHandle::from_parts(3, 0),
            ),
            (
                ShipmentStatus::Returning,
                crate::simulation::economy::agents::AgentHandle::from_parts(4, 0),
            ),
            (
                ShipmentStatus::Fulfilled,
                crate::simulation::economy::agents::AgentHandle::NONE,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let mut shipment = reservation_shipment(idx, 100, 2, 12.0);
            shipment.status = status;
            shipment.cargo_loaded = !carrier.is_none();
            shipment.carrier = carrier;
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
    fn maintained_views_equal_a_fresh_build_through_every_shipment_change() {
        use crate::simulation::buildings::allocator::BuildingAllocator;
        use crate::simulation::economy::agents::AgentSystem;
        use ShipmentEndpoint::{Building, OwaBorder};
        let mut shipments = ShipmentSystem::new();
        // A planning pass moves the views out and records each shipment it creates.
        let mut views = shipments.take_reservation_views(3);
        for (source, destination, resource, amount) in [
            (Building(0), Building(4), 2, 120.0),
            (Building(0), Building(5), 2, 40.0),
            (OwaBorder(9), Building(4), 3, 50.0),
            (Building(5), OwaBorder(9), 1, 100.0),
            (Building(6), Building(4), 1, 60.0),
            (Building(7), Building(4), 1, 80.0),
        ] {
            let mut shipment = reservation_shipment(0, 0, resource, amount);
            shipment.id = shipments.allocate_shipment_id();
            shipment.source = source;
            shipment.destination = destination;
            match (source, destination) {
                (Building(source), Building(destination)) => {
                    views.record_local_shipment(source, destination, resource, amount)
                }
                (OwaBorder(_), Building(destination)) => {
                    views.record_owa_import(destination, resource, amount)
                }
                (Building(source), OwaBorder(_)) => {
                    views.record_owa_export(source, resource, amount)
                }
                _ => unreachable!(),
            }
            shipments.shipments.push(shipment);
        }
        shipments.reservations = Some(views);
        shipments.assert_reservations_current();

        // Loading, delivery, the return trip, expiry and a failure before dispatch.
        type Step = (usize, fn(&mut Shipment));
        let steps: [Step; 6] = [
            (0, |shipment| shipment.cargo_loaded = true),
            (0, |shipment| shipment.status = ShipmentStatus::Returning),
            (0, |shipment| shipment.status = ShipmentStatus::Fulfilled),
            (2, |shipment| shipment.cargo_loaded = true),
            (2, |shipment| shipment.status = ShipmentStatus::Expired),
            (1, |shipment| shipment.status = ShipmentStatus::Failed),
        ];
        for (idx, change) in steps {
            shipments.update_shipment(idx, change);
            shipments.assert_reservations_current();
        }
        let views = shipments.reservations.as_ref().unwrap();
        assert_eq!(views.reserved_outbound_amount(0, 2), 0.0);
        assert_eq!(views.reserved_inbound_amount(4, 1), 140.0);
        assert!(!views.has_open_inbound(4, 3));

        // Removing building 6 cancels its order, and the last building moves into its index.
        let mut allocator = BuildingAllocator::new();
        let mut agents = AgentSystem::new();
        shipments.invalidate_building(6, &mut allocator, &mut agents, &mut 0.0);
        shipments.assert_reservations_current();
        shipments.remap_building_indices(&std::collections::HashMap::from([(7, 6)]));
        assert!(shipments.reservations.is_some());
        shipments.assert_reservations_current();
        assert_eq!(
            shipments
                .reservations
                .as_ref()
                .unwrap()
                .reserved_outbound_amount(6, 1),
            80.0
        );
        // A remap that moves a target as well is rebuilt from the shipments when next used.
        shipments.remap_building_indices(&std::collections::HashMap::from([(5, 6), (6, 5)]));
        assert!(shipments.reservations.is_none());
        let rebuilt = shipments.take_reservation_views(3);
        assert!(rebuilt.matches(&shipments.build_reservation_views(3)));
    }

    #[test]
    fn unit_sums_are_order_independent_with_an_amount_off_the_batch_grid() {
        // A stock amount just under two 50 kg batches, as the quantizer ships when its rounded
        // quotient overshoots, shares source and destination slots with batch multiples.
        let odd = 99.999_99_f32;
        assert_ne!(
            (120.0_f32 + odd) - 120.0,
            odd,
            "an f32 sum would depend on the order"
        );
        let jobs = [120.0, odd, 40.0].map(|amount| reservation_shipment(0, 4, 2, amount));
        let fresh = |remaining: &[usize]| {
            let shipments: Vec<Shipment> = remaining.iter().map(|&idx| jobs[idx].clone()).collect();
            ReservationViews::from_shipments(&shipments, 3)
        };
        let mut forward = ReservationViews::from_shipments(&[], 3);
        let mut backward = ReservationViews::from_shipments(&[], 3);
        for idx in [0, 1, 2] {
            forward.add_shipment(&jobs[idx]);
            backward.add_shipment(&jobs[2 - idx]);
        }
        assert!(forward.matches(&backward) && forward.matches(&fresh(&[0, 1, 2])));
        assert_eq!(
            forward.reserved_outbound_amount(0, 2),
            (120.0 + f64::from(odd) + 40.0) as f32
        );
        for (forward_removed, backward_removed) in [(0, 2), (2, 0)] {
            forward.remove_shipment(&jobs[forward_removed]);
            backward.remove_shipment(&jobs[backward_removed]);
        }
        assert!(forward.matches(&backward) && forward.matches(&fresh(&[1])));
        assert_eq!(forward.reserved_outbound_amount(0, 2), odd);
        assert_eq!(forward.reserved_inbound_amount(4, 2), odd);

        // A load rebuilds the views from the saved shipments: the continuing run must agree.
        let mut running = ShipmentSystem::new();
        running.shipments = jobs.to_vec();
        running.reservations = Some(running.take_reservation_views(3));
        for idx in [0, 2] {
            running.update_shipment(idx, |shipment| shipment.status = ShipmentStatus::Expired);
        }
        let mut reloaded = ShipmentSystem::new();
        reloaded.shipments = running.shipments.clone();
        let reloaded = reloaded.take_reservation_views(3);
        assert!(running.reservations.as_ref().unwrap().matches(&reloaded));
        assert!(reloaded.matches(&forward));
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
