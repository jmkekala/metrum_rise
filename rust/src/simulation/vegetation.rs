// SPDX-License-Identifier: GPL-2.0-only

//! Deterministic vegetation generation parameters and the stand field they drive.
//!
//! The generator is the vegetation population. Saves store the few numbers below rather than
//! the stems themselves: Finnish stand density over the default world is around `10^10` trees,
//! so a stored population is not an option and a stored parameter set costs 16 bytes. Player
//! edits ride on top as a sparse authored delta in [`edits`]; see `VEG-01` in `docs/roadmap.md`.
//!
//! Nothing here may be used to shade the ground. The terrain shader once mirrored the stand
//! field in GLSL and coloured ground as forest whether or not a tree stood on it.

pub mod edits;

use crate::simulation::core::config::WorldConfig;

/// In-stand acceptance rate applied to one canopy grid cell.
///
/// Density is expressed to the player as stems per hectare and delivered by sizing the grid
/// cell, not by moving this rate, so the accepted fraction of candidates stays constant and
/// only the candidate spacing changes.
const CANOPY_STAND_ACCEPT: f32 = 0.78;

/// Acceptance rate for canopy candidates outside a stand, as lone trees in open country.
const CANOPY_OPEN_ACCEPT: f32 = 0.06;

/// In-stand acceptance rate for understory candidates that are not rocks.
const UNDERSTORY_STAND_ACCEPT: f32 = 0.62;

/// Acceptance rate for understory candidates outside a stand.
const UNDERSTORY_OPEN_ACCEPT: f32 = 0.05;

/// Acceptance rate for the one-in-eight understory candidates drawn as rocks.
///
/// Rocks are geology rather than forest, so they do not scale with stand density.
const ROCK_ACCEPT: f32 = 0.010;

/// Understory cell size as a fraction of the canopy cell size.
///
/// The two layers keep a fixed ratio so one density dial moves both, and the shipped 16 m
/// canopy grid keeps its 4 m understory grid exactly. The ratio is not the two layers' relative
/// cost: each layer's cost follows the area it draws across, and the canopy reaches 4500 m where
/// the understory reaches 420 m, a ratio of 115x in area. Quadrupling the understory measured
/// +1.89 ms of GPU frame time where quadrupling the canopy measured +19.8 ms, so the canopy is
/// what the density ceiling below is protecting.
const UNDERSTORY_CELL_RATIO: f32 = 0.25;

/// Smallest canopy grid cell the renderer is allowed to be asked for, in metres.
///
/// This is what caps density, and it is a renderer limit rather than a generator one: halving
/// the cell quadruples canopy instances and sixteens the understory. `RENDER-04` raises the
/// ceiling when distant canopy stops costing a full mesh per stem.
const MIN_CANOPY_CELL_M: f32 = 8.0;

/// Largest canopy grid cell accepted, in metres, so a positive density cannot vanish.
const MAX_CANOPY_CELL_M: f32 = 64.0;

/// Stand feature size in metres on any world large enough to hold it.
///
/// A stand is a real-world scale and does not grow with the map. Making it a fraction of the
/// world instead gives an 18 km world 2250 m features, which is only eight noise lattice cells
/// per axis: local coverage then swings from 0.000 to 1.000 across spawn points, so one player
/// starts in a meadow and another in a closed forest that is also the peak-cost case for the
/// near understory ring. At 500 m the same sweep holds 0.20 to 0.82 with a standard deviation
/// of 0.12, which is landscape variation rather than a coin flip.
const TARGET_FEATURE_M: f32 = 500.0;

/// Stand feature size as a fraction of the world's shorter side on worlds smaller than that.
///
/// Only a world too small for a real stand compresses one, so a 500 m sandbox gets 62.5 m
/// features and still has structure instead of falling wholly inside or outside one stand.
const SMALL_WORLD_FEATURE_FRACTION: f32 = 0.125;

