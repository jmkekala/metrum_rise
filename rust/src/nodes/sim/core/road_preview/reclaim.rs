// SPDX-License-Identifier: GPL-2.0-only

//! Bounded off-thread destruction of superseded road-preview products.

use std::sync::mpsc;
use std::thread::JoinHandle;

/// Frees retired values on one dedicated thread, so neither the preview worker nor a lock holder
/// pays for dropping a planned surface and its caches. At most one value waits in the queue and
/// one is being freed; when both slots are taken the caller drops inline, keeping memory bounded.
pub(super) struct Reclaimer<T: Send + 'static> {
    // Both are `None` without a thread (spawn failure).
    tx: Option<mpsc::SyncSender<T>>,
    thread: Option<JoinHandle<()>>,
}

impl<T: Send + 'static> Reclaimer<T> {
    /// Starts the reclaim thread once; dropping this handle frees what is queued and joins the
    /// thread. Without a thread (spawn failure), every retirement drops inline.
    pub(super) fn spawn(name: &str) -> Self {
        let (tx, rx) = mpsc::sync_channel::<T>(1);
        let thread = std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || rx.into_iter().for_each(drop))
            .ok();
        Self {
            tx: thread.is_some().then_some(tx),
            thread,
        }
    }

    /// Hands `value` to the reclaim thread in O(1), or drops it here when the queue is full.
    pub(super) fn retire(&self, value: T) {
        // A rejected send returns the value inside the error, which drops here.
        if let Some(tx) = &self.tx {
            let _ = tx.try_send(value);
        }
    }
}

impl<T: Send + 'static> Drop for Reclaimer<T> {
    // The thread runs extension code until it returns, so its owner must outlive it (CODE-17).
    fn drop(&mut self) {
        drop(self.tx.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Reclaimer;
    use std::sync::mpsc;

    struct DropProbe(mpsc::Sender<Option<String>>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            let _ = self
                .0
                .send(std::thread::current().name().map(str::to_owned));
        }
    }

    #[test]
    fn retired_values_are_freed_off_the_caller_and_all_drop() {
        let (tx, rx) = mpsc::channel();
        let reclaimer = Reclaimer::spawn("reclaim-test");
        for _ in 0..64 {
            reclaimer.retire(DropProbe(tx.clone()));
        }
        drop(reclaimer);
        // Dropping the handle joined the thread, so every probe has already reported.
        let threads: Vec<_> = rx.try_iter().collect();
        // A full queue falls back to inline drops, but nothing is leaked or lost.
        assert_eq!(threads.len(), 64);
        assert!(
            threads
                .iter()
                .any(|name| name.as_deref() == Some("reclaim-test"))
        );
    }
}
