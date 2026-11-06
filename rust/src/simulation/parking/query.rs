// SPDX-License-Identifier: GPL-2.0-only

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
}
