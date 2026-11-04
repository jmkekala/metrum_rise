// SPDX-License-Identifier: GPL-2.0-only

//! Traffic movement helpers for car following, junctions, and lane changes.

mod idm;
mod junction;
mod lane_change;
mod occupancy;

pub(super) use idm::{braking_speed_for_distance, idm_new_speed, limit_speed_change};
pub(crate) use junction::connector_turn_speed;
pub(super) use junction::{junction_car_speed, junction_entry_speed};
pub(super) use lane_change::{
    LANE_CHANGE_FINISH_EPS_M, LANE_CHANGE_MIN_LENGTH_M, LaneChangeKind, LaneChangeParams,
    OVERTAKE_COOLDOWN_S, OVERTAKE_EDGE_BUFFER_M, OVERTAKE_MIN_SPEED_GAIN_MS, choose_lane_change,
    lane_change_end, lane_change_finished, lane_change_length_for_speed, lane_change_target_toward,
    overtake_follow_gap, planned_lane_change_target, turn_lane_change_length,
};
#[cfg(test)]
pub(super) use lane_change::{cruise_lane_return_target, overtaking_lane_target};
pub(in crate::simulation::economy::agents) use occupancy::lane_attach_slot_clear;
pub(super) use occupancy::{
    ConnectorEntry, claim_connector_entry, claim_lane_entry, idm_gap_bucket, lane_change_gap_clear,
    lane_entry_slot_clear, live_lane_bucket_transit,
};

#[cfg(test)]
mod tests {
    use super::super::claims::LaneClaimContext;
    use super::{
        ConnectorEntry, claim_connector_entry, connector_turn_speed, cruise_lane_return_target,
        junction_car_speed, junction_entry_speed, lane_change_gap_clear,
        lane_change_length_for_speed, lane_change_target_toward, limit_speed_change,
        overtaking_lane_target,
    };
    use crate::config::{CAR_JUNCTION_SPEED_MS, IDM_B};
    use crate::simulation::economy::agents::data::MovementClaimMode;
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::lanes::{Lane, LaneType};
    use godot::prelude::Vector3;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_junction_car_speed_caps_fast_turns() {
        assert_eq!(junction_car_speed(50.0), CAR_JUNCTION_SPEED_MS);
        assert_eq!(junction_car_speed(0.0), 0.0);
        assert_eq!(junction_car_speed(4.0), 4.0);
    }

    #[test]
    fn test_connector_turn_speed_slows_tight_turns() {
        let straight = Lane {
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
            length: 20.0,
            lane_type: LaneType::Vehicle,
            ..Default::default()
        };
        let tight_turn = Lane {
            geometry: vec![
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(3.0, 0.0, 0.0),
                Vector3::new(3.0, 0.0, 3.0),
            ],
            length: 6.0,
            lane_type: LaneType::Vehicle,
            ..Default::default()
        };

        assert_eq!(connector_turn_speed(&straight), CAR_JUNCTION_SPEED_MS);
        assert!(connector_turn_speed(&tight_turn) < CAR_JUNCTION_SPEED_MS);
        assert!(junction_entry_speed(50.0, &tight_turn) < CAR_JUNCTION_SPEED_MS);
    }

    #[test]
    fn test_limit_speed_change_uses_comfortable_braking() {
        let next = limit_speed_change(14.0, 0.0, 0.1);
        assert!((next - (14.0 - IDM_B * 0.1)).abs() < 1.0e-5);
    }

    #[test]
    fn test_claim_connector_entry_reports_entry_blockers() {
        let lane_buckets = vec![Vec::new()];
        let lane_claims = [AtomicUsize::new(usize::MAX)];
        let modes = [MovementClaimMode::Dynamic; 2];
        let claim_context = LaneClaimContext::new(&lane_claims, &modes);
        let mut candidates = vec![0];
        assert_eq!(
            claim_connector_entry(0, &mut candidates, true, 0, &lane_buckets, &claim_context),
            ConnectorEntry::Enter(0)
        );
        assert_eq!(lane_claims[0].load(Ordering::Relaxed), 0);
        assert!(claim_context.claim_lane(0, 0));

        let mut candidates = vec![0];
        assert_eq!(
            claim_connector_entry(1, &mut candidates, true, 0, &lane_buckets, &claim_context),
            ConnectorEntry::ClaimedThisTick
        );

        let occupied_buckets = vec![vec![(0.0, 1)]];
        let lane_claims = [AtomicUsize::new(usize::MAX)];
        let claim_context = LaneClaimContext::new(&lane_claims, &modes);
        let mut candidates = vec![0];
        assert_eq!(
            claim_connector_entry(
                0,
                &mut candidates,
                true,
                0,
                &occupied_buckets,
                &claim_context,
            ),
            ConnectorEntry::Occupied
        );

        let mut candidates = Vec::new();
        assert_eq!(
            claim_connector_entry(0, &mut candidates, false, 0, &lane_buckets, &claim_context,),
            ConnectorEntry::MissingConnection
        );
    }

