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
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::pockets::{
    PocketApplied, PocketDependentsUndo, PocketEndView, PocketPlan, PocketRequest, PocketState,
    migrate_pocket_dependents,
};
use crate::simulation::network::road_types::cost::BUILD_COST_PER_M;
use crate::simulation::network::road_types::{RoadAttributes, RoadCrossSection};
use crate::simulation::network::roadbed::{EdgeEnd, side_band_width_m};
use crate::simulation::network::types::{EdgeClass, TransitFlags, TransitType};
use crate::simulation::network::validate::{
    RoadGeometryLimits, RoadGeometryTier, RoadGeometryViolation, centreline, validate_edge,
};
use godot::prelude::Vector3;
use std::collections::HashSet;

/// What an accepted pocket edit charges, for the Road Properties window's Apply label.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PocketVerdict {
    /// The build cost of the pair after the edit less before it, floored at 0 (D14: the
    /// in-place rule, never refunded).
    pub(crate) cost_delta: f64,
}

/// How far out along an arm the Junction window reads the direction it comes from (round R5c2).
const ARM_BEARING_RUN_M: f32 = 20.0;

/// One road arm of a junction as the Junction window's Turn pockets section lists it (`ROAD-95`,
/// round R5c2): its end at the junction, the direction it comes from and the road it belongs to.
pub(crate) struct JunctionPocketArm {
    /// The arm's edge at the junction (a pocket edge when the arm has a pocket).
    pub(crate) edge: usize,
    /// Its end at the junction.
    pub(crate) end: EdgeEnd,
    /// The bearing from the junction to the arm's point [`ARM_BEARING_RUN_M`] out along it (or
    /// its far end), degrees clockwise from north (-Z) in `[0, 360)`.
    pub(crate) bearing_deg: f32,
    /// The pocket query's view of this end ([`PocketEndView::of`]).
    pub(crate) view: PocketEndView,
    /// The road's cross-section without its pocket, its speed (km/h) and street lights: what
    /// the window names the road by.
    pub(crate) base: (RoadCrossSection, u16, bool),
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

    /// The Junction window's Turn pockets rows at `node_id` (`ROAD-95`, round R5c2): one
    /// [`JunctionPocketArm`] per car road arm with a lane into the junction (an arm without one
    /// has no approach to take a pocket), clockwise from north by
    /// [`JunctionPocketArm::bearing_deg`] (ties: edge id, then the start end first). A pocket
    /// edge arm has its pocket end here. Empty for a lane transition node (a pocket's own,
    /// edited from its junction) or an id that is not a node. `O(degree + arm points)`,
    /// allocating: a UI query.
    pub(crate) fn junction_pocket_arms_internal(&self, node_id: u32) -> Vec<JunctionPocketArm> {
        let graph = &self.region_graph;
        if node_id as usize >= graph.node_count() {
            return Vec::new();
        }
        let node = graph.get_valid_node(node_id);
        if graph.is_lane_transition(node) {
            return Vec::new();
        }
        let mut arms: Vec<JunctionPocketArm> = graph
            .node_adjacency(node)
            .iter()
            .flat_map(|&edge| [(edge, EdgeEnd::Start), (edge, EdgeEnd::End)])
            .filter(|&(edge, end)| is_car_approach(graph, edge, end, node))
            .filter_map(|(edge, end)| {
                let view = PocketEndView::of(graph, edge, end)?;
                debug_assert_eq!(view.junction, node, "an arm's pocket end is its junction");
                Some(JunctionPocketArm {
                    edge,
                    end,
                    bearing_deg: bearing_out(&view.road),
                    base: base_section(graph.edge(edge)),
                    view,
                })
            })
            .collect();
        arms.sort_by(|a, b| {
            a.bearing_deg
                .total_cmp(&b.bearing_deg)
                .then(a.edge.cmp(&b.edge))
                .then((a.end == EdgeEnd::End).cmp(&(b.end == EdgeEnd::End)))
        });
        arms
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

// Whether end `end` of `edge` is a live car road arm at `node` with a lane into it. `O(1)`.
fn is_car_approach(graph: &RegionGraph, edge: usize, end: EdgeEnd, node: u32) -> bool {
    let record = graph.edge(edge);
    let (at, lanes_in) = match end {
        EdgeEnd::Start => (record.start_node, record.bkw_lanes),
        EdgeEnd::End => (record.end_node, record.fwd_lanes),
    };
    !record.deleted
        && record.primary_type == TransitType::Road
        && record.allowed_types & TransitFlags::CAR != 0
        && lanes_in > 0
        && graph.get_valid_node(at) == node
}

// The bearing of `road` (a polyline from the junction outwards) at `ARM_BEARING_RUN_M` along it
// in XZ (or at its far end), degrees clockwise from north (-Z); 0 for a degenerate road.
// `O(points)`.
fn bearing_out(road: &[Vector3]) -> f32 {
    let Some(&origin) = road.first() else {
        return 0.0;
    };
    let mut out = road.last().copied().unwrap_or(origin);
    let mut run = 0.0;
    for pair in road.windows(2) {
        let step = (pair[1].x - pair[0].x).hypot(pair[1].z - pair[0].z);
        if run + step >= ARM_BEARING_RUN_M {
            out = pair[0].lerp(pair[1], (ARM_BEARING_RUN_M - run) / step);
            break;
        }
        run += step;
    }
    let (east, north) = (out.x - origin.x, origin.z - out.z);
    if east.hypot(north) < 1e-4 {
        return 0.0;
    }
    east.atan2(north).to_degrees().rem_euclid(360.0)
}

// The cross-section, speed (km/h) and street lights `edge` names its road by: a pocket edge's
// without its pocket lanes and with the median they narrowed. `O(1)`.
fn base_section(edge: &Edge) -> (RoadCrossSection, u16, bool) {
    let section = RoadCrossSection {
        fwd_lanes: edge.fwd_lanes - edge.pocket_lanes(true),
        bkw_lanes: edge.bkw_lanes - edge.pocket_lanes(false),
        median: edge.base_median(),
        ..RoadCrossSection::of_edge(edge)
    };
    (
        section,
        RoadAttributes::speed_kmh_of(edge.speed_limit),
        !edge.no_street_lights,
    )
}
