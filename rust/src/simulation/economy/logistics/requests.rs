// SPDX-License-Identifier: GPL-2.0-only

//! The freight request queue (`SIM-02-J`, `docs/simulation.md#request-queues`).
//!
//! At its hourly phase a building checks whether it has an input to restock or output to export
//! and queues `(enqueue slot, building handle, kind)`. The `freight requests` queue serves the
//! oldest requests first, up to its quota per slot, against the live stock and reservations and
//! the supplier index the previous hour's sweep finished. Phases run in slot order and visit
//! their buildings in handle order, inbound before export, so the queue's insertion order is
//! already oldest first with ties by handle. A request whose building has left since is dropped
//! when served; one whose building changed is planned on the building as it is then. A building
//! queues at most one request of each kind at a time: a kind still queued is not queued again.
//!
//! The shipments a slot plans load their cargo at the end of that slot, as the hourly pass
//! loaded its plans after planning, and each carrier is listed for a departure slot
//! (`crate::simulation::core::scheduler::spread_slot`, levelled per minute).
//!
//! Freight cooldowns are slot deadlines checked at the phase (`SIM-02-J`). A failure or export
//! planned for a request runs until the phase that queued it comes round again, so the same
//! phase's export still waits behind a failed inbound plan. A failure at any other time (a
//! carrier that cannot leave, a shipment that fails on the road, an endpoint removed) runs
//! until the building's next hourly phase, which the hourly pass's "next pass" became.

use crate::debug;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::handles::Handle;
use crate::simulation::core::phases::{PHASES_PER_HOUR, daily_phase};
use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
use crate::simulation::economy::definitions::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;

use super::data::ShipmentSystem;
use super::planning::FreightPlanningContext;

/// What a queued freight request asks its building to plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FreightRequestKind {
    /// Restock the building's recipe inputs from a local supplier or the outside world.
    Inbound,
    /// Export the building's output surplus to an outside-world border.
    Export,
}

impl FreightRequestKind {
    // Bit of this kind in a building's pending set.
    fn pending_bit(self) -> u8 {
        match self {
            Self::Inbound => 1,
            Self::Export => 2,
        }
    }

    /// Saved code.
    pub(crate) fn code(self) -> i64 {
        match self {
            Self::Inbound => 0,
            Self::Export => 1,
        }
    }

    /// Inverse of [`Self::code`]; `None` for an unknown code.
    pub(crate) fn from_code(code: i64) -> Option<Self> {
        match code {
            0 => Some(Self::Inbound),
            1 => Some(Self::Export),
            _ => None,
        }
    }
}

/// One queued freight request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FreightRequest {
    /// Slot whose building phase queued it: the plan checks the building's cooldown against
    /// it, and a cooldown the plan sets runs from it.
    pub(crate) slot: u64,
    /// Building that asks; a handle that no longer resolves drops the request.
    pub(crate) building: Handle<Building>,
    /// What to plan.
    pub(crate) kind: FreightRequestKind,
}

/// Queue counts since the logistics hour started, for the hourly `economy_phases` line.
/// Diagnostics only: not saved, and nothing reads them back.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct FreightRequestStats {
    queued: u32,
    served: u32,
    dropped: u32,
    // Most requests one slot served, slots that left requests queued, and the longest queue
    // a slot left.
    busiest_slot: usize,
    carried_slots: u32,
    longest_queue: usize,
    // The first shipment id of the hour; unknown in the first hour after a load.
    first_shipment_id: Option<u64>,
    // Phase checks a freight cooldown turned away.
    cooldown_rejections: u32,
    // Wall time of the serves, measured only while the channel is on.
    serve_ms: f64,
    slowest_slot_ms: f64,
}

impl ShipmentSystem {
    /// Logs the finished logistics hour's queue counts on the `economy_phases` channel and
    /// starts the next hour's. O(1).
    pub(super) fn close_request_stats_hour(&mut self) {
        let stats = std::mem::take(&mut self.request_stats);
        crate::debug_log!(
            "economy_phases",
            "freight_requests_hour hour={} queued={} served={} dropped={} cooldown_rejections={} shipments={} busiest_slot={} carried_slots={} longest_queue={} serve_ms={:.3} slowest_slot_ms={:.3} shipment_list={}",
            self.hour_ticks,
            stats.queued,
            stats.served,
            stats.dropped,
            stats.cooldown_rejections,
            stats
                .first_shipment_id
                .map_or(0, |first| self.next_shipment_id.saturating_sub(first)),
            stats.busiest_slot,
            stats.carried_slots,
            stats.longest_queue,
            stats.serve_ms,
            stats.slowest_slot_ms,
            self.shipments.len(),
        );
        self.request_stats.first_shipment_id = Some(self.next_shipment_id);
    }

