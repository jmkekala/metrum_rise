// SPDX-License-Identifier: GPL-2.0-only

//! The product cache (`RENDER-32-D`, `docs/terrain.md` "Streaming core"): prepared products
//! waiting for the renderer's poll, with the bytes they hold, and spare product storage that
//! a finished poll returns for the next job, so a steady stream of products allocates nothing
//! once the storage has grown.
//!
//! Results are taken oldest first. The byte cap is checked by the layer before it starts a job
//! ([`ProductCache::over_cap`]): a job never waits on it, and the products held never exceed the
//! cap by more than the jobs already in flight hold. `O(1)` per operation.

use std::collections::VecDeque;

/// A landed job: its chunk, its request serial and its product (`None` when the job could not
/// take its inputs and the chunk must be asked for again).
pub(crate) struct Landed<K, P> {
    /// The chunk.
    pub(crate) key: K,
    /// The request serial ([`super::prepare::PrepareQueue::start`]).
    pub(crate) serial: u64,
    /// The product, or `None` when the inputs were busy.
    pub(crate) product: Option<P>,
    bytes: usize,
}

/// Prepared products waiting for their poll, and spare storage.
pub(crate) struct ProductCache<K, P> {
    landed: VecDeque<Landed<K, P>>,
    spare: Vec<P>,
    bytes: usize,
    cap_bytes: usize,
}

impl<K, P: Default> ProductCache<K, P> {
    /// An empty cache whose layer starts no job while it holds `cap_bytes` or more.
    pub(crate) fn new(cap_bytes: usize) -> Self {
        Self {
            landed: VecDeque::new(),
            spare: Vec::new(),
            bytes: 0,
            cap_bytes,
        }
    }

    /// Storage for the next job's product: a spare one when there is one.
    pub(crate) fn storage(&mut self) -> P {
        self.spare.pop().unwrap_or_default()
    }

    /// Holds a landed product (`bytes` of it) for the poll.
    pub(crate) fn land(&mut self, key: K, serial: u64, product: Option<P>, bytes: usize) {
        self.bytes += bytes;
        self.landed.push_back(Landed {
            key,
            serial,
            product,
            bytes,
        });
    }

    /// The oldest landed result, which leaves the byte count.
    pub(crate) fn take(&mut self) -> Option<Landed<K, P>> {
        let landed = self.landed.pop_front()?;
        self.bytes -= landed.bytes;
        Some(landed)
    }

    /// Returns a polled product's storage for reuse.
    pub(crate) fn recycle(&mut self, product: P) {
        self.spare.push(product);
    }

    /// Whether the products held reach the cap.
    pub(crate) fn over_cap(&self) -> bool {
        self.bytes >= self.cap_bytes
    }

    /// Bytes held by landed products.
    #[cfg(test)]
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    /// Results waiting for the poll.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.landed.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oldest_first_bytes_and_reuse() {
        let mut cache: ProductCache<u32, Vec<u8>> = ProductCache::new(10);
        cache.land(1, 1, Some(vec![1; 6]), 6);
        assert!(!cache.over_cap());
        cache.land(2, 2, None, 0);
        cache.land(3, 3, Some(vec![3; 4]), 4);
        assert!(cache.over_cap());
        assert_eq!((cache.len(), cache.bytes()), (3, 10));
        let first = cache.take().unwrap();
        assert_eq!((first.key, first.serial), (1, 1));
        assert_eq!(cache.bytes(), 4);
        cache.recycle(first.product.unwrap());
        assert_eq!(
            cache.storage().len(),
            6,
            "a recycled product's storage is reused"
        );
        assert!(cache.take().unwrap().product.is_none());
        assert_eq!(cache.take().unwrap().key, 3);
        assert!(cache.take().is_none());
    }
}
