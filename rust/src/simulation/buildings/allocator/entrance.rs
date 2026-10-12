// SPDX-License-Identifier: GPL-2.0-only

//! Derived entrance/access cache built from building placement, asset anchors, and live lanes.

use crate::assets::AnchorType;
use crate::config::AGENT_DRIVEWAY_SPEED_MS;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator, BuildingEntrance};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::{LaneSystem, LaneType};
use crate::simulation::network::types::{TransitFlags, TransitType, VehicleFrontageAccess};
use crate::simulation::pathing::cch::CchCostsFrom;
use godot::prelude::Vector2;
use rayon::prelude::*;

const INVALID_LANE_ID: usize = usize::MAX;

const ENTRANCE_FOOT_VALID: u8 = 0x01;
const ENTRANCE_CAR_VALID: u8 = 0x02;
const ENTRANCE_FOOT_FWD_VALID: u8 = 0x04;
const ENTRANCE_FOOT_BKW_VALID: u8 = 0x08;
const ENTRANCE_CAR_FWD_VALID: u8 = 0x10;
const ENTRANCE_CAR_BKW_VALID: u8 = 0x20;

/// Exact proposed-door inputs; network generations cover frontage geometry and lane IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DemandSiteEntranceKey {
    door: Option<[u32; 2]>,
    edge: usize,
    side: i8,
    topology: u32,
    lanes: u64,
}

/// Exact entrance inputs for derived geometry/proofs, including every floating-point bit.
/// Unrelated allocator revisions may retain work only when this key is unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BuildingEntranceKey {
    edge: usize,
    side: i8,
    frontage: VehicleFrontageAccess,
    geometry: [u32; 5],
    lanes: [usize; 4],
    flags: u8,
}

impl BuildingEntranceKey {
    /// Captures the complete current door, lane and access configuration without allocation.
    pub(crate) fn new(entrance: &BuildingEntrance) -> Self {
        Self {
            edge: entrance.edge_idx,
            side: entrance.side,
            frontage: entrance.vehicle_frontage_access,
            geometry: [entrance.entrance_s_m, entrance.door_pos.x, entrance.door_pos.y,
                entrance.curb_pos.x, entrance.curb_pos.y].map(f32::to_bits),
            lanes: [entrance.foot_lane_fwd, entrance.foot_lane_bkw,
                entrance.car_lane_fwd, entrance.car_lane_bkw],
            flags: entrance.flags,
        }
    }
}

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

/// Fixed freight access geometry for a door, reusable until its entrance or lanes change.
/// Traffic-dependent frontage times remain priced by the query.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FreightEntranceAccess {
    edge_idx: usize,
    // Departure and arrival slots indexed by the existing deterministic node rank.
    departure: [Option<FreightAccessEnd>; 2],
    arrival: [Option<FreightAccessEnd>; 2],
}

#[derive(Clone, Copy, Debug)]
struct FreightAccessEnd {
    node: u32,
    lane_id: usize,
    lane_d: f32,
    local_time_s: f32,
}

impl FreightEntranceAccess {
    /// Projects each usable car lane once; storage is fixed and query-local allocation is absent.
    pub(crate) fn new(entrance: &BuildingEntrance, network: &TransitNetwork, graph: &RegionGraph) -> Self {
        let mut access = Self { edge_idx: entrance.edge_idx, ..Self::default() };
        let Some(edge) = graph.get_edge(entrance.edge_idx).filter(|edge| !edge.deleted) else {
            return access;
        };
        for (rank, lane_id) in [entrance.car_lane_fwd, entrance.car_lane_bkw].into_iter().enumerate() {
            if lane_id == INVALID_LANE_ID { continue; }
            let Some(lane_d) = projected_lane_distance_for_entrance(entrance, lane_id, network, graph) else { continue; };
            let Some(distance) = local_access_distance_car(entrance, lane_id, lane_d, network, graph) else { continue; };
            let arrival_node = if rank == 0 { edge.start_node } else { edge.end_node };
            let departure_node = if rank == 0 { edge.end_node } else { edge.start_node };
            let end = FreightAccessEnd { node: arrival_node, lane_id, lane_d,
                local_time_s: local_access_time_s(distance) };
            if lane_origin_node(lane_id, network, graph) == Some(arrival_node) {
                access.arrival[rank] = Some(end);
            }
            if lane_terminal_node(lane_id, network, graph) == Some(departure_node) {
                access.departure[1 - rank] = Some(FreightAccessEnd { node: departure_node, ..end });
            }
        }
        access
    }

    /// Prices a directed delivery using prepared access and current frontage penalties.
    pub(crate) fn eta_to(&self, destination: &Self, network: &TransitNetwork, graph: &RegionGraph,
        mut network_time_s: impl FnMut(u32, u32) -> Option<f32>) -> Option<f32> {
        let mut best: Option<FreightCarCandidate> = None;
        for origin_rank in [0_u8, 1_u8] {
            for destination_rank in [0_u8, 1_u8] {
                let Some(candidate) = freight_candidate_between_access(origin_rank, destination_rank,
                    self, destination, network, graph, &mut network_time_s) else { continue; };
                if best.as_ref().is_none_or(|current| freight_candidate_better(&candidate, current)) {
                    best = Some(candidate);
                }
            }
        }
        best.map(|candidate| candidate.total_time_s)
    }

