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
use crate::simulation::core::pure_job::PureJob;
use crate::simulation::pathing::cch::CchGraph;

/// Frames a routing rebuild has before its adoption frame: the frames after the commit that
/// hold, about 100 ms at 60 Hz. The simulated game does not depend on it, since held frames
/// advance nothing; it trades the adoption latency against how often the adoption frame
/// waits for a late build.
pub(crate) const CCH_ADOPTION_FRAMES: u32 = 6;

/// A routing hierarchy rebuild waiting for its adoption frame.
pub(super) struct PendingCch {
    // `None` once a reader installed the result before the adoption frame.
    job: Option<PureJob<CchGraph>>,
    frames_left: u32,
}

impl TransitNetwork {
    /// Starts rebuilding the routing hierarchy from `graph`, the live graph's immutable
    /// snapshot, replacing any rebuild still pending; the hold restarts. The old hierarchy stays
    /// installed but unread until adoption. O(1) here; the build runs on the shared pool.
    pub(crate) fn schedule_cch_rebuild(&mut self, graph: Arc<RegionGraph>) {
        #[cfg(test)]
        let delay = self.cch_job_delay;
        let job = PureJob::spawn(move || {
            #[cfg(test)]
            std::thread::sleep(delay);
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
    }

    /// Installs a pending rebuild now, waiting for it (or running it here) if it is not done,
    /// with a fresh build generation. The hold continues to its adoption frame. O(1) without a
    /// pending result.
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
        self.cch_job_delay = delay;
    }
}
