// SPDX-License-Identifier: GPL-2.0-only

//! IDM speed and braking helpers.

use crate::config::{IDM_A_MAX, IDM_B, IDM_S_MIN, IDM_T_HEAD};

/// Returns the next speed for one simplified IDM time step.
pub(in crate::simulation::economy::agents::tick) fn idm_new_speed(
    v: f32,
    v_max: f32,
    gap: f32,
    dt: f32,
) -> f32 {
    let free = (v / v_max.max(0.1)).powi(4);
    let acc = if gap < f32::MAX / 2.0 {
        let s_star = IDM_S_MIN + v * IDM_T_HEAD;
        IDM_A_MAX * (1.0 - free - (s_star / gap).powi(2))
    } else {
        IDM_A_MAX * (1.0 - free)
    };
    (v + acc * dt).clamp(0.0, v_max)
}

/// Limits a speed change by acceleration or comfortable braking.
#[inline(always)]
pub(in crate::simulation::economy::agents::tick) fn limit_speed_change(
    current: f32,
    target: f32,
    dt: f32,
) -> f32 {
    if target >= current {
        target.min(current + IDM_A_MAX * dt)
    } else {
        target.max(current - IDM_B * dt)
    }
}

/// Returns the highest speed that can brake to `target_speed` within `distance_m`.
#[inline(always)]
pub(in crate::simulation::economy::agents::tick) fn braking_speed_for_distance(
    target_speed: f32,
    distance_m: f32,
) -> f32 {
    (target_speed * target_speed + 2.0 * IDM_B * distance_m.max(0.0)).sqrt()
}
