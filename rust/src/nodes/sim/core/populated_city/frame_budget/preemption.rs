// SPDX-License-Identifier: GPL-2.0-only

//! Run-queue waits of the threads that do a `PERF-07` frame's work (`SIM-02-K` part 7).
//!
//! The benchmark's pinned CPUs are shared with the rest of the machine, and another task that
//! keeps a pool worker or the sim thread off its CPU for milliseconds stalls whichever phase of
//! the frame that thread holds work in. The kernel's per-thread scheduler statistics
//! (`/proc/self/task/<tid>/schedstat`: time on a CPU, time runnable but waiting for one, and
//! timeslices) tell those frames apart. [`RunQueueWaits`] reads every recorded thread's wait
//! before and after a frame, outside the frame's timing, and reports the largest growth. Linux
//! only: elsewhere no thread is recorded and every frame reads no wait.

use std::fs::File;

/// The run-queue wait counters of the shared pool's workers and the calling thread.
pub(super) struct RunQueueWaits {
    threads: Vec<File>,
    // Each thread's wait in nanoseconds at the last `start`.
    before: Vec<u64>,
}

impl RunQueueWaits {
    /// Records every worker of the shared pool (each reads its own thread id) and the calling
    /// thread, which runs the frame's serial work. The shared pool keeps its workers until it is
    /// stopped, so the set holds for the whole run.
    pub(super) fn record_pool_and_caller() -> Self {
        let mut ids: Vec<u32> = crate::thread_pools::install(|| rayon::broadcast(|_| thread_id()))
            .into_iter()
            .flatten()
            .collect();
        ids.extend(thread_id());
        let threads: Vec<File> = ids.into_iter().filter_map(open_schedstat).collect();
        let before = vec![0; threads.len()];
        Self { threads, before }
    }

    /// Watches no thread: every frame reads no wait. For measuring the reads' own overhead.
    pub(super) fn unwatched() -> Self {
        Self {
            threads: Vec::new(),
            before: Vec::new(),
        }
    }

    /// How many threads are watched; 0 where the statistics are unavailable.
    pub(super) fn len(&self) -> usize {
        self.threads.len()
    }

    /// Reads every thread's wait before a frame. O(threads) file reads, no allocation.
    pub(super) fn start(&mut self) {
        for (before, file) in self.before.iter_mut().zip(&self.threads) {
            *before = read_wait_ns(file);
        }
    }

    /// The largest growth of one thread's wait since [`Self::start`], in milliseconds.
    pub(super) fn largest_growth_ms(&self) -> f64 {
        let largest = self
            .before
            .iter()
            .zip(&self.threads)
            .map(|(&before, file)| read_wait_ns(file).saturating_sub(before))
            .max()
            .unwrap_or(0);
        largest as f64 / 1e6
    }
}

// `std` has no gettid; `/proc/thread-self` links to `<pid>/task/<tid>`.
#[cfg(target_os = "linux")]
fn thread_id() -> Option<u32> {
    std::fs::read_link("/proc/thread-self")
        .ok()?
        .file_name()?
        .to_str()?
        .parse()
        .ok()
}

#[cfg(not(target_os = "linux"))]
fn thread_id() -> Option<u32> {
    None
}

fn open_schedstat(tid: u32) -> Option<File> {
    File::open(format!("/proc/self/task/{tid}/schedstat")).ok()
}

// The second field of `schedstat`. A read at offset 0 renders the statistics afresh.
#[cfg(target_os = "linux")]
fn read_wait_ns(file: &File) -> u64 {
    use std::os::unix::fs::FileExt;
    let mut text = [0_u8; 96];
    let len = file.read_at(&mut text, 0).unwrap_or(0);
    std::str::from_utf8(&text[..len])
        .ok()
        .and_then(|text| text.split_whitespace().nth(1))
        .and_then(|wait| wait.parse().ok())
        .unwrap_or(0)
}

#[cfg(not(target_os = "linux"))]
fn read_wait_ns(_: &File) -> u64 {
    0
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::RunQueueWaits;

    // Every pool worker and the caller are found, and a frame on an idle machine reads a
    // finite, non-negative wait.
    #[test]
    fn records_every_pool_worker_and_the_caller() {
        let mut waits = RunQueueWaits::record_pool_and_caller();
        let workers = crate::thread_pools::install(rayon::current_num_threads);
        assert_eq!(waits.len(), workers + 1);
        waits.start();
        let grown = waits.largest_growth_ms();
        assert!(grown.is_finite() && grown >= 0.0);
    }
}
