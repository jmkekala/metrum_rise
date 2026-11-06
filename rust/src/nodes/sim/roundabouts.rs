// SPDX-License-Identifier: GPL-2.0-only

//! The roundabout edit (`ROAD-62`) on the turn pocket's transaction shape (`ROAD-95`): the plan
//! ([`RoundaboutLayout`]) is checked against water and parcels, applied to the graph, the ring
//! nodes and the arms' far nodes are laid out, their closure is compiled into a scratch roadbed
//! (fields, painted cells, steepened profiles, node pieces) and the graph is put back. The preview
//! is that check, so the verdict is the commit's (D11). The commit applies the plan again, moves
//! the trimmed arms' buildings and occupancy (journaled), rebuilds lanes, cars and entrances
//! locally, pushes the check's checkpoint with the dependents journal as its undo record and
//! charges the ring's build cost. The check is `O(arm points + ring points + closure pieces)`; the
//! commit adds `O(the arms' buildings + occupancy + local lanes)` and the undo's parallel
//! `O(A / P + B / P)` membership passes. Nothing here runs per tick.

use super::core::{RoadRejection, SimCore};
use crate::config;
use crate::debug_log;
use crate::simulation::network::graph::RegionGraphUndoDelta;
use crate::simulation::network::pockets::{PocketDependentsUndo, migrate_arm_trims};
use crate::simulation::network::road_types::cost::BUILD_COST_PER_M;
use crate::simulation::network::roadbed::{
    LayoutError, NodeError, check_node, visual_roadbed_half_width_m,
};
use crate::simulation::network::roundabouts::{
    ArmTrim, RoundaboutApplied, RoundaboutLayout, RoundaboutRequest, RoundaboutSite,
    default_diameter_m, ring_walk,
};
use godot::prelude::Vector3;
use std::collections::HashSet;

/// How near a node the tool's point must be to replace it, the select tool's reach (D13).
const SITE_NODE_REACH_M: f32 = 8.0;

/// What the roundabout tool reads under its point (D12): the site the point means, the ring's
/// lanes there and the diameters it takes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RoundaboutSiteView {
    /// The site at the point: the node within reach, else empty ground.
    pub(crate) site: RoundaboutSite,
    /// The ring's lanes at this site.
    pub(crate) lanes: u8,
    /// The smallest diameter here.
    pub(crate) min_diameter_m: f32,
    /// The largest diameter here (D14 caps a replacement).
    pub(crate) max_diameter_m: f32,
    /// The diameter a ring of these lanes starts at.
    pub(crate) default_diameter_m: f32,
}

/// What an accepted roundabout edit builds, for the tool's overlay and label.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RoundaboutVerdict {
    /// What the commit charges: the ring's build cost. The arms give back nothing.
    pub(crate) cost: f64,
    /// The ring's centreline as one closed polyline.
    pub(crate) ring: Vec<Vector3>,
    /// Each arm's id with the point it is cut at.
    pub(crate) arms: Vec<(usize, Vector3)>,
}

// One tried edit: its undo checkpoint, the plan, the closure and the verdict.
struct TriedRoundabout {
    checkpoint: RegionGraphUndoDelta,
    layout: RoundaboutLayout,
    edges: HashSet<usize>,
    nodes: HashSet<u32>,
    verdict: RoundaboutVerdict,
}

impl SimCore {
    /// What the roundabout tool reads at `point` for a road type of `type_lanes` lanes per
    /// direction: a junction within reach decides the ring's lanes by its arms, empty ground by
    /// the type, capped at two. `O(log N + degree)`.
    pub(crate) fn roundabout_site_at(&self, point: Vector3, type_lanes: u8) -> RoundaboutSiteView {
        let graph = &self.region_graph;
        let node = self.get_closest_node_internal(point, SITE_NODE_REACH_M);
        // A node a bulldozed road left without arms is no site, and no obstacle.
        let node = u32::try_from(node)
            .ok()
            .filter(|&node| graph.live_node_connection_count(node) > 0);
        let site = match node {
            Some(node) => RoundaboutSite::Junction(node),
            None => RoundaboutSite::Ground {
                centre: point,
                lanes: type_lanes.clamp(1, 2),
            },
        };
        let lanes = site.lanes(graph);
        let (min_diameter_m, max_diameter_m) = site.diameter_range_m(graph);
        RoundaboutSiteView {
            site,
            lanes,
            min_diameter_m,
            max_diameter_m,
            default_diameter_m: default_diameter_m(lanes).clamp(min_diameter_m, max_diameter_m),
        }
    }