/// Samples per axis used to turn a requested coverage fraction into a field threshold.
const COVERAGE_SAMPLES_PER_AXIS: usize = 192;

/// Authored vegetation generation parameters, chosen when a game starts and then persisted.
///
/// These are saved state and not a video setting. Once a tree can be planted and cleared it is
/// gameplay state, so every machine loading one save must generate the same forest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VegetationConfig {
    /// Whether the world generates any vegetation at all. A cleared world starts `false`.
    pub enabled: bool,
    /// Seed separating one world's stand layout from another's on identical terrain.
    pub seed: u32,
    /// Fraction of the world that falls inside a stand, from 0.0 to 1.0.
    pub coverage: f32,
    /// Canopy stems per hectare inside a stand.
    pub canopy_stems_per_ha: f32,
}

impl VegetationConfig {
    /// Coverage of the shipped generator.
    ///
    /// Measured, not chosen: the two-sine field this replaces put 0.5655 of the 18 km world
    /// inside a stand, so the default world keeps the density of forest it had.
    pub const DEFAULT_COVERAGE: f32 = 0.565;

    /// Canopy density of the shipped generator: a 16 m grid at 0.78 acceptance.
    ///
    /// Kept exact rather than rounded so the default world generates the measured population.
    pub const DEFAULT_CANOPY_STEMS_PER_HA: f32 = 30.468_75;

    /// Highest canopy density the renderer currently accepts, in stems per hectare.
    pub const MAX_CANOPY_STEMS_PER_HA: f32 =
        10_000.0 * CANOPY_STAND_ACCEPT / (MIN_CANOPY_CELL_M * MIN_CANOPY_CELL_M);

    /// Lowest non-empty canopy density, in stems per hectare.
    pub const MIN_CANOPY_STEMS_PER_HA: f32 =
        10_000.0 * CANOPY_STAND_ACCEPT / (MAX_CANOPY_CELL_M * MAX_CANOPY_CELL_M);

    /// Returns these parameters with out-of-range and non-finite values corrected.
    ///
    /// Authored values cross the Godot boundary from a menu, so this is where they become
    /// valid simulation state: a NaN coverage would otherwise reach the quantile sweep and
    /// select an arbitrary threshold. Validity is enforced here and not in GDScript so that
    /// no caller can start a world the generator cannot reproduce.
    pub fn sanitized(self) -> Self {
        Self {
            enabled: self.enabled,
            seed: self.seed,
            coverage: if self.coverage.is_finite() {
                self.coverage.clamp(0.0, 1.0)
            } else {
                Self::DEFAULT_COVERAGE
            },
            canopy_stems_per_ha: if self.canopy_stems_per_ha.is_finite() {
                self.canopy_stems_per_ha
                    .clamp(Self::MIN_CANOPY_STEMS_PER_HA, Self::MAX_CANOPY_STEMS_PER_HA)
            } else {
                Self::DEFAULT_CANOPY_STEMS_PER_HA
            },
        }
    }
}

impl Default for VegetationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            seed: 0,
            coverage: Self::DEFAULT_COVERAGE,
            canopy_stems_per_ha: Self::DEFAULT_CANOPY_STEMS_PER_HA,
        }
    }
}

/// A [`VegetationConfig`] resolved against one world into the numbers the scatter reads.
///
/// Resolution happens once per world load. Every field below is derived, so this is a cache and
/// never a second source of truth; rebuild it with [`VegetationGenerator::resolve`] whenever the
/// config or the world extent changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VegetationGenerator {
    /// The authored parameters this was resolved from.
    pub config: VegetationConfig,
    /// Canopy candidate grid spacing in metres.
    pub canopy_cell_m: f32,
    /// Understory candidate grid spacing in metres.
    pub understory_cell_m: f32,
    // Stand field wavelength in metres, scaled to the world so a small world still has stands.
    feature_m: f32,
    // Field value at or above which a position is inside a stand. Derived from coverage by
    // sampling the field across the world, so coverage means an area fraction on every world.
    threshold: f32,
}

