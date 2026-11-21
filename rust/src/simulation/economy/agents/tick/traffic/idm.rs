// SPDX-License-Identifier: GPL-3.0-or-later

//! IDM speed and braking helpers.

use crate::config::{IDM_A_MAX, IDM_B, IDM_S_MIN, IDM_T_HEAD};

/// Returns the next speed for one simplified IDM time step.
pub(in crate::simulation::economy::agents::tick) fn idm_new_speed(
    v: f32,
    v_max: f32,
    gap: f32,
    dt: f32,
) -> f32 {
    (v + idm_acceleration(v, v_max, gap) * dt).clamp(0.0, v_max)
}

/// The simplified IDM's acceleration, m/s², at speed `v` under limit `v_max` with `gap` metres
/// to the car ahead (`f32::MAX` with none): the step [`idm_new_speed`] takes, before its clamp.
/// The vehicle renderer reads its sign for the brake lamps (`RENDER-66`).
#[inline(always)]
pub(crate) fn idm_acceleration(v: f32, v_max: f32, gap: f32) -> f32 {
    let free = (v / v_max.max(0.1)).powi(4);
    if gap < f32::MAX / 2.0 {
        let s_star = IDM_S_MIN + v * IDM_T_HEAD;
        IDM_A_MAX * (1.0 - free - (s_star / gap).powi(2))
    } else {
        IDM_A_MAX * (1.0 - free)
    }
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
