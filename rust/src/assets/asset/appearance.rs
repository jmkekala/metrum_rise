// SPDX-License-Identifier: GPL-2.0-only

//! Coordinated building texture schemes; geometry and simulation state remain shared.

use super::MeshPart;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Authored appearance choices. Absent metadata preserves source materials.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuildingAppearance {
    /// Stable scheme ID used when random selection is disabled.
    pub default_scheme: String,
    /// Selection policy applied by [`BuildingAppearance::scheme_for`] at spawn.
    #[serde(default)]
    pub spawn: SpawnAppearance,
    /// Coordinated choices shared by every part and LOD of an instance.
    pub schemes: Vec<ColourScheme>,
}

/// How a consumer should choose an instance's persistent appearance.
#[derive(Debug, Default, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpawnAppearance {
    /// Always use the authored default.
    DefaultOnly,
    /// Choose uniformly from authored schemes, independently of simulation randomness.
    /// Authoring several schemes usually means a varied street, so this is the default;
    /// a branded asset whose colour must not vary states `default_only` explicitly.
    #[default]
    RandomScheme,
}

/// A named set of texture replacements, not a whole-model tint.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ColourScheme {
    /// Stable lowercase identifier, unchanged when the display name is edited.
    pub id: String,
    /// Human-readable editor and library label.
    pub name: String,
    /// Unlisted materials and texture channels retain their source values.
    #[serde(default)]
    pub overrides: Vec<MaterialOverride>,
}

/// Explicit material mapping across one part's LOD chain.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialOverride {
    /// Unique authored mesh-part name, not its mutable list position.
    pub part: String,
    /// Exact source material name for each LOD, nearest first. Ambiguity is an error.
    pub materials: Vec<String>,
    /// Replacement albedo path relative to the exported asset directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub albedo: Option<String>,
    /// Optional packed occlusion/roughness/metallic texture replacement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orm: Option<String>,
    /// Optional tangent-space normal texture replacement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal: Option<String>,
    /// Optional window emission mask replacement, used by gameplay and preview schedules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emission: Option<String>,
}

impl MaterialOverride {
    pub(crate) fn textures(&self) -> impl Iterator<Item = &str> {
        [&self.albedo, &self.orm, &self.normal, &self.emission]
            .into_iter()
            .filter_map(|path| path.as_deref())
    }
}

impl BuildingAppearance {
    /// Position of the authored default, or the first scheme when the ID is unresolvable.
    pub fn default_index(&self) -> usize {
        self.schemes
            .iter()
            .position(|scheme| scheme.id == self.default_scheme)
            .unwrap_or(0)
    }

    /// Scheme ordinal for one instance's stable appearance key.
    ///
    /// Allocation-free and O(schemes) at worst, so a renderer derives an instance's scheme
    /// on demand rather than storing it in simulation state or saves. The caller owns key
    /// stability; an unstable key would recolour a building whenever it is recomputed.
    pub fn scheme_for(&self, key: u64) -> usize {
        if self.schemes.is_empty() {
            return 0;
        }
        match self.spawn {
            SpawnAppearance::DefaultOnly => self.default_index(),
            // Multiply-high consumes the well-mixed upper bits of the FNV-style placement
            // hash and stays uniform for scheme counts that are not powers of two, which a
            // modulo of the weaker low bits would not.
            SpawnAppearance::RandomScheme => {
                ((key as u128 * self.schemes.len() as u128) >> 64) as usize
            }
        }
    }

