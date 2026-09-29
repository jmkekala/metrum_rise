// SPDX-License-Identifier: GPL-2.0-only

//! Bounded latest-input mailbox for the existing road-preview worker.

use super::RoadPreviewRequest;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

/// Producer with one replaceable pending input, regardless of pointer event rate.
pub(crate) struct RoadPreviewSender {
    pending: Arc<Mutex<Option<RoadPreviewRequest>>>,
    started: Arc<AtomicU64>,
    wake: mpsc::SyncSender<()>,
}

/// Consumer owned by the existing preview worker; sender destruction still terminates it.
pub(crate) struct RoadPreviewReceiver {
    pending: Arc<Mutex<Option<RoadPreviewRequest>>>,
    started: Arc<AtomicU64>,
    wake: mpsc::Receiver<()>,
}

/// Creates one pending-input slot and one bounded wake notification, not a request backlog.
pub(crate) fn road_preview_channel() -> (RoadPreviewSender, RoadPreviewReceiver) {
    let pending = Arc::new(Mutex::new(None));
    let started = Arc::new(AtomicU64::new(0));
    let (wake_tx, wake_rx) = mpsc::sync_channel(1);
    (
        RoadPreviewSender {
            pending: Arc::clone(&pending),
            started: Arc::clone(&started),
            wake: wake_tx,
        },
        RoadPreviewReceiver {
            pending,
            started,
            wake: wake_rx,
        },
    )
}

impl RoadPreviewSender {
    /// Replaces pending work in O(1), without waiting for the running geometry compile.
    pub(crate) fn submit(&self, mut request: RoadPreviewRequest) -> bool {
        request.enqueued_at = crate::debug::is_perf_enabled().then(std::time::Instant::now);
        *self
            .pending
            .lock()
            .expect("road preview input lock poisoned") = Some(request);
        match self.wake.try_send(()) {
            Ok(()) | Err(mpsc::TrySendError::Full(())) => true,
            Err(mpsc::TrySendError::Disconnected(())) => false,
        }
    }

    /// Latest request the worker has taken. Every older request has already published its
    /// result or been abandoned, and every newer one is pending or was displaced unrun.
    pub(crate) fn started(&self) -> u64 {
        self.started.load(Ordering::Acquire)
    }
}

impl RoadPreviewReceiver {
    /// Waits for the latest pending input; redundant wake notifications contain no work.
    pub(crate) fn recv(&self) -> Result<RoadPreviewRequest, mpsc::RecvError> {
        loop {
            self.wake.recv()?;
            if let Some(request) = self.take() {
                return Ok(request);
            }
        }
    }

    /// Permits a completed compile waiting for SimCore to be superseded by fresh input.
    pub(crate) fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<RoadPreviewRequest, mpsc::RecvTimeoutError> {
        self.wake.recv_timeout(timeout)?;
        self.take().ok_or(mpsc::RecvTimeoutError::Timeout)
    }

    // Publishes the start under the input lock, so a submission never observes a taken
    // request as still pending. The previous result is already published at this point.
    fn take(&self) -> Option<RoadPreviewRequest> {
        let mut pending = self
            .pending
            .lock()
            .expect("road preview input lock poisoned");
        let request = pending.take()?;
        self.started.store(request.request_id, Ordering::Release);
        Some(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: u64) -> RoadPreviewRequest {
        RoadPreviewRequest {
            enqueued_at: None,
            include_terrain: false,
            request_id: id,
            surface_generation: 1,
            points: Vec::new(),
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
        }
    }

    #[test]
    fn input_bursts_keep_only_the_latest_request() {
        let (sender, receiver) = road_preview_channel();
        for id in 1..=10000 {
            assert!(sender.submit(request(id)));
        }
        assert_eq!(receiver.recv().unwrap().request_id, 10000);
        assert!(matches!(
            receiver.recv_timeout(Duration::ZERO),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        // New input after the slot was consumed must wake the worker again.
        assert!(sender.submit(request(10001)));
        assert_eq!(receiver.recv().unwrap().request_id, 10001);
    }

    #[test]
    fn started_id_separates_finished_from_pending_requests() {
        let (sender, receiver) = road_preview_channel();
        assert_eq!(sender.started(), 0);
        assert!(sender.submit(request(1)));
        assert_eq!(sender.started(), 0, "pending input has not started");
        assert_eq!(receiver.recv().unwrap().request_id, 1);
        assert_eq!(sender.started(), 1);
        // Displaced input never starts; only the surviving latest request does.
        assert!(sender.submit(request(2)));
        assert!(sender.submit(request(3)));
        assert_eq!(sender.started(), 1);
        assert_eq!(receiver.recv_timeout(Duration::ZERO).unwrap().request_id, 3);
        assert_eq!(sender.started(), 3);
    }

    #[test]
    fn dropping_sender_terminates_the_worker_after_pending_input() {
        let (sender, receiver) = road_preview_channel();
        assert!(sender.submit(request(1)));
        drop(sender);
        assert_eq!(receiver.recv().unwrap().request_id, 1);
        assert!(receiver.recv().is_err());
    }

    #[test]
    fn closed_worker_rejects_submission() {
        let (sender, receiver) = road_preview_channel();
        drop(receiver);
        assert!(!sender.submit(request(1)));
    }
}
