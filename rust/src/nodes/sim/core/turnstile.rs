// SPDX-License-Identifier: GPL-2.0-only

//! The order in which threads that wait for the simulation core get it (`CODE-25`).
//!
//! `std`'s mutex is not fair. The simulation thread frees the core at the end of a frame and,
//! once a frame overruns its 16.7 ms interval, takes it again for the next frame some 40 µs
//! later: the frames run back to back. A thread blocked on the core (Godot's main thread in a
//! click or any other blocking bridge call) is woken when the core is freed, but the simulation
//! thread is still on its CPU and wins the race again and again; with 20 ms frames on an idle
//! machine a waiting query waited up to 0.6 s (round C25). A waiting thread therefore holds the
//! turnstile while it waits for the core, and the simulation thread passes the turnstile before
//! it takes the core: it cannot take the core again ahead of a thread already waiting, which
//! gets the core when the frame or edit in progress ends. Per-frame queries that only try the
//! core (`try_lock`) never touch the turnstile.

use std::sync::{LockResult, Mutex, MutexGuard, PoisonError, TryLockError};

/// Keeps the simulation thread from taking the core again ahead of a thread waiting for it.
#[derive(Default)]
pub(crate) struct CoreTurnstile(Mutex<()>);

impl CoreTurnstile {
    /// The core for a thread that waits for it: at once when it is free, else when the
    /// simulation thread's frame or edit in progress ends, before its next one. `O(1)` besides
    /// that wait; no allocation.
    pub(crate) fn wait_for<'a, T>(&self, core: &'a Mutex<T>) -> LockResult<MutexGuard<'a, T>> {
        match core.try_lock() {
            Ok(core) => Ok(core),
            Err(TryLockError::Poisoned(poisoned)) => Err(poisoned),
            Err(TryLockError::WouldBlock) => {
                // Held until the core is ours: the simulation thread's next `take` waits here.
                let _turn = self.0.lock().unwrap_or_else(PoisonError::into_inner);
                core.lock()
            }
        }
    }

    /// The core for the simulation thread's next frame or edit, after every thread that was
    /// already waiting for it. `O(1)` besides that wait; no allocation.
    pub(crate) fn take<'a, T>(&self, core: &'a Mutex<T>) -> LockResult<MutexGuard<'a, T>> {
        let _turn = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        core.lock()
    }

    /// Whether a thread is waiting for the core: it holds the turnstile until it has the core.
    pub(crate) fn has_waiter(&self) -> bool {
        matches!(self.0.try_lock(), Err(TryLockError::WouldBlock))
    }
}

#[cfg(test)]
mod tests {
    use super::CoreTurnstile;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    // A simulation thread whose frames run back to back, each holding the core until a thread
    // waits for it and a little longer, so the waiter is asleep on the core when it is freed;
    // the core counts the frames begun. Each wait starts behind a frame in progress and must get
    // the core when that frame ends, however the threads are scheduled: without the turnstile
    // the simulation thread takes the core again first and the count runs ahead.
    #[test]
    fn a_waiting_thread_gets_the_core_before_the_next_frame() {
        let core = Arc::new(Mutex::new(0_u64));
        let turnstile = Arc::new(CoreTurnstile::default());
        let begun = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let simulation = {
            let (core, turnstile, begun, stop) = (
                Arc::clone(&core),
                Arc::clone(&turnstile),
                Arc::clone(&begun),
                Arc::clone(&stop),
            );
            std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    let mut frames = turnstile.take(&core).unwrap();
                    *frames += 1;
                    begun.store(*frames, Ordering::Release);
                    while !turnstile.has_waiter() && !stop.load(Ordering::Acquire) {
                        std::thread::yield_now();
                    }
                    // The rest of the frame's work.
                    std::thread::sleep(Duration::from_micros(200));
                }
            })
        };
        let mut previous = 0;
        for wait in 0..100 {
            while begun.load(Ordering::Acquire) == previous {
                std::thread::yield_now();
            }
            let frames = *turnstile.wait_for(&core).unwrap();
            assert_eq!(
                frames,
                previous + 1,
                "wait {wait}: the core came after frame {frames}, not the frame in progress"
            );
            previous = frames;
        }
        stop.store(true, Ordering::Release);
        simulation.join().unwrap();
    }

    // `has_waiter` names a thread blocked in `wait_for` while the core is held.
    #[test]
    fn the_holder_sees_a_waiter() {
        let core = Arc::new(Mutex::new(()));
        let turnstile = Arc::new(CoreTurnstile::default());
        let held = turnstile.take(&core).unwrap();
        assert!(!turnstile.has_waiter());
        let waiter = {
            let (core, turnstile) = (Arc::clone(&core), Arc::clone(&turnstile));
            std::thread::spawn(move || drop(turnstile.wait_for(&core).unwrap()))
        };
        while !turnstile.has_waiter() {
            std::thread::yield_now();
        }
        drop(held);
        waiter.join().unwrap();
        assert!(!turnstile.has_waiter());
    }
}