    #[test]
    fn test_lane_change_length_scales_with_speed() {
        let slow = lane_change_length_for_speed(4.0);
        let urban = lane_change_length_for_speed(14.0);
        let fast = lane_change_length_for_speed(40.0);

        assert!(urban > slow);
        assert!(fast >= urban);
    }

    #[test]
    fn test_lane_change_gap_clear_requires_speed_scaled_space() {
        let bucket = vec![(17.0, 0)];

        assert!(lane_change_gap_clear(&bucket, 10.0, 4.0));
        assert!(!lane_change_gap_clear(&bucket, 10.0, 14.0));
    }

    #[test]
    fn test_lane_change_target_steps_one_lane_at_a_time() {
        let mut network = TransitNetwork::new();
        for lane_idx in 0..3 {
            network.lane_system.lanes.push(Lane {
                edge_id: 7,
                is_fwd: true,
                lane_idx,
                lane_type: LaneType::Vehicle,
                ..Default::default()
            });
        }
        network.lane_system.edge_lanes.insert(7, vec![0, 1, 2]);

        assert_eq!(lane_change_target_toward(0, 2, &network), Some(1));
        assert_eq!(lane_change_target_toward(1, 2, &network), Some(2));
    }

    #[test]
    fn test_overtaking_targets_center_and_returns_outward() {
        let mut network = TransitNetwork::new();
        for (is_fwd, lane_idx) in [(true, 0), (true, 1), (false, -1), (false, -2)] {
            network.lane_system.lanes.push(Lane {
                edge_id: 7,
                is_fwd,
                lane_idx,
                lane_type: LaneType::Vehicle,
                ..Default::default()
            });
        }
        network.lane_system.edge_lanes.insert(7, vec![0, 1, 2, 3]);

        assert_eq!(overtaking_lane_target(1, &network), Some(0));
        assert_eq!(overtaking_lane_target(0, &network), None);
        assert_eq!(cruise_lane_return_target(0, &network), Some(1));
        assert_eq!(overtaking_lane_target(3, &network), Some(2));
        assert_eq!(overtaking_lane_target(2, &network), None);
        assert_eq!(cruise_lane_return_target(2, &network), Some(3));
    }

    #[test]
    fn solid_approach_zone_blocks_turn_and_frontage_lane_changes() {
        use super::{LaneChangeParams, choose_lane_change};
        use crate::config::LANE_WIDTH;
        use crate::simulation::network::graph::RegionGraph;
        use crate::simulation::network::lanes::APPROACH_SOLID_LENGTH_M;
        use crate::simulation::network::roadbed::fixtures::{finalize, road_edge};
        use crate::simulation::network::types::{NodeType, TransitFlags};
        // A 2+2 T: every arm's forward lanes approach the centre, so they serve different
        // movements and are flagged.
        let mut graph = RegionGraph::new();
        let node = graph.add_node(Vector3::ZERO, NodeType::Junction);
        for p in [
            Vector3::new(-180.0, 0.0, 0.0),
            Vector3::new(180.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, 180.0),
        ] {
            let end = graph.add_node(p, NodeType::Junction);
            let mut edge = road_edge(end, node, vec![p, Vector3::ZERO], 4.0 * LANE_WIDTH);
            edge.allowed_types = TransitFlags::CAR;
            edge.fwd_lanes = 2;
            edge.bkw_lanes = 2;
            graph.add_edge(edge);
        }
        finalize(&mut graph);
        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut graph);
        let lane = |idx| {
            network.lane_system.edge_lanes[&0]
                .iter()
                .copied()
                .find(|&id| {
                    let lane = &network.lane_system.lanes[id];
                    lane.lane_type == LaneType::Vehicle && lane.is_fwd && lane.lane_idx == idx
                })
                .unwrap()
        };
        let (from, to) = (lane(0), lane(1));
        assert!(network.lane_system.needs_turn_preparation(from));
        let length = network.lane_system.lanes[from].length;
        let buckets = vec![Vec::new(); network.lane_system.lanes.len()];
        for frontage in [false, true] {
            let params = |d, turn_preparation| LaneChangeParams {
                lane_id: from,
                lane_d: d,
                speed: 4.0,
                detach_lane_id: to,
                detach_lane_d: length,
                has_access_plan: frontage,
                blocked_time: 0.0,
                cooldown: 0.0,
                turn_preparation,
            };
            let change = |d, flagged| {
                choose_lane_change(params(d, flagged), &network, &buckets, to, || None)
            };
            let early = change(0.0, true).unwrap();
            assert!(early.length <= length - APPROACH_SOLID_LENGTH_M);
            assert!(change(length - 19.0, true).is_none());
            // 35 m before the end an 18 m change fits before the lane end and its buffer, but not
            // before the zone and the buffer it extends.
            assert!(change(length - 35.0, false).is_some());
            assert!(change(length - 35.0, true).is_none());
        }
    }
}
