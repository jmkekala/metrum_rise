// SPDX-License-Identifier: GPL-2.0-only

//! In-place cross-section edits of an existing road (`ROAD-80`): its lane counts, median and
//! kerbside parking (`PARK-05`), and its road type's attributes (`ROAD-101`: sidewalks, speed,
//! zoning), changed on the
//! crosswalk toggle's transaction shape (`ROAD-74`). The check is the commit's: the edit is
//! applied to the graph, its two nodes are laid out again, their closure is compiled into a
//! scratch roadbed (fields, painted cells, steepened profiles) and the graph is put back; a
//! widening of the roadbed (lanes, a median, a sidewalk added on either side, `ROAD-107`) is also
//! checked against water, parcels and the curve radius, on the corridor a stroke of that type
//! checks: the larger side, reached to both sides. The preview runs the same check, so the select
//! tool's verdict is the commit's, with the buildings a zoning-off change demolishes and the
//! difference in build cost Apply charges; the commit pushes the check's local checkpoint (whole
//! `Node`/`Edge` records) as its undo record and rebuilds lanes, cars and entrances locally, as
//! an undo does. `O(closure pieces)` for the check, plus a parallel `O(B / P)` count when zoning
//! goes off; the commit tail is `O(local lanes + invalidated)` plus the undo's parallel
//! `O(A / P + B / P)` membership passes over agents and buildings, and when zoning goes off the
//! "No buildings" checkbox's own maintenance pass.

use super::core::{RoadRejection, SimCore};
use crate::debug_log;
use crate::simulation::network::graph::{Edge, RegionGraphUndoDelta, RoadMedian, RoadParking};
use crate::simulation::network::road_types::cost::{BUILD_COST_PER_M, road_weight};
use crate::simulation::network::road_types::{RoadAttributes, RoadCrossSection};
use crate::simulation::network::roadbed::{visual_roadbed_half_width_m, widest_side_band_width_m};
use crate::simulation::network::types::EdgeClass;
use crate::simulation::network::validate::{
    RoadGeometryLimits, RoadGeometryTier, RoadGeometryViolation, centreline, validate_edge,
};
use rayon::prelude::*;
use std::collections::HashSet;

/// What an accepted in-place edit does beyond the road itself (`ROAD-101`), for the Road
/// Properties window's confirmation and its Apply label.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct CrossSectionVerdict {
    /// Buildings fronting the road that a change to zoning off demolishes (as the "No buildings"
    /// checkbox does); 0 otherwise.
    pub(crate) buildings_removed: usize,
    /// Whether that demolition is permanent (any `buildings_removed`): an undo restores the
    /// road and its parcels, empty, but no undo path restores a road's demolished buildings
    /// (the bulldoze journal restores one building per entry), so the confirmation says so.
    pub(crate) demolition_permanent: bool,
    /// What Apply charges: the new road type's build cost over the road's length less the
    /// current one's, floored at 0 (a cheaper type refunds nothing; `DESIGN-RB3` D10).
    pub(crate) cost_delta: f64,
}

impl SimCore {
    /// Changes road `edge_id` to `fwd` forward and `bkw` backward lanes, `median`, `parking`
    /// (`PARK-05`) and the road type's `attributes` in place (`ROAD-80`, `ROAD-101`), keeping
    /// its centreline, id and class.
    /// Both end nodes are laid out again at the new width and the lanes of their arms rebuilt;
    /// cars and walkers on a lane that no longer exists reattach to the nearest rebuilt lane of
    /// their kind, or stay laneless on their edge (`TRAFFIC-19`), never moved. Zoning turned off
    /// removes the road's parcels and demolishes their buildings as the "No buildings" checkbox
    /// does (its undo restores the parcels, empty). The [`CrossSectionVerdict::cost_delta`] is
    /// charged outside `benchmark_mode`.
    /// Returns whether the edit was applied, with an undo record and the render marked dirty; a
    /// refused edit ([`Self::preview_edge_cross_section_internal`]) changes nothing. The caller
    /// publishes the rebuilt pieces ([`Self::finalize_network_render_products`]), as after a
    /// crossing toggle.
    pub fn set_edge_cross_section_internal(
        &mut self,
        edge_id: i32,
        fwd: u8,
        bkw: u8,
        median: Option<RoadMedian>,
        parking: RoadParking,
        attributes: RoadAttributes,
    ) -> bool {
        // A commit's deferred lots first: this edit reads or writes them (`ROAD-68-G` part 7).
        self.adopt_pending_cell_lots();
        let (checkpoint, nodes, verdict) =
            match self.try_cross_section_edit(edge_id, fwd, bkw, median, parking, attributes) {
                Ok(tried) => tried,
                Err(rejection) => {
                    debug_log!(
                        "road",
                        "cross_section_edit_rejected edge={} lanes={}+{} median={:?} parking={:?} attributes={:?} reason={}",
                        edge_id,
                        fwd,
                        bkw,
                        median,
                        parking,
                        attributes,
                        rejection.reason
                    );
                    return false;
                }
            };
        let edge = edge_id as usize;
        let zoning_off = !self.region_graph.edge(edge).no_building_spawn && !attributes.zoning;
        let incident = self.live_edges_at(&nodes, edge);
        let changed = self.apply_edge_cross_section(
            edge,
            (fwd, bkw, median, parking, attributes),
            &nodes,
        );
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
        // Godot's main thread runs this edit, and the passes are parallel: shared pool (CODE-17).
        crate::thread_pools::install(|| self.restore_lanes_locally(&affected, &nodes));
        // The checkbox's side effects (`DESIGN-RB3` B.2): the road's buildings go now and its
        // parcels with them; other roads' buildings keep their rezone grace. The undo record
        // also restores the parcels, emptied (buildings stay demolished, as the checkbox's).
        let mut parcels = None;
        if zoning_off {
            self.run_building_allocator_maintenance_internal();
            parcels = Some(self.zoning.capture_parcel_removal_undo(edge))
                .filter(|parcels| !parcels.is_empty());
            self.zoning.remove_parcels_attached_to_edge(edge);
            self.allocator.dirty = true;
        }
        self.push_network_undo_delta(checkpoint, parcels);
        if !self.benchmark_mode && verdict.cost_delta > 0.0 {
            self.treasury.deduct_build_cost(verdict.cost_delta);
        }
        // Deferred like a commit's (D7, `ROAD-68-G`).
        self.schedule_cch_rebuild_of_live_graph();
        self.mark_local_network_render_dirty();
        true
    }

