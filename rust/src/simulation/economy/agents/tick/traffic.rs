// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: traffic.rs
//  script_path: rust/src/simulation/economy/agents/tick/traffic.rs
//  module_name: traffic
//  version: 0.1.0
//  description: Traffic movement helpers for car following, junctions, and
//           lane changes.
//  kind: module
//  spec: none
//  internal_dependencies: []
//  external_dependencies: []
//  features: []
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-27
// ========================================================================

//! Traffic movement helpers for car following, junctions, and lane changes.

// ========================================================================
// SUBMODULES
// ========================================================================

pub mod emergency;
mod idm;
mod junction;
mod lane_change;
mod occupancy;
pub mod report;

// ========================================================================
// RE-EXPORTS
// ========================================================================

pub(super) use idm::{braking_speed_for_distance, idm_new_speed, limit_speed_change};
pub(super) use junction::{connector_turn_speed, junction_car_speed, junction_entry_speed};
#[cfg(test)]
pub(super) use lane_change::lane_change_target_toward;
pub(super) use lane_change::{
    LANE_CHANGE_FINISH_EPS_M, LANE_CHANGE_MIN_LENGTH_M, OVERTAKE_COOLDOWN_S,
    OVERTAKE_DETACH_BUFFER_M, OVERTAKE_EDGE_BUFFER_M, OVERTAKE_MIN_GAP_GAIN_M,
    OVERTAKE_MIN_SPEED_GAIN_MS, OVERTAKE_RETURN_TARGET_GAP_M, OVERTAKE_STUCK_TIME_S,
    OVERTAKE_TARGET_AHEAD_GAP_M, cruise_lane_return_target, lane_change_length_for_speed,
    overtake_follow_gap, overtaking_lane_target, planned_detach_distance_on_current_edge,
    planned_lane_change_target, turn_lane_target,
};
pub(super) use occupancy::{
    ConnectorEntry, claim_connector_entry, claim_lane_entry, conflicting_movements_clear,
    deterministic_choice_index, idm_gap_bucket, lane_attach_slot_clear, lane_change_gap_clear,
    lane_entry_slot_clear, live_lane_bucket_transit,
};

// ========================================================================
// TESTS
// ========================================================================

#[cfg(test)]
mod tests {
    use super::super::claims::LaneClaimContext;
    use super::{
        ConnectorEntry, claim_connector_entry, conflicting_movements_clear, connector_turn_speed,
        cruise_lane_return_target, junction_car_speed, junction_entry_speed, lane_change_gap_clear,
        lane_change_length_for_speed, lane_change_target_toward, limit_speed_change,
        overtaking_lane_target,
    };
    use crate::config::{CAR_JUNCTION_SPEED_MS, IDM_B};
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::lanes::{Lane, LaneType};
    use godot::prelude::Vector3;
    use std::sync::atomic::{AtomicBool, Ordering};

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
        let lane_claims = [AtomicBool::new(false)];
        let serial_agents = [true];
        let claim_context = LaneClaimContext::new(&lane_claims, &serial_agents);
        let mut candidates = vec![0];
        assert_eq!(
            claim_connector_entry(0, &mut candidates, true, 0, &lane_buckets, &claim_context, None, |_| 0.0),
            ConnectorEntry::Enter(0)
        );
        assert!(lane_claims[0].load(Ordering::Acquire));

        let mut candidates = vec![0];
        assert_eq!(
            claim_connector_entry(0, &mut candidates, true, 0, &lane_buckets, &claim_context, None, |_| 0.0),
            ConnectorEntry::ClaimedThisTick
        );

        let occupied_buckets = vec![vec![(0.0, 1)]];
        let lane_claims = [AtomicBool::new(false)];
        let claim_context = LaneClaimContext::new(&lane_claims, &serial_agents);
        let mut candidates = vec![0];
        assert_eq!(
            claim_connector_entry(
                0,
                &mut candidates,
                true,
                0,
                &occupied_buckets,
                &claim_context,
                None,
                |_| 0.0,
            ),
            ConnectorEntry::Occupied
        );

        let mut candidates = Vec::new();
        assert_eq!(
            claim_connector_entry(0, &mut candidates, false, 0, &lane_buckets, &claim_context, None, |_| 0.0),
            ConnectorEntry::MissingConnection
        );
    }

    #[test]
    fn a_car_may_follow_a_moving_queue_into_a_junction() {
        // The rule the old test got wrong. A car rolling just past the exit
        // mouth is clearing it, so following it in is correct: demanding a
        // larger gap here is what deadlocked every junction, because ordinary
        // moving traffic keeps the mouth occupied.
        let mut network = TransitNetwork::new();
        // Connector lane 0 leads onto road lane 1.
        network.lane_system.lanes.push(Lane {
            edge_id: usize::MAX,
            node_id: 0,
            lane_type: LaneType::Vehicle,
            next_lanes: vec![1],
            ..Default::default()
        });
        network.lane_system.lanes.push(Lane {
            edge_id: 7,
            lane_type: LaneType::Vehicle,
            ..Default::default()
        });

        // A car half a length into the exit lane, still rolling.
        let buckets = vec![Vec::new(), vec![(1.0, 5)]];
        assert!(
            conflicting_movements_clear(0, 0, &network.lane_system, &buckets, |_| 6.0),
            "a moving queue can be followed into"
        );
    }

    #[test]
    fn a_car_may_not_enter_behind_a_stopped_queue() {
        // The gridlock case. The car ahead has stopped inside the exit mouth,
        // so entering leaves this car parked in the box blocking every crossing
        // movement, which is what strands a responder behind traffic.
        let mut network = TransitNetwork::new();
        network.lane_system.lanes.push(Lane {
            edge_id: usize::MAX,
            node_id: 0,
            lane_type: LaneType::Vehicle,
            next_lanes: vec![1],
            ..Default::default()
        });
        network.lane_system.lanes.push(Lane {
            edge_id: 7,
            lane_type: LaneType::Vehicle,
            ..Default::default()
        });

        let buckets = vec![Vec::new(), vec![(1.0, 5)]];
        assert!(
            !conflicting_movements_clear(0, 0, &network.lane_system, &buckets, |_| 0.0),
            "a stopped queue is a wall"
        );
    }

    #[test]
    fn a_crawling_queue_still_counts_as_moving() {
        // A car easing forward reads as a small positive speed and is still
        // clearing the mouth. Treating only exact zero as stopped would let a
        // crawling queue block a junction it is in fact emptying.
        let mut network = TransitNetwork::new();
        network.lane_system.lanes.push(Lane {
            edge_id: usize::MAX,
            node_id: 0,
            lane_type: LaneType::Vehicle,
            next_lanes: vec![1],
            ..Default::default()
        });
        network.lane_system.lanes.push(Lane {
            edge_id: 7,
            lane_type: LaneType::Vehicle,
            ..Default::default()
        });

        let buckets = vec![Vec::new(), vec![(1.0, 5)]];
        assert!(conflicting_movements_clear(
            0,
            0,
            &network.lane_system,
            &buckets,
            |_| 1.0
        ));
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
}
