// SPDX-License-Identifier: GPL-2.0-only

//! The walkers' drawn buffers between snapshots (`RENDER-28`): a bucket uploads only when its
//! snapshot buffer differs bit for bit from the one drawn, so a paused city, and every frame
//! between two snapshots, uploads no walker. Presentation state on the main thread; the
//! simulation never reads it.

use std::collections::HashMap;

/// The walker buffers drawn per bucket (`pedestrian_type`) and the buckets the last
/// [`Self::update`] changed. Buffers keep their capacity, so a steady frame allocates nothing.
#[derive(Default)]
pub(crate) struct DrawnWalkers {
    drawn: HashMap<u8, Vec<f32>>,
    changed: Vec<u8>,
}

impl DrawnWalkers {
    /// Takes the snapshot's walker buckets (16 floats a walker: transform, then custom data).
    /// A bucket whose buffer differs from the drawn one bit for bit is copied and listed in
    /// [`Self::changed`]; an empty or absent bucket that drew walkers is dropped and listed too.
    /// O(drawn floats) compares, a copy of the changed buckets only.
    pub(crate) fn update(&mut self, snapshot: &HashMap<u8, Vec<f32>>) {
        self.changed.clear();
        for (&key, buffer) in snapshot {
            if buffer.is_empty() {
                continue;
            }
            let drawn = self.drawn.entry(key).or_default();
            if !same_bits(drawn, buffer) {
                drawn.clear();
                drawn.extend_from_slice(buffer);
                self.changed.push(key);
            }
        }
        for key in 0..=u8::MAX {
            if snapshot.get(&key).is_none_or(Vec::is_empty)
                && self.drawn.get(&key).is_some_and(|drawn| !drawn.is_empty())
            {
                self.drawn.remove(&key);
                self.changed.push(key);
            }
        }
    }

    /// The buckets the last [`Self::update`] changed, with those that left (now empty).
    pub(crate) fn changed(&self) -> &[u8] {
        &self.changed
    }

    /// The drawn buffer of `key`, empty when it draws none.
    pub(crate) fn drawn(&self, key: u8) -> &[f32] {
        self.drawn.get(&key).map_or(&[], Vec::as_slice)
    }
}

fn same_bits(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walker(x: f32, phase: f32) -> [f32; 16] {
        let mut row = [0.0; 16];
        row[0] = 1.0;
        row[3] = x;
        row[12] = phase;
        row
    }

    #[test]
    fn only_changed_buckets_upload_and_a_bucket_that_leaves_draws_none() {
        let mut walkers = DrawnWalkers::default();
        let mut snapshot: HashMap<u8, Vec<f32>> = HashMap::new();
        snapshot.insert(0, walker(1.0, 0.25).to_vec());
        snapshot.insert(2, walker(5.0, 0.5).to_vec());
        snapshot.insert(3, Vec::new());
        walkers.update(&snapshot);
        let mut changed = walkers.changed().to_vec();
        changed.sort_unstable();
        assert_eq!(
            changed,
            [0, 2],
            "first frame uploads every non-empty bucket"
        );

        // A paused frame: the same bits, nothing to upload.
        walkers.update(&snapshot);
        assert!(walkers.changed().is_empty());

        // One walker steps; -0.0 against 0.0 is a change of bits too.
        snapshot.get_mut(&2).unwrap()[12] = 0.75;
        snapshot.get_mut(&0).unwrap()[1] = -0.0;
        walkers.update(&snapshot);
        let mut changed = walkers.changed().to_vec();
        changed.sort_unstable();
        assert_eq!(changed, [0, 2]);
        assert_eq!(walkers.drawn(2), snapshot[&2].as_slice());

        // A bucket emptied by the snapshot, and one gone from it, both leave once.
        snapshot.get_mut(&0).unwrap().clear();
        snapshot.remove(&2);
        walkers.update(&snapshot);
        let mut changed = walkers.changed().to_vec();
        changed.sort_unstable();
        assert_eq!(changed, [0, 2]);
        assert!(walkers.drawn(0).is_empty() && walkers.drawn(2).is_empty());
        walkers.update(&snapshot);
        assert!(walkers.changed().is_empty());
    }
}
