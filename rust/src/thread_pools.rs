// SPDX-License-Identifier: GPL-2.0-only

//! Rayon pools whose workers the extension joins before Godot unloads it (`CODE-17`).
//!
//! A thread still running extension code when Godot unloads the library executes unmapped
//! code and crashes the process. Rayon never stops its global pool, and dropping a
//! [`rayon::ThreadPool`] only tells its workers to exit, so the extension uses neither:
//!
//! - [`JoinedPool`] is a Rayon pool that joins its workers when dropped.
//! - The shared pool replaces the global pool for all extension work. It is sized like the
//!   global pool and built on first use, in the game and in test binaries alike. Threads
//!   outside every Rayon pool (Godot's main thread and `WorkerThreadPool`, the simulation
//!   thread, test threads) enter it through [`install`] and [`spawn`]; code already on a pool
//!   worker stays in that pool.
//! - [`disable_global_pool`] runs when the extension loads, so work that bypasses the entry
//!   points panics where it starts instead of starting threads nobody joins. [`stop`] waits
//!   for work in flight and joins the shared pool's workers before the library unloads.

use std::ops::Deref;
use std::sync::{PoisonError, RwLock, RwLockReadGuard};
use std::thread::JoinHandle;

use rayon::{ThreadPool, ThreadPoolBuildError, ThreadPoolBuilder};

/// A Rayon pool that joins its worker threads when dropped.
pub(crate) struct JoinedPool {
    // Field order is the shutdown order: dropping the pool tells its workers to finish their
    // queued and spawned work and exit, then `_workers` joins them.
    pool: ThreadPool,
    _workers: Workers,
}

struct Workers(Vec<JoinHandle<()>>);

impl Drop for Workers {
    fn drop(&mut self) {
        // A worker's panic already aborted the process (Rayon's default panic handling).
        for worker in self.0.drain(..) {
            let _ = worker.join();
        }
    }
}

impl JoinedPool {
    /// Builds `builder`'s pool on threads that this value joins, keeping its names and sizes.
    pub(crate) fn build(builder: ThreadPoolBuilder) -> Result<Self, ThreadPoolBuildError> {
        let mut workers = Workers(Vec::new());
        // If a spawn fails, Rayon has already told the started workers to exit and `workers`
        // joins them on the early return.
        let pool = builder
            .spawn_handler(|thread| {
                let mut spawn = std::thread::Builder::new();
                if let Some(name) = thread.name() {
                    spawn = spawn.name(name.to_owned());
                }
                if let Some(size) = thread.stack_size() {
                    spawn = spawn.stack_size(size);
                }
                workers.0.push(spawn.spawn(|| thread.run())?);
                Ok(())
            })
            .build()?;
        Ok(Self {
            pool,
            _workers: workers,
        })
    }
}

impl Deref for JoinedPool {
    type Target = ThreadPool;

    fn deref(&self) -> &ThreadPool {
        &self.pool
    }
}

enum State {
    // No entry has run yet: an editor session that never simulates starts no threads.
    Idle,
    Running(JoinedPool),
    // Joined: later installs run on the caller and spawns go to Rayon's global pool, which
    // the extension has disabled, so no thread starts after the join.
    Stopped,
}

/// The extension's shared pool, built on first use and joined by [`SharedPool::stop`].
struct SharedPool {
    state: RwLock<State>,
    builder: fn() -> ThreadPoolBuilder,
}

impl SharedPool {
    const fn new(builder: fn() -> ThreadPoolBuilder) -> Self {
        Self {
            state: RwLock::new(State::Idle),
            builder,
        }
    }

    fn stop(&self) {
        // The write lock waits for every `install` in flight; dropping the pool then waits for
        // spawned jobs, which hold Rayon's terminate count, and joins the workers.
        let state = std::mem::replace(
            &mut *self.state.write().unwrap_or_else(PoisonError::into_inner),
            State::Stopped,
        );
        drop(state);
    }

    // `None` when the caller already is a worker of some Rayon pool. The guard is held for the
    // whole entry, so `stop` cannot join the workers under it.
    fn entry(&self) -> Option<RwLockReadGuard<'_, State>> {
        if rayon::current_thread_index().is_some() {
            return None;
        }
        let read = || self.state.read().unwrap_or_else(PoisonError::into_inner);
        let state = read();
        if !matches!(*state, State::Idle) {
            return Some(state);
        }
        drop(state);
        {
            let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
            // Rechecked under the lock: another entry or `stop` may have run since.
            if matches!(*state, State::Idle) {
                let pool = JoinedPool::build((self.builder)());
                *state = State::Running(pool.expect("the shared thread pool must start"));
            }
        }
        Some(read())
    }

    fn install<R: Send>(&self, op: impl FnOnce() -> R + Send) -> R {
        match self.entry().as_deref() {
            Some(State::Running(pool)) => pool.install(op),
            _ => op(),
        }
    }

    fn spawn(&self, op: impl FnOnce() + Send + 'static) {
        match self.entry().as_deref() {
            Some(State::Running(pool)) => pool.spawn(op),
            // A worker queues on its own pool; after `stop`, Rayon's global pool refuses it.
            _ => rayon::spawn(op),
        }
    }
}

// Sized like the global pool it replaces: Rayon's builder reads RAYON_NUM_THREADS, else one
// thread per logical CPU.
static SHARED: SharedPool =
    SharedPool::new(|| ThreadPoolBuilder::new().thread_name(|index| format!("metrum-{index}")));

