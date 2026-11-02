// SPDX-License-Identifier: GPL-2.0-only

//! Pure jobs ([`docs/simulation.md#pure-jobs`]): a pure function of an immutable snapshot,
//! started on the background pool and taken by the simulation at a fixed frame.
//!
//! The work runs on whichever thread reaches it first: a background-pool worker, or the thread
//! that takes the result and finds it not yet started, which then runs it itself (in the
//! background pool when it is outside every pool), which shares no worker with the frame's work
//! on the shared pool (`crate::thread_pools`). The result is the same function of the snapshot
//! either way, and dropping an unstarted job drops its work and the snapshot it holds.
//!
//! Only a thread outside the extension's pools waits for work another thread runs. A pool
//! worker waiting in a join runs whatever it steals, an injected frame among it, so the job that
//! frame would wait for, or a part of the job's own joins, may lie lower on that worker's stack
//! or on another waiting worker's: a blocking wait there deadlocks the pool (`ROAD-68-G` part
//! 4). The owner therefore finishes the job before it enters the pool ([`PureJob::completion`]),
//! and a wait on a pool worker panics instead of blocking.
//!
//! A worker never runs one job's work inside another's (`CODE-18`). Waiting in a join, it
//! takes queued starts, and each started job could join and take the next: a burst of road edits
//! that each start a routing rebuild nested 156 of them on one worker until its 2 MB stack
//! overflowed. A start taken there is deferred instead and queued again once the job in progress
//! returns, so a worker's stack holds at most one started job (plus one job per owner waiting for
//! its own). Superseded deferred jobs run nothing; results do not change.

use std::cell::{Cell, RefCell};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

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

impl<T: Send> Shared<T> {
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    // Runs the work if nobody has started it; O(1) otherwise. A thread outside every pool runs
    // it in the background pool, as a worker would.
    fn run(&self) {
        let work = {
            let mut state = self.lock();
            match std::mem::replace(&mut *state, State::Running) {
                State::Queued(work) => work,
                other => {
                    *state = other;
                    return;
                }
            }
        };
        let result = crate::thread_pools::install_background(move || {
            let _running = Running::enter();
            work()
        });
        *self.lock() = State::Done(result);
        self.done.notify_all();
    }

    // Runs the work if nobody has started it, otherwise waits for the thread that has. Panics
    // where a pool worker would wait (module documentation).
    fn finish(&self) -> MutexGuard<'_, State<T>> {
        self.run();
        let state = self.lock();
        if !matches!(*state, State::Running) {
            return state;
        }
        assert!(
            !crate::thread_pools::on_pool_worker(),
            "a pool worker must not wait for a pure job: its owner finishes it before entering \
             the pool (PureJob::completion)"
        );
        self.done
            .wait_while(state, |state| matches!(state, State::Running))
            .unwrap_or_else(PoisonError::into_inner)
    }
}

// A job's completion and start without its result type, for a thread that only waits for it
// or starts it.
trait Complete: Send + Sync {
    fn complete(&self);
    fn start(&self);
}

impl<T: Send> Complete for Shared<T> {
    fn complete(&self) {
        drop(self.finish());
    }

    fn start(&self) {
        self.run();
    }
}

thread_local! {
    // Pure-job work on this thread's stack, and the starts taken under it.
    static RUNNING: Cell<u32> = const { Cell::new(0) };
    static DEFERRED: RefCell<Vec<Arc<dyn Complete>>> = const { RefCell::new(Vec::new()) };
}

// Marks pure-job work on this thread's stack while it lives. Leaving the outermost queues the
// starts taken meanwhile again, so they run at the top of a worker's stack: this one's once it
// returns, or an idle worker's.
struct Running;

impl Running {
    fn enter() -> Self {
        RUNNING.set(RUNNING.get() + 1);
        Self
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.set(RUNNING.get() - 1);
        if RUNNING.get() == 0 && !std::thread::panicking() {
            for job in DEFERRED.take() {
                crate::thread_pools::spawn(move || start_or_defer(job));
            }
        }
    }
}