    /// Building `building_idx`'s freight check at its hourly phase in `slot`: queues an inbound
    /// request when an input port has no open inbound shipment and has fallen to its reorder
    /// point, and an export request when an output's unreserved surplus above the one-day buffer
    /// reaches the dispatch target. A building on its freight cooldown queues nothing, and a
    /// kind already queued for it is not queued twice. O(input and output ports),
    /// allocation-free once the queue and the pending set have room.
    pub(crate) fn request_freight_at_phase(
        &mut self,
        slot: u64,
        building_idx: usize,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
    ) {
        let building = allocator.building_handles.handle(building_idx);
        if building.is_none() {
            return;
        }
        if allocator.buildings[building_idx].shipment_cooldown_until_slot > slot {
            self.request_stats.cooldown_rejections += 1;
            crate::debug::work::add(crate::debug::work::Work::FreightCooldownRejections, 1);
        }
        let pending = self.pending_requests(building);
        let inbound = pending & FreightRequestKind::Inbound.pending_bit() == 0
            && self.inbound_request_needed(building_idx, slot, allocator, catalog, tuning);
        let export = pending & FreightRequestKind::Export.pending_bit() == 0
            && self.export_request_needed(building_idx, slot, allocator, catalog, tuning);
        for (needed, kind) in [
            (inbound, FreightRequestKind::Inbound),
            (export, FreightRequestKind::Export),
        ] {
            if needed {
                self.push_request(FreightRequest {
                    slot,
                    building,
                    kind,
                });
                self.request_stats.queued += 1;
            }
        }
    }

    // Queues `request` and marks its kind pending for its building. Amortized O(1).
    fn push_request(&mut self, request: FreightRequest) {
        self.mark_pending(request);
        self.freight_requests.push_back(request);
    }

    // Marks `request`'s kind pending for its building; the entry of a handle slot that another
    // generation used is replaced. Amortized O(1): the set grows with the handle slots.
    fn mark_pending(&mut self, request: FreightRequest) {
        let index = request.building.index() as usize;
        if index >= self.request_pending.len() {
            self.request_pending.resize(index + 1, (0, 0));
        }
        let entry = &mut self.request_pending[index];
        if entry.0 != request.building.generation() {
            *entry = (request.building.generation(), 0);
        }
        entry.1 |= request.kind.pending_bit();
    }

    // The kinds queued for `building`, as pending bits. O(1).
    fn pending_requests(&self, building: Handle<Building>) -> u8 {
        self.request_pending
            .get(building.index() as usize)
            .filter(|entry| entry.0 == building.generation())
            .map_or(0, |entry| entry.1)
    }

    // Clears `request`'s kind for its building once it leaves the queue. O(1).
    fn clear_pending(&mut self, request: FreightRequest) {
        if let Some(entry) = self
            .request_pending
            .get_mut(request.building.index() as usize)
            .filter(|entry| entry.0 == request.building.generation())
        {
            entry.1 &= !request.kind.pending_bit();
        }
    }

    /// Rebuilds the pending kinds from the queue, after loading it. They are derived: one per
    /// queued request. O(queue + building handle slots).
    pub(crate) fn rebuild_request_pending(&mut self) {
        self.request_pending.clear();
        for index in 0..self.freight_requests.len() {
            self.mark_pending(self.freight_requests[index]);
        }
    }

