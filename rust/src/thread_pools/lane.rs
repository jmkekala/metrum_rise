// SPDX-License-Identifier: GPL-3.0-or-later

//! A frame's payload handed between the main thread and its background-pool job (`RENDER-47`'s
//! agents' frame, `TERRAIN-14`'s graded preview sets).
//!
//! The payload is one value that is on the main thread ([`Taken::Resident`]), at a job, or back
//! from one ([`Taken::Landed`]). The main thread takes it without waiting: a job still running
//! is late ([`Taken::Late`]), and that frame's `delta` is carried to the next start. Tests and
//! tools take the synchronous path ([`FrameLane::take_waiting`], [`FrameLane::wait_landed`]).
//! Every operation is `O(1)` on the main thread; the payload's box travels and is reused.

use std::sync::{Arc, Condvar, Mutex, PoisonError};

// Where the payload is: on the main thread (or before the first job), at a job, or back from
// one.
enum LaneState<T> {
    Resident(Box<T>),
    Running,
    Landed(Box<T>),
}

// The lane's state with the condition a waiter (tests' synchronous path) blocks on.
type Shared<T> = Arc<(Mutex<LaneState<T>>, Condvar)>;

/// Where a frame's payload is between the main thread and its job, and the deltas of frames
/// whose job was late. Every operation is `O(1)` on the main thread.
pub(crate) struct FrameLane<T> {
    shared: Shared<T>,
    // Seconds of frames that started no job, carried into the next.
    carried_delta: f64,
    late_frames: u64,
}

impl<T: Default> Default for FrameLane<T> {
    fn default() -> Self {
        Self {
            shared: Arc::new((
                Mutex::new(LaneState::Resident(Box::default())),
                Condvar::new(),
            )),
            carried_delta: 0.0,
            late_frames: 0,
        }
    }
}

/// What [`FrameLane::take`] found.
pub(crate) enum Taken<T> {
    /// A payload a job prepared: install it, then start the next.
    Landed(Box<T>),
    /// A payload that was not at a job (the first, or after the synchronous path): start it.
    Resident(Box<T>),
    /// The job still runs: draw what was drawn.
    Late,
}

impl<T: Send + 'static> FrameLane<T> {
    fn state(&self) -> std::sync::MutexGuard<'_, LaneState<T>> {
        self.shared.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes the payload for this frame's `delta` seconds, never waiting: returns it with the
    /// delta its job is to cover (this one's plus every late frame's since the last start), or
    /// [`Taken::Late`] and carries `delta` on. A lane whose jobs read no frame time passes 0.
    pub(crate) fn take(&mut self, delta: f64) -> (Taken<T>, f64) {
        let mut state = self.state();
        let taken = match std::mem::replace(&mut *state, LaneState::Running) {
            LaneState::Running => {
                drop(state);
                self.late_frames += 1;
                self.carried_delta += delta;
                return (Taken::Late, 0.0);
            }
            LaneState::Resident(frame) => Taken::Resident(frame),
            LaneState::Landed(frame) => Taken::Landed(frame),
        };
        drop(state);
        (taken, std::mem::take(&mut self.carried_delta) + delta)
    }

    /// Takes the payload, waiting for a job in flight (the synchronous path: tests and tools),
    /// and whether a job prepared it (its outputs not yet installed).
    pub(crate) fn take_waiting(&mut self) -> (Box<T>, bool) {
        let (lock, landed) = &*self.shared;
        let mut state = landed
            .wait_while(
                lock.lock().unwrap_or_else(PoisonError::into_inner),
                |state| matches!(state, LaneState::Running),
            )
            .unwrap_or_else(PoisonError::into_inner);
        match std::mem::replace(&mut *state, LaneState::Running) {
            LaneState::Resident(frame) => (frame, false),
            LaneState::Landed(frame) => (frame, true),
            LaneState::Running => unreachable!("waited while running"),
        }
    }

    /// Keeps `frame` on the main thread (the synchronous path, or no job to start), readable by
    /// [`Self::resident`].
    pub(crate) fn keep(&mut self, frame: Box<T>) {
        *self.state() = LaneState::Resident(frame);
    }

    /// The job that runs `prepare` on `frame` and lands it for the next [`Self::take`]: the
    /// caller queues it on the background pool. The payload must have come from `take`.
    pub(crate) fn job(
        &self,
        mut frame: Box<T>,
        prepare: impl FnOnce(&mut T) + Send + 'static,
    ) -> impl FnOnce() + Send + 'static {
        let shared = Arc::clone(&self.shared);
        move || {
            prepare(&mut frame);
            land(&shared, frame);
        }
    }

    /// Lands `frame`, prepared on the calling thread, for the next [`Self::take`]. The payload
    /// must have come from `take`.
    pub(crate) fn land(&mut self, frame: Box<T>) {
        land(&self.shared, frame);
    }

    /// The payload kept on the main thread by the synchronous path, if it is there.
    pub(crate) fn resident<R>(&self, read: impl FnOnce(&T) -> R) -> Option<R> {
        match &*self.state() {
            LaneState::Resident(frame) => Some(read(frame)),
            _ => None,
        }
    }

    /// Waits until no job runs (tests).
    #[cfg(test)]
    pub(crate) fn wait_landed(&self) {
        let (lock, landed) = &*self.shared;
        drop(
            landed
                .wait_while(lock.lock().unwrap(), |state| {
                    matches!(state, LaneState::Running)
                })
                .unwrap(),
        );
    }

    /// Frames whose job was still running when they asked for it, since the lane began.
    pub(crate) fn late_frames(&self) -> u64 {
        self.late_frames
    }
}

// Puts a prepared payload where the next `take` finds it and wakes a waiter.
fn land<T>(shared: &Shared<T>, frame: Box<T>) {
    let (lock, landed) = &**shared;
    *lock.lock().unwrap_or_else(PoisonError::into_inner) = LaneState::Landed(frame);
    landed.notify_all();
}
