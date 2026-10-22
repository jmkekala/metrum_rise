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
//! **Costs** (`SIM-02-K`). Work is counted in work units (`hiring/steps.rs`), never measured
//! time. Each window slot gets ceil(R / D) work units, at least one. R is the work
//! left: per step, the previous census's work in the step less this census's, summed (a step
//! that takes more than before leaves the other steps' plans as they were), or, if more, the
//! estimate of
//! [`CensusProgress::remaining_work`] (the rows left at their weights, a serial sweep in
//! progress at its observed rate, as many closure passes as the previous census took). D is
//! max(S − [`SLACK_SLOTS`], S / 4, 1), S the window slots left: the census plans to finish
//! [`SLACK_SLOTS`] slots before the window ends, and from S = 128 on, work it did not plan for
//! spreads over the slots left instead of landing in one. The window's last slot gets all that
//! remains, as the whole-census fallback at publication does.
//! A step that is not sliced (the starts of the route quotes and the input-source reset and
//! the reset's supplier index entries, the consumer-market refresh's first phase and its
//! finish's decision and end, setup's two joins, repair, business seeding's start, its
//! layout's start, index entries and account availability, propagation) runs whole in the
//! slot it starts in. A restart repeats the work done so far.

use super::{CensusInputs, CensusProgress, FILL_SLOTS, JobFunding, StepWork};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::scheduler::{SLOTS_PER_HOUR, SLOTS_PER_MINUTE};
use crate::simulation::economy::definitions::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};

/// First slot of the hour at which the census runs: the copy has filled over the slots before.
pub(crate) const CENSUS_WINDOW_START: u64 = FILL_SLOTS;

/// Window slots the census plans to leave unused at the window's end, for a census that needs
/// more work than the previous one took (`SIM-02-K`): a fifth of the window.
pub(crate) const SLACK_SLOTS: u64 = 96;

/// First slot of the hour at which building and household swap-removes run (`SIM-02-J`): the
/// hour's planning slots, up to the actions slot in minute 3, read the census published at
/// slot 0 before it. It is minute 4's first slot, a game minute after the actions slot's setup
/// and half a minute after its plan (`SIM-02-K`).
pub(crate) const REMOVAL_WINDOW_START: u64 = 4 * SLOTS_PER_MINUTE;

/// First slot of the hour at which a finished census's book is prepared for the planning slots
/// of the hour it serves (`SIM-02-K`): minute 59's first slot. Every census refills its book,
/// whose site rows were last merged and laid out two hours before, at its previous planning;
/// preparing it this late leaves few building changes for the residential slot to merge again,
/// and a game minute before that slot, so at 32× the two never share a frame.
pub(crate) const PLANNING_PREPARATION_SLOT: u64 = SLOTS_PER_HOUR - SLOTS_PER_MINUTE;
const _: () = assert!(PLANNING_PREPARATION_SLOT >= CENSUS_WINDOW_START);

/// Slot of the hour at which a finished census's book has its planning pass rows and its
/// supplier rows built (`SIM-02-K`), half a minute before the preparation's first slot: a
/// lending that opens and closes a pass and runs the supplier rows' factory without their
/// layout, so the slot that lays them out keeps both and recomputes only the rows of the
/// buildings changed since, as every lending of the book the previous pass started on does.
pub(crate) const PLANNING_PREPARATION_ROWS_SLOT: u64 = PLANNING_PREPARATION_SLOT - 8;
const _: () = assert!(PLANNING_PREPARATION_ROWS_SLOT >= CENSUS_WINDOW_START);

/// First slot of the hour at which the preparation's second half runs (`SIM-02-K`): the first
/// slot lays the supplier rows out alone, and this one, nine slots later, everything else. A
/// 32× frame crosses about 8.5 slots, so the two halves never share a frame.
pub(crate) const PLANNING_PREPARATION_REST_SLOT: u64 = PLANNING_PREPARATION_SLOT + 9;
const _: () = assert!(PLANNING_PREPARATION_REST_SLOT < SLOTS_PER_HOUR);

/// Slot of the hour at which a prepared census book's business finance masks are refreshed
/// (`SIM-02-K`), a quarter minute after the second half, which leaves them when it runs in its
/// own slot; a second half run later refreshes them itself. Masks only prune unclaimable
/// buyers, so where they are refreshed changes no result.
pub(crate) const PLANNING_PREPARATION_MASKS_SLOT: u64 = PLANNING_PREPARATION_REST_SLOT + 4;
const _: () = assert!(PLANNING_PREPARATION_MASKS_SLOT < SLOTS_PER_HOUR);

