// SPDX-License-Identifier: GPL-2.0-only

//! One site for the graph-only road edits (`ZONE-10`, round RZ10b, PLAN-RZ 10.3): the graph an
//! edit is tried on, the published roadbed it compiles against, the terrain and water it reads
//! and the land it takes. The live core builds one for its commits and one-shot verdicts
//! ([`SimCore::road_edit_site`]); the road tool's context builds one on its replica for hovers
//! that must not wait for the core ([`RoadPreviewContext::road_edit_site`]). The check is the
//! same code on both, so a hover's verdict is the commit's on the same inputs.
//!
//! [`RoadEditSite::try_graph_edit`] applies an edit to the graph, compiles the closure it
//! dirtied into an `O(1)` copy of the published roadbed, judges the land the new pieces newly
//! cover ([`RoadLandTake::of`]) and the grades they steepen, and puts the graph back from a local
//! checkpoint: nothing outside the graph changes, so a refusal is atomic and the checkpoint is
//! the edit's undo record. Each edit's `apply` and corridor checks live beside the edit, as
//! methods on the site. `O(local records + closure pieces * reservations in their bounds)`.

use std::collections::{HashMap, HashSet};

use super::road_edit_plan::{CoreLand, RoadLandSource, RoadLandTake};
use super::road_tool_context::RoadPreviewContext;
use super::road_tool_land::ViewLand;
use super::{RoadRejection, SimCore};
use crate::simulation::agriculture::PolygonFootprint;
use crate::simulation::network::graph::{RegionGraph, RegionGraphUndoDelta};
use crate::simulation::network::roadbed::{RoadbedDelta, RoadbedScratch, RoadbedSystem};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;

/// The inputs a graph-only road edit is tried on: a graph it may edit and must put back, the
/// published roadbed of that graph, the terrain and water, and the land it takes.
pub(crate) struct RoadEditSite<'a, L> {
    /// The graph the edit is applied to: the live core's, or the road tool's replica.
    pub(crate) graph: &'a mut RegionGraph,
    /// The roadbed published for `graph`: the pieces an edit replaces.
    pub(crate) roadbed: &'a RoadbedSystem,
    /// The terrain the profile solve reads.
    pub(crate) terrain: &'a TerrainSystem,
    /// The authored water the corridor checks read.
    pub(crate) water: &'a WaterSystem,
    /// The land the edit takes ([`RoadLandTake::of`]).
    pub(crate) land: L,
    // The roadbed builders' buffers, reused by a caller that compiles every frame (the road
    // tool's context); `None` builds with fresh buffers. The pieces do not depend on it.
    scratch: Option<&'a mut RoadbedScratch>,
}

/// A graph-only edit [`RoadEditSite::try_graph_edit`] accepted (`ZONE-10`): its undo record and
/// the zoned land its pieces newly cover, buildings named as the site's land names them.
pub(crate) struct TriedGraphEdit<B = usize> {
    /// The local checkpoint of the edit's records: its undo record.
    pub(crate) checkpoint: RegionGraphUndoDelta,
    /// What the edit takes; never plotted.
    pub(crate) take: RoadLandTake<B>,
}

impl SimCore {
    /// The live core as a [`RoadEditSite`]: its graph, published roadbed, terrain, water and
    /// land ([`CoreLand`]). The take reads the explicit sites through the site query index as it
    /// stands: a commit prepares it first.
    pub(crate) fn road_edit_site(&mut self) -> RoadEditSite<'_, CoreLand<'_>> {
        RoadEditSite {
            graph: &mut self.region_graph,
            roadbed: &self.transit_network.roadbed,
            terrain: &self.heightmap,
            water: &self.watermap,
            land: CoreLand::from_parts(&self.allocator, &self.zoning, self.config.zone_cell_m),
            scratch: None,
        }
    }
}

impl RoadPreviewContext {
    /// The road tool's context as a [`RoadEditSite`]: its replica of the published graph, the
    /// view's roadbed, terrain and water, and the view's land ([`ViewLand`]), with the context's
    /// roadbed scratch. No core lock.
    pub(crate) fn road_edit_site(&mut self) -> RoadEditSite<'_, ViewLand<'_>> {
        RoadEditSite {
            graph: &mut self.replica,
            roadbed: &self.view.roadbed,
            terrain: &self.view.terrain,
            water: &self.view.water,
            land: ViewLand {
                land: &self.land,
                cells: &self.view.cells,
            },
            scratch: Some(&mut self.roadbed_scratch),
        }
    }
}

