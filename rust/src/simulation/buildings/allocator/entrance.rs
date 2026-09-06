// SPDX-License-Identifier: GPL-2.0-only

//! Derived entrance/access cache built from building placement, asset anchors, and live lanes.

use crate::assets::AnchorType;
use crate::config::AGENT_DRIVEWAY_SPEED_MS;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator, BuildingEntrance};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::{LaneSystem, LaneType};
use crate::simulation::network::types::{TransitFlags, TransitType, VehicleFrontageAccess};
use godot::prelude::Vector2;

const INVALID_LANE_ID: usize = usize::MAX;

const ENTRANCE_FOOT_VALID: u8 = 0x01;
const ENTRANCE_CAR_VALID: u8 = 0x02;
const ENTRANCE_FOOT_FWD_VALID: u8 = 0x04;
const ENTRANCE_FOOT_BKW_VALID: u8 = 0x08;
const ENTRANCE_CAR_FWD_VALID: u8 = 0x10;
const ENTRANCE_CAR_BKW_VALID: u8 = 0x20;

#[derive(Clone, Copy)]
struct FreightCarCandidate {
    total_time_s: f32,
    origin_rank: u8,
    destination_rank: u8,
    attach_lane_id: usize,
    detach_lane_id: usize,
    attach_lane_d: f32,
    detach_lane_d: f32,
}

#[derive(Clone, Copy)]
struct FreightBorderCandidate {
    total_time_s: f32,
    destination_rank: u8,
    detach_lane_id: usize,
    detach_lane_d: f32,
}

impl BuildingAllocator {
    pub(crate) fn rebuild_entrance_cache(&mut self, graph: &RegionGraph, lanes: &LaneSystem) {
        let traffic_debug = crate::debug::is_traffic_enabled();
        let old_entrances = traffic_debug.then(|| self.entrances.clone());
        self.entrances.clear();
        self.entrances.reserve(self.buildings.len());
        for building in &self.buildings {
            self.entrances.push(Self::derive_building_entrance(
                building,
                graph,
                lanes,
                &self.registry,
            ));
        }
        self.entrances_dirty = false;
        self.bump_entrance_ref_revision();
        if let Some(old_entrances) = old_entrances.as_deref() {
            log_entrance_cache_rebuild(old_entrances, &self.entrances, &self.buildings);
        }
    }

    /// Rebuilds saved entrances using the live catalogue before its ownership is transferred.
    pub(crate) fn rebuild_loaded_entrance_cache(
        &mut self,
        graph: &RegionGraph,
        lanes: &LaneSystem,
        registry: &crate::assets::AssetRegistry,
    ) {
        self.entrances.clear();
        self.entrances.reserve(self.buildings.len());
        for building in &self.buildings {
            self.entrances.push(Self::derive_building_entrance(
                building, graph, lanes, registry,
            ));
        }
        self.entrances_dirty = false;
        self.bump_entrance_ref_revision();
    }

    /// Appends the derived entrance for a newly appended building to a clean cache.
    pub(crate) fn append_entrance_cache_for_building(
        &mut self,
        building_idx: usize,
        graph: &RegionGraph,
        lanes: &LaneSystem,
    ) -> bool {
        if building_idx >= self.buildings.len()
            || building_idx + 1 != self.buildings.len()
            || self.entrances.len() != building_idx
        {
            return false;
        }
        let entrance = Self::derive_building_entrance(
            &self.buildings[building_idx],
            graph,
            lanes,
            &self.registry,
        );
        self.entrances.push(entrance);
        self.entrances_dirty = self.entrances.len() != self.buildings.len();
        self.bump_entrance_ref_revision();
        true
    }

    fn derive_building_entrance(
        building: &Building,
        graph: &RegionGraph,
        lanes: &LaneSystem,
        registry: &crate::assets::AssetRegistry,
    ) -> BuildingEntrance {
        let mut entrance = BuildingEntrance {
            edge_idx: building.edge_idx,
            side: building.side,
            ..BuildingEntrance::default()
        };

        let Some(asset_entry) = registry.get(&building.asset_id) else {
            return entrance;
        };
        let Some(anchor) = main_entrance_anchor(asset_entry.manifest.anchors.as_slice()) else {
            return entrance;
        };

        entrance.door_pos = world_door_pos(
            building,
            anchor.position,
            asset_entry.manifest.building_frontage_forward(),
        );
        entrance.curb_pos = entrance.door_pos;

        if building.edge_idx >= graph.edge_count() {
            return entrance;
        }

        let edge = graph.edge(building.edge_idx);
        entrance.vehicle_frontage_access = edge.vehicle_frontage_access;
        if edge.deleted || edge.physical_geometry.len() < 2 || edge.physical_length <= 1e-6 {
            return entrance;
        }

        entrance.entrance_s_m =
            Self::project_point_to_polyline_s(&edge.physical_geometry, entrance.door_pos);
        let entrance_t = entrance.entrance_s_m / edge.physical_length;
        let edge_pos = Self::sample_pos_on_edge(graph, building.edge_idx, entrance_t);

        derive_foot_lanes(&mut entrance, edge, building.side, lanes);
        derive_car_lanes(&mut entrance, edge, building.side, lanes);
        derive_flags(&mut entrance);
        derive_curb_pos(&mut entrance, edge_pos, lanes);

        entrance
    }

