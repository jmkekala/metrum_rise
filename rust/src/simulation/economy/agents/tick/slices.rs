// SPDX-License-Identifier: GPL-2.0-only

//! Unsafe SoA slice wrappers used by the parallel agent movement pass.

use crate::simulation::economy::agents::data::AgentVec;

// Safety invariant upheld by callers:
// Rayon's `(0..n).into_par_iter()` visits each index `i` exactly once. All
// mutable field accesses below index into disjoint SoA slots, so the raw
// pointers do not alias for a given field during one parallel pass. The wrapper
// is never stored beyond the lifetime of the tick scope.
/// Raw pointer view over one mutable SoA column during a parallel tick pass.
pub(super) struct RawSlice<T> {
    ptr: *mut T,
    len: usize,
}

unsafe impl<T: Send> Send for RawSlice<T> {}
unsafe impl<T: Send> Sync for RawSlice<T> {}

impl<T> RawSlice<T> {
    /// Creates a raw view over a vector that is borrowed by the surrounding tick scope.
    pub(super) fn new(v: &mut Vec<T>) -> Self {
        Self {
            ptr: v.as_mut_ptr(),
            len: v.len(),
        }
    }

    #[inline(always)]
    /// Returns an immutable reference to one index after caller-proven bounds and alias checks.
    pub(super) unsafe fn get(&self, i: usize) -> &T {
        debug_assert!(i < self.len);
        unsafe { &*self.ptr.add(i) }
    }

    #[inline(always)]
    /// Returns a mutable reference to one index that must be unique to the current worker.
    pub(super) unsafe fn get_mut(&self, i: usize) -> &mut T {
        debug_assert!(i < self.len);
        unsafe { &mut *self.ptr.add(i) }
    }
}

/// Disjoint SoA slices used by `process_agent_movement` for parallel data access.
pub(crate) struct MovementSlices {
    pub(super) home: RawSlice<usize>,
    pub(super) work: RawSlice<usize>,
    pub(super) age_group: RawSlice<u8>,
    pub(super) pos_x: RawSlice<f32>,
    pub(super) pos_y: RawSlice<f32>,
    pub(super) activity: RawSlice<u8>,
    pub(super) transit: RawSlice<u8>,
    pub(super) happiness: RawSlice<f32>,
    pub(super) jstart: RawSlice<f32>,
    pub(super) schedule_seed: RawSlice<u32>,
    pub(super) cached_commute_minutes: RawSlice<u16>,
    pub(super) next_commute_refresh_time: RawSlice<f32>,
    pub(super) next_departure_day: RawSlice<u32>,
    pub(super) next_departure_minute: RawSlice<u16>,
    pub(super) next_departure_origin: RawSlice<usize>,
    pub(super) next_departure_target: RawSlice<usize>,
    pub(super) next_departure_activity: RawSlice<u8>,
    pub(super) cached_schedule_work_building: RawSlice<usize>,
    pub(super) cached_work_profile_index: RawSlice<u16>,
    pub(super) pending_household_size: RawSlice<u16>,
    pub(super) freight_shipment_id: RawSlice<u64>,
    pub(super) cur_b: RawSlice<usize>,
    pub(super) tgt_b: RawSlice<usize>,
    pub(super) planned_tgt_b: RawSlice<usize>,
    pub(super) freight_target_border_node: RawSlice<u32>,
    pub(super) cur_n: RawSlice<u32>,
    pub(super) planned_attach_n: RawSlice<u32>,
    pub(super) planned_detach_n: RawSlice<u32>,
    pub(super) planned_attach_lane: RawSlice<u32>,
    pub(super) planned_detach_lane: RawSlice<u32>,
    pub(super) planned_attach_lane_d: RawSlice<f32>,
    pub(super) planned_detach_lane_d: RawSlice<f32>,
    pub(super) access_flags: RawSlice<u8>,
    pub(super) next_replan_time: RawSlice<f32>,
    pub(super) network_replan_failures: RawSlice<u8>,
    pub(super) cur_e: RawSlice<usize>,
    pub(super) lane_id: RawSlice<usize>,
    pub(super) lane_d: RawSlice<f32>,
    pub(super) lane_change_from_lane: RawSlice<u32>,
    pub(super) lane_change_start_d: RawSlice<f32>,
    pub(super) lane_change_length: RawSlice<f32>,
    pub(super) overtake_blocked_time: RawSlice<f32>,
    pub(super) overtake_cooldown: RawSlice<f32>,
    pub(super) tmode: RawSlice<u8>,
    pub(super) planned_activity: RawSlice<u8>,
    pub(super) path: RawSlice<Vec<u32>>,
    pub(super) path_idx: RawSlice<usize>,
    pub(super) has_car: RawSlice<bool>,
    pub(super) speed: RawSlice<f32>,
    pub(super) walk_phase: RawSlice<f32>,
}