    /// Builds the roundabout `request` asks for, with an undo record (`ROAD-62`). Returns whether
    /// it was applied; a refused edit ([`Self::preview_roundabout_internal`]) changes nothing and
    /// leaves its reason in `last_road_rejection`. The caller publishes the rebuilt pieces
    /// ([`Self::finalize_network_render_products`]), as after a pocket edit. The ring's build
    /// cost is charged outside `benchmark_mode`.
    pub(crate) fn place_roundabout_internal(&mut self, request: RoundaboutRequest) -> bool {
        // A commit's deferred lots first: this edit reads or writes them (`ROAD-68-G` part 7).
        self.adopt_pending_cell_lots();
        let tried = match self.try_roundabout_edit(request) {
            Ok(tried) => tried,
            Err(rejection) => {
                debug_log!(
                    "road",
                    "roundabout_rejected request={:?} reason={} detail={}",
                    request,
                    rejection.reason,
                    rejection.detail
                );
                self.last_road_rejection = Some(rejection);
                return false;
            }
        };
        let TriedRoundabout {
            checkpoint,
            layout,
            edges,
            nodes,
            verdict,
        } = tried;
        let trims = layout.arm_trims(&self.region_graph);
        let (applied, changed) = self.apply_roundabout_layout(&layout, &nodes);
        let affected: HashSet<usize> = edges
            .into_iter()
            .chain(changed)
            .chain(applied.ring.iter().copied())
            .chain(applied.arms.iter().copied())
            .collect();
        let dependents = self.migrate_trimmed_arms(&trims);
        // The frontage moves with the cut: the arms' lots and their neighbours' mouths.
        self.zoning
            .mark_cell_lots_for_roads(&self.region_graph, affected.iter().copied());
        self.transit_network
            .mark_surface_dirty_for_nodes(&self.region_graph, &nodes);
        self.transit_network
            .mark_surface_dirty_from_sets(&affected, &HashSet::new());
        // The ring's lanes, the arms' and the cars and walkers on them. The retired junction has
        // no arm left to rebuild its connectors from. Godot's main thread runs this edit, and the
        // passes are parallel: shared pool (CODE-17).
        crate::thread_pools::install(|| {
            self.restore_lanes_locally(&affected, &nodes);
            if let Some(old) = layout.old_node() {
                let lanes = &mut self.transit_network.lane_system;
                lanes.retire_node_lanes([old as usize]);
            }
        });
        self.push_pocket_undo(checkpoint, dependents);
        if !self.benchmark_mode && verdict.cost > 0.0 {
            self.treasury.deduct_build_cost(verdict.cost);
        }
        // Deferred like a commit's (D7, `ROAD-68-G`).
        self.schedule_cch_rebuild_of_live_graph();
        self.transit_network.flow_fields.mark_all_dirty();
        self.mark_local_network_render_dirty();
        self.terrain_dirty = true;
        true
    }

    /// The commit's verdict on `request` now ([`Self::place_roundabout_internal`]): the
    /// [`RoundaboutVerdict`] when it would build it, else why it would refuse it. The commit's own
    /// check, run on the graph and put back (D11). `O(arm points + ring points + closure pieces)`.
    pub(crate) fn preview_roundabout_internal(
        &mut self,
        request: RoundaboutRequest,
    ) -> Result<RoundaboutVerdict, RoadRejection> {
        self.adopt_pending_cell_lots();
        self.try_roundabout_edit(request).map(|tried| tried.verdict)
    }

    // The edit's check: the plan's refusals, the land under the ring, then the graph edit, whose
    // closure must compile (arms closer than their mouths are `roundabout_arms_too_close`, any
    // other node that fails has too short an approach), keep clear of reserved land and steepen
    // no profile.
    fn try_roundabout_edit(
        &mut self,
        request: RoundaboutRequest,
    ) -> Result<TriedRoundabout, RoadRejection> {
        let layout = RoundaboutLayout::prepare(&self.region_graph, request)
            .map_err(|reason| RoadRejection::new(reason, format!("{request:?}")))?;
        if let Some(rejection) = self.roundabout_land_refusal(&layout) {
            return Err(rejection);
        }
        let (edges, nodes) = self.roundabout_closure(&layout);
        let mut crowded = None;
        let mut cost = 0.0;
        let tried = self.try_graph_edit(&edges, &nodes, |core| {
            let (applied, changed) = core.apply_roundabout_layout(&layout, &nodes);
            cost = core.build_cost_of(&applied.ring) * BUILD_COST_PER_M;
            crowded = ring_corner_refusal(core, &applied);
            changed
                .into_iter()
                .chain(applied.ring)
                .chain(applied.arms)
                .collect()
        });
        let checkpoint = tried.map_err(|rejection| {
            if rejection.reason == "node_piece_failed" {
                let reason = crowded.unwrap_or("roundabout_arm_too_short");
                RoadRejection::new(reason, rejection.detail)
            } else {
                rejection
            }
        })?;
        let verdict = RoundaboutVerdict {
            cost,
            ring: layout.outline(),
            arms: layout.arm_cuts().collect(),
        };
        Ok(TriedRoundabout {
            checkpoint,
            layout,
            edges,
            nodes,
            verdict,
        })
    }

