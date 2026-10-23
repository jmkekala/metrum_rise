// SPDX-License-Identifier: GPL-2.0-only

//! Exact source ranges in cached road meshes for reversible local editor replacements.

mod preview;
pub(super) use preview::{Polygon, Vertex};

use super::NetworkMeshData;

/// Authoritative graph owner of a contiguous rendered triangle range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum NetworkMeshOwner {
    /// A physical road span, including its markings and earthwork.
    Edge(usize),
    /// A junction, including its crosswalks and earthwork.
    Node(u32),
}

#[derive(Debug)]
pub(super) struct OwnedVertexRange {
    owner: Option<NetworkMeshOwner>,
    end: usize,
}

impl NetworkMeshData {
    pub(super) fn set_owner(&mut self, owner: NetworkMeshOwner) {
        self.current_owner = Some(owner);
    }

    pub(super) fn record_owner_triangle(&mut self, layer: usize, end: usize) {
        let ranges = &mut self.owner_ranges[layer];
        if let Some(last) = ranges.last_mut()
            && last.owner == self.current_owner
        {
            last.end = end;
        } else {
            ranges.push(OwnedVertexRange {
                owner: self.current_owner,
                end,
            });
        }
    }
}
