//! Compilation and validation of authored zoning profiles.

use super::authored::AuthoredZoneProfile;
use super::registry::ZoningProfileRegistry;
use super::runtime::{ZoneDensity, ZoneProfileRuntime};
use crate::simulation::zoning::ZoneType;
use std::collections::{BTreeSet, HashMap, HashSet};

pub(super) fn compile_registry(
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
        validate_authored_profile_text(&authored)?;
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
        validate_profile_ids(
            &authored,
            zone_type,
            density,
            &mut seen_ids,
            growth_profile_ids,
        )?;
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

fn validate_authored_profile_text(authored: &AuthoredZoneProfile) -> Result<(), String> {
    if authored.id.trim().is_empty() {
        return Err("zoning profile id must not be empty".to_owned());
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
    Ok(())
}

fn validate_profile_ids(
    authored: &AuthoredZoneProfile,
    zone_type: ZoneType,
    density: ZoneDensity,
    seen_ids: &mut BTreeSet<String>,
    growth_profile_ids: &HashSet<String>,
) -> Result<(), String> {
    if !seen_ids.insert(authored.id.clone()) {
        return Err(format!("duplicate zoning profile id '{}'", authored.id));
    }
    if !growth_profile_ids.contains(&authored.growth_profile_id) {
        return Err(format!(
            "zoning profile '{}': unknown growth_profile_id '{}'",
            authored.id, authored.growth_profile_id
        ));
    }
    let expected_growth_profile_id = format!("{}_{}_default", zone_type.as_str(), density.as_str());
    if authored.growth_profile_id != expected_growth_profile_id {
        return Err(format!(
            "zoning profile '{}': baseline growth_profile_id must be '{}', found '{}'",
            authored.id, expected_growth_profile_id, authored.growth_profile_id
        ));
    }
    Ok(())
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
