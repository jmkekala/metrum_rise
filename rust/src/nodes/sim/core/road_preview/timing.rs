// SPDX-License-Identifier: GPL-2.0-only

//! Optional fixed-size preview diagnostics. All clocks stay in Rust's Instant domain.

use std::time::Instant;

/// Disjoint worker stages; excludes bridge export and frontend rendering.
#[derive(Clone, Debug)]
pub(crate) struct RoadPreviewTiming {
    /// Mailbox submission to worker receipt.
    pub(crate) queue_ms: f64,
    /// Worker free (previous publication or abandonment) to this receipt; outside `worker_ms`.
    pub(crate) idle_ms: f64,
    /// Immutable context read-lock acquisition and Arc copies.
    pub(crate) context_ms: f64,
    /// Candidate preparation, junction solve and planned road mesh construction.
    pub(crate) road_ms: f64,
    /// Retained-cache lookup and optional earthwork preparation.
    pub(crate) earthworks_ms: f64,
    /// Waiting to acquire the simulation lock for local inputs.
    pub(crate) core_wait_ms: f64,
    /// Local terrain sites and source mesh Arc capture under the simulation lock.
    pub(crate) capture_ms: f64,
    /// Terrain compilation outside the simulation lock.
    pub(crate) terrain_ms: f64,
    /// Retained geometry filtering and cache publication.
    pub(crate) retained_ms: f64,
    /// Receipt through final worker stage; includes waits, not worker CPU time.
    pub(crate) worker_ms: f64,
    /// Last completed worker stage, used to measure result age at bridge polling.
    pub(crate) completed_at: Instant,
    started_at: Instant,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disjoint_stages_account_for_the_worker_interval() {
        let mut timing = RoadPreviewTiming::new(Instant::now(), Instant::now());
        timing.finish_context();
        timing.finish_road();
        timing.finish_earthworks();
        timing.finish_core_wait();
        timing.finish_capture();
        timing.finish_terrain();
        timing.finish_retained();
        let sum = timing.context_ms
            + timing.road_ms
            + timing.earthworks_ms
            + timing.core_wait_ms
            + timing.capture_ms
            + timing.terrain_ms
            + timing.retained_ms;
        assert!((sum - timing.worker_ms).abs() < 1e-9);
        assert!(timing.completed_at >= timing.started_at);
    }

    #[test]
    fn reused_products_leave_skipped_stages_zero() {
        let mut timing = RoadPreviewTiming::new(Instant::now(), Instant::now());
        timing.finish_context();
        timing.finish_road();
        timing.finish_earthworks();
        timing.finish_retained();
        assert_eq!(timing.core_wait_ms, 0.0);
        assert_eq!(timing.capture_ms, 0.0);
        assert_eq!(timing.terrain_ms, 0.0);
    }
}

impl RoadPreviewTiming {
    pub(super) fn new(enqueued_at: Instant, free_since: Instant) -> Self {
        let now = Instant::now();
        Self {
            queue_ms: now.duration_since(enqueued_at).as_secs_f64() * 1000.0,
            idle_ms: now.duration_since(free_since).as_secs_f64() * 1000.0,
            context_ms: 0.0,
            road_ms: 0.0,
            earthworks_ms: 0.0,
            core_wait_ms: 0.0,
            capture_ms: 0.0,
            terrain_ms: 0.0,
            retained_ms: 0.0,
            worker_ms: 0.0,
            completed_at: now,
            started_at: now,
        }
    }

    fn lap(&mut self) -> f64 {
        let now = Instant::now();
        let elapsed = now.duration_since(self.completed_at).as_secs_f64() * 1000.0;
        self.completed_at = now;
        self.worker_ms = now.duration_since(self.started_at).as_secs_f64() * 1000.0;
        elapsed
    }

    pub(super) fn finish_context(&mut self) {
        self.context_ms = self.lap();
    }
    pub(super) fn finish_road(&mut self) {
        self.road_ms = self.lap();
    }
    pub(super) fn finish_earthworks(&mut self) {
        self.earthworks_ms = self.lap();
    }
    pub(super) fn finish_core_wait(&mut self) {
        self.core_wait_ms = self.lap();
    }
    pub(super) fn finish_capture(&mut self) {
        self.capture_ms = self.lap();
    }
    pub(super) fn finish_terrain(&mut self) {
        self.terrain_ms = self.lap();
    }
    pub(super) fn finish_retained(&mut self) {
        self.retained_ms = self.lap();
    }
}
