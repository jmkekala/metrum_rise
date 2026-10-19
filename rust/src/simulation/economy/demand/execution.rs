// SPDX-License-Identifier: GPL-2.0-only

//! Construction permits and level-change proofs at the hour's planning slot (`SIM-02-I`).
//!
//! After the hourly pass has queued its spawns, the slot opens one construction epoch on the
//! census published at it. The census's customer, business and input books are lent under the
//! pass journal, as planning lends them; the snapshot is the planner's rolling front with the
//! live workforce; every constructing building and queued project is replayed in queue order,
//! counting queued homes as the planner does. Each queued commercial or industrial spawn the
//! epoch can fund and staff gets a permit and starts later in the hour only with it, at the
//! door, profile and level it was proved for. Immediate level changes are proved in the same
//! epoch, which closes, restoring the books, before any despawn swap-removes a building. No
//! census runs outside the census window and its publication.

use super::actions::DemandSpawnAction;
use super::snapshot::{DailyDemandSnapshot, SweepInputs, connected_road_border_count};
use super::spawn_need::{OutputAbsorptionContext, candidate_economy_profile};
use super::supply::{ProjectReservations, SiteSupply};
use super::system::DemandSystem;
use super::workforce::WorkforceReservations;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator, BuildingEntrance};
use crate::simulation::core::stable_hash64;
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use crate::simulation::economy::hiring::JobFunding;
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::economy::logistics::{
    RecurringSupply, ShipmentSystem, collect_connected_border_nodes,
};
use crate::simulation::network::lanes::LaneSystem;
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};
use crate::simulation::zoning::{ZoneType, ZoningSystem};
use std::sync::Arc;

mod level_change;
pub(crate) use level_change::DemandLevelChangeGuard;
pub(crate) use level_change::{LevelChangeRefusal, QueuedLevelChange};

/// What a queued commercial or industrial spawn's planning-slot proof was for (`SIM-02-I`):
/// its door, profile and level. Saved with the spawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ConstructionPermit {
    /// Stable digest of the door the proof quoted from ([`door_digest`]).
    pub(crate) door: u64,
    /// Economy profile the proof funded.
    pub(crate) profile: u16,
    /// Authored level the proof priced.
    pub(crate) level: u8,
}

/// A queued commercial or industrial spawn's standing for its start (`SIM-02-I`). Saved with
/// the spawn; a residential spawn needs none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SpawnPermit {
    /// The last planning slot could not fund or staff it, or none has proved it yet: the start
    /// rejects it.
    #[default]
    Denied,
    /// Proved at the last planning slot.
    Granted(ConstructionPermit),
    /// Granted, then voided by a home that failed to place after its applicants backed the
    /// proof: the start waits for the next planning slot, which proves it again.
    Voided,
}

impl SpawnPermit {
    /// The permit a granted spawn holds.
    #[cfg(test)]
    pub(crate) fn granted(self) -> Option<ConstructionPermit> {
        match self {
            Self::Granted(permit) => Some(permit),
            Self::Denied | Self::Voided => None,
        }
    }
}

/// Why a queued spawn does not place at its start (`SIM-02-I`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StartRefusal {
    /// Its permit was voided: it starts after the next planning slot proves it again.
    Wait,
    /// No permit for its current door, profile and level: it is rejected.
    Denied(&'static str),
}

