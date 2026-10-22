// SPDX-License-Identifier: GPL-2.0-only

//! A planning slot's setup, held for its plan (`SIM-02-K`). The setup slot, the first slot of
//! the planning slot's game minute, counts the live workforce and scans the free lots and the
//! existing buildings; the plan, [`PLANNING_SETUP_LEAD`] slots later in the same minute, lends
//! the books, proves the queue and the earlier slots' plans again and decides on what the setup
//! found. Each half then holds about half of the pass. Saves in between keep the setup.

use super::actions::{PLANNING_SETUP_LEAD, PlanningSlot};
use super::building_actions::PlanningScans;
use super::snapshot::{
    DailyDemandSnapshot, SweepInputs, WorkforceView, connected_road_border_count,
};
use super::system::DemandSystem;
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingSiteEnvironment};
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::economy::logistics::collect_connected_border_nodes;
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};
use crate::simulation::zoning::ZoningSystem;
use std::sync::Arc;

/// What a planning slot's setup slot counted and scanned (`SIM-02-K`), for the plan at
/// absolute slot `plan_at`.
#[derive(Debug)]
pub(super) struct PlanningSetup {
    slot: PlanningSlot,
    plan_at: u64,
    workforce: WorkforceView,
    scans: PlanningScans,
}

impl DemandSystem {
    /// Planning slot `slot`'s setup at absolute slot `setup_at`, the first slot of its minute
    /// (`SIM-02-K`): counts the live workforce, after the queues served so far, and scans the
    /// zone's free lots and existing buildings (the actions slot: every zone's downgrades), and
    /// holds them for the plan [`PLANNING_SETUP_LEAD`] slots later. The scans rank lots by the
    /// commercial demand a snapshot composed now leaves, as the plan's snapshot does. The
    /// caller prepares the site indices, the road surface and the construction queue as the
    /// plan does. Books not valid for the live city hold nothing, and the plan then skips as it
    /// does without a setup. O((H + B + P)/p) in parallel, the pass's own count and scans.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_planning_setup(
        &mut self,
        slot: PlanningSlot,
        setup_at: u64,
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        graph: &RegionGraph,
        zoning: &ZoningSystem,
        treasury_balance: f64,
        fiscal_policy: &CityFiscalPolicy,
        network: &TransitNetwork,
        environment: BuildingSiteEnvironment<'_>,
        minute_of_day: u16,
    ) {
        self.planning_setup = None;
        let mut borders = std::mem::take(&mut self.freight_border_nodes);
        collect_connected_border_nodes(graph, &mut borders);
        if self.lent_books_current(households, network, &borders, minute_of_day) {
            let catalog = Arc::clone(&self.runtime_catalog);
            let tuning = Arc::clone(&self.runtime_tuning);
            let front = self.snapshot_sweep.take_front(&SweepInputs {
                allocator,
                households,
                config: &self.config,
                catalog: &catalog,
                tuning: &tuning,
                policy: fiscal_policy,
                treasury: treasury_balance,
            });
            let border_count = connected_road_border_count(graph, &borders);
            let compose = |workforce: &WorkforceView| {
                let mut snapshot = DailyDemandSnapshot::compose(
                    &front.totals,
                    workforce,
                    households,
                    border_count,
                    &catalog,
                    &tuning,
                    fiscal_policy,
                    treasury_balance,
                );
                snapshot.reserve_queued_construction(&catalog, &self.queued_construction);
                snapshot
            };
            // The scans read only the snapshot's output absorption and unmet commercial demand,
            // which `compose` and the queue derive without the workforce, so they run beside
            // the count on a snapshot composed without one (`SIM-02-K`).
            let mut workforce = WorkforceView::default();
            let ((), scans) = rayon::join(
                || {
                    workforce.count_live(
                        allocator,
                        households,
                        &catalog,
                        &tuning,
                        fiscal_policy.income_tax_rate,
                    )
                },
                || {
                    self.scan_for_planning(
                        slot,
                        allocator,
                        graph,
                        zoning,
                        &compose(&WorkforceView::default()),
                        &catalog,
                        &tuning,
                        &front.occupants,
                        environment,
                    )
                },
            );
            // Test builds: the counted workforce's snapshot ranks the lots alike.
            #[cfg(test)]
            if crate::debug::work::city_checks() {
                let ranks = |snapshot: DailyDemandSnapshot| {
                    super::building_actions::commercial_spawn_priorities(&snapshot, &catalog)
                        .into_iter()
                        .map(|(resource, priority)| (resource, priority.to_bits()))
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    ranks(compose(&workforce)),
                    ranks(compose(&WorkforceView::default())),
                    "the setup's scans must not depend on the workforce"
                );
            }
            self.snapshot_sweep.restore_front(front);
            self.planning_setup = Some(PlanningSetup {
                slot,
                plan_at: setup_at + PLANNING_SETUP_LEAD,
                workforce,
                scans,
            });
        }
        self.freight_border_nodes = borders;
    }

    // The setup held for planning slot `slot`'s plan at absolute slot `now`, taken; `None`
    // without one (a world's first hour, a clock set directly, a test driver, or books the
    // setup found not valid), and any other held setup is dropped. O(1).
    pub(super) fn take_planning_setup(
        &mut self,
        slot: PlanningSlot,
        now: u64,
    ) -> Option<(WorkforceView, PlanningScans)> {
        self.planning_setup
            .take()
            .filter(|setup| setup.slot == slot && setup.plan_at == now)
            .map(|setup| (setup.workforce, setup.scans))
    }

    /// The held planning setup's saved form, `None` without one (`SIM-02-K`): its planning
    /// slot and plan slot, the counted workforce and the scans' candidates, building candidates
    /// by dense index. O(setup).
    pub(crate) fn saved_planning_setup(&self) -> Option<Vec<u8>> {
        self.planning_setup.as_ref().map(|setup| {
            let mut out = Vec::new();
            put_u64(&mut out, setup.slot as u64);
            put_u64(&mut out, setup.plan_at);
            setup.workforce.write_saved(&mut out);
            setup.scans.write_saved(&mut out);
            out
        })
    }

    /// Restores [`Self::saved_planning_setup`] on the loaded `allocator`, whose buildings keep
    /// their dense order, so each building candidate's key is its building's. A malformed buffer
    /// or a building the loaded city does not hold is an error. O(setup).
    pub(crate) fn restore_planning_setup(
        &mut self,
        bytes: &[u8],
        allocator: &BuildingAllocator,
    ) -> Result<(), String> {
        let malformed = || "malformed saved planning setup".to_owned();
        let mut reader = SavedReader { rest: bytes };
        let slot = match reader.u64().ok_or_else(malformed)? {
            slot if slot == PlanningSlot::Residential as u64 => PlanningSlot::Residential,
            slot if slot == PlanningSlot::Commercial as u64 => PlanningSlot::Commercial,
            slot if slot == PlanningSlot::Industrial as u64 => PlanningSlot::Industrial,
            slot if slot == PlanningSlot::Actions as u64 => PlanningSlot::Actions,
            _ => return Err(malformed()),
        };
        let plan_at = reader.u64().ok_or_else(malformed)?;
        let workforce = WorkforceView::read_saved(&mut reader).ok_or_else(malformed)?;
        let scans = PlanningScans::read_saved(&mut reader, allocator).ok_or_else(malformed)?;
        if !reader.rest.is_empty() {
            return Err(malformed());
        }
        self.planning_setup = Some(PlanningSetup {
            slot,
            plan_at,
            workforce,
            scans,
        });
        Ok(())
    }
}

