// SPDX-License-Identifier: GPL-2.0-only

//! Exact source ranges in cached road meshes for reversible local editor replacements.

use super::NetworkMeshData;
use std::collections::HashSet;

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

    /// Copies only unaffected owners, preserving every vertex attribute exactly.
    /// Work is O(cached chunk ranges + retained vertices), never a city-wide scan.
    pub(crate) fn without_owners(&self, removed: &HashSet<NetworkMeshOwner>) -> Self {
        let mut result = Self::new();
        macro_rules! layer {
            ($index:expr, $vertices:ident, $normals:ident, $uvs:ident, $colors:ident) => {{
                let mut start = 0;
                for range in &self.owner_ranges[$index] {
                    if !range.owner.is_some_and(|owner| removed.contains(&owner)) {
                        result
                            .$vertices
                            .extend_from_slice(&self.$vertices[start..range.end]);
                        result
                            .$normals
                            .extend_from_slice(&self.$normals[start..range.end]);
                        result.$uvs.extend_from_slice(&self.$uvs[start..range.end]);
                        result
                            .$colors
                            .extend_from_slice(&self.$colors[start..range.end]);
                    }
                    start = range.end;
                }
                // Diagnostic meshes may be filled directly instead of through the owned sink.
                result.$vertices.extend_from_slice(&self.$vertices[start..]);
                result.$normals.extend_from_slice(&self.$normals[start..]);
                result.$uvs.extend_from_slice(&self.$uvs[start..]);
                result.$colors.extend_from_slice(&self.$colors[start..]);
            }};
        }
        layer!(
            0,
            earthwork_vertices,
            earthwork_normals,
            earthwork_uvs,
            earthwork_colors
        );
        layer!(1, curb_vertices, curb_normals, curb_uvs, curb_colors);
        layer!(
            2,
            raised_step_vertices,
            raised_step_normals,
            raised_step_uvs,
            raised_step_colors
        );
        layer!(
            3,
            sidewalk_vertices,
            sidewalk_normals,
            sidewalk_uvs,
            sidewalk_colors
        );
        layer!(4, road_vertices, road_normals, road_uvs, road_colors);
        layer!(
            5,
            marking_vertices,
            marking_normals,
            marking_uvs,
            marking_colors
        );
        layer!(
            6,
            concrete_vertices,
            concrete_normals,
            concrete_uvs,
            concrete_colors
        );
        result
    }
}