/// Stable digest of a proposed door, for comparing the door a permit was proved at with the
/// door at the start, in the same session or after a load. Saves renumber edges and loads
/// rebuild lanes compactly, so the frontage edge counts by its end nodes' ground positions and
/// each door lane by its direction, lane index and kind on it; positions count by their bits.
/// O(1).
pub(crate) fn door_digest(
    entrance: Option<&BuildingEntrance>,
    graph: &RegionGraph,
    lanes: &LaneSystem,
) -> u64 {
    let Some(entrance) = entrance else {
        return stable_hash64(u64::MAX);
    };
    let ends = graph
        .get_edge(entrance.edge_idx)
        .map_or([u64::MAX; 4], |edge| {
            let [start, end] = [edge.start_node, edge.end_node].map(|node| {
                graph.get_node(node).map_or([u64::MAX; 2], |node| {
                    [node.pos.x, node.pos.z].map(|c| u64::from(c.to_bits()))
                })
            });
            [start[0], start[1], end[0], end[1]]
        });
    let lane = |id: usize| {
        lanes.lanes.get(id).map_or(u64::MAX, |lane| {
            u64::from(lane.is_fwd)
                | u64::from(lane.lane_idx as u8) << 1
                | (lane.lane_type as u64) << 9
        })
    };
    ends.into_iter()
        .chain([
            entrance.side as u8 as u64,
            entrance.vehicle_frontage_access as u64,
            u64::from(entrance.entrance_s_m.to_bits()),
            u64::from(entrance.door_pos.x.to_bits()),
            u64::from(entrance.door_pos.y.to_bits()),
            u64::from(entrance.curb_pos.x.to_bits()),
            u64::from(entrance.curb_pos.y.to_bits()),
            lane(entrance.foot_lane_fwd),
            lane(entrance.foot_lane_bkw),
            lane(entrance.car_lane_fwd),
            lane(entrance.car_lane_bkw),
            u64::from(entrance.flags),
        ])
        .fold(0, |digest, field| stable_hash64(digest ^ field))
}

