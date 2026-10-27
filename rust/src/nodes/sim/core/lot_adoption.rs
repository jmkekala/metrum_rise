// SPDX-License-Identifier: GPL-2.0-only

//! A commit's cell lots, planned off the commit and adopted the next frame (`ROAD-68-G` part 7,
//! `docs/roads.md` question 33 (c)), as D7 adopts the routing rebuild.
//!
//! A commit whose edit queued lot regions takes them at its entrance stage and publishes its
//! road without planning them. After the publication it starts a pure job on the background
//! pool that plans the regions in order on the road tool view's copy of the cell store, the
//! parcel store as the commit left it (shared copy-on-write), the view's graph and roadbed and
//! snapshots of the fields and explicit cell sites: the inputs the commit's own plan read. The
//! attachment repair and entrance rebuild that followed the plan wait with it.
//!
//! The first frame after the commit adopts the plan, waiting for it outside the pools if it is
//! late; that frame is the routing hold's, which advances nothing (D7). Every editor path that
//! reads or writes the lots, parcels or building attachments before that frame adopts the plan
//! first ([`SimCore::adopt_pending_cell_lots`]); a load drops it. So the adopted zoning,
//! attachments and entrances equal what planning at the commit would have left, bitwise, and
//! nothing simulated ever reads the state between. Readers that only show the lots (the render
//! snapshot, the cell overlay) show them one frame later.

use std::collections::HashSet;
use std::sync::Arc;

use super::road_tool_context::RoadToolPublication;
use super::state::SimCore;
use crate::nodes::sim::zoning::footprint_blocked;
use crate::simulation::core::pure_job::{JobCompletion, JobStart, PureJob};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::RoadbedSystem;
use crate::simulation::zoning::cells::{CellBounds, CellStore, SharedMaps};
use crate::simulation::zoning::{CellLotGeneration, PlannedCellLots};

/// A commit's deferred cell-lot plan, running or finished, with what the commit left to do
/// after it.
pub(crate) struct PendingCellLots {
    job: PureJob<PlannedCellLots>,
    // The commit's dirty edges, for its attachment repair and entrance rebuild.
    dirty: HashSet<usize>,
    // Debug builds check at adoption that nothing wrote the parcel or cell store since the
    // plan read them: the store's identity (a write copies it) and the cell revision.
    #[cfg(debug_assertions)]
    guard: (usize, u64),
}

/// What the road tool view of the commit gives a deferred plan to read.
pub(crate) struct CommitCellLotInputs<'a> {
    /// The graph the commit published.
    pub(crate) graph: &'a Arc<RegionGraph>,
    /// The published view's roadbed and cell copy, when the commit published one.
    pub(crate) view: Option<(&'a Arc<RoadbedSystem>, &'a CellStore<SharedMaps>)>,
}

/// Test switches of the deferral (`ROAD-68-G` part 7).
#[cfg(test)]
#[derive(Default)]
pub(crate) struct CellLotTestHooks {
    /// Plan the lots in the commit, as before the deferral: the reference the deferred lots
    /// must equal.
    pub(crate) in_commit: bool,
    /// Every deferred plan's job sleeps this long first, so the edit after a commit finds it
    /// running.
    pub(crate) job_delay: Option<std::time::Duration>,
}

/// What the adoption timed, for the frame report.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CellLotAdoption {
    /// Whether a plan was adopted.
    pub(crate) adopted: bool,
    /// Installing the plan, reattaching, the repair and the entrance rebuild, in ms.
    pub(crate) install_ms: f64,
    /// Of `install_ms`: the plan's cell entries and parcel calls (`ROAD-68-G` part 8).
    pub(crate) plan_install_ms: f64,
    /// Of `install_ms`: reattaching the lots' occupants.
    pub(crate) reattach_ms: f64,
    /// Of `install_ms`: the commit's attachment repair and entrance rebuild.
    pub(crate) repair_ms: f64,
    /// Publishing the road tool's view of the adopted lots, in ms.
    pub(crate) publish_ms: f64,
}

impl SimCore {
    /// The lot regions a commit's edit queued, taken at its entrance stage for a deferred plan,
    /// with the site queries prepared as the plan reads them; empty while a road edit is
    /// staged or nothing is queued, when the commit's entrances are rebuilt at once.
    /// `O(regions)` plus the site index repair the commit's own plan would have made.
    pub(super) fn take_commit_cell_lot_regions(&mut self) -> Vec<CellBounds> {
        #[cfg(test)]
        if self.cell_lot_hooks.in_commit {
            return Vec::new();
        }
        if self.transit_network.road_edit_is_staged() || !self.zoning.has_dirty_cell_lots() {
            return Vec::new();
        }
        let regions = self.zoning.take_dirty_cell_lot_regions();
        if !regions.is_empty() {
            self.prepare_cell_site_queries();
        }
        regions
    }