impl MovementSlices {
    /// Borrows the movement columns without allocating; callers must keep their storage
    /// fixed and alive until the parallel pass and its serial claim phase finish.
    #[inline(always)]
    pub(super) fn new(agents: &mut AgentVec) -> Self {
        Self {
            home: RawSlice::new(&mut agents.home_building),
            work: RawSlice::new(&mut agents.work_building),
            age_group: RawSlice::new(&mut agents.age_group),
            pos_x: RawSlice::new(&mut agents.pos_x),
            pos_y: RawSlice::new(&mut agents.pos_y),
            activity: RawSlice::new(&mut agents.activity),
            transit: RawSlice::new(&mut agents.transit),
            happiness: RawSlice::new(&mut agents.happiness),
            jstart: RawSlice::new(&mut agents.journey_start_time),
            schedule_seed: RawSlice::new(&mut agents.schedule_seed),
            cached_commute_minutes: RawSlice::new(&mut agents.cached_commute_minutes),
            next_commute_refresh_time: RawSlice::new(&mut agents.next_commute_refresh_time),
            next_departure_day: RawSlice::new(&mut agents.next_departure_day),
            next_departure_minute: RawSlice::new(&mut agents.next_departure_minute),
            next_departure_origin: RawSlice::new(&mut agents.next_departure_origin_building),
            next_departure_target: RawSlice::new(&mut agents.next_departure_target_building),
            next_departure_activity: RawSlice::new(&mut agents.next_departure_activity),
            cached_schedule_work_building: RawSlice::new(&mut agents.cached_schedule_work_building),
            cached_work_profile_index: RawSlice::new(&mut agents.cached_work_profile_index),
            pending_household_size: RawSlice::new(&mut agents.pending_household_size),
            freight_shipment_id: RawSlice::new(&mut agents.freight_shipment_id),
            cur_b: RawSlice::new(&mut agents.current_building),
            tgt_b: RawSlice::new(&mut agents.target_building),
            planned_tgt_b: RawSlice::new(&mut agents.planned_target_building),
            freight_target_border_node: RawSlice::new(&mut agents.freight_target_border_node),
            cur_n: RawSlice::new(&mut agents.current_node),
            planned_attach_n: RawSlice::new(&mut agents.planned_attach_node),
            planned_detach_n: RawSlice::new(&mut agents.planned_detach_node),
            planned_attach_lane: RawSlice::new(&mut agents.planned_attach_lane_id),
            planned_detach_lane: RawSlice::new(&mut agents.planned_detach_lane_id),
            planned_attach_lane_d: RawSlice::new(&mut agents.planned_attach_lane_d),
            planned_detach_lane_d: RawSlice::new(&mut agents.planned_detach_lane_d),
            access_flags: RawSlice::new(&mut agents.access_flags),
            next_replan_time: RawSlice::new(&mut agents.next_replan_time),
            network_replan_failures: RawSlice::new(&mut agents.network_replan_failures),
            cur_e: RawSlice::new(&mut agents.current_edge),
            lane_id: RawSlice::new(&mut agents.current_lane_id),
            lane_d: RawSlice::new(&mut agents.lane_distance),
            lane_change_from_lane: RawSlice::new(&mut agents.lane_change_from_lane_id),
            lane_change_start_d: RawSlice::new(&mut agents.lane_change_start_d),
            lane_change_length: RawSlice::new(&mut agents.lane_change_length_m),
            overtake_blocked_time: RawSlice::new(&mut agents.overtake_blocked_time_s),
            overtake_cooldown: RawSlice::new(&mut agents.overtake_cooldown_s),
            tmode: RawSlice::new(&mut agents.transit_mode),
            planned_activity: RawSlice::new(&mut agents.planned_activity),
            path: RawSlice::new(&mut agents.current_path),
            path_idx: RawSlice::new(&mut agents.current_path_index),
            has_car: RawSlice::new(&mut agents.has_car),
            speed: RawSlice::new(&mut agents.speed),
            walk_phase: RawSlice::new(&mut agents.walk_phase),
        }
    }
}
