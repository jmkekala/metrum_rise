//! Built-in zoning-profile registry loading and deterministic runtime compilation.

use super::ZoneType;
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

const PROFILES_FILE: &str = "zoning/profiles.toml";
const GROWTH_PROFILES_FILE: &str = "demand/growth_profiles.toml";

/// Density band for a zoning profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ZoneDensity {
    /// Low-density development.
    Low,
    /// Medium-density development.
    Medium,
    /// High-density development.
    High,
}

impl ZoneDensity {
    /// Parses one authored density string.
    pub fn from_str_name(value: &str) -> Option<Self> {
        match value.trim() {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }

    /// Returns the canonical snake-case density key.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// One validated runtime zoning profile.
#[derive(Clone, Debug)]
pub struct ZoneProfileRuntime {
    /// Runtime profile id assigned to parcels. `0` is reserved for free/unzoned parcels.
    pub runtime_id: u16,
    /// Stable authored TOML id.
    pub id: String,
    /// Player-facing display name.
    pub display_name: String,
    /// Deterministic UI ordering key inside one top-level category.
    pub ui_order: u32,
    /// Broad zone family derived from the profile.
    pub zone_type: ZoneType,
    /// Density band for this profile.
    pub density: ZoneDensity,
    /// Secondary required asset tags for legality filtering.
    pub required_asset_tags: Vec<String>,
    /// Matching demand-side growth profile id.
    pub growth_profile_id: String,
    /// Parsed RGB colour used by the UI and overlay LUT.
    pub ui_color_rgb: [u8; 3],
    /// Stable UI icon key.
    pub ui_icon: String,
    /// Player-facing tooltip/description text.
    pub ui_description: String,
}

/// Validated built-in zoning-profile registry.
#[derive(Clone, Debug, Default)]
pub struct ZoningProfileRegistry {
    profiles: Vec<ZoneProfileRuntime>,
    by_id: HashMap<String, u16>,
    default_ids_by_zone_density: HashMap<(ZoneType, ZoneDensity), u16>,
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
    /// Test-only compatibility helper kept while migration coverage still needs a broad-family to
    /// runtime-profile mapping. It returns the low-density default for the requested family.
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

#[derive(Deserialize)]
struct ProfilesFile {
    #[serde(default)]
    profiles: Vec<AuthoredZoneProfile>,
}

#[derive(Deserialize)]
struct AuthoredZoneProfile {
    id: String,
    display_name: String,
    ui_order: u32,
    zone_type: String,
    density: String,
    #[serde(default)]
    required_asset_tags: Vec<String>,
    growth_profile_id: String,
    ui: AuthoredZoneProfileUi,
}

#[derive(Deserialize)]
struct AuthoredZoneProfileUi {
    color: String,
    icon: String,
    description: String,
}

#[derive(Deserialize)]
struct GrowthProfilesFile {
    #[serde(default)]
    profiles: Vec<AuthoredGrowthProfile>,
}

#[derive(Deserialize)]
struct AuthoredGrowthProfile {
    id: String,
}

static BUILTIN_REGISTRY: OnceLock<Result<ZoningProfileRegistry, String>> = OnceLock::new();

/// Loads and caches the shipped zoning-profile registry.
pub fn load_builtin_profile_registry() -> Result<Arc<ZoningProfileRegistry>, String> {
    match BUILTIN_REGISTRY.get_or_init(load_registry_from_disk) {
        Ok(registry) => Ok(Arc::new(registry.clone())),
        Err(err) => Err(err.clone()),
    }
}

fn load_registry_from_disk() -> Result<ZoningProfileRegistry, String> {
    let profiles_path = repo_relative_path(PROFILES_FILE);
    let content = std::fs::read_to_string(&profiles_path)
        .map_err(|err| format!("could not read '{}': {err}", profiles_path.display()))?;
    let authored: ProfilesFile = toml::from_str(&content)
        .map_err(|err| format!("could not parse '{}': {err}", profiles_path.display()))?;
    let growth_profile_ids = load_builtin_growth_profile_ids()?;
    compile_registry(authored.profiles, &growth_profile_ids)
}

fn load_builtin_growth_profile_ids() -> Result<HashSet<String>, String> {
    let growth_profiles_path = repo_relative_path(GROWTH_PROFILES_FILE);
    let content = std::fs::read_to_string(&growth_profiles_path)
        .map_err(|err| format!("could not read '{}': {err}", growth_profiles_path.display()))?;
    let authored: GrowthProfilesFile = toml::from_str(&content).map_err(|err| {
        format!(
            "could not parse '{}': {err}",
            growth_profiles_path.display()
        )
    })?;
    Ok(authored
        .profiles
        .into_iter()
        .map(|profile| profile.id)
        .collect())
}

fn repo_relative_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(relative)
}

fn compile_registry(
    authored_profiles: Vec<AuthoredZoneProfile>,
    growth_profile_ids: &HashSet<String>,
) -> Result<ZoningProfileRegistry, String> {
    let mut seen_ids = BTreeSet::new();
    let mut compiled = Vec::with_capacity(authored_profiles.len());
    let mut by_id = HashMap::new();
    let mut default_ids_by_zone_density = HashMap::new();

    let mut authored_profiles = authored_profiles;
    authored_profiles.sort_by(|a, b| {
        let a_zone = parse_baseline_zone_type(&a.zone_type).map(baseline_zone_rank);
        let b_zone = parse_baseline_zone_type(&b.zone_type).map(baseline_zone_rank);
        a_zone
            .cmp(&b_zone)
            .then(a.ui_order.cmp(&b.ui_order))
            .then(a.id.cmp(&b.id))
    });

    for authored in authored_profiles {
        if authored.id.trim().is_empty() {
            return Err("zoning profile id must not be empty".to_owned());
        }
        if !seen_ids.insert(authored.id.clone()) {
            return Err(format!("duplicate zoning profile id '{}'", authored.id));
        }
        if authored.display_name.trim().is_empty() {
            return Err(format!(
                "zoning profile '{}': display_name must not be empty",
                authored.id
            ));
        }
        if authored.ui.icon.trim().is_empty() {
            return Err(format!(
                "zoning profile '{}': ui.icon must not be empty",
                authored.id
            ));
        }
        if authored.ui.description.trim().is_empty() {
            return Err(format!(
                "zoning profile '{}': ui.description must not be empty",
                authored.id
            ));
        }
        let zone_type = parse_baseline_zone_type(&authored.zone_type).ok_or_else(|| {
            format!(
                "zoning profile '{}': unsupported baseline zone_type '{}'",
                authored.id, authored.zone_type
            )
        })?;
        let density = ZoneDensity::from_str_name(&authored.density).ok_or_else(|| {
            format!(
                "zoning profile '{}': unknown density '{}'",
                authored.id, authored.density
            )
        })?;
        if !growth_profile_ids.contains(&authored.growth_profile_id) {
            return Err(format!(
                "zoning profile '{}': unknown growth_profile_id '{}'",
                authored.id, authored.growth_profile_id
            ));
        }
        let expected_growth_profile_id =
            format!("{}_{}_default", zone_type.as_str(), density.as_str());
        if authored.growth_profile_id != expected_growth_profile_id {
            return Err(format!(
                "zoning profile '{}': baseline growth_profile_id must be '{}', found '{}'",
                authored.id, expected_growth_profile_id, authored.growth_profile_id
            ));
        }
        let ui_color_rgb = parse_hex_rgb(&authored.ui.color).ok_or_else(|| {
            format!(
                "zoning profile '{}': invalid ui.color '{}'",
                authored.id, authored.ui.color
            )
        })?;

        let mut required_asset_tags: Vec<String> = authored
            .required_asset_tags
            .into_iter()
            .map(|tag| tag.trim().to_owned())
            .filter(|tag| !tag.is_empty())
            .collect();
        required_asset_tags.sort();
        required_asset_tags.dedup();

        let runtime_id = compiled.len() as u16 + 1;
        let profile = ZoneProfileRuntime {
            runtime_id,
            id: authored.id.clone(),
            display_name: authored.display_name.trim().to_owned(),
            ui_order: authored.ui_order,
            zone_type,
            density,
            required_asset_tags,
            growth_profile_id: authored.growth_profile_id,
            ui_color_rgb,
            ui_icon: authored.ui.icon.trim().to_owned(),
            ui_description: authored.ui.description.trim().to_owned(),
        };

        by_id.insert(profile.id.clone(), runtime_id);
        default_ids_by_zone_density
            .entry((profile.zone_type, profile.density))
            .or_insert(runtime_id);
        compiled.push(profile);
    }

    Ok(ZoningProfileRegistry {
        profiles: compiled,
        by_id,
        default_ids_by_zone_density,
    })
}

fn parse_baseline_zone_type(value: &str) -> Option<ZoneType> {
    match value.trim() {
        "residential" => Some(ZoneType::Residential),
        "commercial" => Some(ZoneType::Commercial),
        "industrial" => Some(ZoneType::Industrial),
        _ => None,
    }
}

fn baseline_zone_rank(zone_type: ZoneType) -> u8 {
    match zone_type {
        ZoneType::Residential => 0,
        ZoneType::Commercial => 1,
        ZoneType::Industrial => 2,
        ZoneType::None | ZoneType::Office | ZoneType::Mixed => 255,
    }
}

fn parse_hex_rgb(value: &str) -> Option<[u8; 3]> {
    let value = value.trim();
    if value.len() != 7 || !value.starts_with('#') {
        return None;
    }
    Some([
        u8::from_str_radix(&value[1..3], 16).ok()?,
        u8::from_str_radix(&value[3..5], 16).ok()?,
        u8::from_str_radix(&value[5..7], 16).ok()?,
    ])
}