// A queued start: runs the job here, or, taken under another job's work, defers it to the end of
// that work.
fn start_or_defer(job: Arc<dyn Complete>) {
    if RUNNING.get() > 0 {
        DEFERRED.with_borrow_mut(|deferred| deferred.push(job));
    } else {
        job.start();
    }
}

/// Finishes a [`PureJob`] without taking its result: see [`PureJob::completion`].
pub(crate) struct JobCompletion(Arc<dyn Complete>);

impl JobCompletion {
    /// Runs the job in the background pool if nobody has started it, otherwise waits for the
    /// thread that has; returns at once for a finished or dropped job. Call it outside the
    /// extension's pools, before entering one: on a pool worker that would have to wait, it
    /// panics.
    pub(crate) fn wait(&self) {
        self.0.complete();
    }
}

/// Queues a job made by [`PureJob::unstarted`] on the background pool. A start that is
/// dropped instead leaves the work to the first thread that waits for it or takes it, so the
/// result never depends on whether, or when, the job was started.
pub(crate) struct JobStart(Arc<dyn Complete>);

impl JobStart {
    /// Queues the job on the background pool, as [`PureJob::spawn`] does; O(1) from any thread,
    /// a pool worker among them. Work another thread already took is not run twice.
    #[track_caller]
    pub(crate) fn start(self) {
        let job = self.0;
        crate::thread_pools::spawn(move || start_or_defer(job));
    }
}

/// One pure computation in flight. See the module documentation.
pub(crate) struct PureJob<T: Send + 'static> {
    shared: Arc<Shared<T>>,
}

impl<T: Send + 'static> PureJob<T> {
    /// Queues `work` on the background pool (`crate::thread_pools::spawn`); a worker of a test's
    /// own pool queues it on that pool. O(1).
    #[track_caller]
    pub(crate) fn spawn(work: impl FnOnce() -> T + Send + 'static) -> Self {
        let (job, start) = Self::unstarted(work);
        start.start();
        job
    }

    /// A job queued nowhere until its [`JobStart`] starts it, so its owner can start it after
    /// other work (`ROAD-68-G` part 8: a commit's routing rebuild after its lot plan). Waiting
    /// for it or taking it before then runs it on that thread, as for a job no worker reached.
    /// O(1).
    pub(crate) fn unstarted(work: impl FnOnce() -> T + Send + 'static) -> (Self, JobStart) {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::Queued(Box::new(work))),
            done: Condvar::new(),
        });
        let start = JobStart(Arc::clone(&shared) as Arc<dyn Complete>);
        (Self { shared }, start)
    }

    /// A handle that finishes the work from another thread, so the owner can wait for it
    /// outside the extension's pools and without holding what owns the job; [`Self::take`] then
    /// returns at once, also on a pool worker. O(1).
    pub(crate) fn completion(&self) -> JobCompletion {
        JobCompletion(Arc::clone(&self.shared) as Arc<dyn Complete>)
    }

    /// The result, running the work here if no worker has started it, otherwise waiting for
    /// the worker that has: outside the extension's pools, or after [`Self::completion`]
    /// finished it.
    pub(crate) fn take(self) -> T {
        let mut state = self.shared.finish();
        match std::mem::replace(&mut *state, State::Gone) {
            State::Done(result) => result,
            // `take` consumes the job and only the owner removes a result.
            State::Queued(_) | State::Running | State::Gone => {
                unreachable!("a pure job's result is taken once")
            }
        }
    }
}