impl VegetationGenerator {
    /// Resolves authored parameters against a world extent.
    ///
    /// Costs one `COVERAGE_SAMPLES_PER_AXIS` squared field sweep and a sort of the result, which
    /// is why it belongs at world load and not on any per-patch path.
    pub fn resolve(config: VegetationConfig, world: &WorldConfig) -> Self {
        let shorter_side = world.width_m.min(world.height_m).max(1.0);
        let feature_m = TARGET_FEATURE_M
            .min(shorter_side * SMALL_WORLD_FEATURE_FRACTION)
            .max(f32::MIN_POSITIVE);
        let canopy_cell_m = canopy_cell_m_for_density(config.canopy_stems_per_ha);
        let threshold = coverage_threshold(config.coverage, config.seed, feature_m, world);
        Self {
            config,
            canopy_cell_m,
            understory_cell_m: canopy_cell_m * UNDERSTORY_CELL_RATIO,
            feature_m,
            threshold,
        }
    }

    /// Returns whether a world position falls inside a stand.
    pub fn in_stand(&self, x: f32, z: f32) -> bool {
        self.config.enabled && stand_field(x, z, self.feature_m, self.config.seed) >= self.threshold
    }

    /// Returns the acceptance rate for one canopy candidate at a position.
    pub fn canopy_accept(&self, in_stand: bool) -> f32 {
        if !self.config.enabled {
            return 0.0;
        }
        if in_stand {
            CANOPY_STAND_ACCEPT
        } else {
            CANOPY_OPEN_ACCEPT
        }
    }

    /// Returns the acceptance rate for one understory shrub candidate at a position.
    pub fn understory_accept(&self, in_stand: bool) -> f32 {
        if !self.config.enabled {
            return 0.0;
        }
        if in_stand {
            UNDERSTORY_STAND_ACCEPT
        } else {
            UNDERSTORY_OPEN_ACCEPT
        }
    }

    /// Returns the acceptance rate for one rock candidate.
    pub fn rock_accept(&self) -> f32 {
        if self.config.enabled {
            ROCK_ACCEPT
        } else {
            0.0
        }
    }
}

impl Default for VegetationGenerator {
    fn default() -> Self {
        Self::resolve(
            VegetationConfig::default(),
            &WorldConfig::gameplay_default(),
        )
    }
}

/// Returns the canopy grid spacing in metres that produces a requested density.
///
/// This is what a density dial actually sets: acceptance stays fixed and only the candidate
/// spacing moves. Kept separate from [`VegetationGenerator::resolve`] because it needs no
/// world, so a menu can show the spacing a slider will produce before a world exists.
pub fn canopy_cell_m_for_density(stems_per_ha: f32) -> f32 {
    // Quantised to the millimetre. The grid spacing decides how many candidate cells a patch
    // covers, so a value one float ulp either side of a round number could change that count
    // at a patch boundary; rounding makes one density always mean one grid.
    ((10_000.0 * CANOPY_STAND_ACCEPT / stems_per_ha.max(f32::MIN_POSITIVE))
        .sqrt()
        .clamp(MIN_CANOPY_CELL_M, MAX_CANOPY_CELL_M)
        * 1_000.0)
        .round()
        / 1_000.0
}

/// Integer hash used by both the stand field and per-cell candidate placement.
///
/// Hashes integer coordinates rather than load order, so a patch generates the same plants
/// whatever order the renderer asks for patches in.
pub fn hash(x: i32, z: i32, salt: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x9e37_79b9) ^ (z as u32).wrapping_mul(0x85eb_ca6b) ^ salt;
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^ (h >> 16)
}

/// Returns a deterministic value in `[0, 1)` for one integer cell and salt.
pub fn unit(x: i32, z: i32, salt: u32) -> f32 {
    (hash(x, z, salt) >> 8) as f32 / 16_777_216.0
}

