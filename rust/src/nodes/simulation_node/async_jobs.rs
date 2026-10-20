// SPDX-License-Identifier: GPL-2.0-only

//! Rayon jobs that a node waits for before it is freed (`CODE-17`).

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

/// Counts a node's spawned Rayon jobs, so that freeing the node can wait for them.
///
/// Payload jobs hold a reference to the simulation core. Waiting for them keeps the node's
/// own reference the last one, so the core is freed by the thread that frees the node, before
/// Godot unloads the library.
#[derive(Default)]
pub(crate) struct AsyncJobs {
    in_flight: Mutex<usize>,
    finished: Condvar,
}

impl AsyncJobs {
    /// Queues `job` on the extension's shared pool. [`Self::wait`] returns only after `job` has
    /// returned and dropped everything it captured.
    pub(crate) fn spawn(self: &Arc<Self>, job: impl FnOnce() + Send + 'static) {
        *self.count() += 1;
        let jobs = Arc::clone(self);
        crate::thread_pools::spawn(move || {
            // Calling consumes the closure: its captures are dropped before the count falls.
            job();
            let mut in_flight = jobs.count();
            *in_flight -= 1;
            if *in_flight == 0 {
                jobs.finished.notify_all();
            }
        });
    }

    /// Blocks until every job spawned so far has finished. A job that panics aborts the
    /// process (Rayon's handling of a panic in `spawn`), so the count always returns to zero.
    pub(crate) fn wait(&self) {
        let in_flight = self.count();
        drop(
            self.finished
                .wait_while(in_flight, |in_flight| *in_flight > 0)
                .unwrap_or_else(PoisonError::into_inner),
        );
    }

    fn count(&self) -> MutexGuard<'_, usize> {
        self.in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::AsyncJobs;
    use std::sync::{Arc, Condvar, Mutex};

    // The guarantee a freed node relies on: once `wait` returns, no job holds a clone of a
    // shared value any more, so the waiting thread's reference is the last one.
    #[test]
    fn wait_returns_after_every_job_dropped_its_captures() {
        let jobs = Arc::new(AsyncJobs::default());
        let core = Arc::new(());
        // Every job waits for the gate, so all of them still hold their clone at `wait`.
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        for _ in 0..16 {
            let (core, gate) = (Arc::clone(&core), Arc::clone(&gate));
            jobs.spawn(move || {
                let (open, wake) = &*gate;
                drop(
                    wake.wait_while(open.lock().unwrap(), |open| !*open)
                        .unwrap(),
                );
                drop(core);
            });
        }
        assert!(Arc::strong_count(&core) > 1);
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        jobs.wait();
        assert_eq!(Arc::strong_count(&core), 1);
        assert_eq!(*jobs.count(), 0);
    }
}