    // Document-change validation only: O(overrides × parts + mappings log mappings).
    pub(crate) fn validate(&self, parts: &[MeshPart]) -> Result<(), String> {
        let mut ids = BTreeSet::new();
        for scheme in &self.schemes {
            if scheme.id.is_empty()
                || !scheme
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
                || !ids.insert(&scheme.id)
            {
                return Err(format!(
                    "Invalid or duplicate colour scheme ID: {}",
                    scheme.id
                ));
            }
            if scheme.name.trim().is_empty() {
                return Err(format!("Colour scheme {} needs a display name", scheme.id));
            }
            let mut targets = BTreeSet::new();
            for entry in &scheme.overrides {
                let mut matches = parts.iter().filter(|part| part.name == entry.part);
                let Some(part) = matches.next() else {
                    return Err(format!(
                        "Colour scheme {} targets missing part {}",
                        scheme.id, entry.part
                    ));
                };
                if matches.next().is_some() || entry.materials.len() != part.lods.len() {
                    return Err(format!(
                        "Part {} needs an unambiguous material mapping for every LOD",
                        entry.part
                    ));
                }
                for (lod, name) in entry.materials.iter().enumerate() {
                    if name.trim().is_empty() || !targets.insert((&entry.part, lod, name)) {
                        return Err(format!(
                            "Missing or duplicate material target on {} LOD{lod}",
                            entry.part
                        ));
                    }
                }
                if entry.textures().next().is_none() {
                    return Err(format!(
                        "Material override on {} has no textures",
                        entry.part
                    ));
                }
                for path in entry.textures() {
                    if !crate::assets::authoring::files::safe_relative(path) {
                        return Err(format!("Unsafe colour scheme texture path: {path}"));
                    }
                }
            }
        }
        if !ids.contains(&self.default_scheme) {
            return Err("Default colour scheme must identify an existing scheme".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn colour_scheme_contract_and_explicit_lod_mappings() {
        let mut part = MeshPart::single_lod0("house", "house.glb");
        part.lods.push(part.lods[0].clone());
        let mut appearance: BuildingAppearance = serde_json::from_value(json!({
            "default_scheme": "red", "spawn": "random_scheme",
            "schemes": [{"id": "red", "name": "Red", "overrides": [{
                "part": "house", "materials": ["walls", "walls_low"], "albedo": "red.png"
            }]}]
        }))
        .unwrap();
        assert!(appearance.validate(&[part.clone()]).is_ok());
        let encoded = toml::to_string(&appearance).unwrap();
        assert_eq!(appearance, toml::from_str(&encoded).unwrap());
        appearance.schemes.push(appearance.schemes[0].clone());
        assert!(appearance.validate(&[part.clone()]).is_err());
        appearance.schemes.pop();
        appearance.default_scheme = "missing".into();
        assert!(appearance.validate(&[part.clone()]).is_err());
        appearance.default_scheme = "red".into();
        appearance.schemes[0].overrides[0].materials.pop();
        assert!(appearance.validate(&[part.clone()]).is_err());
        appearance.schemes[0].overrides[0]
            .materials
            .push("walls_low".into());
        appearance.schemes[0].overrides[0].albedo = Some("../outside.png".into());
        assert!(appearance.validate(&[part]).is_err());
    }

    #[test]
    fn spawn_policy_maps_stable_keys_onto_every_authored_scheme() {
        let mut appearance: BuildingAppearance = serde_json::from_value(json!({
            "default_scheme": "blue", "spawn": "default_only",
            "schemes": [{"id": "red", "name": "Red"}, {"id": "blue", "name": "Blue"},
                {"id": "green", "name": "Green"}]
        }))
        .unwrap();
        // Golden-ratio stride sweeps the whole key space without needing the placement hash.
        let keys = || (0..3000u64).map(|step| step.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        assert!(keys().all(|key| appearance.scheme_for(key) == 1));

        appearance.spawn = SpawnAppearance::RandomScheme;
        assert!(keys().all(|key| appearance.scheme_for(key) < appearance.schemes.len()));
        // Re-derivation must be stable; the renderer recomputes this instead of storing it.
        assert_eq!(appearance.scheme_for(7), appearance.scheme_for(7));
        let mut counts = [0usize; 3];
        for key in keys() {
            counts[appearance.scheme_for(key)] += 1;
        }
        // A scheme count that is not a power of two must not starve any scheme.
        assert!(counts.iter().all(|&hits| hits > 900), "{counts:?}");

        appearance.default_scheme = "missing".into();
        assert_eq!(appearance.default_index(), 0);
        appearance.schemes.clear();
        assert_eq!(appearance.scheme_for(7), 0);
    }
}
