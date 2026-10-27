// SPDX-License-Identifier: GPL-2.0-only

//! Deferred routing adoption after a topology edit (`docs/roads.md` D7, `ROAD-68-G`).
//!
//! A commit rebuilds the routing hierarchy as a [`PureJob`] on an immutable graph snapshot and
//! adopts it a fixed number of frames later. Until then every route-reading system waits: the
//! simulation frame holds, so the clock, agents and slots do not run on a hierarchy that no
//! longer matches the graph (`docs/simulation.md#pure-jobs`). A late job blocks the adoption
//! frame, so results never depend on machine speed. Outside the frame, a reader that needs
//! routing during the hold installs the result first ([`TransitNetwork::install_pending_cch`]);
//! the clock is held, so installing early changes no simulated outcome.

use std::sync::Arc;
#[cfg(test)]
use std::time::Duration;

use super::graph::RegionGraph;
use super::{TransitNetwork, log_road_connectivity};
use crate::simulation::core::pure_job::{JobCompletion, JobStart, PureJob};
use crate::simulation::pathing::cch::CchGraph;

/// Test-only work a routing rebuild job runs before it builds (`ROAD-68-G`).
#[cfg(test)]
pub(crate) type CchJobPrelude = Arc<dyn Fn() + Send + Sync>;

/// Frames a routing rebuild has before its adoption frame: the frames after the commit that
/// hold. The simulated game does not depend on it, since held frames advance nothing, and the
/// simulation is frozen for whichever is longer, these frames or the build; so the smallest
/// offset, one frame, adds no idle frames to a quick build. The sim thread waits for a late
/// build outside the core lock ([`TransitNetwork::cch_adoption_completion`]), so a short
/// offset never queues Godot's acknowledgements behind it (`ROAD-68-G`).
pub(crate) const CCH_ADOPTION_FRAMES: u32 = 1;

/// A routing hierarchy rebuild waiting for its adoption frame.
pub(super) struct PendingCch {
    // `None` once a reader installed the result before the adoption frame.
    job: Option<PureJob<CchGraph>>,
    frames_left: u32,
}

impl TransitNetwork {
    /// Starts rebuilding the routing hierarchy from `graph`, the live graph's immutable
    /// snapshot, replacing any rebuild still pending; the hold restarts. The old hierarchy stays
    /// installed but unread until adoption. O(1) here; the build runs on the background pool,
    /// beside no frame's work (`crate::thread_pools`).
    pub(crate) fn schedule_cch_rebuild(&mut self, graph: Arc<RegionGraph>) {
        self.schedule_cch_rebuild_unstarted(graph).start();
    }

    /// [`Self::schedule_cch_rebuild`] with the build queued only when the caller starts it
    /// (`ROAD-68-G` part 8: a commit starts it after its lot plan). A build nobody started yet
    /// runs on the thread that first needs it, the adoption frame's caller or a reader that
    /// installs it early, so the result and the frame that adopts it are the same. O(1).
    pub(crate) fn schedule_cch_rebuild_unstarted(&mut self, graph: Arc<RegionGraph>) -> JobStart {
        #[cfg(test)]
        let prelude = self.cch_job_prelude.clone();
        let (job, start) = PureJob::unstarted(move || {
            #[cfg(test)]
            if let Some(prelude) = prelude {
                prelude();
            }
            let cch = CchGraph::build(&graph);
            log_road_connectivity(&graph);
            cch
        });
        // The build covers every marked chunk and customizes on the snapshot's metric.
        self.cch_dirty_chunks.clear();
        self.metric_dirty = false;
        self.pending_cch = Some(PendingCch {
            job: Some(job),
            frames_left: CCH_ADOPTION_FRAMES,
        });
        start
    }

    /// Installs a pending rebuild now, waiting for it (or running it here) if it is not done,
    /// with a fresh build generation. The hold continues to its adoption frame. Call it outside
    /// the extension's pools, or after [`Self::cch_adoption_completion`] finished the job
    /// (`pure_job`).
    /// O(1) without a pending result.
    pub(crate) fn install_pending_cch(&mut self) {
        if let Some(job) = self
            .pending_cch
            .as_mut()
            .and_then(|pending| pending.job.take())
        {
            let mut cch = job.take();
            cch.stamp_generation();
            self.cch_graph = cch;
        }
    }

    /// Advances the routing hold by one simulation frame. Returns true while the frame must not
    /// run; at the adoption frame installs the rebuild (waiting if it is late) and returns
    /// false. Flow fields follow the new hierarchy's generation at that frame. O(1) apart from
    /// the wait.
    pub(crate) fn hold_frame_for_cch(&mut self) -> bool {
        let Some(pending) = &mut self.pending_cch else {
            return false;
        };
        if pending.frames_left > 0 {
            pending.frames_left -= 1;
            return true;
        }
        self.install_pending_cch();
        self.pending_cch = None;
        false
    }

    /// Installs a pending rebuild (waiting for it) and ends the hold, so the next frame runs.
    /// A save does this: held frames advance nothing, so ending them early changes no outcome,
    /// and the saved game then continues frame for frame like the game loaded from it.
    pub(crate) fn settle_cch(&mut self) {
        self.install_pending_cch();
        self.pending_cch = None;
    }

    /// The pending rebuild's completion when the next frame adopts it and it may not have
    /// finished, so the frame's caller can wait for it outside the extension's pools (and the sim
    /// thread without the core lock). O(1).
    pub(crate) fn cch_adoption_completion(&self) -> Option<JobCompletion> {
        let pending = self.pending_cch.as_ref()?;
        if pending.frames_left > 0 {
            return None;
        }
        pending.job.as_ref().map(PureJob::completion)
    }

    /// Whether a routing rebuild is waiting for its adoption frame.
    pub(crate) fn cch_adoption_pending(&self) -> bool {
        self.pending_cch.is_some()
    }

    /// Whether a rebuilt hierarchy waits to be installed.
    pub(crate) fn cch_result_pending(&self) -> bool {
        self.pending_cch
            .as_ref()
            .is_some_and(|pending| pending.job.is_some())
    }

    /// Whether the installed hierarchy is the current graph's: no rebuild result is waiting to
    /// be installed and no edited chunk waits for a rebuild.
    pub(crate) fn cch_current(&self) -> bool {
        self.cch_dirty_chunks.is_empty() && !self.cch_result_pending()
    }

    // A synchronous rebuild of the live graph supersedes a pending one; the hold continues.
    pub(super) fn supersede_pending_cch(&mut self) {
        if let Some(pending) = &mut self.pending_cch {
            pending.job = None;
        }
    }

    /// Delays every later rebuild job by `delay` before it builds, to test that a late job
    /// changes nothing but the wait (`ROAD-68-G`).
    #[cfg(test)]
    pub(crate) fn delay_cch_jobs_for_test(&mut self, delay: Duration) {
        self.cch_job_prelude = Some(Arc::new(move || std::thread::sleep(delay)));
    }

    /// Runs `prelude` in every later rebuild job before it builds (`ROAD-68-G`).
    #[cfg(test)]
    pub(crate) fn prelude_cch_jobs_for_test(&mut self, prelude: CchJobPrelude) {
        self.cch_job_prelude = Some(prelude);
    }
}
