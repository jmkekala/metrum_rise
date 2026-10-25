// SPDX-License-Identifier: GPL-2.0-only

//! What a stroke's profile pins to (`ROAD-68` open question 16): the visible carriageways of the
//! pieces its edit leaves alone, the graph profile of a road it crosses mid-span and the graph
//! height of a node it ends at, never the pieces its own edit rebuilds. Which pieces the edit
//! rebuilds is read off its topology plan, so [`plan_road_edit`] prepares and plans the stroke
//! again until the pieces a preparation read are none its own plan rebuilds.

use super::input::prepare_road_input_excluding;
use super::{PreparedRoadInput, RoadPreviewValidation, RoadTopologyPlan, validate_road_edit};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::{PieceId, RoadbedSystem};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::Vector3;

/// The pieces an edit rebuilds, in live ids, as its topology plan names them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RebuiltPieces {
    // Sorted and deduplicated.
    pieces: Vec<PieceId>,
    // Live edges the stroke splits mid-span, sorted: it meets them at their graph profile.
    crossed: Vec<usize>,
    // The live node each stroke end joins, first and last point.
    joined_ends: [Option<u32>; 2],
}

impl RebuiltPieces {
    pub(super) fn new(
        mut pieces: Vec<PieceId>,
        mut crossed: Vec<usize>,
        joined_ends: [Option<u32>; 2],
    ) -> Self {
        pieces.sort_unstable();
        pieces.dedup();
        crossed.sort_unstable();
        crossed.dedup();
        Self {
            pieces,
            crossed,
            joined_ends,
        }
    }

    // Grows this set by a later plan's: the exclusion only grows, so the iteration ends.
    fn absorb(&mut self, other: Self) {
        let mut pieces = std::mem::take(&mut self.pieces);
        pieces.extend(other.pieces);
        let mut crossed = std::mem::take(&mut self.crossed);
        crossed.extend(other.crossed);
        let ends = [0, 1].map(|end| other.joined_ends[end].or(self.joined_ends[end]));
        *self = Self::new(pieces, crossed, ends);
    }

    fn rebuilds(&self, piece: PieceId) -> bool {
        self.pieces.binary_search(&piece).is_ok()
    }
}

/// Carriageway support of one stroke preparation under a [`RebuiltPieces`] exclusion.
pub(super) struct StrokeSupport<'a> {
    roadbed: &'a RoadbedSystem,
    graph: &'a RegionGraph,
    rebuilt: &'a RebuiltPieces,
    // Pieces outside `rebuilt` whose carriageway this preparation read, in read order.
    read: Vec<PieceId>,
}

impl<'a> StrokeSupport<'a> {
    pub(super) fn new(
        roadbed: &'a RoadbedSystem,
        graph: &'a RegionGraph,
        rebuilt: &'a RebuiltPieces,
    ) -> Self {
        Self {
            roadbed,
            graph,
            rebuilt,
            read: Vec::new(),
        }
    }

    /// Highest carriageway at a world XZ point among the pieces the edit keeps, and the crossed
    /// roads' graph profiles where their rebuilt spans lie. `O(pieces in the point's query cell
    /// + crossed edges' profile points)`.
    pub(super) fn height_at(&mut self, x: f32, z: f32) -> Option<f32> {
        let (rebuilt, graph, read) = (self.rebuilt, self.graph, &mut self.read);
        let mut best: Option<f32> = None;
        self.roadbed.for_each_carriageway_top_at(x, z, |piece, height_m| {
            let height_m = if !rebuilt.rebuilds(piece) {
                read.push(piece);
                height_m
            } else if let PieceId::Span(edge) = piece
                && rebuilt.crossed.binary_search(&edge).is_ok()
            {
                RegionGraph::profile_height_at(&graph.edge(edge).physical_geometry, x, z)
            } else {
                return;
            };
            if best.is_none_or(|best_m| height_m > best_m) {
                best = Some(height_m);
            }
        });
        best
    }

    /// The graph height of the node a stroke end joins (`0` first point, `1` last), so the end
    /// still snaps to a node its edit rebuilds.
    pub(super) fn end_height(&self, end: usize) -> Option<f32> {
        self.rebuilt.joined_ends[end].map(|node| self.graph.node(node).pos.y)
    }

    /// The kept pieces whose carriageway this preparation read.
    pub(super) fn into_read(self) -> Vec<PieceId> {
        self.read
    }
}

/// Prepares, validates and plans a stroke so that its profile reads no piece its own edit
/// rebuilds (`ROAD-68` open question 16). The first pass reads every visible carriageway; while a
/// pass read a piece its own plan rebuilds, the stroke is prepared and planned again with those
/// pieces excluded as well. The exclusion only grows and each further pass adds a piece the
/// previous one read, so the passes end; a stroke that reads no carriageway takes one pass, a
/// join or crossing usually two. The commit, the preview and the editor's checks all plan here.
#[allow(clippy::too_many_arguments)]
pub(crate) fn plan_road_edit(
    raw_points: &[Vector3],
    fwd_lanes: u8,
    bkw_lanes: u8,
    terrain: &TerrainSystem,
    graph: &RegionGraph,
    roadbed: &RoadbedSystem,
    snap_to_existing_roads: bool,
) -> (
    PreparedRoadInput,
    RoadPreviewValidation,
    Option<RoadTopologyPlan>,
) {
    let mut excluded = RebuiltPieces::default();
    loop {
        let (prepared, read) = prepare_road_input_excluding(
            raw_points,
            terrain,
            graph,
            roadbed,
            snap_to_existing_roads,
            &excluded,
        );
        let (validation, topology) =
            validate_road_edit(&prepared, fwd_lanes, bkw_lanes, terrain, graph);
        let rebuilt = topology
            .as_ref()
            .map(RoadTopologyPlan::rebuilt_pieces)
            .unwrap_or_default();
        if !read.iter().any(|&piece| rebuilt.rebuilds(piece)) {
            return (prepared, validation, topology);
        }
        excluded.absorb(rebuilt);
    }
}
