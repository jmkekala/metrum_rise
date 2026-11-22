// SPDX-License-Identifier: GPL-3.0-or-later

//! Each walker's own gait, derived from its saved `schedule_seed`: a walking pace, a side of the
//! sidewalk, a body size and stride, the beat of its steps, and a look for its clothes and hair.
//! All are pure functions of the seed, so they cost no agent storage, survive a save and load
//! unchanged, and keep the simulation deterministic.
//!
//! The pace is simulation state: a walker on a foot lane or an access path moves at the mean
//! walking speed times its pace ([`walker_speed`]). Trip planning keeps the mean. The side
//! offset, size, stride and step beat are presentation only: the render snapshot draws a walker
//! beside its lane's centre line at its size, its legs keep its stride, and the simulation never
//! reads them.

use super::determinism::stable_unit_f32;
use crate::config::AGENT_WALK_SPEED_MS;

// Salts keep the gait independent of the other values drawn from the same seed.
const PACE_SALT: u64 = 0x5741_4C4B_5041_4345;
const PACE_SALT_2: u64 = 0x5741_4C4B_5041_4346;
const SIDE_SALT: u64 = 0x5741_4C4B_5349_4445;
const LOOK_SALT: u64 = 0x5741_4C4B_4C4F_4F4B;
const SIZE_SALT: u64 = 0x5741_4C4B_5349_5A45;
const SIZE_SALT_2: u64 = 0x5741_4C4B_5349_5A46;
const BEAT_SALT: u64 = 0x5741_4C4B_4245_4154;

/// The widest body size difference from the model's, as a fraction: sizes span
/// `1 ± SIZE_SPREAD`.
pub(crate) const SIZE_SPREAD: f32 = 0.05;

/// The widest pace difference from the mean, as a fraction: paces span `1 ± PACE_SPREAD`.
pub(crate) const PACE_SPREAD: f32 = 0.18;

/// How far a walker may be drawn from its foot lane's centre line, metres. Within half the
/// narrowest sidewalk less a body's half width.
pub(crate) const SIDE_OFFSET_MAX_M: f32 = 0.5;

/// The walker's pace in `[1 - PACE_SPREAD, 1 + PACE_SPREAD]`, mean 1: the sum of two uniform
/// draws, so most walkers keep near the mean and few are very slow or very fast.
#[inline(always)]
pub(crate) fn walker_pace(schedule_seed: u32) -> f32 {
    let seed = u64::from(schedule_seed);
    let a = stable_unit_f32(seed ^ PACE_SALT);
    let b = stable_unit_f32(seed ^ PACE_SALT_2);
    1.0 + PACE_SPREAD * (a + b - 1.0)
}

/// The walker's own speed, m/s: the mean walking speed times its pace. What it walks on a foot
/// lane or crossing, before a slower walker ahead holds it (`walker_crowd`), and off the lanes:
/// on a building's access path, from a car to a door, across the ground to a sidewalk. The
/// movement step and the lane claims both take it from here, so a walker never steps further
/// than its claim allowed.
#[inline(always)]
pub(crate) fn walker_speed(schedule_seed: u32) -> f32 {
    AGENT_WALK_SPEED_MS * walker_pace(schedule_seed)
}

/// The walker's drawn distance from its foot lane's centre line, metres, in
/// `[-SIDE_OFFSET_MAX_M, SIDE_OFFSET_MAX_M]`, to the left of its travel when positive.
#[inline(always)]
pub(crate) fn walker_side_offset_m(schedule_seed: u32) -> f32 {
    SIDE_OFFSET_MAX_M * (2.0 * stable_unit_f32(u64::from(schedule_seed) ^ SIDE_SALT) - 1.0)
}

/// The walker's look in `[0, 1)`: the walker shader picks its clothes and hair colours from it.
/// Presentation only.
#[inline(always)]
pub(crate) fn walker_look(schedule_seed: u32) -> f32 {
    stable_unit_f32(u64::from(schedule_seed) ^ LOOK_SALT)
}

/// The walker's drawn size in `[1 - SIZE_SPREAD, 1 + SIZE_SPREAD]`, mean 1, most near the mean.
/// Its stride scales with it, so a taller walker takes fewer steps per metre. Presentation only.
#[inline(always)]
pub(crate) fn walker_size(schedule_seed: u32) -> f32 {
    let seed = u64::from(schedule_seed);
    let a = stable_unit_f32(seed ^ SIZE_SALT);
    let b = stable_unit_f32(seed ^ SIZE_SALT_2);
    1.0 + SIZE_SPREAD * (a + b - 1.0)
}

/// Where in its walk cycle the walker's steps fall, `[0, 1)`: added to its walk phase when drawn,
/// so walkers set off together do not step in time. Presentation only.
#[inline(always)]
pub(crate) fn walker_step_beat(schedule_seed: u32) -> f32 {
    stable_unit_f32(u64::from(schedule_seed) ^ BEAT_SALT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaits_stay_in_range_and_average_to_the_shared_pace() {
        let n = 20_000_u32;
        let (mut sum, mut lo, mut hi) = (0.0_f64, f32::MAX, f32::MIN);
        let (mut left, mut side_hi) = (0_u32, 0.0_f32);
        for seed in 0..n {
            let seed = seed.wrapping_mul(2_654_435_761);
            let pace = walker_pace(seed);
            sum += f64::from(pace);
            lo = lo.min(pace);
            hi = hi.max(pace);
            let size = walker_size(seed);
            assert!((1.0 - SIZE_SPREAD..=1.0 + SIZE_SPREAD).contains(&size));
            assert!((0.0..1.0).contains(&walker_step_beat(seed)));
            let side = walker_side_offset_m(seed);
            left += u32::from(side > 0.0);
            side_hi = side_hi.max(side.abs());
            assert_eq!(
                pace.to_bits(),
                walker_pace(seed).to_bits(),
                "pure in the seed"
            );
        }
        let mean = sum / f64::from(n);
        assert!((mean - 1.0).abs() < 0.005, "mean pace {mean}");
        assert!(
            lo >= 1.0 - PACE_SPREAD && hi <= 1.0 + PACE_SPREAD,
            "pace {lo}..{hi}"
        );
        assert!(hi - lo > PACE_SPREAD, "paces spread: {lo}..{hi}");
        assert!(side_hi <= SIDE_OFFSET_MAX_M && side_hi > 0.9 * SIDE_OFFSET_MAX_M);
        assert!(
            (9_000..11_000).contains(&left),
            "both sides used: {left} of {n} left"
        );
    }
}