/// Runs `op` in the shared pool and returns its result, as Rayon runs work that a thread
/// outside every pool submits to the global pool: the caller blocks and does not steal other
/// work. Runs inline when the caller already is a Rayon worker, or after [`stop`].
pub(crate) fn install<R: Send>(op: impl FnOnce() -> R + Send) -> R {
    SHARED.install(op)
}

/// Queues `op` on the shared pool without waiting for it; [`stop`] waits for it to finish.
/// A Rayon worker queues it on its own pool.
pub(crate) fn spawn(op: impl FnOnce() + Send + 'static) {
    SHARED.spawn(op);
}

/// Waits for the shared pool's work in flight, then joins its workers. Called once before the
/// extension unloads, after the nodes that submit work have been freed.
pub(crate) fn stop() {
    SHARED.stop();
}

/// Spends Rayon's one-time global pool initialization on a pool that cannot start. Every
/// later use of the global pool then panics with "The global thread pool has not been
/// initialized.", which turns work started outside [`install`]/[`spawn`] into a panic at its
/// call site instead of threads that keep running extension code after the library unloads.
/// Returns false when the global pool already existed and so stays usable.
pub(crate) fn disable_global_pool() -> bool {
    let mut refused = false;
    let built = ThreadPoolBuilder::new()
        .num_threads(1)
        .spawn_handler(|_| {
            refused = true;
            Err(std::io::Error::other(
                "the extension runs Rayon work in its shared pool",
            ))
        })
        .build_global();
    // The handler only runs while no global pool exists.
    built.is_err() && refused
}

#[cfg(test)]
mod tests {
    use super::{JoinedPool, SharedPool};
    use rayon::ThreadPoolBuilder;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex};

    // Each test pool counts its own live workers through its start and exit handlers.
    static SHARED_LIVE: AtomicUsize = AtomicUsize::new(0);
    static JOINED_LIVE: AtomicUsize = AtomicUsize::new(0);

    fn counted(name: &'static str, live: &'static AtomicUsize) -> ThreadPoolBuilder {
        ThreadPoolBuilder::new()
            .num_threads(3)
            .thread_name(move |index| format!("{name}-{index}"))
            .start_handler(move |_| {
                live.fetch_add(1, Ordering::SeqCst);
            })
            // The last thing a worker does in Rayon's loop, after its last job.
            .exit_handler(move |_| {
                live.fetch_sub(1, Ordering::SeqCst);
            })
    }

    // Spawned jobs that wait until `open` is called, so all of them are still queued or
    // running when the pool is dropped.
    struct Gate(Arc<(Mutex<bool>, Condvar)>);

    impl Gate {
        fn new() -> Self {
            Self(Arc::new((Mutex::new(false), Condvar::new())))
        }

        fn job(&self, ran: &Arc<AtomicUsize>) -> impl FnOnce() + Send + 'static {
            let (gate, ran) = (Arc::clone(&self.0), Arc::clone(ran));
            move || {
                let (open, wake) = &*gate;
                drop(
                    wake.wait_while(open.lock().unwrap(), |open| !*open)
                        .unwrap(),
                );
                ran.fetch_add(1, Ordering::SeqCst);
            }
        }

        fn open(&self) {
            *self.0.0.lock().unwrap() = true;
            self.0.1.notify_all();
        }
    }

    fn worker_name() -> Option<String> {
        std::thread::current().name().map(str::to_owned)
    }

    // Rayon's own drop only signals the workers. A joined pool returns from drop after every
    // worker has run its spawned work and left Rayon's loop.
    #[test]
    fn dropping_a_joined_pool_waits_for_spawned_work_and_every_worker() {
        let pool = JoinedPool::build(counted("joined-test", &JOINED_LIVE)).unwrap();
        let name = pool.install(worker_name);
        assert!(name.is_some_and(|name| name.starts_with("joined-test-")));
        let (gate, ran) = (Gate::new(), Arc::new(AtomicUsize::new(0)));
        for _ in 0..16 {
            pool.spawn(gate.job(&ran));
        }
        gate.open();
        drop(pool);
        assert_eq!(ran.load(Ordering::SeqCst), 16);
        assert_eq!(JOINED_LIVE.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn shared_pool_starts_on_first_entry_and_stop_joins_it() {
        let shared = SharedPool::new(|| counted("shared-test", &SHARED_LIVE));
        assert_eq!(SHARED_LIVE.load(Ordering::SeqCst), 0);
        let (name, nested, threads) = shared.install(|| {
            // A worker's own entry stays on that worker.
            let nested = shared.install(worker_name);
            (worker_name(), nested, rayon::current_num_threads())
        });
        assert!(
            name.as_ref()
                .is_some_and(|name| name.starts_with("shared-test-"))
        );
        assert_eq!(nested, name);
        assert_eq!(threads, 3);
        // A worker of another pool stays on its own pool too.
        let other = JoinedPool::build(ThreadPoolBuilder::new().num_threads(1)).unwrap();
        assert_eq!(
            other.install(|| shared.install(rayon::current_num_threads)),
            1
        );

        let (gate, ran) = (Gate::new(), Arc::new(AtomicUsize::new(0)));
        for _ in 0..16 {
            shared.spawn(gate.job(&ran));
        }
        gate.open();
        shared.stop();
        assert_eq!(ran.load(Ordering::SeqCst), 16);
        assert_eq!(SHARED_LIVE.load(Ordering::SeqCst), 0);
        // Stopped: entries run on the caller and start no pool.
        assert_eq!(shared.install(rayon::current_thread_index), None);
        assert_eq!(SHARED_LIVE.load(Ordering::SeqCst), 0);
    }
}
