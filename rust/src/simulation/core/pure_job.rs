// SPDX-License-Identifier: GPL-2.0-only

//! Pure jobs ([`docs/simulation.md#pure-jobs`]): a pure function of an immutable snapshot,
//! started on the shared pool and taken by the simulation at a fixed frame.
//!
//! The work runs on whichever thread reaches it first: a shared-pool worker, or the thread that
//! takes the result and finds it not yet started, which then runs it itself. A taker never
//! blocks a pool worker on work queued behind it, so a one-worker pool cannot deadlock, and the
//! result is the same function of the snapshot either way. Dropping an unstarted job drops its
//! work and the snapshot it holds.

use std::sync::{Arc, Condvar, Mutex, PoisonError};

type Work<T> = Box<dyn FnOnce() -> T + Send>;

enum State<T> {
    Queued(Work<T>),
    Running,
    Done(T),
    // Taken by the owner, or dropped before it started.
    Gone,
}

struct Shared<T> {
    state: Mutex<State<T>>,
    done: Condvar,
}

impl<T> Shared<T> {
    // Runs the work if nobody has started it; O(1) otherwise.
    fn run(&self) {
        let work = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            match std::mem::replace(&mut *state, State::Running) {
                State::Queued(work) => work,
                other => {
                    *state = other;
                    return;
                }
            }
        };
        let result = work();
        *self.state.lock().unwrap_or_else(PoisonError::into_inner) = State::Done(result);
        self.done.notify_all();
    }
}

// A job's completion without its result type, for a thread that only waits for it.
trait Complete: Send + Sync {
    fn complete(&self);
}

impl<T: Send> Complete for Shared<T> {
    fn complete(&self) {
        self.run();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        while matches!(*state, State::Running) {
            state = self
                .done
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// Finishes a [`PureJob`] without taking its result: see [`PureJob::completion`].
pub(crate) struct JobCompletion(Arc<dyn Complete>);

impl JobCompletion {
    /// Runs the job here if nobody has started it, otherwise waits for the thread that has;
    /// returns at once for a finished or dropped job. Call it inside the shared pool.
    pub(crate) fn wait(&self) {
        self.0.complete();
    }
}

/// One pure computation in flight. See the module documentation.
pub(crate) struct PureJob<T: Send + 'static> {
    shared: Arc<Shared<T>>,
}

impl<T: Send + 'static> PureJob<T> {
    /// Queues `work` on the shared pool (`crate::thread_pools`); from a pool worker it queues on
    /// that worker's pool. O(1).
    pub(crate) fn spawn(work: impl FnOnce() -> T + Send + 'static) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::Queued(Box::new(work))),
            done: Condvar::new(),
        });
        let worker = Arc::clone(&shared);
        crate::thread_pools::spawn(move || worker.run());
        Self { shared }
    }

    /// A handle that finishes the work from another thread, so the owner can wait for it
    /// without holding what owns the job; [`Self::take`] then returns at once. O(1).
    pub(crate) fn completion(&self) -> JobCompletion {
        JobCompletion(Arc::clone(&self.shared) as Arc<dyn Complete>)
    }

    /// The result, running the work here if no worker has started it, otherwise waiting for
    /// the worker that has.
    pub(crate) fn take(self) -> T {
        self.shared.run();
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        loop {
            match std::mem::replace(&mut *state, State::Gone) {
                State::Done(result) => return result,
                State::Running => {
                    *state = State::Running;
                    state = self
                        .shared
                        .done
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                }
                // `take` consumes the job and only the owner removes a result.
                State::Queued(_) | State::Gone => unreachable!("a pure job's result is taken once"),
            }
        }
    }
}

impl<T: Send + 'static> Drop for PureJob<T> {
    fn drop(&mut self) {
        // A superseded job that has not started never runs; a running one finishes and its
        // result is dropped with the last reference.
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if matches!(*state, State::Queued(_)) {
            *state = State::Gone;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn a_completion_finishes_the_work_before_the_take() {
        let runs = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&runs);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        // The only worker is busy, so the completion runs the work itself; the take finds it done.
        let value = pool.install(|| {
            let job = PureJob::spawn(move || counted.fetch_add(1, Ordering::SeqCst) + 42);
            job.completion().wait();
            assert_eq!(runs.load(Ordering::SeqCst), 1);
            job.take()
        });
        assert_eq!(value, 42);
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_one_worker_pool_runs_the_work_on_the_taker() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        // The taker occupies the only worker, so the spawned copy can only start after it.
        let value = pool.install(|| PureJob::spawn(|| 6 * 7).take());
        assert_eq!(value, 42);
    }

    #[test]
    fn the_work_runs_once_and_a_dropped_job_never_starts() {
        let runs = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&runs);
        let job = PureJob::spawn(move || counted.fetch_add(1, Ordering::SeqCst));
        assert_eq!(job.take(), 0);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        let counted = Arc::clone(&runs);
        // Dropped while the only worker is busy: the queued copy finds nothing to run.
        pool.install(|| {
            drop(PureJob::spawn(move || {
                counted.fetch_add(1, Ordering::SeqCst)
            }))
        });
        pool.install(|| ());
        drop(pool);
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }
}
