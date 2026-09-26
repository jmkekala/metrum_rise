// SPDX-License-Identifier: GPL-2.0-only

//! Stateless cosmetic window schedules derived from saved placement identity.

// No simulation RNG or allocator ordinal: removal, reload and LOD selection cannot change
// a neighbour's lights. SplitMix's finalizer separates this domain from colour selection.
fn mix(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

/// O(1) schedule for GPU custom data; independent of clock, LOD and simulation RNG.
pub(super) fn parameters(key: u64, residential: bool, dark: bool) -> [f32; 4] {
    let bits = mix(key ^ 0x77696e646f777321);
    let unit = |shift| ((bits >> shift) & 0xffff_u64) as f32 / 65535.0;
    // Profile 0 is dark, 1 sleeps, 2 stays on while dark. Eight percent of homes stay up.
    let profile = if dark {
        0.0
    } else if !residential || unit(48) < 0.08 {
        2.0
    } else {
        1.0
    };
    [
        1.0 + 7.0 * unit(0),
        22.0 + 4.5 * unit(16),
        5.0 + 2.5 * unit(32),
        profile,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_are_bounded_varied_and_independent_of_activity() {
        let mut overnight = 0;
        for key in 0..10_000 {
            let home = parameters(key, true, false);
            assert_eq!(home, parameters(key, true, false));
            assert!((1.0..=8.0).contains(&home[0]));
            assert!((22.0..=26.5).contains(&home[1]));
            assert!((5.0..=7.5).contains(&home[2]));
            overnight += usize::from(home[3] == 2.0);
            let dark = parameters(key, true, true);
            assert_eq!(&dark[..3], &home[..3]);
            assert_eq!(dark[3], 0.0);
            assert_eq!(parameters(key, false, false)[3], 2.0);
        }
        assert!((650..950).contains(&overnight));
        assert_ne!(parameters(1, true, false), parameters(2, true, false));
    }
}