/// Stable digest of the building state a level change was proved for (`SIM-02-J`): its asset,
/// level and economy profile and, for a business, its door ([`door_digest`]); a home's door
/// does not count, since a residential change needs no proof. Built from save-stable
/// identities, so a load compares as the running game does. Saved with the queued change,
/// whose execution rejects it when the building no longer matches. O(asset id length).
pub(crate) fn level_change_permit(
    building: &Building,
    entrance: Option<&BuildingEntrance>,
    graph: &RegionGraph,
    lanes: &LaneSystem,
) -> u64 {
    let door = if building.zone_type == ZoneType::Residential {
        0
    } else {
        door_digest(entrance, graph, lanes)
    };
    building
        .asset_id
        .as_bytes()
        .chunks(8)
        .map(|chunk| {
            let mut word = [0_u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            u64::from_le_bytes(word)
        })
        .chain([
            building.asset_id.len() as u64,
            door,
            u64::from(building.economy_profile_runtime_id),
            u64::from(building.level),
        ])
        .fold(0, |digest, field| stable_hash64(digest ^ field))
}

/// The planning slot's construction epoch, and the scratch planning lends the books into.
/// Never saved: an epoch opens and closes within one slot.
#[derive(Default)]
pub(super) struct ConstructionExecution {
    // Open from the permit proof until the level changes are done: the published census's
    // books are lent to `reservations` and `book` under the pass journal.
    open: bool,
    // Whether a business level change was applied inside the epoch. The journal then restores
    // the replaced owner's rows with geometry it did not record, so test builds skip the
    // restored-books comparison; residential changes never set it.
    changed: bool,
    workforce: WorkforceReservations,
    market: Option<OutputAbsorptionContext>,
    pub(super) replay_market: OutputAbsorptionContext,
    borders: Vec<u32>,
    book: RecurringSupply,
    pub(super) reservations: ProjectReservations,
    #[cfg(test)]
    fingerprint: Option<[Vec<(&'static str, u64)>; 3]>,
}

/// Lends the live economic books for a pass: the input book is returned and the customer and
/// business books move into `reservations`; [`return_live_books`] gives them back. O(1).
pub(super) fn lend_live_books(
    reservations: &mut ProjectReservations,
    live: &mut JobFunding,
) -> RecurringSupply {
    std::mem::swap(&mut reservations.customers, &mut live.customers);
    std::mem::swap(&mut reservations.business, &mut live.business);
    std::mem::take(&mut live.input_supply)
}

/// Returns books lent by [`lend_live_books`]. O(1).
pub(super) fn return_live_books(
    book: RecurringSupply,
    reservations: &mut ProjectReservations,
    live: &mut JobFunding,
) {
    live.input_supply = book;
    std::mem::swap(&mut reservations.customers, &mut live.customers);
    std::mem::swap(&mut reservations.business, &mut live.business);
}

impl DemandSystem {
    /// Opens the planning slot's construction epoch after the hourly pass queued its spawns
    /// (`SIM-02-I`): lends the census published at the slot under the pass journal, composes
    /// the planner's snapshot (the rolling front, the live workforce the pass counted and the
    /// queue's capacity) and replays every constructing building and queued project in queue
    /// order, counting queued homes as the planner does. [`Self::construction_permit`] then
    /// reads each spawn's permit, and level changes are proved in the epoch until
    /// [`Self::close_construction_epoch`]. Books that are not valid for the live city are never
    /// lent: the epoch is skipped and counted, and returns `false`. O(B + Q log H + quoted ports
    /// and routes).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn open_construction_epoch(
        &mut self,
        allocator: &BuildingAllocator,
        households: &mut HouseholdSystem,
        zoning: &ZoningSystem,
        graph: &RegionGraph,
        network: &TransitNetwork,
        logistics: &mut ShipmentSystem,
        policy: &CityFiscalPolicy,
        treasury: f64,
        now: u64,
    ) -> bool {
        #[cfg(test)]
        let _contract_phase = crate::debug::work::phases::enter("execution");
        debug_assert!(!self.construction_execution.open);
        let minute = (now % 1440) as u16;
        let mut borders = std::mem::take(&mut self.construction_execution.borders);
        collect_connected_border_nodes(graph, &mut borders);
        let current = self.lent_books_current(households, network, &borders, minute);
        self.construction_execution.borders = borders;
        if !current {
            crate::debug::work::add(crate::debug::work::Work::ConstructionEpochSkips, 1);
            crate::debug_log!(
                "economy",
                "construction epoch skipped: the published census is not valid for the live city: minute={}",
                minute
            );
            return false;
        }
        self.lend_construction_epoch(
            allocator, households, zoning, graph, network, logistics, policy, treasury, now,
        );
        true
    }

    // Opens the epoch on the published books, which the caller has checked for the live city.
    #[allow(clippy::too_many_arguments)]
    fn lend_construction_epoch(
        &mut self,
        allocator: &BuildingAllocator,
        households: &mut HouseholdSystem,
        zoning: &ZoningSystem,
        graph: &RegionGraph,
        network: &TransitNetwork,
        logistics: &mut ShipmentSystem,
        policy: &CityFiscalPolicy,
        treasury: f64,
        now: u64,
    ) {
        crate::debug::work::add(crate::debug::work::Work::ConstructionEpochs, 1);
        let minute = (now % 1440) as u16;
        let catalog = Arc::clone(&self.runtime_catalog);
        let tuning = Arc::clone(&self.runtime_tuning);
        let execution = &mut self.construction_execution;
        collect_connected_border_nodes(graph, &mut execution.borders);
        logistics.prepare_site_carrier_quotes(allocator, network, graph, &execution.borders);
        #[cfg(test)]
        {
            execution.fingerprint = super::system::VERIFY_LIVE_PASSES
                .load(std::sync::atomic::Ordering::Relaxed)
                .then(|| super::system::live_fingerprint(&households.job_funding));
        }
        execution.book = lend_live_books(&mut execution.reservations, &mut households.job_funding);
        let households = &*households;
        let census = &households.job_funding;
        execution.reservations.begin_live_pass(
            census,
            &mut execution.book,
            allocator,
            graph,
            &catalog,
            &tuning,
            policy,
            minute,
        );
        let front = self.snapshot_sweep.take_front(&SweepInputs {
            allocator,
            households,
            config: &self.config,
            catalog: &catalog,
            tuning: &tuning,
            policy,
            treasury,
        });
        let mut snapshot = DailyDemandSnapshot::compose(
            &front.totals,
            &self.workforce_view,
            households,
            connected_road_border_count(graph, &execution.borders),
            &catalog,
            &tuning,
            policy,
            treasury,
        );
        self.snapshot_sweep.restore_front(front);
        snapshot.reserve_queued_construction(&catalog, &self.queued_construction);
        let mut supply = SiteSupply::new(
            &mut execution.book,
            &mut execution.reservations,
            allocator,
            zoning,
            network,
            graph,
            logistics,
            &tuning,
            &execution.borders,
            &catalog,
            minute,
            policy.household_vat_rate,
        );
        supply.reserve_execution_projects(
            self.queued_supply_order
                .iter()
                .map(|&index| &self.queued_construction[index]),
            &catalog,
            &snapshot,
            &mut execution.replay_market,
            policy,
            census,
            &mut execution.workforce,
            &self.queued_construction,
            self.next_construction_start_hour,
            // Placed sites count their deadlines from this minute's hour.
            (now / 60).saturating_sub(self.queued_reference_hour),
        );
        execution.market = Some(snapshot.output_absorption);
        execution.open = true;
        execution.changed = false;
    }

    /// Closes the construction epoch, restoring the lent books exactly and returning them to
    /// the published census, before a despawn swap-removes a building or a household leaves.
    /// O(rows the epoch touched).
    pub(crate) fn close_construction_epoch(
        &mut self,
        households: &mut HouseholdSystem,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
    ) {
        let execution = &mut self.construction_execution;
        if !std::mem::take(&mut execution.open) {
            return;
        }
        execution
            .reservations
            .end_live_pass(&mut execution.book, allocator, graph);
        return_live_books(
            std::mem::take(&mut execution.book),
            &mut execution.reservations,
            &mut households.job_funding,
        );
        execution.market = None;
        #[cfg(test)]
        if let Some(before) = execution.fingerprint.take()
            && !execution.changed
        {
            assert_eq!(
                super::system::live_fingerprint(&households.job_funding),
                before,
                "a construction epoch must return the live books exactly"
            );
        }
    }

    /// The permit the open epoch proved for queue entry `spawn_id` with `action`, granted with
    /// its door, profile and level when its proof was funded and staffed; denied for a project
    /// the epoch could not fund or staff, a later duplicate of a queued parcel, or without an
    /// open epoch. A residential spawn needs none. O(log Q).
    pub(crate) fn construction_permit(
        &self,
        spawn_id: u64,
        action: &DemandSpawnAction,
        graph: &RegionGraph,
        lanes: &LaneSystem,
    ) -> SpawnPermit {
        let execution = &self.construction_execution;
        let Some(project) = self
            .queued_project(spawn_id, action.parcel_id)
            .filter(|project| execution.open && project.household_slots == 0)
        else {
            return SpawnPermit::Denied;
        };
        let queued = &execution.reservations.queued_workers;
        if queued
            .binary_search_by_key(&action.parcel_id, |&(parcel, _)| parcel)
            .is_ok_and(|index| queued[index].1 > 0)
        {
            SpawnPermit::Granted(ConstructionPermit {
                door: door_digest(project.entrance.as_ref(), graph, lanes),
                profile: project.profile_runtime_id,
                level: project.level,
            })
        } else {
            SpawnPermit::Denied
        }
    }

    /// Checks a queued spawn at its start before its owner's equity is spent (`SIM-02-I`): a
    /// commercial or industrial spawn needs a permit proved at the last planning slot for its
    /// current door, profile and level; a residential spawn needs none. A voided permit waits
    /// for the next slot; a denied one rejects. Nothing is proved at the start. O(door
    /// derivation).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn check_construction_permit(
        &self,
        zone: ZoneType,
        action: &DemandSpawnAction,
        permit: SpawnPermit,
        allocator: &BuildingAllocator,
        zoning: &ZoningSystem,
        graph: &RegionGraph,
        network: &TransitNetwork,
    ) -> Result<(), StartRefusal> {
        if zone == ZoneType::Residential || self.cheat_max_demands_enabled {
            return Ok(());
        }
        let permit = match permit {
            SpawnPermit::Granted(permit) => permit,
            SpawnPermit::Voided => return Err(StartRefusal::Wait),
            SpawnPermit::Denied => return Err(StartRefusal::Denied("no-construction-permit")),
        };
        let denied = StartRefusal::Denied;
        let level = allocator
            .registry
            .get(&action.asset_id)
            .and_then(|entry| entry.manifest.building.as_ref())
            .ok_or(denied("missing-asset"))?
            .level;
        let profile = candidate_economy_profile(allocator, &self.runtime_catalog, &action.asset_id)
            .ok_or(denied("missing-economy-profile"))?;
        if profile.runtime_id != permit.profile || level != permit.level {
            return Err(denied("permit-recipe-changed"));
        }
        let entrance = allocator.demand_site_entrance(
            action.parcel_id,
            &action.asset_id,
            zoning,
            graph,
            &network.lane_system,
        );
        if door_digest(entrance.as_ref(), graph, &network.lane_system) != permit.door {
            return Err(denied("permit-door-changed"));
        }
        Ok(())
    }

    /// Whether planning counts queue entry `spawn_id`, a home: its project is active, not a
    /// later duplicate of a queued parcel or on a parcel no longer available. O(log Q).
    pub(crate) fn counts_queued_home(&self, spawn_id: u64, action: &DemandSpawnAction) -> bool {
        self.queued_project(spawn_id, action.parcel_id)
            .is_some_and(|project| project.household_slots > 0)
    }

    /// Retires started queue entry `spawn_id`, placed or rejected: the next planning slot's
    /// epoch replays the placed site with the constructing buildings. O(log Q).
    pub(crate) fn finish_queued_spawn(&mut self, spawn_id: u64, action: &DemandSpawnAction) {
        self.retire_queued_construction(spawn_id, action.parcel_id);
    }

    /// Test driver: proves the planning slot's permits for `queue`, in queue order, on the
    /// census `households` publishes, with the workforce counted now, and closes the epoch.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn permits_for_test(
        &mut self,
        queue: &[(ZoneType, &DemandSpawnAction)],
        allocator: &BuildingAllocator,
        households: &mut HouseholdSystem,
        zoning: &ZoningSystem,
        graph: &RegionGraph,
        network: &TransitNetwork,
        logistics: &mut ShipmentSystem,
        policy: &CityFiscalPolicy,
        treasury: f64,
        now: u64,
    ) -> Vec<Option<ConstructionPermit>> {
        let permits = self.open_permits_for_test(
            queue, allocator, households, zoning, graph, network, logistics, policy, treasury, now,
        );
        self.close_construction_epoch(households, allocator, graph);
        permits
    }

    /// [`Self::permits_for_test`] leaving the epoch open, so a test can read its books.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn open_permits_for_test(
        &mut self,
        queue: &[(ZoneType, &DemandSpawnAction)],
        allocator: &BuildingAllocator,
        households: &mut HouseholdSystem,
        zoning: &ZoningSystem,
        graph: &RegionGraph,
        network: &TransitNetwork,
        logistics: &mut ShipmentSystem,
        policy: &CityFiscalPolicy,
        treasury: f64,
        now: u64,
    ) -> Vec<Option<ConstructionPermit>> {
        self.set_queued_construction_with_entrances(
            allocator,
            zoning,
            now,
            queue.iter().enumerate().map(|(index, &(zone, action))| {
                (index as u64, now + 1 + index as u64, zone, action)
            }),
            |action| {
                allocator.demand_site_entrance(
                    action.parcel_id,
                    &action.asset_id,
                    zoning,
                    graph,
                    &network.lane_system,
                )
            },
        );
        self.workforce_view.count_live(
            allocator,
            households,
            &self.runtime_catalog,
            &self.runtime_tuning,
            policy.income_tax_rate,
        );
        // Fixtures build their census with `JobFunding::refresh` at a minute of their own.
        self.lend_construction_epoch(
            allocator, households, zoning, graph, network, logistics, policy, treasury, now,
        );
        queue
            .iter()
            .enumerate()
            .map(|(index, &(zone, action))| {
                (zone != ZoneType::Residential)
                    .then(|| {
                        self.construction_permit(index as u64, action, graph, &network.lane_system)
                            .granted()
                    })
                    .flatten()
            })
            .collect()
    }

    /// The open epoch's proof of a queued or constructing project, for exact comparisons.
    #[cfg(test)]
    pub(crate) fn project_proof_for_test(
        &self,
        parcel: u64,
        zoning: &ZoningSystem,
    ) -> Option<super::supply::ProjectProof> {
        let execution = &self.construction_execution;
        execution.reservations.project_proof_for_test(
            &execution.book,
            parcel,
            zoning,
            &self.runtime_catalog,
            &self.runtime_tuning,
        )
    }

    /// Workers the last epoch or planning pass reserved for constructing building `index`.
    #[cfg(test)]
    pub(crate) fn constructing_workers_for_test(&self, index: usize) -> u32 {
        self.construction_execution
            .reservations
            .building_workers
            .get(index)
            .copied()
            .unwrap_or(0)
    }
}
