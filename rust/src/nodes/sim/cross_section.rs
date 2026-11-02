// SPDX-License-Identifier: GPL-2.0-only

//! In-place cross-section edits of an existing road (`ROAD-80`): its lane counts, changed on the
//! crosswalk toggle's transaction shape (`ROAD-74`). The check is the commit's: the edit is
//! applied to the graph, its two nodes are laid out again, their closure is compiled into a
//! scratch roadbed (fields, painted cells, steepened profiles) and the graph is put back; a
//! widening is also checked against water, parcels and the curve radius. The preview runs the
//! same check, so the select tool's verdict is the commit's; the commit pushes the check's local
//! checkpoint (whole `Node`/`Edge` records) as its undo record and rebuilds lanes, cars and
//! entrances locally, as an undo does. `O(closure pieces)` for the check; the commit tail is
//! `O(local lanes + invalidated)` plus the undo's parallel `O(A / P + B / P)` membership passes
//! over agents and buildings.

use super::core::{RoadRejection, SimCore};
use crate::config;
use crate::debug_log;
use crate::simulation::network::graph::{Edge, RegionGraphUndoDelta};
use crate::simulation::network::roadbed::visual_roadbed_half_width_m;
use crate::simulation::network::types::EdgeClass;
use crate::simulation::network::validate::{
    RoadGeometryLimits, RoadGeometryTier, RoadGeometryViolation, centreline, validate_edge,
};
use std::collections::HashSet;

impl SimCore {
    /// Changes road `edge_id` to `fwd` forward and `bkw` backward lanes in place, keeping its
    /// centreline, id and class (`ROAD-80`). Both end nodes are laid out again at the new width
    /// and the lanes of their arms rebuilt; cars on a lane that no longer exists reattach to the
    /// nearest rebuilt lane. Returns whether the edit was applied, with an undo record and the
    /// render marked dirty; a refused edit
    /// ([`Self::preview_edge_cross_section_internal`]) changes nothing. The caller publishes the
    /// rebuilt pieces ([`Self::finalize_network_render_products`]), as after a crossing toggle.
    pub fn set_edge_cross_section_internal(&mut self, edge_id: i32, fwd: u8, bkw: u8) -> bool {
        // A commit's deferred lots first: this edit reads or writes them (`ROAD-68-G` part 7).
        self.adopt_pending_cell_lots();
        let (checkpoint, nodes) = match self.try_cross_section_edit(edge_id, fwd, bkw) {
            Ok(tried) => tried,
            Err(rejection) => {
                debug_log!(
                    "road",
                    "cross_section_edit_rejected edge={} lanes={}+{} reason={}",
                    edge_id,
                    fwd,
                    bkw,
                    rejection.reason
                );
                return false;
            }
        };
        let edge = edge_id as usize;
        let incident = self.live_edges_at(&nodes, edge);
        self.push_network_undo_delta(checkpoint, None);
        let changed = self.apply_edge_lanes(edge, fwd, bkw, &nodes);
        let affected: HashSet<usize> = incident
            .into_iter()
            .chain(changed.iter().copied())
            .collect();
        // The frontage moves with the width: the edited road's lots and its neighbours' mouths.
        self.zoning
            .mark_cell_lots_for_roads(&self.region_graph, affected.iter().copied());
        self.transit_network
            .mark_surface_dirty_for_nodes(&self.region_graph, &nodes);
        self.transit_network
            .mark_surface_dirty_from_sets(&changed, &HashSet::new());
        // The lanes of both nodes' arms, the cars on them and the entrances fronting them, as
        // an undo restores them: local, where the entrance cache rebuild would scan the city.
        self.restore_lanes_locally(&affected, &nodes);
        // Deferred like a commit's (D7, `ROAD-68-G`).
        self.schedule_cch_rebuild_of_live_graph();
        self.mark_local_network_render_dirty();
        true
    }

    /// Why [`Self::set_edge_cross_section_internal`] would refuse the same edit now, or `None`
    /// when it would apply it: the commit's own check, run on the graph and put back, so the
    /// Road Properties window's verdict is the commit's (`ROAD-80`). O(closure pieces).
    pub(crate) fn preview_edge_cross_section_internal(
        &mut self,
        edge_id: i32,
        fwd: u8,
        bkw: u8,
    ) -> Option<RoadRejection> {
        self.adopt_pending_cell_lots();
        self.try_cross_section_edit(edge_id, fwd, bkw).err()
    }