    // Why the ring cannot stand on its land: water under its corridor, or a parcel on the disc
    // it takes, the island included (the road it would front is gone: a lot there would keep no
    // street). Painted cells and fields are the closure's. `O(ring points + parcel hits)`.
    fn roundabout_land_refusal(&self, layout: &RoundaboutLayout) -> Option<RoadRejection> {
        for edge in layout.ring_edges() {
            if self.watermap.road_corridor_overlaps_visible_water(
                &edge.geometry,
                visual_roadbed_half_width_m(edge),
            ) {
                return Some(RoadRejection::new(
                    "water_requires_bridge",
                    "the ring would reach visible water",
                ));
            }
        }
        let circle = layout.circle();
        let half = 0.5 * f32::from(layout.lanes()) * config::LANE_WIDTH;
        let disc_m = circle.radius_m + half + config::SIDEWALK_WIDTH;
        let parcels = self
            .zoning
            .parcel_ids_overlapping_disc(circle.centre, disc_m);
        parcels.first().map(|first| {
            RoadRejection::new(
                "parcel_overlap",
                format!(
                    "the ring would overlap {} parcel(s), first {first}",
                    parcels.len()
                ),
            )
        })
    }

    // The closure of the edit: the old junction, the arms' far nodes and every arm of them, and
    // the ring nodes the plan appends, the next ids (ids past the graph are skipped until they
    // exist). `O(arms x degree)`.
    fn roundabout_closure(&self, layout: &RoundaboutLayout) -> (HashSet<usize>, HashSet<u32>) {
        let graph = &self.region_graph;
        let first_new = graph.node_count() as u32;
        let mut nodes: HashSet<u32> = (first_new..first_new + layout.ring_edges().len() as u32)
            .chain(layout.old_node())
            .collect();
        for (arm, _) in layout.arm_cuts() {
            let edge = graph.edge(arm);
            nodes.extend([edge.start_node, edge.end_node].map(|node| graph.get_valid_node(node)));
        }
        let edges = nodes
            .iter()
            .filter(|&&node| (node as usize) < graph.node_count())
            .flat_map(|&node| graph.node_adjacency(node).iter().copied())
            .filter(|&edge| !graph.edge(edge).deleted)
            .collect();
        (edges, nodes)
    }

    // Applies the plan on the graph and lays out `nodes`: their clips and the profile solve that
    // pins their arms up to them. Returns what the plan installed and the edges whose profiles
    // the solve changed.
    fn apply_roundabout_layout(
        &mut self,
        layout: &RoundaboutLayout,
        nodes: &HashSet<u32>,
    ) -> (RoundaboutApplied, HashSet<usize>) {
        let applied = layout.apply(&mut self.region_graph);
        self.region_graph
            .rebuild_intersection_clips_for_nodes(nodes);
        let mut changed = HashSet::new();
        self.region_graph.finalize_junction_endpoint_profiles(
            nodes,
            Some(&self.heightmap),
            &mut changed,
        );
        (applied, changed)
    }

