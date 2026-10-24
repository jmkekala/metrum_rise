// SPDX-License-Identifier: GPL-2.0-only

//! Exact source ranges in cached road meshes for reversible local editor replacements.

use super::NetworkMeshData;
use godot::prelude::{Color, Vector2, Vector3};
use std::ops::Range;

/// Authoritative graph owner of a contiguous rendered triangle range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum NetworkMeshOwner {
    /// A physical road span, including its markings and earthwork.
    Edge(usize),
    /// A junction, including its crosswalks and earthwork.
    Node(u32),
}

#[derive(Debug, PartialEq)]
pub(super) struct OwnedVertexRange {
    owner: Option<NetworkMeshOwner>,
    end: usize,
}

impl NetworkMeshData {
    pub(super) fn set_owner(&mut self, owner: NetworkMeshOwner) {
        self.current_owner = Some(owner);
    }

    /// `old`, a published chunk mesh, with the triangles of every owner `keep` rejects replaced
    /// by `fresh`'s, which holds only such owners: per layer, `old`'s kept runs and all of
    /// `fresh`'s merge in owner order. Each layer is written by one emission pass that visits
    /// spans, then nodes, by id, so the result is the chunk a full emission builds whenever the
    /// kept owners' triangles are unchanged. Street lights and signs are `fresh`'s. `None` when a
    /// run has no owner. O(chunk vertices) copies, no geometry.
    pub(super) fn splice_owners(
        old: &Self,
        fresh: &Self,
        keep: impl Fn(NetworkMeshOwner) -> bool,
    ) -> Option<Self> {
        let mut spliced = Self::new();
        for layer in 0..LAYERS {
            let old_runs = old.owner_runs(layer)?;
            let fresh_runs = fresh.owner_runs(layer)?;
            let mut old_runs = old_runs.into_iter().filter(|(owner, _)| keep(*owner)).peekable();
            let mut fresh_runs = fresh_runs.into_iter().peekable();
            loop {
                let next = match (old_runs.peek(), fresh_runs.peek()) {
                    (Some(kept), Some(new)) if owner_order(kept.0) < owner_order(new.0) => {
                        old_runs.next().map(|run| (old, run))
                    }
                    (Some(_), None) => old_runs.next().map(|run| (old, run)),
                    _ => fresh_runs.next().map(|run| (fresh, run)),
                };
                let Some((source, (owner, range))) = next else {
                    break;
                };
                spliced.append_run(source, layer, owner, range);
            }
        }
        spliced.street_lights = fresh.street_lights.clone();
        spliced.road_signs = fresh.road_signs.clone();
        Some(spliced)
    }

    /// Whether two meshes carry the same payload: every layer's attributes and owner ranges,
    /// street lights and signs.
    #[cfg(debug_assertions)]
    pub(super) fn same_payload(&self, other: &Self) -> bool {
        (0..LAYERS).all(|layer| {
            let (a, b) = (self.layer(layer), other.layer(layer));
            a.0 == b.0 && a.1 == b.1 && a.2 == b.2 && a.3 == b.3
        }) && self.road_crossings == other.road_crossings
            && self.owner_ranges == other.owner_ranges
            && self.street_lights == other.street_lights
            && self.road_signs == other.road_signs
    }

    // The layer's owner runs in buffer order, as vertex ranges; `None` if a run has no owner.
    fn owner_runs(&self, layer: usize) -> Option<Vec<(NetworkMeshOwner, Range<usize>)>> {
        let mut start = 0;
        self.owner_ranges[layer]
            .iter()
            .map(|range| {
                let run = (range.owner?, start..range.end);
                start = range.end;
                Some(run)
            })
            .collect()
    }

    // Appends `source`'s vertices `range` of `layer` as one run of `owner`.
    fn append_run(
        &mut self,
        source: &Self,
        layer: usize,
        owner: NetworkMeshOwner,
        range: Range<usize>,
    ) {
        let (vertices, normals, uvs, colors) = source.layer(layer);
        if layer == ROAD_LAYER {
            self.road_crossings
                .extend_from_slice(&source.road_crossings[range.clone()]);
        }
        let target = self.layer_mut(layer);
        target.0.extend_from_slice(&vertices[range.clone()]);
        target.1.extend_from_slice(&normals[range.clone()]);
        target.2.extend_from_slice(&uvs[range.clone()]);
        target.3.extend_from_slice(&colors[range]);
        let end = target.0.len();
        self.owner_ranges[layer].push(OwnedVertexRange {
            owner: Some(owner),
            end,
        });
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

// Layers in `owner_ranges` order: earthwork, curb, raised step, sidewalk, road, marking,
// concrete (the `MeshLayer` discriminants); only the road layer carries crossing frames.
const LAYERS: usize = 7;
const ROAD_LAYER: usize = 4;

// Emission order within a layer: every span by edge id, then every node piece by node id.
fn owner_order(owner: NetworkMeshOwner) -> (u8, usize) {
    match owner {
        NetworkMeshOwner::Edge(edge) => (0, edge),
        NetworkMeshOwner::Node(node) => (1, node as usize),
    }
}

// One layer's vertex attribute buffers, shared and mutable, by layer index.
macro_rules! layer_buffers {
    ($mesh:expr, $layer:expr, $($ref:tt)+) => {
        match $layer {
            0 => ($($ref)+ $mesh.earthwork_vertices, $($ref)+ $mesh.earthwork_normals, $($ref)+ $mesh.earthwork_uvs, $($ref)+ $mesh.earthwork_colors),
            1 => ($($ref)+ $mesh.curb_vertices, $($ref)+ $mesh.curb_normals, $($ref)+ $mesh.curb_uvs, $($ref)+ $mesh.curb_colors),
            2 => ($($ref)+ $mesh.raised_step_vertices, $($ref)+ $mesh.raised_step_normals, $($ref)+ $mesh.raised_step_uvs, $($ref)+ $mesh.raised_step_colors),
            3 => ($($ref)+ $mesh.sidewalk_vertices, $($ref)+ $mesh.sidewalk_normals, $($ref)+ $mesh.sidewalk_uvs, $($ref)+ $mesh.sidewalk_colors),
            4 => ($($ref)+ $mesh.road_vertices, $($ref)+ $mesh.road_normals, $($ref)+ $mesh.road_uvs, $($ref)+ $mesh.road_colors),
            5 => ($($ref)+ $mesh.marking_vertices, $($ref)+ $mesh.marking_normals, $($ref)+ $mesh.marking_uvs, $($ref)+ $mesh.marking_colors),
            _ => ($($ref)+ $mesh.concrete_vertices, $($ref)+ $mesh.concrete_normals, $($ref)+ $mesh.concrete_uvs, $($ref)+ $mesh.concrete_colors),
        }
    };
}

type Layer<'a> = (&'a [Vector3], &'a [Vector3], &'a [Vector2], &'a [Color]);
type LayerMut<'a> = (
    &'a mut Vec<Vector3>,
    &'a mut Vec<Vector3>,
    &'a mut Vec<Vector2>,
    &'a mut Vec<Color>,
);

impl NetworkMeshData {
    fn layer(&self, layer: usize) -> Layer<'_> {
        let (vertices, normals, uvs, colors) = layer_buffers!(self, layer, &);
        (vertices, normals, uvs, colors)
    }

    fn layer_mut(&mut self, layer: usize) -> LayerMut<'_> {
        layer_buffers!(self, layer, &mut)
    }
}
