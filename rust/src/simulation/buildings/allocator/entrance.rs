//! Derived entrance/access cache built from building placement, asset anchors, and live lanes.

use crate::assets::AnchorType;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator, BuildingEntrance};
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

impl BuildingAllocator {
    pub(crate) fn rebuild_entrance_cache(&mut self, graph: &RegionGraph, lanes: &LaneSystem) {
        self.entrances.clear();
        self.entrances.reserve(self.buildings.len());
        for building in &self.buildings {
            self.entrances
                .push(self.derive_building_entrance(building, graph, lanes));
        }
        self.entrances_dirty = false;
    }

    fn derive_building_entrance(
        &self,
        building: &Building,
        graph: &RegionGraph,
        lanes: &LaneSystem,
    ) -> BuildingEntrance {
        let mut entrance = BuildingEntrance {
            edge_idx: building.edge_idx,
            side: building.side,
            ..BuildingEntrance::default()
        };

        let Some(asset_entry) = self.registry.get(&building.asset_id) else {
            return entrance;
        };
        let Some(anchor) = main_entrance_anchor(asset_entry.manifest.anchors.as_slice()) else {
            return entrance;
        };

        entrance.door_pos = world_door_pos(building, asset_entry, anchor.position);
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
}

fn main_entrance_anchor(anchors: &[crate::assets::Anchor]) -> Option<&crate::assets::Anchor> {
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
    entry: &crate::assets::AssetEntry,
    anchor_position: [f32; 3],
) -> Vector2 {
    let basis_z = if building.facing_dir.length_squared() > 1e-12 {
        building.facing_dir.normalized()
    } else {
        Vector2::new(0.0, 1.0)
    };
    let basis_x = Vector2::new(basis_z.y, -basis_z.x);
    let scale = entry
        .manifest
        .building
        .as_ref()
        .and_then(|building| building.preview_scale)
        .unwrap_or(crate::config::BUILDING_VISUAL_SCALE);
    let pivot = entry.manifest.pivot_offset.unwrap_or([0.0, 0.0, 0.0]);
    let local_x = (pivot[0] + anchor_position[0]) * scale;
    let local_z = (pivot[2] + anchor_position[2]) * scale;

    Vector2::new(building.center_x, building.center_y) + basis_x * local_x + basis_z * local_z
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