    /// Removes the whole ring through `edge` with one undo record (D15): every ring edge goes and
    /// the arms stay, each ending at its ring node on the ring's old line as a road end. A
    /// refused removal changes nothing and leaves its reason in `last_road_rejection`.
    /// `O(ring + the arms' closure)`, plus the undo's membership passes.
    pub(super) fn bulldoze_ring(&mut self, edge: usize) -> bool {
        let Some(walk) = ring_walk(&self.region_graph, edge) else {
            return false;
        };
        let ring_nodes: HashSet<u32> = walk.iter().map(|&(node, _)| node).collect();
        let ring: Vec<usize> = walk.iter().map(|&(_, edge)| edge).collect();
        let graph = &self.region_graph;
        let edges: HashSet<usize> = ring_nodes
            .iter()
            .flat_map(|&node| graph.node_adjacency(node).iter().copied())
            .filter(|&edge| !graph.edge(edge).deleted)
            .collect();
        let nodes: HashSet<u32> = edges
            .iter()
            .flat_map(|&id| {
                let edge = graph.edge(id);
                [edge.start_node, edge.end_node].map(|node| graph.get_valid_node(node))
            })
            .collect();
        let removal = |core: &mut Self, changed: &mut HashSet<usize>| {
            for &id in &ring {
                core.region_graph.delete_edge_locally(id);
            }
            core.region_graph
                .rebuild_intersection_clips_for_nodes(&nodes);
            core.region_graph.finalize_junction_endpoint_profiles(
                &ring_nodes,
                Some(&core.heightmap),
                changed,
            );
        };
        let tried = self.try_graph_edit(&edges, &nodes, |core| {
            let mut changed = HashSet::new();
            removal(core, &mut changed);
            changed
        });
        let checkpoint = match tried {
            Ok(checkpoint) => checkpoint,
            Err(rejection) => {
                debug_log!(
                    "road",
                    "ring_removal_rejected edge={edge} reason={} detail={}",
                    rejection.reason,
                    rejection.detail
                );
                self.last_road_rejection = Some(rejection);
                return false;
            }
        };
        self.push_network_undo_delta(checkpoint, None);
        let mut changed = HashSet::new();
        removal(self, &mut changed);
        debug_assert!(changed.is_subset(&edges));
        self.zoning
            .mark_cell_lots_for_roads(&self.region_graph, edges.iter().copied());
        self.transit_network
            .mark_surface_dirty_from_sets(&edges, &nodes);
        let bare = ring_nodes
            .iter()
            .filter(|&&node| self.region_graph.live_node_connection_count(node) == 0)
            .map(|&node| node as usize);
        let bare: Vec<usize> = bare.collect();
        crate::thread_pools::install(|| {
            self.restore_lanes_locally(&edges, &nodes);
            let lanes = &mut self.transit_network.lane_system;
            lanes.retire_node_lanes(bare);
        });
        // Deferred like a commit's (D7, `ROAD-68-G`); the road tool's view shares its graph
        // copy (`ROAD-68-E`).
        let routing_graph = self.schedule_cch_rebuild_of_live_graph();
        self.road_tool_publisher.offer_graph(routing_graph);
        self.transit_network.flow_fields.mark_all_dirty();
        self.mark_local_network_render_dirty();
        self.terrain_dirty = true;
        true
    }

    /// The bulldoze hover's name for the ring through `edge`, with its centreline as one closed
    /// polyline (D15): "Roundabout, 4 arms". `O(ring)`.
    pub(super) fn ring_hover(&self, edge: usize) -> Option<(String, Vec<Vector3>)> {
        let graph = &self.region_graph;
        let walk = ring_walk(graph, edge)?;
        let arms = walk
            .iter()
            .filter(|&&(node, _)| graph.live_node_connection_count(node) > 2)
            .count();
        let mut points: Vec<Vector3> = walk
            .iter()
            .flat_map(|&(_, id)| {
                let points = &graph.edge(id).physical_geometry;
                points[..points.len() - 1].iter().copied()
            })
            .collect();
        points.extend(points.first().copied());
        let plural = if arms == 1 { "arm" } else { "arms" };
        Some((format!("Roundabout, {arms} {plural}"), points))
    }

    // Moves the buildings and occupancy of the trimmed arms by what the cuts took off them,
    // journaled. `O(the arms' buildings + occupancy cells)`.
    fn migrate_trimmed_arms(&mut self, trims: &[ArmTrim]) -> PocketDependentsUndo {
        migrate_arm_trims(&self.region_graph, trims, &self.zoning, &mut self.allocator)
    }
}

// The refusal of arms closer than their mouths at a ring node of the applied plan. `O(ring nodes
// x node plan)`.
fn ring_corner_refusal(core: &SimCore, applied: &RoundaboutApplied) -> Option<&'static str> {
    let corner = |&node: &u32| {
        matches!(
            check_node(&core.region_graph, node),
            Err(NodeError::Layout(LayoutError::CornerDoesNotFit { .. }))
        )
    };
    applied
        .nodes
        .iter()
        .any(corner)
        .then_some("roundabout_arms_too_close")
}