    /// The commit's verdict on the same edit as [`Self::set_edge_cross_section_internal`], now:
    /// its [`CrossSectionVerdict`] when it would apply it, else why it would refuse it. The
    /// commit's own check, run on the graph and put back, so the Road Properties window's
    /// verdict is the commit's (`ROAD-80`). O(closure pieces), plus `O(B / P)` when zoning goes
    /// off.
    pub(crate) fn preview_edge_cross_section_internal(
        &mut self,
        edge_id: i32,
        fwd: u8,
        bkw: u8,
        median: Option<RoadMedian>,
        parking: RoadParking,
        attributes: RoadAttributes,
    ) -> Result<CrossSectionVerdict, RoadRejection> {
        self.adopt_pending_cell_lots();
        self.try_cross_section_edit(edge_id, fwd, bkw, median, parking, attributes)
            .map(|(_, _, verdict)| verdict)
    }

    // The edit's check with its undo record, the edge's two (surviving) end nodes and its
    // verdict. Cheap refusals first: a live road, a legal cross-section, a change, and no tunnel
    // widened in place (R3-j: its cover was checked across the bore it was drawn with). Then the
    // graph edit, whose closure must compile; a widening must also keep clear of water and
    // parcels and fit its curves, checked on the applied graph.
    fn try_cross_section_edit(
        &mut self,
        edge_id: i32,
        fwd: u8,
        bkw: u8,
        median: Option<RoadMedian>,
        parking: RoadParking,
        attributes: RoadAttributes,
    ) -> Result<(RegionGraphUndoDelta, HashSet<u32>, CrossSectionVerdict), RoadRejection> {
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
        // A pocket edge's lanes are its base road's plus the pocket's (`ROAD-95`): the pocket is
        // removed first, and its base road edited.
        if current.pocket.is_some() {
            return Err(RoadRejection::new(
                "pocket_edit_first",
                format!("edge {edge} carries a turn pocket"),
            ));
        }
        current
            .validate_cross_section(fwd, bkw, median, parking)
            .map_err(|reason| {
                RoadRejection::new(
                    reason,
                    format!(
                        "{fwd}+{bkw} lanes, median {median:?}, parking {parking:?} on edge {edge}"
                    ),
                )
            })?;
        if (
            current.fwd_lanes,
            current.bkw_lanes,
            current.median,
            current.parking,
        ) == (fwd, bkw, median, parking)
            && attributes.same_as(RoadAttributes::of_edge(current))
        {
            return Err(RoadRejection::new(
                "cross_section_unchanged",
                format!(
                    "edge {edge} already has {fwd}+{bkw} lanes, median {median:?}, parking {parking:?}, {attributes:?}"
                ),
            ));
        }
        // The roadbed's larger side, which a tunnel's bore and the corridor checks reach to both
        // sides of it: a sidewalk added on a road's shoulder side leaves that unchanged.
        let widens = Self::roadbed_half_width_m(current, fwd, bkw, median, parking, attributes)
            > visual_roadbed_half_width_m(current);
        if widens && current.class == EdgeClass::Tunnel {
            return Err(RoadRejection::new(
                "edit_tunnel_width",
                "a tunnel is widened by drawing it again, which checks the cover across its bore",
            ));
        }
        let verdict = self.cross_section_verdict(edge, fwd, bkw, median, parking, attributes);
        let nodes: HashSet<u32> = [current.start_node, current.end_node]
            .into_iter()
            .map(|node| graph.get_valid_node(node))
            .collect();
        let edges = self.live_edges_at(&nodes, edge);
        // A side given a sidewalk widens there whatever the larger side does (`ROAD-107`).
        let check_corridor = widens || attributes.sidewalks.adds_to(current.sidewalk_sides());
        let mut corridor = None;
        let tried = self.try_graph_edit(&edges, &nodes, |core| {
            let changed = core.apply_edge_cross_section(
                edge,
                (fwd, bkw, median, parking, attributes),
                &nodes,
            );
            if check_corridor {
                corridor = core.widened_corridor_refusal(edge);
            }
            changed
        });
        match corridor {
            Some(rejection) => Err(rejection),
            None => tried.map(|checkpoint| (checkpoint, nodes, verdict)),
        }
    }