// One octave of smoothed value noise on the integer lattice, in [0, 1).
fn value_noise(x: f32, z: f32, salt: u32) -> f32 {
    let (fx, fz) = (x.floor(), z.floor());
    let (ix, iz) = (fx as i32, fz as i32);
    // Smoothstep the cell-local fraction so stand edges curve instead of showing the lattice.
    let u = {
        let t = x - fx;
        t * t * (3.0 - 2.0 * t)
    };
    let v = {
        let t = z - fz;
        t * t * (3.0 - 2.0 * t)
    };
    let a = unit(ix, iz, salt) + (unit(ix + 1, iz, salt) - unit(ix, iz, salt)) * u;
    let b = unit(ix, iz + 1, salt) + (unit(ix + 1, iz + 1, salt) - unit(ix, iz + 1, salt)) * u;
    a + (b - a) * v
}

// Stand field in [0, 1). Three octaves: broad stands, their lobes, and a broken edge. The
// wavelength is a metric length that only shrinks on worlds too small to hold one stand, which
// is what the two-sine field it replaces could not do - its 2027 m and 785 m periods never
// completed across a 500 m sandbox world, leaving the whole map inside one stand.
fn stand_field(x: f32, z: f32, feature_m: f32, seed: u32) -> f32 {
    let f = feature_m.max(f32::MIN_POSITIVE);
    0.62 * value_noise(x / f, z / f, seed)
        + 0.26 * value_noise(x / (f * 0.45), z / (f * 0.45), seed ^ 0x9e37_79b9)
        + 0.12 * value_noise(x / (f * 0.18), z / (f * 0.18), seed ^ 0x85eb_ca6b)
}

