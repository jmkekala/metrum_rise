// SPDX-License-Identifier: GPL-2.0-only

//! The turn pocket edit (`ROAD-95`, PLAN-R5 2.5) on the cross-section edit's transaction shape
//! (`ROAD-80`, `ROAD-74`): the plan ([`PocketPlan`]) is applied to the graph, the junction, the
//! transition and the base road's far node are laid out again, their closure is compiled into a
//! scratch roadbed (fields, painted cells, steepened profiles, node pieces) and the graph is put
//! back; a pocket that widens the road is also checked against water, parcels and the curve
//! radius on its carriageway's own axis (D17: the offset polyline, exact on both sides). The
//! preview is that check, so the verdict is the commit's (D11). The commit applies the plan
//! again, moves the pair's buildings and occupancy (journaled), rebuilds lanes, cars and
//! entrances locally and pushes the check's checkpoint with the dependents journal as its undo
//! record. The check is `O(edge points + closure pieces)`; the commit adds `O(the pair's
//! buildings + occupancy + local lanes)` and the undo's parallel `O(A / P + B / P)` membership
//! passes. Nothing here runs per tick.

use super::core::{RoadRejection, SimCore, SimulationRuntimeSnapshot, SimulationSnapshot};
use crate::debug_log;
use crate::simulation::network::graph::RegionGraphUndoDelta;
use crate::simulation::network::pockets::{
    PocketApplied, PocketDependentsUndo, PocketPlan, PocketRequest, PocketState,
    migrate_pocket_dependents,
};
use crate::simulation::network::road_types::cost::BUILD_COST_PER_M;
use crate::simulation::network::roadbed::{EdgeEnd, side_band_width_m};
use crate::simulation::network::types::EdgeClass;
use crate::simulation::network::validate::{
    RoadGeometryLimits, RoadGeometryTier, RoadGeometryViolation, centreline, validate_edge,
};
use std::collections::HashSet;

/// What an accepted pocket edit charges, for the Road Properties window's Apply label.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PocketVerdict {
    /// The build cost of the pair after the edit less before it, floored at 0 (D14: the
    /// in-place rule, never refunded).
    pub(crate) cost_delta: f64,
}

// One tried edit: its undo checkpoint, the plan, the nodes laid out and the verdict.
struct TriedPocket {
    checkpoint: RegionGraphUndoDelta,
    plan: PocketPlan,
    edges: HashSet<usize>,
    nodes: HashSet<u32>,
    verdict: PocketVerdict,
}

impl SimCore {
    /// The turn pocket at end `end` of road `edge_id` (or reached through its transition), if
    /// any. `O(degree)`.
    pub(crate) fn get_road_pocket_internal(
        &self,
        edge_id: i32,
        end: EdgeEnd,
    ) -> Option<PocketState> {
        let edge = usize::try_from(edge_id)
            .ok()
            .filter(|&edge| edge < self.region_graph.edge_count())?;
        PocketState::find(&self.region_graph, edge, end)
    }

