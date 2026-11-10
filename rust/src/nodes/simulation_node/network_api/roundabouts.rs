// SPDX-License-Identifier: GPL-2.0-only

//! The roundabout tool's Godot API (`ROAD-62`, D11-D14): what stands under its point, the commit's
//! verdict with the outline the tool draws, and the commit. The hover's two queries read the road
//! tool's context (`ZONE-10`, round RZ10b): they never wait for the core.

use super::*;
use crate::nodes::sim::roundabouts::{RoundaboutSiteView, roundabout_site_at};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roundabouts::{RoundaboutRequest, RoundaboutSite, circle_outline};

// A road type's lanes per direction, what a ring on empty ground is sized by.
fn type_lanes(lanes: i32) -> u8 {
    lanes.clamp(1, i32::from(u8::MAX)) as u8
}

// The site's centre and whether it replaces a junction, -1 on empty ground.
fn site_centre(graph: &RegionGraph, site: RoundaboutSite) -> (Vector3, i64) {
    match site {
        RoundaboutSite::Ground { centre, .. } => (centre, -1),
        RoundaboutSite::Junction(node) => (graph.node(node).pos, i64::from(node)),
    }
}

#[godot_api(secondary)]
impl SimulationNode {
    /// What the roundabout tool reads at `point` for a road type of `lanes` per direction:
    /// `lanes` (the ring's, shown read-only), `min_diameter_m`, `max_diameter_m`,
    /// `default_diameter_m`, `node` (the junction it would replace, -1 on empty ground) and
    /// `center`. Read on the road tool's context (its replica of the published graph): no core
    /// lock (`CODE-25`).
    #[func]
    pub fn get_roundabout_site(&self, point: Vector3, lanes: i32) -> VarDictionary {
        let context = self
            .road_tool
            .context
            .lock()
            .expect("road tool context lock poisoned");
        let view: RoundaboutSiteView =
            roundabout_site_at(&context.replica, point, type_lanes(lanes));
        let (center, node) = site_centre(&context.replica, view.site);
        vdict! {
            "lanes": i64::from(view.lanes),
            "min_diameter_m": view.min_diameter_m,
            "max_diameter_m": view.max_diameter_m,
            "default_diameter_m": view.default_diameter_m,
            "node": node,
            "center": center,
        }
    }

    /// The commit's verdict on a ring of `diameter_m` at `point` ([`Self::place_roundabout`]):
    /// `is_valid`, `ring` (its centreline as one closed polyline), and when valid `arms` (each
    /// `edge` with its `cut` point), `cost`, `demolish_count` and `demolish_lots` (the zoned land
    /// the ring and its island take, `ZONE-10`: its buildings, and four corners per lot as the
    /// road preview's), else `invalid_reason` (stable key) and `detail`. The check is the
    /// commit's ([`RoadEditSite`](crate::nodes::sim::core::RoadEditSite)), run on the road
    /// tool's context, the published graph and land, and put back: a hover, no core lock
    /// (`CODE-25`); the click asks the core again. A refused ring's outline is its circle.
    /// `O(arm points + ring points + closure pieces)`.
    #[func]
    pub fn preview_roundabout(&self, point: Vector3, diameter_m: f32, lanes: i32) -> VarDictionary {
        let land = self.road_tool.land();
        let mut context = self
            .road_tool
            .context
            .lock()
            .expect("road tool context lock poisoned");
        context.land = land;
        let view = roundabout_site_at(&context.replica, point, type_lanes(lanes));
        let request = RoundaboutRequest {
            site: view.site,
            diameter_m,
        };
        match context.road_edit_site().try_roundabout_edit(request) {
            Ok(tried) => {
                let verdict = tried.verdict();
                let arms: VarArray = verdict
                    .arms
                    .iter()
                    .map(|&(edge, cut)| vdict! { "edge": edge as i64, "cut": cut }.to_variant())
                    .collect();
                vdict! {
                    "is_valid": true,
                    "ring": PackedVector3Array::from_iter(verdict.ring.iter().copied()),
                    "arms": arms,
                    "cost": verdict.cost,
                    "demolish_count": verdict.take.buildings.len() as i64,
                    "demolish_lots": road_tool::view_lot_outlines(&context, &verdict.take),
                }
            }
            Err(rejection) => {
                let (centre, _) = site_centre(&context.replica, view.site);
                let ring = circle_outline(centre, diameter_m, view.lanes);
                vdict! {
                    "is_valid": false,
                    "invalid_reason": rejection.reason,
                    "detail": rejection.detail,
                    "ring": PackedVector3Array::from_iter(ring),
                }
            }
        }
    }

    /// Builds the ring [`Self::preview_roundabout`] checks, with one undo record; its pieces
    /// publish now, as a pocket's do, and its build cost is charged. Returns whether it was
    /// applied: a refused edit changes nothing.
    #[func]
    pub fn place_roundabout(&mut self, point: Vector3, diameter_m: f32, lanes: i32) -> bool {
        let (snapshot, publication) = {
            let mut core = self.lock_core();
            let view = core.roundabout_site_at(point, type_lanes(lanes));
            let request = RoundaboutRequest {
                site: view.site,
                diameter_m,
            };
            if !core.place_roundabout_internal(request) {
                return false;
            }
            let publication = core.finalize_network_render_products();
            (core.build_snapshot(), publication)
        };
        *self.snapshot.write().unwrap() = snapshot;
        if let Some(publication) = publication {
            self.road_tool.publish(publication);
        }
        true
    }
}