    // The edit's check with its undo record and the edge's two (surviving) end nodes. Cheap
    // refusals first: a live road, legal lane counts, a change, and no tunnel widened in place
    // (R3-j: its cover was checked across the bore it was drawn with). Then the graph edit,
    // whose closure must compile; a widening must also keep clear of water and parcels and fit
    // its curves, checked on the applied graph.
    fn try_cross_section_edit(
        &mut self,
        edge_id: i32,
        fwd: u8,
        bkw: u8,
    ) -> Result<(RegionGraphUndoDelta, HashSet<u32>), RoadRejection> {
        let graph = &self.region_graph;
        let Some(edge) = usize::try_from(edge_id)
            .ok()
            .filter(|&edge| edge < graph.edge_count() && !graph.edge(edge).deleted)
        else {
            return Err(RoadRejection::new(
                "invalid_road",
                format!("edge {edge_id} is not a live road"),
            ));
        };
        let current = graph.edge(edge);
        current.validate_lanes(fwd, bkw).map_err(|reason| {
            RoadRejection::new(reason, format!("{fwd}+{bkw} lanes on edge {edge}"))
        })?;
        if (current.fwd_lanes, current.bkw_lanes) == (fwd, bkw) {
            return Err(RoadRejection::new(
                "cross_section_unchanged",
                format!("edge {edge} already has {fwd}+{bkw} lanes"),
            ));
        }
        let widens = Edge::lanes_width_m(fwd, bkw) > current.width;
        if widens && current.class == EdgeClass::Tunnel {
            return Err(RoadRejection::new(
                "edit_tunnel_width",
                "a tunnel is widened by drawing it again, which checks the cover across its bore",
            ));
        }
        let nodes: HashSet<u32> = [current.start_node, current.end_node]
            .into_iter()
            .map(|node| graph.get_valid_node(node))
            .collect();
        let edges = self.live_edges_at(&nodes, edge);
        let mut corridor = None;
        let tried = self.try_graph_edit(&edges, &nodes, |core| {
            let changed = core.apply_edge_lanes(edge, fwd, bkw, &nodes);
            if widens {
                corridor = core.widened_corridor_refusal(edge);
            }
            changed
        });
        match corridor {
            Some(rejection) => Err(rejection),
            None => tried.map(|checkpoint| (checkpoint, nodes)),
        }
    }

    // The live edges at `nodes`, and `edge`. O(local degree).
    fn live_edges_at(&self, nodes: &HashSet<u32>, edge: usize) -> HashSet<usize> {
        let graph = &self.region_graph;
        nodes
            .iter()
            .flat_map(|&node| graph.node_adjacency(node).iter().copied())
            .filter(|&incident| !graph.edge(incident).deleted)
            .chain([edge])
            .collect()
    }

    // Sets the lanes and re-lays out both end nodes on the graph, since the width moves every
    // mouth there: their clips and the profile solve that pins their arms up to them. Returns
    // the edges whose profiles changed.
    fn apply_edge_lanes(
        &mut self,
        edge: usize,
        fwd: u8,
        bkw: u8,
        nodes: &HashSet<u32>,
    ) -> HashSet<usize> {
        self.region_graph.set_edge_lanes(edge, fwd, bkw);
        self.region_graph
            .rebuild_intersection_clips_for_nodes(nodes);
        let mut changed = HashSet::new();
        self.region_graph.finalize_junction_endpoint_profiles(
            nodes,
            Some(&self.heightmap),
            &mut changed,
        );
        changed
    }

    // Why widened `edge` (applied on the graph) cannot stay: a standard road over water (as the
    // class change refuses one), a parcel inside its new corridor (the stroke's check and
    // half-width: the frontage), or a curve tighter than the illegal radius at its new width.
    // O(points + parcel hits).
    fn widened_corridor_refusal(&self, edge_idx: usize) -> Option<RoadRejection> {
        let edge = self.region_graph.edge(edge_idx);
        let points = centreline(edge);
        if edge.class == EdgeClass::Standard
            && self
                .watermap
                .road_corridor_overlaps_visible_water(points, visual_roadbed_half_width_m(edge))
        {
            return Some(RoadRejection::new(
                "water_requires_bridge",
                format!("widened edge {edge_idx} would reach visible water"),
            ));
        }
        let parcels = self.zoning.parcel_ids_overlapping_road_corridor(
            points,
            edge.width * 0.5 + config::SIDEWALK_WIDTH,
        );
        if let Some(first) = parcels.first() {
            return Some(RoadRejection::new(
                "parcel_overlap",
                format!(
                    "widened edge {edge_idx} would overlap {} parcel(s), first {first}",
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
                    "edge {edge_idx} turns at a {measured:.1} m radius; this width needs {limit:.1} m"
                ),
            )),
            _ => None,
        })
    }
}