    /// Gives a road end the turn pocket `request` asks for, resizes or changes its pocket, or
    /// removes it (both roles off), with an undo record (`ROAD-95`). Returns whether it was
    /// applied; a refused edit ([`Self::preview_road_pocket_internal`]) changes nothing and
    /// leaves its reason in `last_road_rejection`. The caller publishes the rebuilt pieces
    /// ([`Self::finalize_network_render_products`]), as after a crossing toggle. The verdict's
    /// cost is charged outside `benchmark_mode`.
    pub(crate) fn set_road_pocket_internal(&mut self, request: PocketRequest) -> bool {
        // A commit's deferred lots first: this edit reads or writes them (`ROAD-68-G` part 7).
        self.adopt_pending_cell_lots();
        let tried = match self.try_pocket_edit(request) {
            Ok(tried) => tried,
            Err(rejection) => {
                debug_log!(
                    "road",
                    "pocket_edit_rejected request={:?} reason={} detail={}",
                    request,
                    rejection.reason,
                    rejection.detail
                );
                self.last_road_rejection = Some(rejection);
                return false;
            }
        };
        let TriedPocket {
            checkpoint,
            plan,
            edges,
            nodes,
            verdict,
        } = tried;
        let (applied, changed) = self.apply_pocket_plan(&plan, &nodes);
        let affected: HashSet<usize> = edges
            .into_iter()
            .chain(changed.iter().copied())
            .chain(applied.edges())
            .collect();
        let nodes: HashSet<u32> = nodes.into_iter().chain(applied.transition).collect();
        let dependents = migrate_pocket_dependents(
            &self.region_graph,
            &applied,
            &self.zoning,
            &mut self.allocator,
        );
        // The frontage moves with the split: the pair's lots and its neighbours' mouths.
        self.zoning
            .mark_cell_lots_for_roads(&self.region_graph, affected.iter().copied());
        self.transit_network
            .mark_surface_dirty_for_nodes(&self.region_graph, &nodes);
        self.transit_network
            .mark_surface_dirty_from_sets(&affected, &HashSet::new());
        // The lanes of the three nodes' arms, the cars on them and the entrances fronting them,
        // as an undo restores them. Godot's main thread runs this edit, and the passes are
        // parallel: shared pool (CODE-17).
        crate::thread_pools::install(|| self.restore_lanes_locally(&affected, &nodes));
        self.push_pocket_undo(checkpoint, dependents);
        if !self.benchmark_mode && verdict.cost_delta > 0.0 {
            self.treasury.deduct_build_cost(verdict.cost_delta);
        }
        // Deferred like a commit's (D7, `ROAD-68-G`).
        self.schedule_cch_rebuild_of_live_graph();
        self.mark_local_network_render_dirty();
        true
    }

    /// The commit's verdict on `request` now ([`Self::set_road_pocket_internal`]): its
    /// [`PocketVerdict`] when it would apply it, else why it would refuse it. The commit's own
    /// check, run on the graph and put back (D11). `O(edge points + closure pieces)`.
    pub(crate) fn preview_road_pocket_internal(
        &mut self,
        request: PocketRequest,
    ) -> Result<PocketVerdict, RoadRejection> {
        self.adopt_pending_cell_lots();
        self.try_pocket_edit(request).map(|tried| tried.verdict)
    }

    // The edit's check: the plan's cheap refusals, then the graph edit, whose closure must
    // compile (a junction corner that no longer fits beside the wider arm is
    // `pocket_road_too_short`, PLAN-R5 risk 3), keep clear of reserved land and steepen no
    // profile; a widening pocket's corridor must also keep clear of water and parcels and fit
    // its curves.
    fn try_pocket_edit(&mut self, request: PocketRequest) -> Result<TriedPocket, RoadRejection> {
        let plan = PocketPlan::prepare(&self.region_graph, request)
            .map_err(|reason| RoadRejection::new(reason, format!("{request:?}")))?;
        let graph = &self.region_graph;
        // The transition an add appends is the next node id; ids past the graph are skipped
        // until it exists.
        let nodes: HashSet<u32> = plan
            .nodes()
            .into_iter()
            .map(|node| graph.get_valid_node(node))
            .chain([graph.node_count() as u32])
            .collect();
        let edges: HashSet<usize> = plan
            .source_edges()
            .into_iter()
            .chain(
                nodes
                    .iter()
                    .filter(|&&node| (node as usize) < graph.node_count())
                    .flat_map(|&node| graph.node_adjacency(node).iter().copied()),
            )
            .filter(|&edge| !graph.edge(edge).deleted)
            .collect();
        let cost_before = self.build_cost_of(&plan.source_edges());
        // Any planned pocket is checked: a role added widens the road, a longer body widens more
        // of it.
        let widens = plan.pocket().is_some();
        let mut corridor = None;
        let mut cost_after = 0.0;
        let tried = self.try_graph_edit(&edges, &nodes, |core| {
            let (applied, changed) = core.apply_pocket_plan(&plan, &nodes);
            let pieces: Vec<usize> = applied
                .edges()
                .filter(|&edge| !core.region_graph.edge(edge).deleted)
                .collect();
            cost_after = core.build_cost_of(&pieces);
            if widens && let Some(pocket_edge) = applied.pocket_edge {
                corridor = core.pocket_corridor_refusal(pocket_edge);
            }
            changed.into_iter().chain(applied.edges()).collect()
        });
        let tried = tried.map_err(|rejection| {
            if rejection.reason == "node_piece_failed" {
                RoadRejection::new("pocket_road_too_short", rejection.detail)
            } else {
                rejection
            }
        });
        if let Some(rejection) = corridor {
            return Err(rejection);
        }
        let checkpoint = tried?;
        Ok(TriedPocket {
            checkpoint,
            plan,
            edges,
            nodes,
            verdict: PocketVerdict {
                cost_delta: (cost_after - cost_before).max(0.0) * BUILD_COST_PER_M,
            },
        })
    }