// Little-endian fields of a saved planning setup.
pub(super) fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(super) fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(super) fn put_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_le_bytes());
}

// A count of the items that follow.
pub(super) fn put_len(out: &mut Vec<u8>, len: usize) {
    put_u64(out, len as u64);
}

pub(super) fn put_str(out: &mut Vec<u8>, value: &str) {
    put_len(out, value.len());
    out.extend_from_slice(value.as_bytes());
}

// Reads what the `put_*` helpers wrote, in order; `None` on a short or malformed buffer.
pub(super) struct SavedReader<'a> {
    rest: &'a [u8],
}

impl SavedReader<'_> {
    pub(super) fn u32(&mut self) -> Option<u32> {
        let (head, tail) = self.rest.split_first_chunk::<4>()?;
        self.rest = tail;
        Some(u32::from_le_bytes(*head))
    }

    pub(super) fn u64(&mut self) -> Option<u64> {
        let (head, tail) = self.rest.split_first_chunk::<8>()?;
        self.rest = tail;
        Some(u64::from_le_bytes(*head))
    }

    pub(super) fn f32(&mut self) -> Option<f32> {
        self.u32().map(f32::from_bits)
    }

    // A count no larger than the bytes left, so a malformed count cannot reserve memory.
    pub(super) fn count(&mut self) -> Option<usize> {
        usize::try_from(self.u64()?)
            .ok()
            .filter(|&len| len <= self.rest.len())
    }

    pub(super) fn string(&mut self) -> Option<String> {
        let len = self.count()?;
        let (head, tail) = self.rest.split_at(len);
        self.rest = tail;
        String::from_utf8(head.to_vec()).ok()
    }
}

#[cfg(test)]
impl DemandSystem {
    /// Drops the held planning setup, so the next plan counts and scans itself.
    pub(crate) fn drop_planning_setup_for_test(&mut self) {
        self.planning_setup = None;
    }
}
