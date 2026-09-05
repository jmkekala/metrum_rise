// SPDX-License-Identifier: GPL-2.0-only

//! Character source-asset schema.

use serde::Deserialize;

// ── Character ─────────────────────────────────────────────────────────────────

/// Character archetype family, determining which VAT data pool is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchetypeFamily {
    /// Adult male proportions.
    AdultMale,
    /// Adult female proportions.
    AdultFemale,
    /// Child proportions (separate rest mesh and animation bakes).
    Child,
}

/// One skin or clothing texture variant for a character.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkinVariant {
    /// Short display name (e.g. `"default"`, `"summer"`).
    pub name: String,
    /// Path to the albedo texture for this variant, relative to the asset folder.
    pub albedo_file: String,
}

/// Class-specific data for a character source asset.
///
/// Runtime packs ship baked VAT outputs only. Source clip references
/// are editor-only and are not included in exported runtime packs.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterData {
    /// Archetype family this character belongs to.
    pub archetype_family: ArchetypeFamily,
    /// Informal age group label (e.g. `"adult"`, `"elderly"`). Optional.
    pub age_group: Option<String>,
    /// Informal body type label (e.g. `"average"`, `"athletic"`). Optional.
    pub body_type: Option<String>,
    /// Available skin or clothing variants. At least one is expected.
    #[serde(default)]
    pub skin_variants: Vec<SkinVariant>,
}
