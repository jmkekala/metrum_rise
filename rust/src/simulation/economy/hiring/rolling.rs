// SPDX-License-Identifier: GPL-2.0-only

//! The hourly funding census over the hour's second half (`SIM-02-I`,
//! `docs/simulation.md#pure-jobs`).
//!
//! The input copy fills over the hour's first half (`hiring/inputs.rs`). From slot
//! [`CENSUS_WINDOW_START`] the census runs in steps on it (`hiring/steps.rs`) into its own
//! [`JobFunding`] while every reader keeps the live one, and the next hour's first slot swaps it
//! in. A census records the [`CensusKey`] it started with: a network or structural change before
//! it is published restarts it at the next slot of the window, and at the publishing slot a
//! census that is missing, unfinished or stale runs whole there. Its results are a function of
//! the copy and the network alone, so a loaded game recomputes a census in progress at load.
//!
//! **Costs.** Each window slot gets ceil(R / S) rows of work, R the rows left assuming the
//! current closure pass is the last ([`CensusProgress::remaining`]) and S the window slots left;
//! the window's last slot gets all that remains. A step that is not sliced (the consumer-market
//! refresh, setup, the input-source reset, repair, business seeding, propagation) runs whole in
//! the slot it starts in. A restart repeats the work done so far.

use super::{CensusInputs, CensusProgress, FILL_SLOTS, JobFunding};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
use crate::simulation::economy::definitions::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};

/// First slot of the hour at which the census runs: the copy has filled over the slots before.
pub(crate) const CENSUS_WINDOW_START: u64 = FILL_SLOTS;

/// Revisions that move only when building or household indices shift or are reused
/// (`SIM-02-I`); appends and in-place changes keep them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct StructureRevisions {
    /// Building swap-removes, remaps, clears and undo restores.
    pub(crate) buildings: u64,
    /// Household swap-removes (exits) and clears.
    pub(crate) households: u64,
}

/// What a census depends on besides its input copy: the network revisions it routes and prices
/// on, the structural revisions of the building and household stores and the revision of the
/// policy its copy freezes. Appends and in-place changes keep it: a census covers the rows of
/// its copy, and readers treat later rows as not in the census.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CensusKey {
    structure: StructureRevisions,
    policy: u64,
    cch_build: u32,
    cch_metric: u64,
    lanes: u64,
    frontage: u64,
}

impl CensusKey {
    /// The key of the live city with the households' `structure` and `policy` revisions.
    pub(crate) fn live(
        structure: StructureRevisions,
        policy: u64,
        network: &TransitNetwork,
    ) -> Self {
        Self {
            structure,
            policy,
            cch_build: network.cch_graph.build_generation,
            cch_metric: network.cch_graph.metric_revision,
            lanes: network.lane_system.revision(),
            frontage: network.frontage_delay_revision,
        }
    }

    /// Whether books a census published under this key stay valid for readers under the live
    /// key: the same network and building indices. A household removed since stays in the
    /// books as a customer until the next publication; readers that join book rows to live
    /// households check identity. Books published before a policy change stand until the
    /// next publication, whose census follows it. A census in progress needs the whole key.
    pub(crate) fn books_valid_for(self, live: Self) -> bool {
        Self {
            structure: StructureRevisions {
                households: live.structure.households,
                ..self.structure
            },
            policy: live.policy,
            ..self
        } == live
    }
}

// A census in progress or finished: the slot that publishes it and the key it started with,
// `None` for a census loaded stale, which restarts.
#[derive(Clone, Copy, Debug)]
struct Run {
    publish_slot: u64,
    key: Option<CensusKey>,
}

/// The census built over the window and its progress (`SIM-02-I`). Saved as
/// [`SavedRollingCensus`]; a loaded census in progress is recomputed from the saved copy.
#[derive(Default)]
pub(crate) struct RollingCensus {
    funding: JobFunding,
    progress: CensusProgress,
    run: Option<Run>,
    // A census current at the save, to recompute once the loaded world is ready.
    complete_on_load: bool,
    /// Census restarts so far, for tests.
    #[cfg(test)]
    pub(crate) restarts: usize,
}

/// The rolling census as saved: the slot that publishes the census in progress and whether it
/// was current. Its steps are not saved; a load recomputes a current census whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SavedRollingCensus {
    /// The hour's first slot the census publishes at.
    pub(crate) publish_slot: u64,
    /// Whether the census's key matched the live city; a stale one restarts.
    pub(crate) current: bool,
}

/// What to do with the census at a window slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowAction {
    /// No census for this hour: start one.
    Start,
    /// A network or structural change since it started: start it again.
    Restart,
    /// Continue the census in progress, if any work remains.
    Continue,
}