    pub(crate) fn freight_car_eta_between_buildings(
        &self,
        source_idx: usize,
        destination_idx: usize,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<f32> {
        if source_idx >= self.buildings.len()
            || destination_idx >= self.buildings.len()
            || source_idx >= self.entrances.len()
            || destination_idx >= self.entrances.len()
        {
            return None;
        }

        let origin_entrance = &self.entrances[source_idx];
        let destination_entrance = &self.entrances[destination_idx];
        let origin_edge = graph.get_edge(origin_entrance.edge_idx)?;
        let destination_edge = graph.get_edge(destination_entrance.edge_idx)?;
        if origin_edge.deleted || destination_edge.deleted {
            return None;
        }
        if !entrance_has_car_access(origin_entrance)
            || !entrance_has_car_access(destination_entrance)
        {
            return None;
        }

        let mut best: Option<FreightCarCandidate> = None;
        for origin_rank in [0_u8, 1_u8] {
            for destination_rank in [0_u8, 1_u8] {
                let Some(candidate) = freight_candidate_between_entrances(
                    origin_rank,
                    destination_rank,
                    origin_entrance,
                    destination_entrance,
                    transit_network,
                    graph,
                ) else {
                    continue;
                };
                if best
                    .as_ref()
                    .is_none_or(|current| freight_candidate_better(&candidate, current))
                {
                    best = Some(candidate);
                }
            }
        }

        best.map(|candidate| candidate.total_time_s)
    }

    pub(crate) fn freight_car_eta_from_border_node(
        &self,
        border_node: u32,
        destination_idx: usize,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<f32> {
        if destination_idx >= self.buildings.len() || destination_idx >= self.entrances.len() {
            crate::debug_log!(
                "spawn",
                "border admission route rejected: border_node={} home_building={} reason=destination_out_of_range buildings={} entrances={}",
                border_node,
                destination_idx,
                self.buildings.len(),
                self.entrances.len()
            );
            return None;
        }

        let destination_entrance = &self.entrances[destination_idx];
        let Some(destination_edge) = graph.get_edge(destination_entrance.edge_idx) else {
            crate::debug_log!(
                "spawn",
                "border admission route rejected: border_node={} home_building={} dest_edge={} reason=destination_edge_missing edge_count={}",
                border_node,
                destination_idx,
                destination_entrance.edge_idx,
                graph.edge_count()
            );
            return None;
        };
        if destination_edge.deleted || !entrance_has_car_access(destination_entrance) {
            crate::debug_log!(
                "spawn",
                "border admission route rejected: border_node={} home_building={} dest_edge={} reason={} car_fwd={} car_bkw={} edge_deleted={}",
                border_node,
                destination_idx,
                destination_entrance.edge_idx,
                if destination_edge.deleted {
                    "destination_edge_deleted"
                } else {
                    "destination_has_no_car_access"
                },
                destination_entrance.car_lane_fwd,
                destination_entrance.car_lane_bkw,
                destination_edge.deleted
            );
            return None;
        }

        let mut best: Option<FreightBorderCandidate> = None;
        for destination_rank in [0_u8, 1_u8] {
            let Some(candidate) = freight_candidate_from_border(
                border_node,
                destination_rank,
                destination_entrance,
                transit_network,
                graph,
            ) else {
                continue;
            };
            if best
                .as_ref()
                .is_none_or(|current| freight_border_candidate_better(&candidate, current))
            {
                best = Some(candidate);
            }
        }

        best.map(|candidate| candidate.total_time_s)
    }
}

fn log_entrance_cache_rebuild(
    old_entrances: &[BuildingEntrance],
    new_entrances: &[BuildingEntrance],
    buildings: &[Building],
) {
    let mut changed = 0usize;
    let mut edge_changed = 0usize;
    let mut lanes_changed = 0usize;
    let mut lost_foot = 0usize;
    let mut lost_car = 0usize;
    let mut gained_foot = 0usize;
    let mut gained_car = 0usize;

    for (idx, new) in new_entrances.iter().enumerate() {
        let Some(old) = old_entrances.get(idx) else {
            changed += 1;
            continue;
        };
        if !entrance_debug_changed(old, new) {
            continue;
        }

        changed += 1;
        edge_changed += usize::from(old.edge_idx != new.edge_idx);
        lanes_changed += usize::from(entrance_lane_fields_changed(old, new));

        let old_foot = entrance_has_foot_access_debug(old);
        let new_foot = entrance_has_foot_access_debug(new);
        let old_car = entrance_has_car_access(old);
        let new_car = entrance_has_car_access(new);
        lost_foot += usize::from(old_foot && !new_foot);
        gained_foot += usize::from(!old_foot && new_foot);
        lost_car += usize::from(old_car && !new_car);
        gained_car += usize::from(!old_car && new_car);

        let building = buildings.get(idx);
        crate::traffic_log!(
            "[ENTRANCE_CACHE]   bldg={} building_edge={} frontage_t={:.3} edge {}->{} side {}->{} flags 0x{:02x}->0x{:02x} s {:.2}->{:.2} foot_fwd {}->{} foot_bkw {}->{} car_fwd {}->{} car_bkw {}->{} curb ({:.2},{:.2})->({:.2},{:.2})",
            idx,
            building.map(|b| b.edge_idx).unwrap_or(usize::MAX),
            building.map(|b| b.frontage_t).unwrap_or(0.0),
            old.edge_idx,
            new.edge_idx,
            old.side,
            new.side,
            old.flags,
            new.flags,
            old.entrance_s_m,
            new.entrance_s_m,
            old.foot_lane_fwd,
            new.foot_lane_fwd,
            old.foot_lane_bkw,
            new.foot_lane_bkw,
            old.car_lane_fwd,
            new.car_lane_fwd,
            old.car_lane_bkw,
            new.car_lane_bkw,
            old.curb_pos.x,
            old.curb_pos.y,
            new.curb_pos.x,
            new.curb_pos.y,
        );
    }

    crate::traffic_log!(
        "[ENTRANCE_CACHE] rebuild old_entries={} new_entries={} changed={} edge_changed={} lanes_changed={} lost_foot={} gained_foot={} lost_car={} gained_car={}",
        old_entrances.len(),
        new_entrances.len(),
        changed,
        edge_changed,
        lanes_changed,
        lost_foot,
        gained_foot,
        lost_car,
        gained_car,
    );
}

fn entrance_debug_changed(old: &BuildingEntrance, new: &BuildingEntrance) -> bool {
    old.edge_idx != new.edge_idx
        || old.side != new.side
        || old.vehicle_frontage_access != new.vehicle_frontage_access
        || old.flags != new.flags
        || entrance_lane_fields_changed(old, new)
        || (old.entrance_s_m - new.entrance_s_m).abs() > 0.01
        || old.curb_pos.distance_squared_to(new.curb_pos) > 0.0001
}

fn entrance_lane_fields_changed(old: &BuildingEntrance, new: &BuildingEntrance) -> bool {
    old.foot_lane_fwd != new.foot_lane_fwd
        || old.foot_lane_bkw != new.foot_lane_bkw
        || old.car_lane_fwd != new.car_lane_fwd
        || old.car_lane_bkw != new.car_lane_bkw
}

fn entrance_has_foot_access_debug(entrance: &BuildingEntrance) -> bool {
    entrance.foot_lane_fwd != INVALID_LANE_ID || entrance.foot_lane_bkw != INVALID_LANE_ID
}

pub(crate) fn main_entrance_anchor(
    anchors: &[crate::assets::Anchor],
) -> Option<&crate::assets::Anchor> {
    let mut match_idx = None;
    for (idx, anchor) in anchors.iter().enumerate() {
        if anchor.anchor_type == AnchorType::Entrance && anchor.name == "main" {
            if match_idx.is_some() {
                return None;
            }
            match_idx = Some(idx);
        }
    }
    match_idx.map(|idx| &anchors[idx])
}

fn world_door_pos(
    building: &Building,
    anchor_position: [f32; 3],
    anchor_forward: [f32; 3],
) -> Vector2 {
    building_local_xz_pos(building, anchor_position, anchor_forward)
}

pub(crate) fn building_local_xz_pos(
    building: &Building,
    anchor_position: [f32; 3],
    anchor_forward: [f32; 3],
) -> Vector2 {
    let (basis_x, basis_z) = building_local_xz_basis(building.facing_dir, anchor_forward);
    let local_x = anchor_position[0];
    let local_z = anchor_position[2];

    Vector2::new(building.center_x, building.center_y) + basis_x * local_x + basis_z * local_z
}

pub(crate) fn building_local_xz_basis(
    facing_dir: Vector2,
    anchor_forward: [f32; 3],
) -> (Vector2, Vector2) {
    let world_front = if facing_dir.length_squared() > 1e-12 {
        facing_dir.normalized()
    } else {
        Vector2::new(0.0, 1.0)
    };
    let local_front = asset_local_front_xz(anchor_forward);
    let world_right = Vector2::new(world_front.y, -world_front.x);
    let basis_x = world_right * local_front.y + world_front * local_front.x;
    let basis_z = world_front * local_front.y - world_right * local_front.x;

    (basis_x, basis_z)
}

fn asset_local_front_xz(anchor_forward: [f32; 3]) -> Vector2 {
    let front = Vector2::new(anchor_forward[0], anchor_forward[2]);
    if front.length_squared() > 1e-12 {
        front.normalized()
    } else {
        Vector2::new(0.0, 1.0)
    }
}

fn derive_foot_lanes(
    entrance: &mut BuildingEntrance,
    edge: &crate::simulation::network::graph::Edge,
    side: i8,
    lanes: &LaneSystem,
) {
    if (edge.allowed_types & TransitFlags::FOOT) == 0 {
        return;
    }

    if edge.primary_type == TransitType::Foot {
        entrance.foot_lane_fwd = unique_lane_id(lanes, entrance.edge_idx, LaneType::Foot, true, 0);
        entrance.foot_lane_bkw = unique_lane_id(lanes, entrance.edge_idx, LaneType::Foot, false, 0);
        return;
    }

    let sidewalk_idx = side * 100;
    let foot_fwd = unique_lane_id(lanes, entrance.edge_idx, LaneType::Foot, true, sidewalk_idx);
    let foot_bkw = unique_lane_id(
        lanes,
        entrance.edge_idx,
        LaneType::Foot,
        false,
        sidewalk_idx,
    );
    if foot_fwd != INVALID_LANE_ID && foot_bkw != INVALID_LANE_ID {
        entrance.foot_lane_fwd = foot_fwd;
        entrance.foot_lane_bkw = foot_bkw;
    }
}

fn derive_car_lanes(
    entrance: &mut BuildingEntrance,
    edge: &crate::simulation::network::graph::Edge,
    side: i8,
    lanes: &LaneSystem,
) {
    if (edge.allowed_types & TransitFlags::CAR) == 0 || edge.primary_type == TransitType::Foot {
        return;
    }

    let mut car_fwd = best_vehicle_lane(lanes, entrance.edge_idx, true);
    let mut car_bkw = best_vehicle_lane(lanes, entrance.edge_idx, false);

    match entrance.vehicle_frontage_access {
        VehicleFrontageAccess::SameSideOnly => {
            if side == -1 {
                car_bkw = INVALID_LANE_ID;
            } else if side == 1 {
                car_fwd = INVALID_LANE_ID;
            }
        }
        VehicleFrontageAccess::BothSides => {}
    }

    entrance.car_lane_fwd = car_fwd;
    entrance.car_lane_bkw = car_bkw;
}

fn derive_flags(entrance: &mut BuildingEntrance) {
    entrance.flags = 0;

    if entrance.foot_lane_fwd != INVALID_LANE_ID {
        entrance.flags |= ENTRANCE_FOOT_FWD_VALID;
    }
    if entrance.foot_lane_bkw != INVALID_LANE_ID {
        entrance.flags |= ENTRANCE_FOOT_BKW_VALID;
    }
    if entrance.car_lane_fwd != INVALID_LANE_ID {
        entrance.flags |= ENTRANCE_CAR_FWD_VALID;
    }
    if entrance.car_lane_bkw != INVALID_LANE_ID {
        entrance.flags |= ENTRANCE_CAR_BKW_VALID;
    }
    if entrance.flags & (ENTRANCE_FOOT_FWD_VALID | ENTRANCE_FOOT_BKW_VALID) != 0 {
        entrance.flags |= ENTRANCE_FOOT_VALID;
    }
    if entrance.flags & (ENTRANCE_CAR_FWD_VALID | ENTRANCE_CAR_BKW_VALID) != 0 {
        entrance.flags |= ENTRANCE_CAR_VALID;
    }
}

fn derive_curb_pos(entrance: &mut BuildingEntrance, edge_pos: Vector2, lanes: &LaneSystem) {
    let curb_lane = if entrance.foot_lane_fwd != INVALID_LANE_ID {
        entrance.foot_lane_fwd
    } else if entrance.foot_lane_bkw != INVALID_LANE_ID {
        entrance.foot_lane_bkw
    } else {
        INVALID_LANE_ID
    };

    if curb_lane == INVALID_LANE_ID {
        entrance.curb_pos = entrance.door_pos;
        return;
    }

    let lane = &lanes.lanes[curb_lane];
    let lane_d = BuildingAllocator::project_point_to_polyline_s(&lane.geometry, edge_pos);
    entrance.curb_pos = BuildingAllocator::sample_pos_on_lane(lane, lane_d);
}

fn entrance_has_car_access(entrance: &BuildingEntrance) -> bool {
    entrance.car_lane_fwd != INVALID_LANE_ID || entrance.car_lane_bkw != INVALID_LANE_ID
}

fn freight_candidate_lane_id(
    entrance: &BuildingEntrance,
    toward_start: bool,
    origin: bool,
) -> usize {
    match (toward_start, origin) {
        (true, true) => entrance.car_lane_bkw,
        (false, true) => entrance.car_lane_fwd,
        (true, false) => entrance.car_lane_fwd,
        (false, false) => entrance.car_lane_bkw,
    }
}

fn lane_origin_node(
    lane_id: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<u32> {
    let lane = transit_network.lane_system.lanes.get(lane_id)?;
    if lane.edge_id == usize::MAX || lane.edge_id >= graph.edge_count() {
        return None;
    }
    let edge = graph.edge(lane.edge_id);
    Some(if lane.is_fwd {
        edge.start_node
    } else {
        edge.end_node
    })
}

fn lane_terminal_node(
    lane_id: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<u32> {
    let lane = transit_network.lane_system.lanes.get(lane_id)?;
    if lane.edge_id == usize::MAX || lane.edge_id >= graph.edge_count() {
        return None;
    }
    let edge = graph.edge(lane.edge_id);
    Some(if lane.is_fwd {
        edge.end_node
    } else {
        edge.start_node
    })
}

fn entrance_edge_pos(entrance: &BuildingEntrance, graph: &RegionGraph) -> Option<Vector2> {
    let edge = graph.get_edge(entrance.edge_idx)?;
    if edge.deleted || edge.physical_length <= 1e-6 {
        return None;
    }
    Some(BuildingAllocator::sample_pos_on_edge(
        graph,
        entrance.edge_idx,
        entrance.entrance_s_m / edge.physical_length,
    ))
}

fn entrance_edge_normal(entrance: &BuildingEntrance, graph: &RegionGraph) -> Option<Vector2> {
    let edge = graph.get_edge(entrance.edge_idx)?;
    if edge.deleted || edge.physical_length <= 1e-6 {
        return None;
    }
    let tangent = BuildingAllocator::sample_tangent_on_edge(
        graph,
        entrance.edge_idx,
        entrance.entrance_s_m / edge.physical_length,
    );
    Some(Vector2::new(tangent.y, -tangent.x) * entrance.side as f32)
}

fn projected_lane_distance_for_entrance(
    entrance: &BuildingEntrance,
    lane_id: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<f32> {
    let edge_pos = entrance_edge_pos(entrance, graph)?;
    let lane = transit_network.lane_system.lanes.get(lane_id)?;
    Some(BuildingAllocator::project_point_to_polyline_s(
        &lane.geometry,
        edge_pos,
    ))
}

fn local_access_point(
    _entrance: &BuildingEntrance,
    lane_id: usize,
    lane_d: f32,
    transit_network: &TransitNetwork,
) -> Option<Vector2> {
    let lane = transit_network.lane_system.lanes.get(lane_id)?;
    Some(BuildingAllocator::sample_pos_on_lane(lane, lane_d))
}

fn same_side_car_lane(entrance: &BuildingEntrance) -> usize {
    if entrance.side == -1 {
        entrance.car_lane_fwd
    } else {
        entrance.car_lane_bkw
    }
}

fn opposite_side_car_lane(entrance: &BuildingEntrance) -> usize {
    if entrance.side == -1 {
        entrance.car_lane_bkw
    } else {
        entrance.car_lane_fwd
    }
}

fn local_access_distance_car(
    entrance: &BuildingEntrance,
    lane_id: usize,
    lane_d: f32,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<f32> {
    let chosen_lane_point = local_access_point(entrance, lane_id, lane_d, transit_network)?;
    let same_side_lane = same_side_car_lane(entrance);
    let opposite_side_lane = opposite_side_car_lane(entrance);

    if entrance.vehicle_frontage_access == VehicleFrontageAccess::SameSideOnly
        || lane_id == same_side_lane
    {
        return Some((entrance.door_pos - chosen_lane_point).length());
    }
    if entrance.vehicle_frontage_access != VehicleFrontageAccess::BothSides
        || lane_id != opposite_side_lane
    {
        return None;
    }

    let edge_pos = entrance_edge_pos(entrance, graph)?;
    let normal = entrance_edge_normal(entrance, graph)?;
    let edge = graph.edge(entrance.edge_idx);
    let same_side_cross_point = edge_pos + normal * (edge.width * 0.5);
    let opposite_side_cross_point = edge_pos - normal * (edge.width * 0.5);
    Some(
        (entrance.door_pos - same_side_cross_point).length()
            + (same_side_cross_point - opposite_side_cross_point).length()
            + (opposite_side_cross_point - chosen_lane_point).length(),
    )
}

fn local_access_time_s(distance: f32) -> f32 {
    distance / AGENT_DRIVEWAY_SPEED_MS
}

fn frontage_time_s(
    lane_id: usize,
    lane_d: f32,
    from_attach_point: bool,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<f32> {
    let lane = transit_network.lane_system.lanes.get(lane_id)?;
    if lane.edge_id == usize::MAX || lane.edge_id >= graph.edge_count() {
        return None;
    }
    let edge = graph.edge(lane.edge_id);
    let frontage_distance = if from_attach_point {
        (lane.length - lane_d).max(0.0)
    } else {
        lane_d.max(0.0)
    };
    if frontage_distance <= 1e-6 {
        return Some(0.0);
    }
    if !edge.speed_limit.is_finite() || edge.speed_limit <= 1e-6 {
        return None;
    }

    let free_flow_time = frontage_distance / edge.speed_limit;
    if lane.length <= 1e-6 {
        return Some(free_flow_time);
    }

    let penalty_ratio = if from_attach_point {
        (lane.length - lane_d) / lane.length
    } else {
        lane_d / lane.length
    };
    Some(free_flow_time + penalty_ratio.max(0.0) * lane.frontage_delay_penalty_s)
}

fn direct_frontage_segment_time_s(
    lane_id: usize,
    start_lane_d: f32,
    end_lane_d: f32,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<f32> {
    let lane = transit_network.lane_system.lanes.get(lane_id)?;
    if lane.edge_id == usize::MAX || lane.edge_id >= graph.edge_count() {
        return None;
    }
    if end_lane_d + 1e-6 < start_lane_d {
        return None;
    }
    let segment_distance = (end_lane_d - start_lane_d).max(0.0);
    if segment_distance <= 1e-6 {
        return Some(0.0);
    }

    let edge = graph.edge(lane.edge_id);
    if !edge.speed_limit.is_finite() || edge.speed_limit <= 1e-6 {
        return None;
    }

    let free_flow_time = segment_distance / edge.speed_limit;
    if lane.length <= 1e-6 {
        return Some(free_flow_time);
    }

    let penalty_ratio = segment_distance / lane.length;
    Some(free_flow_time + penalty_ratio.max(0.0) * lane.frontage_delay_penalty_s)
}

fn freight_candidate_better(
    new_candidate: &FreightCarCandidate,
    best: &FreightCarCandidate,
) -> bool {
    new_candidate.total_time_s < best.total_time_s
        || (new_candidate.total_time_s == best.total_time_s
            && (
                new_candidate.origin_rank,
                new_candidate.destination_rank,
                new_candidate.attach_lane_id,
                new_candidate.detach_lane_id,
                new_candidate.attach_lane_d.to_bits(),
                new_candidate.detach_lane_d.to_bits(),
            ) < (
                best.origin_rank,
                best.destination_rank,
                best.attach_lane_id,
                best.detach_lane_id,
                best.attach_lane_d.to_bits(),
                best.detach_lane_d.to_bits(),
            ))
}

fn freight_border_candidate_better(
    new_candidate: &FreightBorderCandidate,
    best: &FreightBorderCandidate,
) -> bool {
    new_candidate.total_time_s < best.total_time_s
        || (new_candidate.total_time_s == best.total_time_s
            && (
                new_candidate.destination_rank,
                new_candidate.detach_lane_id,
                new_candidate.detach_lane_d.to_bits(),
            ) < (
                best.destination_rank,
                best.detach_lane_id,
                best.detach_lane_d.to_bits(),
            ))
}

fn freight_candidate_between_entrances(
    origin_rank: u8,
    destination_rank: u8,
    origin_entrance: &BuildingEntrance,
    destination_entrance: &BuildingEntrance,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<FreightCarCandidate> {
    if origin_entrance.edge_idx >= graph.edge_count()
        || destination_entrance.edge_idx >= graph.edge_count()
    {
        return None;
    }
    let origin_edge = graph.edge(origin_entrance.edge_idx);
    let destination_edge = graph.edge(destination_entrance.edge_idx);
    if origin_edge.deleted || destination_edge.deleted {
        return None;
    }

    let planned_attach_node = if origin_rank == 0 {
        origin_edge.start_node
    } else {
        origin_edge.end_node
    };
    let planned_detach_node = if destination_rank == 0 {
        destination_edge.start_node
    } else {
        destination_edge.end_node
    };

    let planned_attach_lane_id = freight_candidate_lane_id(origin_entrance, origin_rank == 0, true);
    let planned_detach_lane_id =
        freight_candidate_lane_id(destination_entrance, destination_rank == 0, false);
    if planned_attach_lane_id == INVALID_LANE_ID || planned_detach_lane_id == INVALID_LANE_ID {
        return None;
    }
    if lane_terminal_node(planned_attach_lane_id, transit_network, graph)? != planned_attach_node {
        return None;
    }
    if lane_origin_node(planned_detach_lane_id, transit_network, graph)? != planned_detach_node {
        return None;
    }

    let planned_attach_lane_d = projected_lane_distance_for_entrance(
        origin_entrance,
        planned_attach_lane_id,
        transit_network,
        graph,
    )?;
    let planned_detach_lane_d = projected_lane_distance_for_entrance(
        destination_entrance,
        planned_detach_lane_id,
        transit_network,
        graph,
    )?;

    let egress_local_time_s = local_access_time_s(local_access_distance_car(
        origin_entrance,
        planned_attach_lane_id,
        planned_attach_lane_d,
        transit_network,
        graph,
    )?);
    let ingress_local_time_s = local_access_time_s(local_access_distance_car(
        destination_entrance,
        planned_detach_lane_id,
        planned_detach_lane_d,
        transit_network,
        graph,
    )?);

    let same_lane_direct_frontage = origin_entrance.edge_idx == destination_entrance.edge_idx
        && planned_attach_lane_id == planned_detach_lane_id
        && planned_attach_lane_d <= planned_detach_lane_d + 1e-6;

    let total_time_s = if same_lane_direct_frontage {
        let direct_frontage_time_s = direct_frontage_segment_time_s(
            planned_attach_lane_id,
            planned_attach_lane_d,
            planned_detach_lane_d,
            transit_network,
            graph,
        )?;
        egress_local_time_s + direct_frontage_time_s + ingress_local_time_s
    } else {
        let origin_frontage_time_s = frontage_time_s(
            planned_attach_lane_id,
            planned_attach_lane_d,
            true,
            transit_network,
            graph,
        )?;
        let destination_frontage_time_s = frontage_time_s(
            planned_detach_lane_id,
            planned_detach_lane_d,
            false,
            transit_network,
            graph,
        )?;
        let network_path_time_s = if planned_attach_node == planned_detach_node {
            0.0
        } else {
            transit_network
                .cch_graph
                .find_path(
                    planned_attach_node,
                    planned_detach_node,
                    usize::MAX,
                    graph,
                    TransitFlags::CAR,
                )
                .map(|(travel_seconds, _, _)| travel_seconds)?
        };
        egress_local_time_s
            + origin_frontage_time_s
            + network_path_time_s
            + destination_frontage_time_s
            + ingress_local_time_s
    };

    if !total_time_s.is_finite() {
        return None;
    }

    Some(FreightCarCandidate {
        total_time_s,
        origin_rank,
        destination_rank,
        attach_lane_id: planned_attach_lane_id,
        detach_lane_id: planned_detach_lane_id,
        attach_lane_d: planned_attach_lane_d,
        detach_lane_d: planned_detach_lane_d,
    })
}

fn freight_candidate_from_border(
    border_node: u32,
    destination_rank: u8,
    destination_entrance: &BuildingEntrance,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<FreightBorderCandidate> {
    if destination_entrance.edge_idx >= graph.edge_count() {
        crate::debug_log!(
            "spawn",
            "border admission candidate rejected: border_node={} dest_edge={} rank={} reason=dest_edge_out_of_range edge_count={}",
            border_node,
            destination_entrance.edge_idx,
            destination_rank,
            graph.edge_count()
        );
        return None;
    }
    let destination_edge = graph.edge(destination_entrance.edge_idx);
    if destination_edge.deleted {
        crate::debug_log!(
            "spawn",
            "border admission candidate rejected: border_node={} dest_edge={} rank={} reason=dest_edge_deleted",
            border_node,
            destination_entrance.edge_idx,
            destination_rank
        );
        return None;
    }

    let planned_detach_node = if destination_rank == 0 {
        destination_edge.start_node
    } else {
        destination_edge.end_node
    };
    let planned_detach_lane_id =
        freight_candidate_lane_id(destination_entrance, destination_rank == 0, false);
    if planned_detach_lane_id == INVALID_LANE_ID {
        crate::debug_log!(
            "spawn",
            "border admission candidate rejected: border_node={} dest_edge={} rank={} detach_node={} reason=missing_detach_lane car_fwd={} car_bkw={}",
            border_node,
            destination_entrance.edge_idx,
            destination_rank,
            planned_detach_node,
            destination_entrance.car_lane_fwd,
            destination_entrance.car_lane_bkw
        );
        return None;
    }
    let Some(detach_lane_origin) = lane_origin_node(planned_detach_lane_id, transit_network, graph)
    else {
        crate::debug_log!(
            "spawn",
            "border admission candidate rejected: border_node={} dest_edge={} rank={} detach_lane={} reason=detach_lane_origin_missing",
            border_node,
            destination_entrance.edge_idx,
            destination_rank,
            planned_detach_lane_id
        );
        return None;
    };
    if detach_lane_origin != planned_detach_node {
        crate::debug_log!(
            "spawn",
            "border admission candidate rejected: border_node={} dest_edge={} rank={} detach_lane={} reason=detach_lane_origin_mismatch expected={} actual={}",
            border_node,
            destination_entrance.edge_idx,
            destination_rank,
            planned_detach_lane_id,
            planned_detach_node,
            detach_lane_origin
        );
        return None;
    }

    let Some(planned_detach_lane_d) = projected_lane_distance_for_entrance(
        destination_entrance,
        planned_detach_lane_id,
        transit_network,
        graph,
    ) else {
        crate::debug_log!(
            "spawn",
            "border admission candidate rejected: border_node={} dest_edge={} rank={} detach_lane={} reason=project_detach_distance_failed",
            border_node,
            destination_entrance.edge_idx,
            destination_rank,
            planned_detach_lane_id
        );
        return None;
    };
    let Some(ingress_local_distance_m) = local_access_distance_car(
        destination_entrance,
        planned_detach_lane_id,
        planned_detach_lane_d,
        transit_network,
        graph,
    ) else {
        crate::debug_log!(
            "spawn",
            "border admission candidate rejected: border_node={} dest_edge={} rank={} detach_lane={} detach_d={:.2} reason=local_access_failed frontage_access={:?}",
            border_node,
            destination_entrance.edge_idx,
            destination_rank,
            planned_detach_lane_id,
            planned_detach_lane_d,
            destination_entrance.vehicle_frontage_access
        );
        return None;
    };
    let ingress_local_time_s = local_access_time_s(ingress_local_distance_m);
    let Some(destination_frontage_time_s) = frontage_time_s(
        planned_detach_lane_id,
        planned_detach_lane_d,
        false,
        transit_network,
        graph,
    ) else {
        crate::debug_log!(
            "spawn",
            "border admission candidate rejected: border_node={} dest_edge={} rank={} detach_lane={} detach_d={:.2} reason=frontage_time_failed",
            border_node,
            destination_entrance.edge_idx,
            destination_rank,
            planned_detach_lane_id,
            planned_detach_lane_d
        );
        return None;
    };
    let network_path_time_s = if border_node == planned_detach_node {
        0.0
    } else {
        let Some((travel_seconds, _, _)) = transit_network.cch_graph.find_path(
            border_node,
            planned_detach_node,
            usize::MAX,
            graph,
            TransitFlags::CAR,
        ) else {
            crate::debug_log!(
                "spawn",
                "border admission candidate rejected: border_node={} dest_edge={} rank={} detach_node={} detach_lane={} reason=cch_path_failed",
                border_node,
                destination_entrance.edge_idx,
                destination_rank,
                planned_detach_node,
                planned_detach_lane_id
            );
            return None;
        };
        travel_seconds
    };

    let total_time_s = network_path_time_s + destination_frontage_time_s + ingress_local_time_s;
    if !total_time_s.is_finite() {
        return None;
    }

    Some(FreightBorderCandidate {
        total_time_s,
        destination_rank,
        detach_lane_id: planned_detach_lane_id,
        detach_lane_d: planned_detach_lane_d,
    })
}

fn unique_lane_id(
    lanes: &LaneSystem,
    edge_idx: usize,
    lane_type: LaneType,
    is_fwd: bool,
    lane_idx: i8,
) -> usize {
    let mut found = INVALID_LANE_ID;
    let Some(edge_lanes) = lanes.edge_lanes.get(&edge_idx) else {
        return INVALID_LANE_ID;
    };

    for &lane_id in edge_lanes {
        let lane = &lanes.lanes[lane_id];
        if lane.lane_type == lane_type && lane.is_fwd == is_fwd && lane.lane_idx == lane_idx {
            if found != INVALID_LANE_ID {
                return INVALID_LANE_ID;
            }
            found = lane_id;
        }
    }

    found
}

fn best_vehicle_lane(lanes: &LaneSystem, edge_idx: usize, is_fwd: bool) -> usize {
    let Some(edge_lanes) = lanes.edge_lanes.get(&edge_idx) else {
        return INVALID_LANE_ID;
    };

    let mut best_lane = INVALID_LANE_ID;
    let mut best_idx = if is_fwd { i8::MIN } else { i8::MAX };

    for &lane_id in edge_lanes {
        let lane = &lanes.lanes[lane_id];
        if lane.lane_type != LaneType::Vehicle || lane.is_fwd != is_fwd {
            continue;
        }

        let better = if is_fwd {
            lane.lane_idx > best_idx || (lane.lane_idx == best_idx && lane_id < best_lane)
        } else {
            lane.lane_idx < best_idx || (lane.lane_idx == best_idx && lane_id < best_lane)
        };
        if better {
            best_idx = lane.lane_idx;
            best_lane = lane_id;
        }
    }

    best_lane
}
