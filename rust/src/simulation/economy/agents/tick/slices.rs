//! Unsafe SoA slice wrappers used by the parallel agent movement pass.

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
    pub(super) pos_x: RawSlice<f32>,
    pub(super) pos_y: RawSlice<f32>,
    pub(super) activity: RawSlice<u8>,
    pub(super) transit: RawSlice<u8>,
    pub(super) happiness: RawSlice<f32>,
    pub(super) jstart: RawSlice<f32>,
    pub(super) schedule_seed: RawSlice<u32>,
    pub(super) cached_commute_minutes: RawSlice<u16>,
    pub(super) next_commute_refresh_time: RawSlice<f32>,
    pub(super) cur_b: RawSlice<usize>,
    pub(super) tgt_b: RawSlice<usize>,
    pub(super) planned_tgt_b: RawSlice<usize>,
    pub(super) cur_n: RawSlice<u32>,
    pub(super) planned_attach_n: RawSlice<u32>,
    pub(super) planned_detach_n: RawSlice<u32>,
    pub(super) planned_attach_lane: RawSlice<u32>,
    pub(super) planned_detach_lane: RawSlice<u32>,
    pub(super) planned_attach_lane_d: RawSlice<f32>,
    pub(super) planned_detach_lane_d: RawSlice<f32>,
    pub(super) access_flags: RawSlice<u8>,
    pub(super) next_replan_time: RawSlice<f32>,
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
