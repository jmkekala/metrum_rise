// SPDX-License-Identifier: GPL-2.0-only

//! The hourly funding census on the households' side (`SIM-02-I`,
//! `docs/simulation.md#pure-jobs`): the input copy over the hour's first half, the census in
//! steps over its second half and its publication at the next hour's first slot, before the
//! operational hour, so the hour's readers (planning, restock and visits, job search) all see
//! it. See `economy/hiring/rolling.rs`.

use super::HouseholdSystem;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
use crate::simulation::economy::definitions::{
    RuntimeEconomyCatalog, load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use crate::simulation::economy::hiring::{
    CENSUS_WINDOW_START, CensusKey, FrozenCensusParameters, JobFunding, WindowAction,
};
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;

impl HouseholdSystem {
    /// The live city's census key (`SIM-02-I`). O(1).
    pub(crate) fn census_key(&self, network: &TransitNetwork) -> CensusKey {
        CensusKey::live(self.structure, self.census_policy_revision, network)
    }

    /// Follows a change of the fiscal policy or service funding the census freezes
    /// (`SIM-02-I`): the finished copy takes `fiscal` and `electricity_funding`, and a census
    /// in progress restarts at its next window slot, or runs whole at its publishing slot.
    /// The published census stands until the next publication. O(1).
    pub(crate) fn follow_census_policy(
        &mut self,
        fiscal: CityFiscalPolicy,
        electricity_funding: f32,
    ) {
        self.census_inputs
            .follow_policy(fiscal, electricity_funding);
        self.census_policy_revision = self.census_policy_revision.wrapping_add(1);
    }

    /// Slot `slot`'s chunk of the census input copy (`SIM-02-I`); `parameters` are frozen if it
    /// is the fill's last.
    pub(crate) fn advance_census_inputs(
        &mut self,
        slot: u64,
        allocator: &mut BuildingAllocator,
        logistics: &ShipmentSystem,
        parameters: FrozenCensusParameters,
    ) {
        self.census_inputs
            .advance(slot, allocator, &self.households, logistics, parameters);
    }

    /// Whether the census window has not started this hour's census yet at `slot`: the caller
    /// publishes the frontage prices first.
    pub(crate) fn census_starts_at(&self, slot: u64, key: CensusKey) -> bool {
        self.census_run.window_action(publish_slot(slot), key) == WindowAction::Start
    }

    /// Window slot `slot`'s share of the hour's census (`SIM-02-I`): it starts at the first
    /// window slot it sees, starts again after a change of `key` (the live city's), then runs
    /// its share of the steps. `fallback` holds the parameters of a copy taken now, frozen as a
    /// finished copy's, when the fill produced none. Costs: see `hiring/rolling.rs`; a start
    /// adds the copy's preparation, O(changed and created rows + B).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn advance_census(
        &mut self,
        slot: u64,
        allocator: &mut BuildingAllocator,
        logistics: &ShipmentSystem,
        network: &TransitNetwork,
        graph: &RegionGraph,
        fallback: FrozenCensusParameters,
        key: CensusKey,
    ) {
        let chunk = slot % SLOTS_PER_HOUR;
        debug_assert!(chunk >= CENSUS_WINDOW_START);
        let publish = publish_slot(slot);
        let action = self.census_run.window_action(publish, key);
        if action == WindowAction::Continue && self.census_run.is_done() {
            return;
        }
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        match action {
            WindowAction::Start => {
                crate::debug::work::add(crate::debug::work::Work::LiveCensuses, 1);
                self.start_census(
                    allocator, logistics, network, graph, &catalog, fallback, publish, key,
                );
            }
            WindowAction::Restart => {
                crate::debug::work::add(crate::debug::work::Work::CensusRestarts, 1);
                #[cfg(test)]
                {
                    self.census_run.restarts += 1;
                }
                self.start_census(
                    allocator, logistics, network, graph, &catalog, fallback, publish, key,
                );
            }
            WindowAction::Continue => {}
        }
        self.census_run.step(
            &mut self.census_inputs,
            network,
            graph,
            &catalog,
            &tuning,
            chunk,
        );
    }

    // Prepares the copy for a census starting now and starts it. The copy takes the live
    // entrances, which a stale cache (after a removal) would leave older than the copied rows,
    // so the cache is brought up to date first: a loaded game derives the same entrances from
    // the copied rows. O(changed, deferred and created rows + B); a copy taken now adds
    // O(B + H + S), a stale cache O(B) derivations.
    #[allow(clippy::too_many_arguments)]
    fn start_census(
        &mut self,
        allocator: &mut BuildingAllocator,
        logistics: &ShipmentSystem,
        network: &TransitNetwork,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        fallback: FrozenCensusParameters,
        publish: u64,
        key: CensusKey,
    ) {
        if allocator.entrances_dirty || allocator.entrances.len() != allocator.buildings.len() {
            allocator.rebuild_entrance_cache(graph, &network.lane_system);
        }
        allocator.prepare_production_changes();
        self.census_inputs
            .prepare(allocator, &self.households, logistics, catalog, fallback);
        self.census_run.begin(publish, key);
    }

    /// Whether the census publishing at hour start `slot` is finished and current for `key`;
    /// otherwise [`Self::publish_census`] runs it whole, and the caller publishes the frontage
    /// prices first, as at a census start.
    pub(crate) fn census_ready(&self, slot: u64, key: CensusKey) -> bool {
        self.census_run.is_ready(slot, key)
    }

    /// The finished census's book while no planning pass has prepared it, as after loading
    /// (`SIM-02-K`). O(1).
    pub(crate) fn unprepared_census_book(&mut self) -> Option<&mut JobFunding> {
        self.census_run.unprepared_finished_book()
    }

    /// The finished census's book, prepared or not (`SIM-02-K`). O(1).
    pub(crate) fn finished_census_book(&mut self) -> Option<&mut JobFunding> {
        self.census_run.finished_book()
    }

    /// At load, gives census storage that holds no census a copy of the live books
    /// (`SIM-02-K`, [`RollingCensus::warm_storage_from`](crate::simulation::economy::hiring::RollingCensus::warm_storage_from))
    /// and returns the copy, for the caller to prepare as it prepared the live books.
    /// O(books) once.
    pub(crate) fn warm_census_storage(&mut self) -> Option<&mut JobFunding> {
        self.census_run.warm_storage_from(&self.job_funding)
    }

    /// Publishes the hour's census at its first slot `slot`, before the operational hour
    /// (`SIM-02-I`): swaps the census the window built into the live one and copies funded jobs
    /// and activity onto the buildings. A census that is missing, unfinished or stale for the
    /// live `key` runs whole first, on a copy taken now with `fallback` if the fill produced
    /// none. O(B) for the copy onto the buildings, plus the whole census when it runs here.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn publish_census(
        &mut self,
        slot: u64,
        allocator: &mut BuildingAllocator,
        logistics: &ShipmentSystem,
        network: &TransitNetwork,
        graph: &RegionGraph,
        fallback: FrozenCensusParameters,
        key: CensusKey,
    ) {
        debug_assert_eq!(slot % SLOTS_PER_HOUR, 0);
        if !self.census_run.is_ready(slot, key) {
            let catalog = load_runtime_economy_catalog().unwrap_or_else(|err| {
                panic!("could not load built-in runtime economy catalog: {err}")
            });
            let tuning = load_runtime_economy_tuning().unwrap_or_else(|err| {
                panic!("could not load built-in economy runtime tuning: {err}")
            });
            crate::debug::work::add(
                if self.census_run.has_run(slot) {
                    crate::debug::work::Work::CensusRestarts
                } else {
                    crate::debug::work::Work::LiveCensuses
                },
                1,
            );
            self.start_census(
                allocator, logistics, network, graph, &catalog, fallback, slot, key,
            );
            self.census_run.step(
                &mut self.census_inputs,
                network,
                graph,
                &catalog,
                &tuning,
                SLOTS_PER_HOUR,
            );
        }
        self.census_run
            .publish_into(&mut self.job_funding, allocator.buildings.len());
        self.census_inputs.consume();
        // The hour's planning slots read this census; a save before its actions slot keeps
        // the copy for the load to recompute it.
        self.census_inputs.keep_published_front();
        self.publish_funded_job_capacities(allocator);
    }

    /// Restores, once the loaded world is ready, the census the saved hour's first slot
    /// published, from the copy and ceilings a save before the hour's actions slot kept
    /// (`SIM-02-I`, [`RollingCensus::restore_published`](crate::simulation::economy::hiring::RollingCensus::restore_published)).
    /// O(census).
    pub(crate) fn restore_published_census(
        &mut self,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        adopted: &[u32],
    ) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let key = self.census_key(network);
        self.census_run.restore_published(
            &mut self.census_inputs,
            &mut self.job_funding,
            adopted,
            allocator,
            network,
            graph,
            &catalog,
            &tuning,
            key,
        );
    }

    /// Recomputes, once the loaded world is ready, a census that was current at the save
    /// (`SIM-02-I`); the uninterrupted game finishes the same census over its window.
    pub(crate) fn complete_loaded_census(
        &mut self,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let key = self.census_key(network);
        self.census_run.complete_loaded(
            &mut self.census_inputs,
            allocator,
            network,
            graph,
            &catalog,
            &tuning,
            key,
        );
        // The census's own book takes the walking proofs the loaded census built, as an
        // uninterrupted game's census storage, the previous published census, holds them
        // (`SIM-02-K`); the first window after loading then has no network-scale build.
        self.census_run.adopt_walking_proofs(&self.job_funding);
    }
}

#[cfg(test)]
impl HouseholdSystem {
    /// A census run at once, into fresh storage, on the front the window's census reads.
    pub(crate) fn census_of_front_for_test(
        &mut self,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> crate::simulation::economy::hiring::JobFunding {
        self.census_of_front_on_for_test(Default::default(), network, graph)
    }

    /// The same census run at once into `funding`, storage holding an earlier book.
    pub(crate) fn census_of_front_on_for_test(
        &mut self,
        mut funding: crate::simulation::economy::hiring::JobFunding,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> crate::simulation::economy::hiring::JobFunding {
        let catalog = load_runtime_economy_catalog().unwrap();
        let tuning = load_runtime_economy_tuning().unwrap();
        funding.step_from_inputs(
            &mut self.census_inputs,
            &mut Default::default(),
            network,
            graph,
            &catalog,
            &tuning,
            u64::MAX,
        );
        funding
    }
}

// The hour start at which the census of `slot`'s window is published: the next hour's first.
fn publish_slot(slot: u64) -> u64 {
    (slot / SLOTS_PER_HOUR + 1) * SLOTS_PER_HOUR
}