impl<L: RoadLandSource> RoadEditSite<'_, L> {
    /// Tries a graph-only edit that dirties nodes (`ROAD-68` open questions 2 and 11 (c)):
    /// applies it, compiles the closure of `edges`, `nodes` and the edges `apply` returns into a
    /// scratch roadbed, and puts the graph back from a local checkpoint of `edges` and `nodes`,
    /// which must cover every record `apply` changes. `Ok` with that checkpoint, the edit's undo
    /// record, and the zoned land the closure's pieces and the `extra` footprints newly cover
    /// (`ZONE-10`, [`RoadLandTake`]), when the closure compiles, covers no plotted reservation (a
    /// field, an extraction area or an explicit site's lot) and steepens no profile past the 1:3
    /// maximum grade; nothing outside the graph has changed either way. O(local records +
    /// closure pieces * reservations in their bounds).
    pub(crate) fn try_graph_edit(
        &mut self,
        edges: &HashSet<usize>,
        nodes: &HashSet<u32>,
        extra: &[PolygonFootprint<'static>],
        apply: impl FnOnce(&mut Self) -> HashSet<usize>,
    ) -> Result<TriedGraphEdit<L::Building>, RoadRejection> {
        let checkpoint = self
            .graph
            .capture_undo_for_local_topology(edges.clone(), nodes.clone());
        let grades_before: HashMap<usize, f32> = edges
            .iter()
            .filter(|&&edge| edge < self.graph.edge_count())
            .map(|&edge| (edge, edge_grade(self.graph, edge)))
            .collect();
        let changed = apply(self);
        let dirty: HashSet<usize> = edges.iter().copied().chain(changed).collect();
        let tried = self.closure_take(&dirty, nodes, extra).and_then(|take| {
            steepened_profile(self.graph, &dirty, &grades_before).map_or(Ok(take), Err)
        });
        self.graph.restore_undo_delta_local(&checkpoint);
        tried.map(|take| TriedGraphEdit { checkpoint, take })
    }

    // What the pieces of a node-dirtying edit take (`ROAD-68` open questions 2 and 11 (c),
    // `ZONE-10`), or why they cannot publish: a node of the closure fails its layout or profile
    // solve, or a piece or `extra` footprint newly covers a plotted reservation. Compiles the
    // closure of `edges` and `nodes` into an `O(1)` copy of the published roadbed, with the
    // graph as it stands; the replaced pieces are the published ones of the same ids, so an
    // owner the edit deletes (a ring's retired junction) counts as covering its land already.
    // O(closure pieces * reservations in their bounds).
    fn closure_take(
        &mut self,
        edges: &HashSet<usize>,
        nodes: &HashSet<u32>,
        extra: &[PolygonFootprint<'static>],
    ) -> Result<RoadLandTake<L::Building>, RoadRejection> {
        let delta = compile_closure(
            self.graph,
            self.roadbed,
            self.scratch.as_deref_mut(),
            edges,
            nodes,
        );
        if let Some((node, error)) = delta.failed_nodes().next() {
            return Err(RoadRejection::new(
                "node_piece_failed",
                format!("node {node} fails its junction layout or profile solve: {error:?}"),
            ));
        }
        let before: Vec<_> = delta
            .pieces()
            .filter_map(|(id, _)| self.roadbed.piece_mesh(id))
            .collect();
        let installed: Vec<_> = delta.pieces().filter_map(|(_, mesh)| mesh).collect();
        let take = RoadLandTake::of(&self.land, &installed, &before, extra);
        match take.plotted {
            Some(overlap) => Err(overlap.rejection_on(&self.land)),
            None => Ok(take),
        }
    }
}

impl<L> RoadEditSite<'_, L> {
    /// The build cost of `edges` in units of `BUILD_COST_PER_M`: each one's road weight times
    /// its length (D14). O(edges).
    pub(crate) fn build_cost_of(&self, edges: &[usize]) -> f64 {
        edges
            .iter()
            .map(|&edge| {
                let edge = self.graph.edge(edge);
                edge.cost_weight() * f64::from(edge.physical_length)
            })
            .sum()
    }
}

/// The closure of `edges` and `nodes` compiled, unpublished, into an `O(1)` copy of `roadbed`
/// with `graph` as it stands; ids past the graph's records are skipped. `scratch` holds the
/// builders' buffers (fresh ones without it); the pieces do not depend on it. O(closure pieces).
pub(crate) fn compile_closure(
    graph: &RegionGraph,
    roadbed: &RoadbedSystem,
    scratch: Option<&mut RoadbedScratch>,
    edges: &HashSet<usize>,
    nodes: &HashSet<u32>,
) -> RoadbedDelta {
    let mut copy = roadbed.snapshot();
    for &edge in edges.iter().filter(|&&edge| edge < graph.edge_count()) {
        copy.mark_edge_dirty(edge);
    }
    for &node in nodes
        .iter()
        .filter(|&&node| (node as usize) < graph.node_count())
    {
        copy.mark_node_dirty(graph.get_valid_node(node));
    }
    match scratch {
        Some(scratch) => copy.compile_delta_with(graph, scratch),
        None => copy.compile_delta(graph),
    }
}

/// The grade of one edge's profile by the one grade (`grade::steepest_grade`); zero without a
/// graded segment.
pub(crate) fn edge_grade(graph: &RegionGraph, edge: usize) -> f32 {
    crate::simulation::network::grade::steepest_grade(
        crate::simulation::network::validate::centreline(graph.edge(edge)),
    )
    .map_or(0.0, |steepest| steepest.grade)
}

/// The first of `edges`, in id order, whose profile an edit made illegally steep (`ROAD-68`
/// open question 37): above the 1:3 maximum and steeper than `before` held it. A profile
/// already that steep is the existing road's, so editing beside it stays possible.
/// O(points on `edges`).
pub(crate) fn steepened_profile(
    graph: &RegionGraph,
    edges: &HashSet<usize>,
    before: &HashMap<usize, f32>,
) -> Option<RoadRejection> {
    use crate::simulation::network::validate::{
        RoadGeometryLimits, RoadGeometryTier, RoadGeometryViolation, grade_violation,
    };
    let mut edges: Vec<usize> = edges
        .iter()
        .copied()
        .filter(|&edge| edge < graph.edge_count())
        .collect();
    edges.sort_unstable();
    let limits = RoadGeometryLimits::default();
    edges.into_iter().find_map(|edge| {
        let old = before.get(&edge).copied().unwrap_or(-1.0);
        match grade_violation(graph, edge, &limits)? {
            RoadGeometryViolation::Grade {
                measured,
                limit,
                tier: RoadGeometryTier::Illegal,
                ..
            } if measured > old => Some(RoadRejection::new(
                "road_too_steep",
                format!(
                    "edge {edge} would climb at {:.0}%, steeper than the {:.0}% (1:3) maximum \
                     grade",
                    measured * 100.0,
                    limit * 100.0
                ),
            )),
            _ => None,
        }
    })
}
