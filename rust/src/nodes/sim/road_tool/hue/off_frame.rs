// SPDX-License-Identifier: GPL-3.0-or-later

//! The footprint hue built off the frame with its preview (`ROAD-123` round HF2, decision HF-1
//! (b)). A single stroke's preview is synchronous on the frame's thread; its hue's mesh is the
//! largest part of the pose after it (rounds HU3, HF). The preview pushes the stroke and copies
//! its grounded pieces' triangles ([`FootprintHue::push_preview`]) right after its compile, then
//! launches the build on a shared-pool worker with owned parts of the view the preview read
//! ([`ViewGround`]: no live borrow crosses threads). The build runs beside the preview's land
//! take, graded stage and payload and the frame's work after them (packing, the overlay's
//! upload); the frame joins it where it uploads the hue, so the hue and its preview always
//! arrive in the same frame: the frame announces the revision the build will have, and every
//! read of the hue joins first.
//!
//! Allocation: the hue's buffers are moved to the worker and back, reused pose to pose; the
//! worker's graded scratch is per thread. One allocation a pose remains on the frame side,
//! Rayon's box for the spawned job (its captures: the hue's handles, the view's four shared
//! handles, the slot).

use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Instant;

use super::{FootprintHue, HueState};
use crate::nodes::sim::core::{RoadEditPlan, RoadToolQuerySnapshot, ViewGround};
use crate::simulation::network::roadbed::RoadbedDelta;

// What a build hands back: the hue, whether it shows, the build's milliseconds.
type Built = (FootprintHue, bool, f64);

// Where the worker leaves the built hue; one per hue, made once.
#[derive(Default)]
struct Slot {
    built: Mutex<Option<Built>>,
    ready: Condvar,
}

// A build in flight: the hue's revision at its launch and its preview's verdict.
struct InFlight {
    from: u64,
    state: HueState,
}

/// The draft's footprint hue with its build off the frame (`ROAD-123` round HF2): read it
/// through [`Self::hue`], which joins a build in flight first.
#[derive(Default)]
pub(crate) struct OffFrameHue {
    hue: FootprintHue,
    slot: Arc<Slot>,
    in_flight: Option<InFlight>,
    // The last joined build's milliseconds on its worker and the join's wait.
    build_ms: f64,
    wait_ms: f64,
}

impl OffFrameHue {
    /// The hue, a build in flight joined first.
    pub(crate) fn hue(&mut self) -> &mut FootprintHue {
        self.join();
        &mut self.hue
    }

    /// The hue's revision, without joining: while a build is in flight, the revision it will
    /// have once joined.
    pub(crate) fn revision(&self) -> u64 {
        self.in_flight
            .as_ref()
            .map_or(self.hue.revision(), |flight| flight.from + 1)
    }

    /// The last joined build's milliseconds on its worker, and how long its join waited for it
    /// (what the build still added to the frame).
    pub(crate) fn last_times(&self) -> (f64, f64) {
        (self.build_ms, self.wait_ms)
    }

    /// Pushes the stroke `plan` prepared and, for a ground or cut stroke, the grounded pieces
    /// of `delta`, then builds the hue on a shared-pool worker reading `view`'s ground (an
    /// earlier build joined first). Its state is the preview's verdict ([`Self::set_verdict`],
    /// checking until set).
    pub(crate) fn launch(
        &mut self,
        plan: &RoadEditPlan,
        delta: Option<&RoadbedDelta>,
        view: &RoadToolQuerySnapshot,
        half_width_m: f32,
    ) {
        self.join();
        let from = self.hue.revision();
        let mut hue = std::mem::take(&mut self.hue);
        hue.push_preview(plan.prepared(), plan.profile(), delta);
        let ground = ViewGround::of(view);
        let slot = Arc::clone(&self.slot);
        self.in_flight = Some(InFlight {
            from,
            state: HueState::Checking,
        });
        crate::thread_pools::spawn_shared(move || {
            let start = Instant::now();
            let shown = hue.build_on(&ground, half_width_m);
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            *slot.built.lock().unwrap_or_else(PoisonError::into_inner) = Some((hue, shown, ms));
            slot.ready.notify_one();
        });
    }

    /// The verdict of the preview whose hue is in flight: the state it shows once joined.
    pub(crate) fn set_verdict(&mut self, state: HueState) {
        if let Some(flight) = &mut self.in_flight {
            flight.state = state;
        }
    }

    /// Waits for the build in flight, if any, and shows it.
    pub(crate) fn join(&mut self) {
        let Some(InFlight { from, state }) = self.in_flight.take() else {
            return;
        };
        let start = Instant::now();
        let mut built = self.slot.built.lock().unwrap_or_else(PoisonError::into_inner);
        let (hue, shown, build_ms) = loop {
            if let Some(done) = built.take() {
                break done;
            }
            built = self
                .slot
                .ready
                .wait(built)
                .unwrap_or_else(PoisonError::into_inner);
        };
        drop(built);
        self.hue = hue;
        self.hue.conclude(from, shown, state);
        self.build_ms = build_ms;
        self.wait_ms = start.elapsed().as_secs_f64() * 1000.0;
    }
}