/// Whether building and household swap-removes may run at absolute `slot` (`SIM-02-J`): slots
/// [`REMOVAL_WINDOW_START`] to [`CENSUS_WINDOW_START`] − 1 of its hour. A removal earlier in the
/// hour would invalidate the published books the planning slots read; one in the census window
/// would restart the census. Over these slots the input copy is filling, and it mirrors each
/// swap-remove in O(1) (`CensusInputs::remove_building`, `CensusInputs::remove_household`).
pub(crate) fn removals_allowed(slot: u64) -> bool {
    (REMOVAL_WINDOW_START..CENSUS_WINDOW_START).contains(&(slot % SLOTS_PER_HOUR))
}

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
    // Closure passes the last finished census took, which the next one's slices plan for;
    // slicing never changes a census's result.
    expected_passes: usize,
    // Work units the last finished census used in each step, which the next one's slices plan
    // for (`SIM-02-K`).
    expected_work: StepWork,
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

    /// The finished census's book while no planning pass has prepared it since the census
    /// started (`SIM-02-K`). O(1).
    pub(crate) fn unprepared_finished_book(&mut self) -> Option<&mut JobFunding> {
        (self.progress.is_done() && !self.funding.planning_prepared).then_some(&mut self.funding)
    }

    /// The finished census's book, prepared for planning or not (`SIM-02-K`). O(1).
    pub(crate) fn finished_book(&mut self) -> Option<&mut JobFunding> {
        self.progress.is_done().then_some(&mut self.funding)
    }

    /// Gives census storage that holds no census, as after loading, a copy of the live books
    /// `live` (`SIM-02-K`): an uninterrupted game's census storage holds the previous
    /// published census, with the planning storage, site rows and walking proofs its planning
    /// passes prepared, so the first census after loading finishes on warm storage rather
    /// than on fresh storage that its first planning pass grows in one slot. What a census's
    /// storage held never changes its result: what it reuses from earlier content is keyed by
    /// exactly its inputs, as with the previous census an uninterrupted game's storage holds.
    /// The copy keeps the live books' capacity for the arenas a census refills (the business
    /// receipts and the consumer claim staging): [`Clone`] gives a copy exact to its rows, so
    /// the next census grew them and the planning slot after it grew the journals they size.
    /// Call at load only; a finished census keeps its book. O(books) once.
    pub(crate) fn warm_storage_from(&mut self, live: &JobFunding) -> Option<&mut JobFunding> {
        if self.progress.is_done() || self.funding.planning_prepared {
            return None;
        }
        self.funding.clone_from(live);
        self.funding.reserve_census_arenas_like(live);
        Some(&mut self.funding)
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
        self.funding.set_published(Some(key));
        // The census refills the book: it is prepared again once finished (`SIM-02-K`).
        self.funding.planning_prepared = false;
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
            u64::MAX
        } else {
            let (buildings, households) = inputs.front_len();
            let rows = self
                .progress
                .remaining_work(buildings, households, self.expected_passes);
            let planned = self.progress.planned_work(&self.expected_work).max(rows);
            let divisor = slots_left
                .saturating_sub(SLACK_SLOTS)
                .max(slots_left / 4)
                .max(1);
            planned.div_ceil(divisor).max(1)
        };
        let done = self.funding.step_from_inputs(
            inputs,
            &mut self.progress,
            network,
            graph,
            catalog,
            tuning,
            budget,
        );
        if done {
            self.expected_passes = self.progress.passes();
            self.expected_work = self.progress.step_work();
        }
        done
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
            u64::MAX,
        );
        // As the uninterrupted game records when the same census finishes (`Self::step`, which
        // the whole run at publication goes through too).
        self.expected_passes = self.progress.passes();
        self.expected_work = self.progress.step_work();
    }

    /// Recomputes, at load, the census the hour's first slot published into `live`, from the
    /// restored front it read (`SIM-02-I`): the hour's planning slots, permit epoch, workforce
    /// reset and project reservations read it, and the books are a function of the copy and
    /// the network. As [`Self::publish_into`] left it: the census, then `adopted`, the service
    /// ceilings it adopted for buildings appended after the copy; the buildings keep their
    /// saved funded jobs and activity. Call once the loaded network, allocator and entrances
    /// are rebuilt. O(census).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn restore_published(
        &mut self,
        inputs: &mut CensusInputs,
        live: &mut JobFunding,
        adopted: &[u32],
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        key: CensusKey,
    ) {
        debug_assert!(
            self.run.is_none(),
            "no census runs before the hour's window"
        );
        inputs.bind_loaded(allocator, graph, &network.lane_system, catalog);
        let mut progress = CensusProgress::default();
        let mut funding = JobFunding::default();
        funding.step_from_inputs(
            inputs,
            &mut progress,
            network,
            graph,
            catalog,
            tuning,
            u64::MAX,
        );
        inputs.consume();
        inputs.keep_published_front();
        funding.service_capacities.extend_from_slice(adopted);
        funding.set_published(Some(key));
        *live = funding;
        self.expected_passes = progress.passes();
        self.expected_work = progress.step_work();
    }
}

impl RollingCensus {
    /// Gives the census's own book `live`'s walking proofs if it has none
    /// ([`ConsumerMarket::adopt_walking_proofs`](crate::simulation::economy::customers::ConsumerMarket::adopt_walking_proofs)).
    /// A function of the network alone, so no census result changes. O(network) once, at load.
    pub(crate) fn adopt_walking_proofs(&mut self, live: &JobFunding) {
        self.funding.customers.adopt_walking_proofs(&live.customers);
    }
}

#[cfg(test)]
impl RollingCensus {
    /// The census being built.
    pub(crate) fn funding(&self) -> &JobFunding {
        &self.funding
    }
}