    /// Prices arrivals from a prepared network-cost table, preserving border ETA arithmetic.
    /// The table includes zero at each allowed source, and unreachable nodes return `None`.
    pub(crate) fn eta_from_network(&self, network: &TransitNetwork, graph: &RegionGraph,
        mut cost_to: impl FnMut(u32) -> Option<f32>) -> Option<f32> {
        self.arrival.iter().flatten().filter_map(|end| {
            let frontage = frontage_time_s(end.lane_id, end.lane_d, false, network, graph)?;
            let seconds = cost_to(end.node)? + frontage + end.local_time_s;
            seconds.is_finite().then_some(seconds)
        }).min_by(f32::total_cmp)
    }
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
        let traffic_debug = crate::debug::Channel::Traffic.enabled();
        let old_entrances = traffic_debug.then(|| self.entrances.clone());
        self.buildings.par_iter().map(|building| {
            Self::derive_building_entrance(building, graph, lanes, &self.registry)
        }).collect_into_vec(&mut self.entrances);
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
        self.buildings.par_iter().map(|building| {
            Self::derive_building_entrance(building, graph, lanes, registry)
        }).collect_into_vec(&mut self.entrances);
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
        Self::derive_site_entrance(
            Vector2::new(building.center_x, building.center_y),
            building.facing_dir,
            building.edge_idx,
            building.side,
            registry.get(&building.asset_id),
            graph,
            lanes,
        )
    }

    /// Uses the placement transform and actual asset door without allocating a temporary building.
    /// Work is bounded by the candidate asset's anchors and its frontage lane geometry.
    pub(crate) fn demand_site_entrance(
        &self,
        parcel_id: u64,
        asset_id: &str,
        zoning: &crate::simulation::zoning::ZoningSystem,
        graph: &RegionGraph,
        lanes: &LaneSystem,
    ) -> Option<BuildingEntrance> {
        self.demand_site_entrance_reusing(parcel_id, asset_id, zoning, graph, lanes, 0, None)
            .map(|(_, entrance)| entrance)
    }

    /// Reuses a prepared entrance only when its complete geometry inputs still match.
    /// Lookup and key construction allocate nothing; a miss projects onto frontage geometry.
    pub(crate) fn demand_site_entrance_reusing(
        &self,
        parcel_id: u64,
        asset_id: &str,
        zoning: &crate::simulation::zoning::ZoningSystem,
        graph: &RegionGraph,
        lanes: &LaneSystem,
        topology: u32,
        previous: Option<(DemandSiteEntranceKey, &BuildingEntrance)>,
    ) -> Option<(DemandSiteEntranceKey, BuildingEntrance)> {
        let parcel = zoning.parcel_by_raw_id(parcel_id)?;
        let asset = self.registry.get(asset_id)?;
        let authored = asset.manifest.building.as_ref()?;
        let (center, facing) = if let Some(index) = parcel.occupied_building() {
            let building = self.buildings.get(index)?;
            (
                Vector2::new(building.center_x, building.center_y),
                building.facing_dir,
            )
        } else {
            let depth = authored.lot_depth_cells as f32 * zoning.config.zone_cell_m;
            (
                parcel.front_center() + parcel.normal() * (depth * 0.5),
                -parcel.normal(),
            )
        };
        let door = Self::site_door_position(center, facing, Some(asset));
        let key = DemandSiteEntranceKey {
            door: door.map(|door| [door.x.to_bits(), door.y.to_bits()]),
            edge: parcel.edge_idx(), side: parcel.side(), topology, lanes: lanes.revision(),
        };
        let entrance = match previous {
            Some((old, entrance)) if old == key => entrance.clone(),
            _ => Self::derive_door_entrance(door, key.edge, key.side, graph, lanes),
        };
        Some((key, entrance))
    }

    fn derive_site_entrance(
        center: Vector2,
        facing: Vector2,
        edge_idx: usize,
        side: i8,
        asset_entry: Option<&crate::assets::registry::AssetEntry>,
        graph: &RegionGraph,
        lanes: &LaneSystem,
    ) -> BuildingEntrance {
        Self::derive_door_entrance(Self::site_door_position(center, facing, asset_entry),
            edge_idx, side, graph, lanes)
    }

    fn site_door_position(center: Vector2, facing: Vector2,
        asset_entry: Option<&crate::assets::registry::AssetEntry>) -> Option<Vector2> {
        let asset = asset_entry?;
        let anchor = main_entrance_anchor(asset.manifest.anchors.as_slice())?;
        let (basis_x, basis_z) = building_local_xz_basis(facing, asset.manifest.building_frontage_forward());
        Some(center + basis_x * anchor.position[0] + basis_z * anchor.position[2])
    }

    fn derive_door_entrance(door: Option<Vector2>, edge_idx: usize, side: i8,
        graph: &RegionGraph, lanes: &LaneSystem) -> BuildingEntrance {
        let mut entrance = BuildingEntrance {
            edge_idx,
            side,
            ..BuildingEntrance::default()
        };
        let Some(door) = door else { return entrance; };
        entrance.door_pos = door;
        entrance.curb_pos = entrance.door_pos;
        let Some(edge) = graph.get_edge(edge_idx) else {
            return entrance;
        };
        entrance.vehicle_frontage_access = edge.vehicle_frontage_access;
        if edge.deleted || edge.physical_geometry.len() < 2 || edge.physical_length <= 1e-6 {
            return entrance;
        }
        entrance.entrance_s_m =
            Self::project_point_to_polyline_s(&edge.physical_geometry, entrance.door_pos);
        let entrance_t = entrance.entrance_s_m / edge.physical_length;
        let edge_pos = Self::sample_pos_on_edge(graph, edge_idx, entrance_t);
        derive_foot_lanes(&mut entrance, edge, side, lanes);
        derive_car_lanes(&mut entrance, edge, side, lanes);
        derive_flags(&mut entrance);
        derive_curb_pos(&mut entrance, edge_pos, lanes);
        entrance
    }

    /// Uncached geometry oracle for the prepared freight quote regressions.
    #[cfg(test)]
    pub(crate) fn freight_car_eta_between_buildings(
        &self,
        source_idx: usize,
        destination_idx: usize,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        network_time_s: impl FnMut(u32, u32) -> Option<f32>,
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
        Self::freight_car_eta_between_entrances(
            origin_entrance, destination_entrance, transit_network, graph, network_time_s,
        )
    }

    /// Quotes directed freight between real or proposed doors using the dispatch geometry.
    /// Node costs are caller-cached; no temporary building or index mutation is required.
    #[cfg(test)]
    pub(crate) fn freight_car_eta_between_entrances(
        origin_entrance: &BuildingEntrance,
        destination_entrance: &BuildingEntrance,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        mut network_time_s: impl FnMut(u32, u32) -> Option<f32>,
    ) -> Option<f32> {
        FreightEntranceAccess::new(origin_entrance, transit_network, graph).eta_to(
            &FreightEntranceAccess::new(destination_entrance, transit_network, graph),
            transit_network, graph, &mut network_time_s,
        )
    }

    /// Freight car ETA from `border_node` to a building, with the network leg read from `costs`,
    /// the car costs from `border_node`: O(1) per call instead of a route query.
    pub(crate) fn freight_car_eta_from_border_costs(
        &self,
        border_node: u32,
        destination_idx: usize,
        costs: &CchCostsFrom,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<f32> {
        Self::freight_car_eta_to_entrance(
            border_node,
            self.entrances.get(destination_idx)?,
            costs,
            transit_network,
            graph,
        )
    }

    /// Quotes an existing or proposed entrance from a prepared border sweep in O(1).
    pub(crate) fn freight_car_eta_to_entrance(
        border_node: u32,
        destination_entrance: &BuildingEntrance,
        costs: &CchCostsFrom,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<f32> {
        let edge = graph.get_edge(destination_entrance.edge_idx)?;
        if edge.deleted || !entrance_has_car_access(destination_entrance) {
            return None;
        }
        let network_time_s = |node| costs.cost_to(node);
        let mut best: Option<FreightBorderCandidate> = None;
        for destination_rank in [0_u8, 1_u8] {
            let Some(candidate) = freight_candidate_from_border(
                border_node,
                destination_rank,
                destination_entrance,
                transit_network,
                graph,
                &network_time_s,
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

fn freight_candidate_between_access(
    origin_rank: u8,
    destination_rank: u8,
    origin: &FreightEntranceAccess,
    destination: &FreightEntranceAccess,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    network_time_s: &mut impl FnMut(u32, u32) -> Option<f32>,
) -> Option<FreightCarCandidate> {
    let from = origin.departure[usize::from(origin_rank)]?;
    let to = destination.arrival[usize::from(destination_rank)]?;
    let planned_attach_node = from.node;
    let planned_detach_node = to.node;
    let planned_attach_lane_id = from.lane_id;
    let planned_detach_lane_id = to.lane_id;
    let planned_attach_lane_d = from.lane_d;
    let planned_detach_lane_d = to.lane_d;
    let egress_local_time_s = from.local_time_s;
    let ingress_local_time_s = to.local_time_s;
    let same_lane_direct_frontage = origin.edge_idx == destination.edge_idx
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
            network_time_s(planned_attach_node, planned_detach_node)?
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
    network_time_s: &impl Fn(u32) -> Option<f32>,
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
        let Some(travel_seconds) = network_time_s(planned_detach_node) else {
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
