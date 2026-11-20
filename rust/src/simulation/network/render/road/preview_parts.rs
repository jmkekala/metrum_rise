// SPDX-License-Identifier: GPL-3.0-or-later

//! A batch preview's overlay built over several frames (`ROAD-112` round G1b): the chunks
//! [`RoadRenderer::generate_preview_overlay`](super::RoadRenderer::generate_preview_overlay)
//! builds in one call over a batch's changed chunks, built one owner group a frame. The groups
//! are runs of the closure in emission order (spans, then nodes, by id), so in every layer a
//! group's runs follow the runs built before and append to them; an owner's triangles do not
//! depend on which other owners are emitted with it (the rule the commit's splice rests on), so
//! the appended chunk is the one-call chunk bit for bit. Groups are cut at equal shares of the
//! built pieces' triangles, so a frame's cost follows its share, not the batch.

use super::super::NetworkMeshData;
use super::super::ownership::LAYERS;
use super::{NetworkMeshOwner, PreviewChunk, RoadChunkMap, emit_chunks, standard_surface};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::{PieceId, RoadbedSystem, SurfaceChunkKey};
use crate::simulation::terrain::graded::SiteSource;
use godot::prelude::Vector3;
use std::collections::{BTreeMap, BTreeSet};

/// A batch preview's overlay under construction, one owner group a frame (module docs).
pub(crate) struct PreviewOverlayParts {
    // Every owner the batch's deltas cover, with whether the batch's roadbed holds a piece for
    // it, in emission order.
    closure: Vec<(PieceId, bool)>,
    // Each group's end in `closure`, ascending; the last is `closure.len()`.
    group_ends: Vec<usize>,
    next_group: usize,
    target_chunks: BTreeSet<SurfaceChunkKey>,
    chunks: BTreeMap<SurfaceChunkKey, PartChunk>,
}

// One chunk's overlay so far, with the published runs its owners hold, by layer.
struct PartChunk {
    overlay: NetworkMeshData,
    // Every appended part passed the seal's check.
    valid: bool,
    hidden: [Vec<[u32; 3]>; LAYERS],
    positions: [Vec<Vector3>; LAYERS],
}

impl PreviewOverlayParts {
    /// The overlay of `target_chunks` (every chunk the batch changed) for `closure` (every
    /// owner its deltas cover, with whether `roadbed` holds a piece for it, ascending: the
    /// emission order), cut into at most `groups` runs of equal triangle shares.
    /// `O(closure)`.
    pub(crate) fn new(
        roadbed: &RoadbedSystem,
        target_chunks: BTreeSet<SurfaceChunkKey>,
        closure: Vec<(PieceId, bool)>,
        groups: usize,
    ) -> Self {
        debug_assert!(closure.windows(2).all(|pair| pair[0].0 < pair[1].0));
        let weight = |&(id, built): &(PieceId, bool)| {
            built
                .then(|| roadbed.piece_mesh(id))
                .flatten()
                .map_or(0, |mesh| mesh.triangles().len())
        };
        let total: usize = closure.iter().map(weight).sum();
        let groups = groups.max(1);
        let mut group_ends = Vec::with_capacity(groups);
        let mut sum = 0;
        for (index, entry) in closure.iter().enumerate() {
            sum += weight(entry);
            // A group ends at the owner that reaches the next share.
            if group_ends.len() + 1 < groups && sum * groups >= total * (group_ends.len() + 1) {
                group_ends.push(index + 1);
            }
        }
        group_ends.push(closure.len());
        group_ends.dedup();
        Self {
            closure,
            group_ends,
            next_group: 0,
            target_chunks,
            chunks: BTreeMap::new(),
        }
    }

    /// Whether every group is built.
    pub(crate) fn is_done(&self) -> bool {
        self.next_group == self.group_ends.len()
    }

    /// Emits the next group into the target chunks and finds its owners' published runs, on
    /// the graph and `roadbed` with every stroke applied. `None` when a searched published run
    /// has no owner (the one-call overlay is `None` then too). `O(the group's pieces' triangles
    /// + chunks * (group * layers * log runs) + its hidden vertices)`.
    pub(crate) fn build_next(
        &mut self,
        graph: &RegionGraph,
        terrain: &crate::simulation::terrain::TerrainSystem,
        sites: &dyn SiteSource,
        roadbed: &RoadbedSystem,
        published: &RoadChunkMap,
    ) -> Option<()> {
        let start = self
            .next_group
            .checked_sub(1)
            .map_or(0, |group| self.group_ends[group]);
        let group = &self.closure[start..self.group_ends[self.next_group]];
        self.next_group += 1;
        let rebuilt = standard_surface::coverage_of_pieces(
            graph,
            group.iter().filter(|(_, built)| *built).map(|(id, _)| *id),
        );
        let mut emitted = emit_chunks(
            graph,
            terrain,
            roadbed,
            sites,
            &self.target_chunks,
            &rebuilt,
            &standard_surface::coverage_of_pieces(graph, []),
        );
        // Ascending already: `PieceId` orders as the emission does.
        let owners: Vec<_> = group
            .iter()
            .map(|(id, _)| match *id {
                PieceId::Span(edge) => NetworkMeshOwner::Edge(edge),
                PieceId::Node(node) => NetworkMeshOwner::Node(node),
            })
            .collect();
        let mut positions = Vec::new();
        for &chunk in &self.target_chunks {
            let entry = self.chunks.entry(chunk).or_insert_with(|| PartChunk {
                overlay: NetworkMeshData::new(),
                valid: true,
                hidden: Default::default(),
                positions: Default::default(),
            });
            if let Some(part) = emitted.remove(&chunk) {
                entry.valid &= part.render_payload_is_valid();
                entry.overlay.append_owner_runs(&part)?;
            }
            let Some(mesh) = published.get(&chunk) else {
                continue;
            };
            positions.clear();
            let mut at = 0;
            for range in mesh.owner_vertex_ranges(&owners, &mut positions)? {
                let [layer, start, end] = range;
                let next = at + (end - start) as usize;
                entry.hidden[layer as usize].push(range);
                entry.positions[layer as usize].extend_from_slice(&positions[at..next]);
                at = next;
            }
        }
        Some(())
    }

    /// The overlay once every group is built: each chunk with anything to change, as the one
    /// call builds it. `O(chunks + hidden ranges and positions)`, no geometry.
    pub(crate) fn finish(self) -> BTreeMap<SurfaceChunkKey, PreviewChunk> {
        debug_assert!(self.is_done());
        self.chunks
            .into_iter()
            .filter_map(|(chunk, part)| {
                let hidden = part.hidden.concat();
                if part.overlay.is_empty() && hidden.is_empty() {
                    return None;
                }
                Some((
                    chunk,
                    PreviewChunk {
                        overlay: part.overlay.seal_parts(part.valid),
                        hidden,
                        hidden_positions: part.positions.concat(),
                    },
                ))
            })
            .collect()
    }
}
