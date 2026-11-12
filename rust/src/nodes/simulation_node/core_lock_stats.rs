// SPDX-License-Identifier: GPL-2.0-only

//! How often Godot's main thread finds the simulation core busy, by call site (`CODE-26`).
//!
//! Perf-channel diagnostics only: off by default, when the whole cost of a core lock is one
//! relaxed atomic load. On (`set_core_lock_stats`), every main-thread `try_lock` and blocking wait
//! through [`super::SimulationNode`]'s lock helpers counts against its call site (tries, tries
//! that found the core busy, waits, the wait time and the longest wait); other threads (the
//! background jobs that try the core too) are not counted. A benchmark takes the counts and resets
//! them (`take_core_lock_stats`). A site's entry is made the first time it locks while on; after
//! that a count takes a short uncontended mutex and no allocation.

use std::panic::Location;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread::ThreadId;
use std::time::Instant;

static ENABLED: AtomicBool = AtomicBool::new(false);
static STATE: Mutex<State> = Mutex::new(State {
    main: None,
    sites: Vec::new(),
});

struct State {
    // The thread that turned the counts on: Godot's main thread.
    main: Option<ThreadId>,
    sites: Vec<Site>,
}

#[derive(Clone, Copy)]
pub(super) struct Site {
    pub(super) location: &'static Location<'static>,
    pub(super) tries: u64,
    pub(super) busy: u64,
    pub(super) waits: u64,
    pub(super) wait_us: u64,
    pub(super) wait_max_us: u64,
}

/// Turns the counts on for the calling thread (the main thread) or off; either way drops them.
pub(super) fn set_enabled(enabled: bool) {
    let mut state = STATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    state.main = enabled.then(|| std::thread::current().id());
    state.sites.clear();
    ENABLED.store(enabled, Ordering::Relaxed);
}

#[inline]
pub(super) fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

// The main thread's entry for `location`, made on first use.
fn with_site(location: &'static Location<'static>, update: impl FnOnce(&mut Site)) {
    let mut state = STATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if state.main != Some(std::thread::current().id()) {
        return;
    }
    let index = match state
        .sites
        .iter()
        .position(|site| std::ptr::eq(site.location, location))
    {
        Some(index) => index,
        None => {
            state.sites.push(Site {
                location,
                tries: 0,
                busy: 0,
                waits: 0,
                wait_us: 0,
                wait_max_us: 0,
            });
            state.sites.len() - 1
        }
    };
    update(&mut state.sites[index]);
}

/// A `try_lock` at `location`; `busy` when the core was held.
pub(super) fn note_try(location: &'static Location<'static>, busy: bool) {
    with_site(location, |site| {
        site.tries += 1;
        site.busy += u64::from(busy);
    });
}

/// A blocking wait at `location` that began at `start`.
pub(super) fn note_wait(location: &'static Location<'static>, start: Instant) {
    let us = start.elapsed().as_micros() as u64;
    with_site(location, |site| {
        site.waits += 1;
        site.wait_us += us;
        site.wait_max_us = site.wait_max_us.max(us);
    });
}

/// The counts since the last take, by site; resets them.
pub(super) fn take() -> Vec<Site> {
    let mut state = STATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let out = state.sites.clone();
    for site in &mut state.sites {
        *site = Site {
            location: site.location,
            tries: 0,
            busy: 0,
            waits: 0,
            wait_us: 0,
            wait_max_us: 0,
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test: the counts are process-wide, so a second test turning them on would race this one.
    #[test]
    fn counts_the_enabling_thread_by_site_and_resets_on_take() {
        let here = Location::caller();
        note_try(here, true);
        assert!(take().is_empty(), "nothing is counted while off");
        set_enabled(true);
        note_try(here, false);
        note_try(here, true);
        note_wait(here, Instant::now());
        std::thread::spawn(move || note_try(here, true))
            .join()
            .unwrap();
        let sites = take();
        let site = sites
            .iter()
            .find(|site| std::ptr::eq(site.location, here))
            .expect("the site was counted");
        assert_eq!((site.tries, site.busy, site.waits), (2, 1, 1));
        assert!(take().iter().all(|site| site.tries == 0 && site.waits == 0));
        set_enabled(false);
        assert!(take().is_empty());
    }
}
