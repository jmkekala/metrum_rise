// SPDX-License-Identifier: GPL-2.0-only

//! Public zoning-profile registry API.

use super::authored::{load_authored_zone_profiles, load_builtin_growth_profile_ids};
use super::compile::compile_registry;
use super::runtime::{ZoneDensity, ZoneProfileRuntime};
use crate::simulation::zoning::ZoneType;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// Validated built-in zoning-profile registry.
#[derive(Debug)]
pub struct ZoningProfileRegistry {
    pub(super) profiles: Vec<ZoneProfileRuntime>,
    pub(super) by_id: HashMap<String, u16>,
    pub(super) default_ids_by_zone_density: HashMap<(ZoneType, ZoneDensity), u16>,
}

impl ZoningProfileRegistry {
    /// Returns every validated runtime profile in deterministic runtime-id order.
    pub fn profiles(&self) -> &[ZoneProfileRuntime] {
        &self.profiles
    }

    /// Returns the total number of non-zero runtime profiles.
    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    /// Returns the profile for one dense runtime id.
    pub fn profile_by_runtime_id(&self, runtime_id: u16) -> Option<&ZoneProfileRuntime> {
        if runtime_id == 0 {
            return None;
        }
        self.profiles.get(runtime_id as usize - 1)
    }

    /// Returns the profile for one authored string id.
    pub fn profile_by_id(&self, id: &str) -> Option<&ZoneProfileRuntime> {
        let runtime_id = *self.by_id.get(id)?;
        self.profile_by_runtime_id(runtime_id)
    }

    /// Returns the broad zone family for one runtime profile id.
    pub fn zone_type_for_runtime_id(&self, runtime_id: u16) -> ZoneType {
        self.profile_by_runtime_id(runtime_id)
            .map(|profile| profile.zone_type)
            .unwrap_or(ZoneType::None)
    }

    /// Returns the density band for one runtime profile id.
    pub fn density_for_runtime_id(&self, runtime_id: u16) -> Option<ZoneDensity> {
        self.profile_by_runtime_id(runtime_id)
            .map(|profile| profile.density)
    }

    /// Returns the default runtime id for one `(zone_type, density)` pair.
    pub fn runtime_id_for_zone_density(
        &self,
        zone_type: ZoneType,
        density: ZoneDensity,
    ) -> Option<u16> {
        self.default_ids_by_zone_density
            .get(&(zone_type, density))
            .copied()
    }

    /// Returns the baseline default runtime id for one broad zone family.
    ///
    /// Test fixtures use the low-density default for the requested family.
    #[cfg(test)]
    pub fn default_runtime_id_for_zone_type(&self, zone_type: ZoneType) -> Option<u16> {
        self.runtime_id_for_zone_density(zone_type, ZoneDensity::Low)
    }

    /// Returns `true` when one building asset is legal for the given zoning profile.
    pub fn asset_is_legal(
        &self,
        runtime_id: u16,
        asset_zone_type: ZoneType,
        asset_density: &str,
        asset_tags: &[String],
    ) -> bool {
        let Some(profile) = self.profile_by_runtime_id(runtime_id) else {
            return false;
        };
        if asset_zone_type != profile.zone_type {
            return false;
        }
        if ZoneDensity::from_str_name(asset_density) != Some(profile.density) {
            return false;
        }
        profile
            .required_asset_tags
            .iter()
            .all(|tag| asset_tags.iter().any(|asset_tag| asset_tag == tag))
    }

    /// Builds the 1-row RGBA8 style LUT used by the zoning overlay shader.
    ///
    /// Entry `0` is transparent and reserved for `unpainted / none`.
    pub fn style_lut_rgba8(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity((self.profiles.len() + 1) * 4);
        out.extend_from_slice(&[0, 0, 0, 0]);
        for profile in &self.profiles {
            out.extend_from_slice(&[
                profile.ui_color_rgb[0],
                profile.ui_color_rgb[1],
                profile.ui_color_rgb[2],
                255,
            ]);
        }
        out
    }
}

static BUILTIN_REGISTRY: OnceLock<Result<Arc<ZoningProfileRegistry>, String>> = OnceLock::new();

/// Returns the shared immutable shipped registry, compiling it once on first use.
pub fn load_builtin_profile_registry() -> Result<Arc<ZoningProfileRegistry>, String> {
    BUILTIN_REGISTRY
        .get_or_init(|| load_registry_from_disk().map(Arc::new))
        .clone()
}

fn load_registry_from_disk() -> Result<ZoningProfileRegistry, String> {
    let authored_profiles = load_authored_zone_profiles()?;
    let growth_profile_ids = load_builtin_growth_profile_ids()?;
    compile_registry(authored_profiles, &growth_profile_ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_zoning_registry_reuses_one_immutable_instance() {
        let first = load_builtin_profile_registry().unwrap();
        let second = load_builtin_profile_registry().unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert!(!first.profiles().is_empty());
    }

    #[test]
    #[ignore = "manual matched release timing of cached zoning profile loads"]
    fn benchmark_cached_zoning_profile_load() {
        use std::hint::black_box;
        use std::time::Instant;

        let expected = load_builtin_profile_registry().unwrap().style_lut_rgba8();
        for _ in 0..3 {
            black_box(load_builtin_profile_registry().unwrap());
        }
        let mut samples = [0.0; 21];
        for sample in &mut samples {
            let start = Instant::now();
            for _ in 0..10_000 {
                black_box(load_builtin_profile_registry().unwrap());
            }
            *sample = start.elapsed().as_secs_f64() * 1_000.0 / 10_000.0;
        }
        samples.sort_by(f64::total_cmp);
        assert_eq!(
            load_builtin_profile_registry().unwrap().style_lut_rgba8(),
            expected
        );
        eprintln!(
            "cached_zoning_profile_load median_ms={:.9} lut={expected:?}",
            samples[10]
        );
    }
}