    /// Starts the deferred plan of `regions` after the commit's publication, on the inputs the
    /// commit left (module documentation), and the commit's routing rebuild (`routing`) when
    /// the plan ends (`ROAD-68-G` part 8). Without a published view it copies the cell store
    /// and shares the live roadbed. `O(regions + explicit sites)` with a view.
    pub(super) fn spawn_commit_cell_lots(
        &mut self,
        regions: Vec<CellBounds>,
        dirty: HashSet<usize>,
        inputs: CommitCellLotInputs<'_>,
        routing: JobStart,
    ) {
        let (roadbed, cells) = match inputs.view {
            Some((roadbed, cells)) => (Arc::clone(roadbed), cells.clone()),
            None => (
                Arc::new(self.transit_network.roadbed.snapshot()),
                CellStore::copy_of(&self.zoning.cells),
            ),
        };
        let graph = Arc::clone(inputs.graph);
        let fields = Arc::clone(&self.allocator.field_clearance);
        let sites = self.allocator.explicit_site_snapshot();
        let sizes = self.cached_cell_lot_sizes().to_vec();
        let cell_m = self.config.zone_cell_m;
        #[cfg(debug_assertions)]
        let guard = (self.zoning.parcels.identity(), self.zoning.cells.revision());
        let deferred = self.zoning.defer_cell_lots(cells, regions, sizes);
        #[cfg(test)]
        let delay = self.cell_lot_hooks.job_delay;
        let job = PureJob::spawn(move || {
            #[cfg(test)]
            if let Some(delay) = delay {
                std::thread::sleep(delay);
            }
            let planned = deferred.plan(&graph, |corners| {
                footprint_blocked(&fields, &sites, &roadbed, cell_m, corners)
            });
            routing.start();
            planned
        });
        self.pending_cell_lots = Some(PendingCellLots {
            job,
            dirty,
            #[cfg(debug_assertions)]
            guard,
        });
    }

    /// Adopts a pending deferred lot plan: installs it, reattaches the lots' occupants, runs the
    /// commit's attachment repair and entrance rebuild, and publishes the road tool's view of
    /// the lots, which the caller hands to the road tool. Waits for a late plan: call it outside
    /// the extension's pools, or after [`Self::cell_lot_completion`] finished it. O(1) without
    /// a pending plan; otherwise the plan's install and the commit's repair.
    pub(crate) fn adopt_pending_cell_lots(&mut self) -> CellLotAdoption {
        let Some(pending) = self.pending_cell_lots.take() else {
            return CellLotAdoption::default();
        };
        let start = std::time::Instant::now();
        let planned = pending.job.take();
        #[cfg(debug_assertions)]
        assert_eq!(
            pending.guard,
            (self.zoning.parcels.identity(), self.zoning.cells.revision()),
            "nothing writes the parcels or cells before a deferred lot plan is adopted"
        );
        // The entrance cache's state before the plan, as the commit read it.
        let cache_current = self.entrance_cache_current();
        let lap = std::time::Instant::now();
        let lots = self.zoning.adopt_planned_cell_lots(planned);
        let plan_install_ms = lap.elapsed().as_secs_f64() * 1000.0;
        let lap = std::time::Instant::now();
        self.attach_regenerated_cell_lots(&lots);
        let reattach_ms = lap.elapsed().as_secs_f64() * 1000.0;
        let lap = std::time::Instant::now();
        crate::thread_pools::install(|| {
            self.repair_commit_entrances(&pending.dirty, cache_current, &lots)
        });
        let repair_ms = lap.elapsed().as_secs_f64() * 1000.0;
        let install_ms = start.elapsed().as_secs_f64() * 1000.0;
        let start = std::time::Instant::now();
        self.adopted_road_tool_view = self.publish_road_tool_view();
        CellLotAdoption {
            adopted: true,
            install_ms,
            plan_install_ms,
            reattach_ms,
            repair_ms,
            publish_ms: start.elapsed().as_secs_f64() * 1000.0,
        }
    }

    /// The pending plan's completion, so a caller can finish it outside the extension's pools
    /// (and the sim thread without the core lock) before adopting it. O(1).
    pub(crate) fn cell_lot_completion(&self) -> Option<JobCompletion> {
        self.pending_cell_lots
            .as_ref()
            .map(|pending| pending.job.completion())
    }

    /// Whether a commit's lot plan waits to be adopted.
    pub(crate) fn cell_lots_pending(&self) -> bool {
        self.pending_cell_lots.is_some()
    }

    /// The road tool view an adoption published, for the caller to hand to the road tool.
    pub(crate) fn take_adopted_road_tool_view(&mut self) -> Option<RoadToolPublication> {
        self.adopted_road_tool_view.take()
    }

    // Whether the entrance cache covers every building and nothing marked it dirty.
    pub(super) fn entrance_cache_current(&self) -> bool {
        let allocator = &self.allocator;
        !allocator.entrances_dirty && allocator.entrances.len() == allocator.buildings.len()
    }

    // The commit's attachment repair and entrance update after its lot plan (`ROAD-68-G`): of
    // the buildings the repair visits, the lots the plan reattached and the buildings fronting
    // the lane rebuild closure of `dirty`, whose geometry and lanes changed, only these
    // entrances are derived again. The repair's pass also finds the fronting buildings: a
    // building the repair moves is among its visits, so testing the attachment before the
    // repair finds the same set. A cache already stale before the plan (`cache_current`)
    // rebuilds whole. One parallel `O(B / P)` test plus the local work.
    pub(super) fn repair_commit_entrances(
        &mut self,
        dirty: &HashSet<usize>,
        cache_current: bool,
        lots: &CellLotGeneration,
    ) {
        let closure =
            crate::simulation::network::lanes::LaneSystem::incremental_rebuild_edge_closure(
                &self.region_graph,
                dirty,
            );
        let mut changed = self.allocator.repair_road_attachments_that_may_act(
            &self.region_graph,
            &mut self.zoning,
            |building| closure.contains(&building.edge_idx),
        );
        let lanes = &self.transit_network.lane_system;
        if !cache_current {
            self.allocator
                .rebuild_entrance_cache(&self.region_graph, lanes);
            return;
        }
        let buildings = &self.allocator.buildings;
        changed.extend(lots.reattached.iter().filter_map(|&parcel| {
            let index = self.zoning.parcels.get(parcel)?.occupied_building()?;
            (index < buildings.len()).then_some(index)
        }));
        changed.sort_unstable();
        changed.dedup();
        self.allocator
            .rebuild_entrances_of(&changed, &self.region_graph, lanes);
    }
}
