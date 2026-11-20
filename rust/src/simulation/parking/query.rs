// SPDX-License-Identifier: GPL-3.0-or-later

//! Geometry inspection for the parking bridge and local source-product checks.

use super::*;

impl ParkingSystem {
    /// Ordered source products and stable identities for topology/locality acceptance only.
    pub(crate) fn source_bays(&self, source: PlaceSource) -> Vec<(u64, Bay)> {
        let mut result: Vec<_> = self
            .sources
            .get(&source)
            .into_iter()
            .flatten()
            .map(|&h| {
                let p = self.place_handles.resolve(h).expect("source bay");
                (
                    h.to_bits(),
                    Bay {
                        identity: self.places.identity[p],
                        position: self.places.position[p],
                        path: self.places.path[p],
                        heading: self.places.heading[p],
                        width: self.places.width[p],
                        length: self.places.length[p],
                    },
                )
            })
            .collect();
        result.sort_unstable_by_key(|(_, bay)| bay.identity);
        result
    }

    /// Bitwise digest of every place row's source, activity and geometry, in source and
    /// identity order, independent of dense row order: the product a lot-proof refactor must
    /// keep (`PARK-19`). O(places log places), test builds only.
    pub(crate) fn place_product_hash(&self) -> u64 {
        let mut rows: Vec<_> = (0..self.places.len())
            .map(|p| {
                let mut words = vec![
                    u64::from(self.places.active[p]),
                    self.places.identity[p][0],
                    self.places.identity[p][1],
                    u64::from(self.places.heading[p].to_bits()),
                    u64::from(self.places.width[p].to_bits()),
                    u64::from(self.places.length[p].to_bits()),
                ];
                words.extend(self.places.position[p].map(|v| u64::from(v.to_bits())));
                words.extend(self.places.path[p].iter().flatten().map(|v| u64::from(v.to_bits())));
                (self.places.source[p], self.places.identity[p], words)
            })
            .collect();
        rows.sort_unstable_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
        let source_words = |source: PlaceSource| match source {
            PlaceSource::Building { building, anchor } => [0, building, u64::from(anchor)],
            PlaceSource::Kerb { edge, right } => [1, u64::from(edge), u64::from(right)],
        };
        rows.iter()
            .flat_map(|(source, _, words)| source_words(*source).into_iter().chain(words.iter().copied()))
            .fold(rows.len() as u64, |hash, word| {
                crate::simulation::core::stable_hash64(hash ^ word)
            })
    }
}