    /// One slot of the `freight requests` queue: plans up to `quota` queued requests, oldest
    /// first, then loads the cargo of the shipments they created and lists their carriers in
    /// `held_dispatches`. Returns the requests taken from the queue.
    ///
    /// O(1) with nothing queued. A served request costs O(P × (C + K)) for its building's P
    /// ports, the C supplier candidates its nearest-supplier ring scan visits (each one route
    /// quote, cached per revision) and the K border terminals of the import and export quotes;
    /// a dropped one O(1). The planning context is O(1) to assemble unless the road graph
    /// changed or no sweep finished this hour's supplier index (`FreightPlanningContext::build`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn serve_freight_requests(
        &mut self,
        quota: usize,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        minute_of_day: u16,
        treasury_balance: &mut f64,
    ) -> usize {
        if self.freight_requests.is_empty() || quota == 0 {
            return 0;
        }
        #[cfg(test)]
        let _contract_phase = crate::debug::work::phases::enter("logistics");
        let started = debug::Channel::EconomyPhases
            .enabled()
            .then(std::time::Instant::now);
        self.refresh_freight_route_cache(allocator, transit_network);
        let mut planning = FreightPlanningContext::build(self, allocator, transit_network, graph);
        let first_new_id = self.next_shipment_id;
        let mut served = 0;
        while served < quota {
            let Some(request) = self.freight_requests.pop_front() else {
                break;
            };
            served += 1;
            self.clear_pending(request);
            let Some(building_idx) = allocator.building_handles.resolve(request.building) else {
                self.request_stats.dropped += 1;
                continue;
            };
            match request.kind {
                FreightRequestKind::Inbound => self.plan_inbound_for(
                    building_idx,
                    request.slot,
                    allocator,
                    transit_network,
                    graph,
                    minute_of_day,
                    &mut planning,
                    treasury_balance,
                ),
                FreightRequestKind::Export => self.plan_export_for(
                    building_idx,
                    request.slot,
                    allocator,
                    transit_network,
                    graph,
                    minute_of_day,
                    &mut planning,
                ),
            }
        }
        let catalog = planning.catalog.clone();
        let retry_cooldown_hours = planning
            .tuning
            .operational_clock
            .shipment_retry_cooldown_hours;
        planning.finish(self);
        self.load_new_shipments(
            first_new_id,
            allocator,
            treasury_balance,
            retry_cooldown_hours,
            &catalog,
        );
        let stats = &mut self.request_stats;
        stats.served += served as u32;
        stats.busiest_slot = stats.busiest_slot.max(served);
        if !self.freight_requests.is_empty() {
            stats.carried_slots += 1;
            stats.longest_queue = stats.longest_queue.max(self.freight_requests.len());
        }
        if let Some(started) = started {
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            stats.serve_ms += ms;
            stats.slowest_slot_ms = stats.slowest_slot_ms.max(ms);
        }
        served
    }

    /// Every building's phase check at once, in dense order, stamped `slot`: what one hour of
    /// building phases queues, for logistics tests that step an hour without the slot clock.
    #[cfg(test)]
    pub(crate) fn request_freight_for_every_building(
        &mut self,
        slot: u64,
        allocator: &mut BuildingAllocator,
    ) {
        let catalog =
            crate::simulation::economy::definitions::load_runtime_economy_catalog().unwrap();
        let tuning =
            crate::simulation::economy::definitions::load_runtime_economy_tuning().unwrap();
        allocator
            .building_handles
            .register_appended(allocator.buildings.len());
        for building_idx in 0..allocator.buildings.len() {
            self.request_freight_at_phase(slot, building_idx, allocator, &catalog, &tuning);
        }
    }
}

/// Whether `building` may plan freight at its phase slot `phase_slot`: placed, finished,
/// operating, on a road and off its freight cooldown.
pub(super) fn freight_ready(building: &Building, phase_slot: u64) -> bool {
    !building.broken
        && !building.economy_broken
        && !building.is_deserted
        && !building.is_under_construction()
        && building.edge_idx != usize::MAX
        && building.shipment_cooldown_until_slot <= phase_slot
}

/// Cooldown deadline of a failure or export planned for a request queued at phase slot
/// `phase_slot`: `hours` later, the same phase. The phase's own later requests stay blocked,
/// as the hourly pass blocked a building's export after its failed inbound plan. O(1).
pub(super) fn planned_cooldown_until(phase_slot: u64, hours: u16) -> u64 {
    phase_slot + u64::from(hours) * SLOTS_PER_HOUR
}

/// Cooldown deadline of building `building_idx` after a failure outside its plans: its first
/// hourly phase at or after `from_slot`, and whole hours past it for a cooldown of `hours > 1`.
/// The hourly pass let such a building retry at the next pass; its phase is that pass now.
/// Callers that run after the slot's phases pass the next slot. O(1) from the handle's phase.
pub(super) fn event_cooldown_until(
    allocator: &BuildingAllocator,
    building_idx: usize,
    from_slot: u64,
    hours: u16,
) -> u64 {
    let handle = allocator.building_handles.handle(building_idx);
    let phase = u64::from(daily_phase(handle) % PHASES_PER_HOUR);
    let offset = from_slot % SLOTS_PER_HOUR;
    let next_phase = from_slot - offset + phase + if phase < offset { SLOTS_PER_HOUR } else { 0 };
    next_phase + u64::from(hours.saturating_sub(1)) * SLOTS_PER_HOUR
}