    // The build cost of `edges` in units of `BUILD_COST_PER_M`: each one's road weight times its
    // length (D14). O(edges).
    fn build_cost_of(&self, edges: &[usize]) -> f64 {
        edges
            .iter()
            .map(|&edge| {
                let edge = self.region_graph.edge(edge);
                edge.cost_weight() * f64::from(edge.physical_length)
            })
            .sum()
    }

    // Applies the plan on the graph and lays out `nodes` and the transition again: their clips
    // and the profile solve that pins their arms up to them. Returns what the plan changed and
    // the edges whose profiles the solve changed.
    fn apply_pocket_plan(
        &mut self,
        plan: &PocketPlan,
        nodes: &HashSet<u32>,
    ) -> (PocketApplied, HashSet<usize>) {
        let applied = plan.apply_to_graph(&mut self.region_graph);
        let nodes: HashSet<u32> = nodes
            .iter()
            .copied()
            .filter(|&node| (node as usize) < self.region_graph.node_count())
            .chain(applied.transition)
            .collect();
        self.region_graph
            .rebuild_intersection_clips_for_nodes(&nodes);
        let mut changed = HashSet::new();
        self.region_graph.finalize_junction_endpoint_profiles(
            &nodes,
            Some(&self.heightmap),
            &mut changed,
        );
        (applied, changed)
    }

    // Why pocket edge `edge_idx` (applied on the graph) cannot stay, read on its carriageway's
    // axis (D17), whose half-width is exact on both sides: a standard road over water, a parcel
    // inside its corridor, or a curve tighter than the illegal radius at its width. The far side
    // keeps the base road's line, so a parcel there stays legal. O(points + parcel hits).
    fn pocket_corridor_refusal(&self, edge_idx: usize) -> Option<RoadRejection> {
        let edge = self.region_graph.edge(edge_idx);
        let axis = edge.carriageway_axis(centreline(edge));
        let half_width_m = edge.width * 0.5 + side_band_width_m(edge);
        if edge.class == EdgeClass::Standard
            && self
                .watermap
                .road_corridor_overlaps_visible_water(&axis, half_width_m)
        {
            return Some(RoadRejection::new(
                "water_requires_bridge",
                format!("pocket edge {edge_idx} would reach visible water"),
            ));
        }
        let parcels = self
            .zoning
            .parcel_ids_overlapping_road_corridor(&axis, half_width_m);
        if let Some(first) = parcels.first() {
            return Some(RoadRejection::new(
                "parcel_overlap",
                format!(
                    "pocket edge {edge_idx} would overlap {} parcel(s), first {first}",
                    parcels.len()
                ),
            ));
        }
        let mut violations = Vec::new();
        validate_edge(
            &self.region_graph,
            edge_idx,
            &RoadGeometryLimits::default(),
            &mut violations,
        );
        violations.into_iter().find_map(|violation| match violation {
            RoadGeometryViolation::HorizontalRadius {
                measured,
                limit,
                tier: RoadGeometryTier::Illegal,
                ..
            } => Some(RoadRejection::new(
                "cross_section_too_wide_for_curve",
                format!(
                    "pocket edge {edge_idx} turns at a {measured:.1} m radius; its width needs {limit:.1} m"
                ),
            )),
            _ => None,
        })
    }

    // The pocket edit's undo record: the graph checkpoint (whole records; the transition and
    // second edge an add appended drop past the original counts) and the dependents journal.
    fn push_pocket_undo(
        &mut self,
        checkpoint: RegionGraphUndoDelta,
        dependents: PocketDependentsUndo,
    ) {
        self.push_undo_snapshot(SimulationSnapshot {
            roadbed: None,
            graded: None,
            terrain: None,
            water: None,
            trans_graph: Some(checkpoint),
            zoning: None,
            runtime: Some(SimulationRuntimeSnapshot::RoadDependents(dependents)),
        });
    }
}