impl<T: Send + 'static> Drop for PureJob<T> {
    fn drop(&mut self) {
        // A superseded job that has not started never runs; a running one finishes and its
        // result is dropped with the last reference.
        let mut state = self.shared.lock();
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

    // `ROAD-68-G` part 8: an unstarted job runs once, on the taker when nobody started it
    // (a later start then runs nothing), or on the background pool once started.
    #[test]
    fn an_unstarted_job_runs_once_where_it_is_first_needed() {
        let runs = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&runs);
        let (job, start) = PureJob::unstarted(move || counted.fetch_add(1, Ordering::SeqCst));
        assert_eq!(job.take(), 0);
        start.start();
        let counted = Arc::clone(&runs);
        let (job, start) = PureJob::unstarted(move || counted.fetch_add(1, Ordering::SeqCst));
        start.start();
        job.completion().wait();
        assert_eq!(job.take(), 1);
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    // The work runs on a private pool's worker until released. A frame on a shared-pool worker
    // that asked for it would block that worker, so it panics instead; the owner's thread,
    // outside the pool, waits.
    #[test]
    fn a_shared_worker_never_waits_for_a_running_job() {
        let gate = Arc::new((Mutex::new((false, false)), Condvar::new()));
        let held = Arc::clone(&gate);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        let job = pool.install(|| {
            PureJob::spawn(move || {
                let (state, wake) = &*held;
                let mut state = state.lock().unwrap();
                state.0 = true;
                wake.notify_all();
                drop(wake.wait_while(state, |state| !state.1).unwrap());
                7
            })
        });
        let (state, wake) = &*gate;
        drop(wake.wait_while(state.lock().unwrap(), |state| !state.0).unwrap());
        let completion = job.completion();
        let refused = crate::thread_pools::install(|| {
            assert!(crate::thread_pools::on_pool_worker());
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| completion.wait())).is_err()
        });
        state.lock().unwrap().1 = true;
        wake.notify_all();
        assert!(refused);
        assert_eq!(job.take(), 7);
    }

    // `CODE-18`: a worker that takes queued starts while its job waits runs them after that job,
    // never inside it. Each job of a 10,000-long chain starts the next and yields, so a worker
    // that nested them would overflow its stack; here they run one at a time.
    #[test]
    fn a_worker_never_runs_a_started_job_inside_another() {
        const JOBS: usize = 10_000;
        struct Chain {
            depth: AtomicUsize,
            deepest: AtomicUsize,
            jobs: Mutex<Vec<PureJob<()>>>,
            runs: Mutex<usize>,
            done: Condvar,
        }
        fn link(k: usize, chain: Arc<Chain>) -> Box<dyn FnOnce() + Send> {
            Box::new(move || {
                let depth = chain.depth.fetch_add(1, Ordering::SeqCst) + 1;
                chain.deepest.fetch_max(depth, Ordering::SeqCst);
                if k + 1 < JOBS {
                    let next = PureJob::spawn(link(k + 1, Arc::clone(&chain)));
                    chain.jobs.lock().unwrap().push(next);
                    // Runs the queued start on this worker, as a join's wait would.
                    rayon::yield_now();
                }
                chain.depth.fetch_sub(1, Ordering::SeqCst);
                *chain.runs.lock().unwrap() += 1;
                chain.done.notify_all();
            })
        }
        let chain = Arc::new(Chain {
            depth: AtomicUsize::new(0),
            deepest: AtomicUsize::new(0),
            jobs: Mutex::new(Vec::new()),
            runs: Mutex::new(0),
            done: Condvar::new(),
        });
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .stack_size(2 << 20)
            .build()
            .unwrap();
        let first = pool.install(|| PureJob::spawn(link(0, Arc::clone(&chain))));
        let (runs, timeout) = chain
            .done
            .wait_timeout_while(
                chain.runs.lock().unwrap(),
                std::time::Duration::from_secs(60),
                |runs| *runs < JOBS,
            )
            .unwrap();
        assert!(!timeout.timed_out(), "{} of {JOBS} jobs ran", *runs);
        drop(runs);
        assert_eq!(chain.deepest.load(Ordering::SeqCst), 1);
        drop(first);
        drop(pool);
    }
}
