// SPDX-License-Identifier: GPL-2.0-only

//! Bounded off-thread destruction of superseded road-preview products.

use std::sync::mpsc;

/// Frees retired values on one dedicated thread, so neither the preview worker nor a lock holder
/// pays for dropping a planned surface and its caches. At most one value waits in the queue and
/// one is being freed; when both slots are taken the caller drops inline, keeping memory bounded.
pub(super) struct Reclaimer<T: Send + 'static> {
    tx: Option<mpsc::SyncSender<T>>,
}

impl<T: Send + 'static> Reclaimer<T> {
    /// Starts the reclaim thread once; it exits when this handle is dropped. Without a thread
    /// (spawn failure), every retirement drops inline.
    pub(super) fn spawn(name: &str) -> Self {
        let (tx, rx) = mpsc::sync_channel::<T>(1);
        let spawned = std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || rx.into_iter().for_each(drop))
            .is_ok();
        Self {
            tx: spawned.then_some(tx),
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
        drop(tx);
        let threads: Vec<_> = rx.iter().collect();
        // A full queue falls back to inline drops, but nothing is leaked or lost.
        assert_eq!(threads.len(), 64);
        assert!(
            threads
                .iter()
                .any(|name| name.as_deref() == Some("reclaim-test"))
        );
    }
}
