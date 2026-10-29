// SPDX-License-Identifier: GPL-2.0-only

//! Exact source ranges in cached road meshes for reversible local editor replacements.

use super::NetworkMeshData;
use godot::prelude::{Color, Vector2, Vector3};
use std::ops::Range;

/// Authoritative graph owner of a contiguous rendered triangle range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum NetworkMeshOwner {
    /// A physical road span, including its markings, skirts and bridge concrete.
    Edge(usize),
    /// A junction, including its crosswalks and skirts.
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
            // Sized once: growing the layer run by run copied its buffers again and again.
            let kept: usize = old_runs
                .iter()
                .filter(|(owner, _)| keep(*owner))
                .map(|(_, range)| range.len())
                .sum();
            spliced.reserve_layer(layer, kept + fresh.layer(layer).0.len());
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

    // Reserves exactly `vertices` more vertices in every attribute buffer of `layer`.
    fn reserve_layer(&mut self, layer: usize, vertices: usize) {
        if layer == ROAD_LAYER {
            self.road_crossings.reserve_exact(vertices);
        }
        let target = self.layer_mut(layer);
        target.0.reserve_exact(vertices);
        target.1.reserve_exact(vertices);
        target.2.reserve_exact(vertices);
        target.3.reserve_exact(vertices);
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

    /// The vertex ranges of `owners`' runs in this published chunk, by layer, then in buffer
    /// order, as `[layer, start, end]`, with their positions appended to `positions` in the same
    /// order (`ROAD-68-H` round 3): the runs a preview's overlay replaces. `owners` ascend in
    /// emission order, as each layer's runs do, so each owner is one binary search. `None` if a
    /// searched run has no owner. `O(owners * layers * log runs + found vertices)`.
    pub(super) fn owner_vertex_ranges(
        &self,
        owners: &[NetworkMeshOwner],
        positions: &mut Vec<Vector3>,
    ) -> Option<Vec<[u32; 3]>> {
        let mut ranges = Vec::new();
        for (layer, runs) in self.owner_ranges.iter().enumerate() {
            let vertices = self.layer(layer).0;
            for &owner in owners {
                let order = owner_order(owner);
                let index = runs.partition_point(|run| run.owner.map(owner_order) < Some(order));
                let Some(run) = runs.get(index) else {
                    continue;
                };
                if run.owner? != owner {
                    continue;
                }
                let start = index.checked_sub(1).map_or(0, |previous| runs[previous].end);
                positions.extend_from_slice(&vertices[start..run.end]);
                ranges.push([layer as u32, start as u32, run.end as u32]);
            }
        }
        Some(ranges)
    }

    /// Test builds: the `[layer, start, end]` ranges of the runs `keep` rejects, by layer, then
    /// in buffer order: what [`Self::splice_owners`] drops, found by visiting every run.
    #[cfg(debug_assertions)]
    pub(super) fn vertex_ranges_dropped(
        &self,
        keep: impl Fn(NetworkMeshOwner) -> bool,
    ) -> Vec<[u32; 3]> {
        (0..LAYERS)
            .flat_map(|layer| {
                self.owner_runs(layer)
                    .into_iter()
                    .flatten()
                    .filter(|(owner, _)| !keep(*owner))
                    .map(move |(_, range)| [layer as u32, range.start as u32, range.end as u32])
            })
            .collect()
    }

    /// Tests: `published` with the `hidden` runs dropped and `overlay`'s merged in owner order,
    /// as Godot composes a preview chunk; lights and signs are `overlay`'s.
    #[cfg(test)]
    pub(crate) fn compose_preview(
        published: Option<&Self>,
        overlay: &Self,
        hidden: &[[u32; 3]],
    ) -> Self {
        let Some(published) = published else {
            return Self::splice_owners(&Self::new(), overlay, |_| false)
                .expect("an overlay's runs have owners");
        };
        let mut hidden_owners = std::collections::HashSet::new();
        for &[layer, start, end] in hidden {
            let run = published
                .owner_runs(layer as usize)
                .expect("published runs have owners")
                .into_iter()
                .find(|(_, range)| *range == (start as usize..end as usize))
                .expect("a hidden range is one published run");
            hidden_owners.insert(run.0);
        }
        Self::splice_owners(published, overlay, |owner| !hidden_owners.contains(&owner))
            .expect("published runs have owners")
    }

    /// Tests: whether two meshes hold the same layers, crossing frames and owner runs (not
    /// lights or signs).
    #[cfg(test)]
    pub(crate) fn same_surfaces(&self, other: &Self) -> bool {
        (0..LAYERS).all(|layer| {
            let (a, b) = (self.layer(layer), other.layer(layer));
            a.0 == b.0 && a.1 == b.1 && a.2 == b.2 && a.3 == b.3
        }) && self.road_crossings == other.road_crossings
            && self.owner_ranges == other.owner_ranges
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

// Layers in `owner_ranges` order: skirt, curb, raised step, sidewalk, road, marking,
// concrete (the `MeshLayer` discriminants); only the road layer carries crossing frames.
const LAYERS: usize = 7;
const ROAD_LAYER: usize = 4;

// Emission order within a layer: every span by edge id, then every node piece by node id.
pub(super) fn owner_order(owner: NetworkMeshOwner) -> (u8, usize) {
    match owner {
        NetworkMeshOwner::Edge(edge) => (0, edge),
        NetworkMeshOwner::Node(node) => (1, node as usize),
    }
}

// One layer's vertex attribute buffers, shared and mutable, by layer index.
macro_rules! layer_buffers {
    ($mesh:expr, $layer:expr, $($ref:tt)+) => {
        match $layer {
            0 => ($($ref)+ $mesh.skirt_vertices, $($ref)+ $mesh.skirt_normals, $($ref)+ $mesh.skirt_uvs, $($ref)+ $mesh.skirt_colors),
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
