// SPDX-License-Identifier: GPL-3.0-or-later

//! Each walker's own gait, derived from its saved `schedule_seed`: a walking pace, a side of the
//! sidewalk, a body size and stride, the beat of its steps, and a look for its clothes and hair.
//! All are pure functions of the seed, so they cost no agent storage, survive a save and load
//! unchanged, and keep the simulation deterministic.
//!
//! The pace is simulation state: a walker on a foot lane or an access path moves at the mean
//! walking speed times its pace ([`walker_speed`]). Trip planning keeps the mean. The side
//! offset, size, stride and step beat are presentation only: the render snapshot draws a walker
//! beside its lane's centre line at its size (a child's at a child's scale, from its age group),
//! its legs keep its stride, and the simulation never reads them.

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

/// The widest adult body size difference from the model's, as a fraction: adults span
/// `1 ± SIZE_SPREAD`, 1.60 to 2.00 m of the 1.8 m model. The agents' shadow casters keep a
/// margin for a 2 m agent (`agent_frame::TWIN_MARGIN_M`), so this stays at most `1 / 9`.
pub(crate) const SIZE_SPREAD: f32 = 0.11;

/// A child's drawn size range as a fraction of the model's: 1.08 to 1.35 m of the 1.8 m model.
pub(crate) const CHILD_SIZE: [f32; 2] = [0.6, 0.75];

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

/// Whether a walker is drawn at a child's scale: a child of a household. The agents that carry
/// `AGE_CHILD` only to stay out of employment (a stored car's delivery carrier, a drop-off's
/// driver, a freight carrier) belong to no household and are drawn as adults should one ever
/// walk. Presentation only.
#[inline(always)]
pub(crate) fn walker_is_child(age_group: u8, household_id: usize) -> bool {
    age_group == super::AGE_CHILD && household_id != usize::MAX
}

/// The walker's drawn size as a fraction of the model's: an adult's (or an elder's) in
/// `[1 - SIZE_SPREAD, 1 + SIZE_SPREAD]`, a child's in `CHILD_SIZE`, both from the same seed draw.
/// The draw is a wide and a narrow uniform draw summed (three parts to one), so sizes spread
/// evenly over the middle half of the range (two thirds of walkers) and thin out linearly
/// towards its ends. Its stride scales with it, so a taller walker takes fewer steps
/// per metre and a child more. Presentation only.
#[inline(always)]
pub(crate) fn walker_size(schedule_seed: u32, child: bool) -> f32 {
    let seed = u64::from(schedule_seed);
    let a = stable_unit_f32(seed ^ SIZE_SALT);
    let b = stable_unit_f32(seed ^ SIZE_SALT_2);
    // In [-1, 1], mean 0: flat over [-0.5, 0.5].
    let spread = 1.5 * a + 0.5 * b - 1.0;
    if child {
        let [low, high] = CHILD_SIZE;
        (low + high) * 0.5 + (high - low) * 0.5 * spread
    } else {
        1.0 + SIZE_SPREAD * spread
    }
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
            let size = walker_size(seed, false);
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

    // `RENDER-72`: adults use the whole 1.6-2.0 m range, not a narrow band about the mean; a
    // child is drawn at 0.6-0.75 of the model; both are pure functions of the saved seed.
    #[test]
    fn sizes_spread_over_their_ranges_from_the_seed_alone() {
        let n = 20_000_u32;
        let [child_low, child_high] = CHILD_SIZE;
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        let (mut child_lo, mut child_hi) = (f32::MAX, f32::MIN);
        // Adults by fifths of the range, from the shortest.
        let mut fifths = [0_u32; 5];
        let mut sum = 0.0_f64;
        for seed in 0..n {
            let seed = seed.wrapping_mul(2_654_435_761);
            let size = walker_size(seed, false);
            let child = walker_size(seed, true);
            assert_eq!(size.to_bits(), walker_size(seed, false).to_bits(), "pure in the seed");
            assert_eq!(child.to_bits(), walker_size(seed, true).to_bits(), "pure in the seed");
            assert!((1.0 - SIZE_SPREAD..=1.0 + SIZE_SPREAD).contains(&size), "{size}");
            assert!((child_low..=child_high).contains(&child), "{child}");
            // The child's place in its range is the adult's in theirs.
            let t = (size - (1.0 - SIZE_SPREAD)) / (2.0 * SIZE_SPREAD);
            assert!((child - (child_low + (child_high - child_low) * t)).abs() < 1e-5);
            (lo, hi) = (lo.min(size), hi.max(size));
            (child_lo, child_hi) = (child_lo.min(child), child_hi.max(child));
            fifths[((t * 5.0) as usize).min(4)] += 1;
            sum += f64::from(size);
        }
        let mean = sum / f64::from(n);
        assert!((mean - 1.0).abs() < 0.002, "mean size {mean}");
        // The ends of the range are reached: 1.60 m and 2.00 m to within a centimetre.
        let (low, high) = (1.0 - SIZE_SPREAD, 1.0 + SIZE_SPREAD);
        assert!(lo < low + 0.006 && hi > high - 0.006, "{lo}..{hi}");
        assert!(child_lo < child_low + 0.005 && child_hi > child_high - 0.005);
        // Flat over the middle, thinning to the ends: the middle three fifths hold about 26 %
        // each, the outer fifths about 11 % each (the old sum of two equal draws: 8 % and 36 %
        // in the middle fifth).
        let share = |k: usize| f64::from(fifths[k]) / f64::from(n);
        for k in 1..4 {
            assert!((0.22..0.30).contains(&share(k)), "fifth {k}: {fifths:?}");
        }
        for k in [0, 4] {
            assert!((0.08..0.16).contains(&share(k)), "fifth {k}: {fifths:?}");
        }
        // The tallest fifth (1.92-2.00 m) is about as full as the shortest.
        assert!((share(0) - share(4)).abs() < 0.02, "{fifths:?}");
    }

    #[test]
    fn only_a_household_child_is_drawn_as_a_child() {
        use super::super::{AGE_ADULT, AGE_CHILD, AGE_ELDER};
        assert!(walker_is_child(AGE_CHILD, 7));
        // The delivery carrier, the drop-off driver and freight carry `AGE_CHILD` with no
        // household (`PARK-34`).
        assert!(!walker_is_child(AGE_CHILD, usize::MAX));
        assert!(!walker_is_child(AGE_ADULT, 7));
        assert!(!walker_is_child(AGE_ELDER, 7));
    }
}