// Turns a requested area fraction into a field threshold by sampling the field across the world
// and taking the matching quantile. A fixed sweep beats an analytic inverse here because the
// field is a sum of octaves with no closed-form distribution, and because it makes coverage mean
// the same fraction of ground on a 500 m sandbox and an 18 km city.
fn coverage_threshold(coverage: f32, seed: u32, feature_m: f32, world: &WorldConfig) -> f32 {
    if coverage <= 0.0 {
        return f32::INFINITY;
    }
    if coverage >= 1.0 {
        return f32::NEG_INFINITY;
    }
    let n = COVERAGE_SAMPLES_PER_AXIS;
    let mut samples = Vec::with_capacity(n * n);
    let step_x = world.width_m / n as f32;
    let step_z = world.height_m / n as f32;
    let origin_x = -world.width_m * 0.5;
    let origin_z = -world.height_m * 0.5;
    for iz in 0..n {
        for ix in 0..n {
            let x = origin_x + (ix as f32 + 0.5) * step_x;
            let z = origin_z + (iz as f32 + 0.5) * step_z;
            samples.push(stand_field(x, z, feature_m, seed));
        }
    }
    samples.sort_unstable_by(f32::total_cmp);
    let index = (((1.0 - coverage) * samples.len() as f32) as usize).min(samples.len() - 1);
    samples[index]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world(size_m: f32) -> WorldConfig {
        WorldConfig::new(size_m, size_m, 40.0, 10.0)
    }

    #[test]
    fn vegetation_default_config_reproduces_the_shipped_grid_exactly() {
        let generator = VegetationGenerator::default();
        assert_eq!(generator.canopy_cell_m, 16.0);
        assert_eq!(generator.understory_cell_m, 4.0);
        assert_eq!(generator.canopy_accept(true), 0.78);
        assert_eq!(generator.canopy_accept(false), 0.06);
        assert_eq!(generator.understory_accept(true), 0.62);
        assert_eq!(generator.understory_accept(false), 0.05);
    }

    #[test]
    fn vegetation_density_sets_cell_size_and_clamps_to_the_renderer_ceiling() {
        let w = world(18_000.0);
        let quadrupled = VegetationGenerator::resolve(
            VegetationConfig {
                canopy_stems_per_ha: VegetationConfig::DEFAULT_CANOPY_STEMS_PER_HA * 4.0,
                ..Default::default()
            },
            &w,
        );
        // Four times the stems is half the spacing on both grids, not a changed accept rate.
        assert!((quadrupled.canopy_cell_m - 8.0).abs() < 1e-4);
        assert!((quadrupled.understory_cell_m - 2.0).abs() < 1e-4);
        assert_eq!(quadrupled.canopy_accept(true), 0.78);

        let beyond = VegetationGenerator::resolve(
            VegetationConfig {
                canopy_stems_per_ha: 5_000.0,
                ..Default::default()
            },
            &w,
        );
        assert_eq!(beyond.canopy_cell_m, MIN_CANOPY_CELL_M);
        assert!((VegetationConfig::MAX_CANOPY_STEMS_PER_HA - 121.875).abs() < 1e-3);
    }

    #[test]
    fn vegetation_coverage_is_an_area_fraction_on_small_and_large_worlds() {
        // The two-sine field this replaces put the entire 500 m sandbox inside one stand.
        for size_m in [500.0, 18_000.0] {
            let w = world(size_m);
            for coverage in [0.15, 0.5, 0.85] {
                let generator = VegetationGenerator::resolve(
                    VegetationConfig {
                        coverage,
                        ..Default::default()
                    },
                    &w,
                );
                let n = 128;
                let inside = (0..n * n)
                    .filter(|i| {
                        let x = -size_m * 0.5 + ((i % n) as f32 + 0.5) * size_m / n as f32;
                        let z = -size_m * 0.5 + ((i / n) as f32 + 0.5) * size_m / n as f32;
                        generator.in_stand(x, z)
                    })
                    .count() as f32
                    / (n * n) as f32;
                assert!(
                    (inside - coverage).abs() < 0.05,
                    "{size_m} m world at coverage {coverage} measured {inside}"
                );
            }
        }
    }

    #[test]
    fn vegetation_coverage_extremes_clear_and_fill_the_world() {
        let w = world(18_000.0);
        let empty = VegetationGenerator::resolve(
            VegetationConfig {
                coverage: 0.0,
                ..Default::default()
            },
            &w,
        );
        let full = VegetationGenerator::resolve(
            VegetationConfig {
                coverage: 1.0,
                ..Default::default()
            },
            &w,
        );
        let disabled = VegetationGenerator::resolve(
            VegetationConfig {
                enabled: false,
                ..Default::default()
            },
            &w,
        );
        for (x, z) in [(0.0, 0.0), (-4_000.0, 2_500.0), (7_777.0, -1_234.0)] {
            assert!(!empty.in_stand(x, z));
            assert!(full.in_stand(x, z));
            assert!(!disabled.in_stand(x, z));
        }
        // A cleared world generates nothing anywhere, stand or not.
        assert_eq!(disabled.canopy_accept(true), 0.0);
        assert_eq!(disabled.understory_accept(true), 0.0);
        assert_eq!(disabled.rock_accept(), 0.0);
    }

    #[test]
    fn vegetation_seed_changes_the_stand_layout_but_not_its_coverage() {
        let w = world(18_000.0);
        let a = VegetationGenerator::resolve(VegetationConfig::default(), &w);
        let b = VegetationGenerator::resolve(
            VegetationConfig {
                seed: 12_345,
                ..Default::default()
            },
            &w,
        );
        let n = 96;
        let mut differing = 0;
        let (mut inside_a, mut inside_b) = (0, 0);
        for i in 0..n * n {
            let x = -9_000.0 + ((i % n) as f32 + 0.5) * 18_000.0 / n as f32;
            let z = -9_000.0 + ((i / n) as f32 + 0.5) * 18_000.0 / n as f32;
            let (sa, sb) = (a.in_stand(x, z), b.in_stand(x, z));
            inside_a += i32::from(sa);
            inside_b += i32::from(sb);
            differing += i32::from(sa != sb);
        }
        assert!(differing > n * n / 8, "seed barely moved the stands");
        assert!((inside_a - inside_b).abs() < n * n / 20);
    }

    #[test]
    fn vegetation_local_coverage_does_not_swing_between_meadow_and_thicket() {
        // Global coverage being right says nothing about what one player sees. A stand feature
        // size of a world fraction gave the 18 km world eight lattice cells per axis, and the
        // 2 km window around spawn then measured 0.002 where the whole world measured 0.565.
        // That window is also the near understory ring, so its coverage sets peak frame cost.
        let w = world(18_000.0);
        let g = VegetationGenerator::resolve(VegetationConfig::default(), &w);
        let window_m = 2_040.0_f32;
        let mut extremes = (1.0_f32, 0.0_f32);
        for cz in -3..=3 {
            for cx in -3..=3 {
                let (ox, oz) = (cx as f32 * 3_000.0, cz as f32 * 3_000.0);
                let n = 32;
                let inside = (0..n * n)
                    .filter(|i| {
                        let x = ox - window_m * 0.5 + ((i % n) as f32 + 0.5) * window_m / n as f32;
                        let z = oz - window_m * 0.5 + ((i / n) as f32 + 0.5) * window_m / n as f32;
                        g.in_stand(x, z)
                    })
                    .count() as f32
                    / (n * n) as f32;
                extremes = (extremes.0.min(inside), extremes.1.max(inside));
            }
        }
        assert!(
            extremes.0 > 0.1 && extremes.1 < 0.95,
            "local coverage ran {:.3} to {:.3} across spawn windows",
            extremes.0,
            extremes.1
        );
    }

    #[test]
    fn vegetation_stand_field_is_continuous_across_the_lattice() {
        // A visible lattice seam would draw stand edges as straight grid lines.
        let feature = 500.0;
        let mut worst: f32 = 0.0;
        for i in 0..400 {
            let x = 1_000.0 + i as f32 * 2.5;
            let a = stand_field(x, 321.0, feature, 7);
            let b = stand_field(x + 0.5, 321.0, feature, 7);
            worst = worst.max((a - b).abs());
        }
        assert!(worst < 0.02, "largest half-metre step was {worst}");
    }

    #[test]
    fn vegetation_config_sanitises_menu_input_into_a_reproducible_world() {
        // The dialog supplies these, so an out-of-range or non-finite value must not be able
        // to produce a world the generator cannot regenerate from the same save.
        let sane = VegetationConfig {
            enabled: true,
            seed: 9,
            coverage: 4.0,
            canopy_stems_per_ha: 5_000.0,
        }
        .sanitized();
        assert_eq!(sane.coverage, 1.0);
        assert_eq!(
            sane.canopy_stems_per_ha,
            VegetationConfig::MAX_CANOPY_STEMS_PER_HA
        );
        assert_eq!(sane.seed, 9);

        let negative = VegetationConfig {
            coverage: -1.0,
            canopy_stems_per_ha: 0.0,
            ..VegetationConfig::default()
        }
        .sanitized();
        assert_eq!(negative.coverage, 0.0);
        assert_eq!(
            negative.canopy_stems_per_ha,
            VegetationConfig::MIN_CANOPY_STEMS_PER_HA
        );

        // Non-finite input falls back to the shipped value rather than clamping to an extreme.
        let broken = VegetationConfig {
            coverage: f32::NAN,
            canopy_stems_per_ha: f32::INFINITY,
            ..VegetationConfig::default()
        }
        .sanitized();
        assert_eq!(broken.coverage, VegetationConfig::DEFAULT_COVERAGE);
        assert_eq!(
            broken.canopy_stems_per_ha,
            VegetationConfig::DEFAULT_CANOPY_STEMS_PER_HA
        );

        // Sanitised values survive resolution, and the default stays untouched by it.
        assert_eq!(
            VegetationConfig::default().sanitized(),
            VegetationConfig::default()
        );
    }
}