impl RollingCensus {
    /// What a window slot whose hour publishes at `publish_slot` does with the live `key`.
    pub(crate) fn window_action(&self, publish_slot: u64, key: CensusKey) -> WindowAction {
        match self.run {
            Some(run) if run.publish_slot == publish_slot => {
                if run.key == Some(key) {
                    WindowAction::Continue
                } else {
                    WindowAction::Restart
                }
            }
            _ => WindowAction::Start,
        }
    }

    /// Whether the census for `publish_slot` is finished and current for `key`.
    pub(crate) fn is_ready(&self, publish_slot: u64, key: CensusKey) -> bool {
        self.progress.is_done()
            && matches!(self.run, Some(run) if run.publish_slot == publish_slot && run.key == Some(key))
    }

    /// Whether every step of the census has run.
    pub(crate) fn is_done(&self) -> bool {
        self.progress.is_done()
    }

    /// Whether a census for `publish_slot` started on the front, current or stale.
    pub(crate) fn has_run(&self, publish_slot: u64) -> bool {
        matches!(self.run, Some(run) if run.publish_slot == publish_slot)
    }

    /// Starts the census for `publish_slot` on a front prepared now under `key`. O(1).
    pub(crate) fn begin(&mut self, publish_slot: u64, key: CensusKey) {
        self.progress = CensusProgress::default();
        self.run = Some(Run {
            publish_slot,
            key: Some(key),
        });
        self.funding.published = Some(key);
        self.complete_on_load = false;
    }

    /// Runs the census's share of window slot `chunk` (an hour's slot index), or what remains
    /// at its last slot or `chunk` past the window. Returns whether the census is done.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn step(
        &mut self,
        inputs: &mut CensusInputs,
        network: &TransitNetwork,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        chunk: u64,
    ) -> bool {
        let slots_left = SLOTS_PER_HOUR.saturating_sub(chunk);
        let budget = if slots_left <= 1 {
            usize::MAX
        } else {
            let (buildings, households) = inputs.front_len();
            self.progress
                .remaining(buildings, households)
                .div_ceil(slots_left as usize)
                .max(1)
        };
        self.funding.step_from_inputs(
            inputs,
            &mut self.progress,
            network,
            graph,
            catalog,
            tuning,
            budget,
        )
    }

    /// Swaps the finished census into `live`, which becomes this census's storage, and keeps
    /// the fiscal ceilings `live` registered for services appended after the copy, below
    /// `buildings`. O(appended buildings).
    pub(crate) fn publish_into(&mut self, live: &mut JobFunding, buildings: usize) {
        debug_assert!(self.progress.is_done());
        self.funding.adopt_pending_services(live, buildings);
        std::mem::swap(&mut self.funding, live);
        self.run = None;
        self.progress = CensusProgress::default();
    }

    /// Drops the census in progress, as when the census ran at once without the slots.
    pub(crate) fn abandon(&mut self) {
        self.run = None;
        self.progress = CensusProgress::default();
        self.complete_on_load = false;
    }

    /// The saved form under the live `key`; `None` without a census in progress.
    pub(crate) fn save(&self, key: CensusKey) -> Option<SavedRollingCensus> {
        self.run.map(|run| SavedRollingCensus {
            publish_slot: run.publish_slot,
            current: run.key == Some(key),
        })
    }

    /// A loaded census: a current one is recomputed by [`Self::complete_loaded`], a stale one
    /// restarts at the next window slot or runs whole at its publishing slot.
    pub(crate) fn restore(saved: Option<SavedRollingCensus>) -> Self {
        Self {
            run: saved.map(|saved| Run {
                publish_slot: saved.publish_slot,
                key: None,
            }),
            complete_on_load: saved.is_some_and(|saved| saved.current),
            ..Self::default()
        }
    }

    /// Recomputes a census that was current at the save from the loaded copy, whole: the
    /// uninterrupted game finishes the same census over its slots. Call once the loaded
    /// network, allocator and entrances are rebuilt. O(census).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn complete_loaded(
        &mut self,
        inputs: &mut CensusInputs,
        live: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        key: CensusKey,
    ) {
        if !std::mem::take(&mut self.complete_on_load) {
            return;
        }
        let Some(run) = self.run else {
            return;
        };
        if !inputs.is_read() {
            return;
        }
        inputs.bind_loaded(live, graph, &network.lane_system, catalog);
        self.begin(run.publish_slot, key);
        self.funding.step_from_inputs(
            inputs,
            &mut self.progress,
            network,
            graph,
            catalog,
            tuning,
            usize::MAX,
        );
    }
}

#[cfg(test)]
impl RollingCensus {
    /// The census being built.
    pub(crate) fn funding(&self) -> &JobFunding {
        &self.funding
    }
}