    // The roadbed half-width `current` would have with the new spec, sidewalks or shoulders
    // included: the widening the corridor checks compare (`DESIGN-RB3` A.4). `O(1)`; the probe
    // edge has no geometry, so nothing is allocated.
    fn roadbed_half_width_m(
        current: &Edge,
        fwd: u8,
        bkw: u8,
        median: Option<RoadMedian>,
        parking: RoadParking,
        attributes: RoadAttributes,
    ) -> f32 {
        let mut probe = Edge {
            primary_type: current.primary_type,
            allowed_types: current.allowed_types,
            width: Edge::cross_section_width_m(fwd, bkw, median, parking),
            ..Edge::default()
        };
        probe.set_sidewalks(attributes.sidewalks);
        visual_roadbed_half_width_m(&probe)
    }

    // What the edit does beyond the road (`ROAD-101`): the buildings fronting it that zoning
    // off demolishes (every building whose road it is: none survives the checkbox's
    // maintenance), and the build-cost difference over its length at its class, floored at 0.
    // `O(1)`, plus the road's local building count (`buildings_fronting`) only when zoning goes
    // off.
    fn cross_section_verdict(
        &self,
        edge: usize,
        fwd: u8,
        bkw: u8,
        median: Option<RoadMedian>,
        parking: RoadParking,
        attributes: RoadAttributes,
    ) -> CrossSectionVerdict {
        let current = self.region_graph.edge(edge);
        let buildings_removed = if !current.no_building_spawn && !attributes.zoning {
            self.buildings_fronting(edge)
        } else {
            0
        };
        let section = RoadCrossSection {
            fwd_lanes: fwd,
            bkw_lanes: bkw,
            median,
            parking,
            sidewalks: attributes.sidewalks,
        };
        let weight = road_weight(
            section,
            RoadAttributes::speed_kmh_of(attributes.speed_ms),
            current.class,
        );
        let cost_delta = ((weight - current.cost_weight())
            * BUILD_COST_PER_M
            * f64::from(current.physical_length))
        .max(0.0);
        CrossSectionVerdict {
            buildings_removed,
            demolition_permanent: buildings_removed > 0,
            cost_delta,
        }
    }

    // The buildings whose road is `edge`: those among the buildings of the 512 m chunks within
    // its reach (`attachment_repair_candidates`, which hold every building fronting it, as the
    // split's frontage move relies on), or a parallel count over the city while the chunk index
    // is stale. Test builds check the count against the whole city's.
    // `O(segments + chunks + candidates log candidates)`; stale, `O(B / P)`.
    fn buildings_fronting(&self, edge: usize) -> usize {
        let buildings = &self.allocator.buildings;
        let Some(candidates) = self.allocator.attachment_repair_candidates(
            &self.region_graph,
            &HashSet::from([edge]),
            [],
            self.config.zone_cell_m,
        ) else {
            // In the shared pool: a Godot call runs outside every Rayon pool.
            return crate::thread_pools::install(|| {
                buildings
                    .par_iter()
                    .filter(|building| building.edge_idx == edge)
                    .count()
            });
        };
        let count = candidates
            .into_iter()
            .filter(|&idx| buildings[idx].edge_idx == edge)
            .count();
        debug_assert_eq!(
            count,
            buildings.iter().filter(|b| b.edge_idx == edge).count(),
            "the chunk candidates hold every building fronting edge {edge}"
        );
        count
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

    // Sets the cross-section and attributes and re-lays out both end nodes on the graph, since
    // the width moves every mouth there: their clips and the profile solve that pins their arms
    // up to them. Returns the edges whose profiles changed.
    fn apply_edge_cross_section(
        &mut self,
        edge: usize,
        (fwd, bkw, median, parking, attributes): (
            u8,
            u8,
            Option<RoadMedian>,
            RoadParking,
            RoadAttributes,
        ),
        nodes: &HashSet<u32>,
    ) -> HashSet<usize> {
        self.region_graph
            .set_edge_cross_section(edge, fwd, bkw, median, parking, attributes);
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
        // The roadbed's own side band: a sidewalk, or a road without them its shoulder.
        let parcels = self.zoning.parcel_ids_overlapping_road_corridor(
            points,
            edge.width * 0.5 + widest_side_band_width_m(edge),
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
