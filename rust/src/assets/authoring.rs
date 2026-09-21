// SPDX-License-Identifier: GPL-2.0-only

//! Editor-only building presets, capability discovery and explicit conversion previews.
//! These project the existing manifest contract; they are not new runtime asset categories.
//! Queries run on document/catalog changes, never in a simulation or rendering hot path.

use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind as ProfileKind, RuntimeEconomyCatalog,
};
use serde_json::{Value, json};

pub(crate) mod colours;
pub(crate) mod document;
pub(crate) mod edits;
pub(crate) mod files;

/// Editor chains additionally start at zero and use contiguous authored distance bands.
/// This metadata remains separate from the engine's screen-size preview policy.
pub(crate) fn lod_chain_error(lods: &[super::LodEntry]) -> Result<(), String> {
    super::asset::validation::validate_lods("preview", Some("selected"), lods)
        .map_err(|e| e.to_string())?;
    if lods[0].distance_min_m != 0.0 {
        return Err("LOD0 must start at 0 m.".into());
    }
    for (index, pair) in lods.windows(2).enumerate() {
        if pair[0].distance_max_m != Some(pair[1].distance_min_m) {
            return Err(format!(
                "LOD{} must start exactly where LOD{} ends.",
                index + 1,
                index
            ));
        }
    }
    Ok(())
}

const TYPES: [(&str, &str); 7] = [
    ("residential", "Residential"),
    ("commercial", "Commercial"),
    ("industrial", "Industrial"),
    ("extractor", "Resource extractor"),
    ("farm", "Farm"),
    ("service", "Service / utility"),
    ("explicit", "Other explicitly placed building"),
];

const SERVICES: [(&str, &str); 10] = [
    ("power", "Power"),
    ("water", "Water"),
    ("waste", "Wastewater"),
    ("police", "Police"),
    ("fire", "Fire"),
    ("healthcare", "Healthcare"),
    ("education", "Education"),
    ("transit", "Transit"),
    ("parks", "Parks"),
    ("government", "Government"),
];

fn text<'a>(data: &'a Value, key: &str) -> &'a str {
    data[key].as_str().unwrap_or_default().trim()
}

/// Resolve authoring intent strictly from metadata, never from names or catalog tags.
pub(crate) fn kind(data: &Value) -> &'static str {
    if text(data, "asset_class") != "building" {
        return "unsupported";
    }
    match text(data, "placement_mode") {
        "zoned_private" | "" => match text(data, "zone_type") {
            "residential" => "residential",
            "commercial" => "commercial",
            "industrial" => "industrial",
            _ => "unsupported",
        },
        "explicit" => {
            if !text(data, "extractor_resource").is_empty()
                || !text(data, "extractor_area_mode").is_empty()
            {
                "extractor"
            } else if !text(data, "field_resource").is_empty()
                || !text(data, "field_area_mode").is_empty()
            {
                "farm"
            } else if !matches!(text(data, "service_class"), "" | "none") {
                "service"
            } else {
                "explicit"
            }
        }
        _ => "unsupported",
    }
}

fn utility_service(data: &Value) -> Option<&'static str> {
    match text(data, "service_class") {
        "power" => Some("power"),
        "water" => Some("water"),
        "waste" => Some("sewage"),
        _ => None,
    }
}

/// Test a compiled, executable profile against the asset's current authored purpose.
pub(crate) fn profile_matches(
    data: &Value,
    profile: &EconomyProfileRuntime,
    catalog: &RuntimeEconomyCatalog,
) -> bool {
    if !profile.runtime_supported || profile.worker_capacity == 0 {
        return false;
    }
    match kind(data) {
        "commercial" => matches!(profile.kind, ProfileKind::Store | ProfileKind::ServiceStore),
        "industrial" => matches!(profile.kind, ProfileKind::Producer | ProfileKind::Processor),
        "extractor" | "farm" => {
            let (expected, resource) = if kind(data) == "farm" {
                (ProfileKind::FieldProducer, text(data, "field_resource"))
            } else {
                (ProfileKind::Extractor, text(data, "extractor_resource"))
            };
            profile.kind == expected
                && catalog
                    .resource_runtime_id_for_id(resource)
                    .is_some_and(|id| profile.output_port(id).is_some())
        }
        "service" => utility_service(data).is_some_and(|service| {
            matches!(
                profile.kind,
                ProfileKind::UtilityProducer | ProfileKind::UtilityProcessor
            ) && profile.utility_service.as_deref() == Some(service)
        }),
        _ => false,
    }
}

/// Describe selectable types without advertising unimplemented asset classes.
pub(crate) fn types() -> Value {
    json!({
        "types": TYPES.map(|(id, label)| json!({"id": id, "label": label})),
        "services": SERVICES.map(|(id, label)| json!({"id": id, "label": label})),
    })
}

/// Return applicable fields and compatible profiles; do not normalize or mutate input.
/// Complexity is O(P log P + P log R + ports) for cached profiles/resources, independent of city population.
pub(crate) fn describe(data: &Value, catalog: Option<&RuntimeEconomyCatalog>) -> Value {
    let kind = kind(data);
    let zoned = matches!(kind, "residential" | "commercial" | "industrial");
    let utility = kind == "service" && utility_service(data).is_some();
    let has_profiles =
        matches!(kind, "commercial" | "industrial" | "extractor" | "farm") || utility;
    let mut fields = vec!["lot_width_cells", "lot_depth_cells", "frontage_forward"];
    if zoned {
        fields.extend([
            "density",
            "min_zone_width_cells",
            "min_zone_depth_cells",
            "asset_set",
            "level",
        ]);
    }
    if kind == "residential" {
        fields.extend(["household_capacity", "flat_size_m2"]);
    }
    if kind == "farm" {
        fields.extend(["field_resource", "field_area_mode", "flat_size_m2"]);
    }
    if kind == "extractor" {
        fields.extend(["extractor_resource", "extractor_area_mode"]);
    }
    if kind == "service" {
        fields.push("service_class");
    }
    if has_profiles {
        fields.push("economy_profile");
    }
    let profile_id = text(data, "economy_profile");
    let direct_workers =
        matches!(kind, "commercial" | "industrial" | "explicit") || (kind == "service" && !utility);
    if direct_workers && profile_id.is_empty() {
        fields.push("worker_capacity");
    }
    let mut profiles = Vec::new();
    let mut selected = None;
    if let Some(catalog) = catalog {
        for profile in catalog.all_profiles() {
            let compatible = profile_matches(data, profile, catalog);
            let summary = json!({
                "id": profile.id, "worker_capacity": profile.worker_capacity,
                "workers_per_hectare": profile.workers_per_hectare,
            });
            if compatible {
                profiles.push(summary.clone());
            }
            if profile.id == profile_id {
                selected = Some(json!({"profile": summary, "compatible": compatible}));
            }
        }
    }
    profiles.sort_by(|a, b| text(a, "id").cmp(text(b, "id")));
    json!({
        "kind": kind, "fields": fields, "profiles": profiles,
        "selected_profile": selected, "catalog_available": catalog.is_some(),
        "profile_required": matches!(kind, "extractor" | "farm") || utility,
        "placement": if zoned { "Grows in painted zoning" } else { "Placed explicitly" },
        "service_note": if kind == "service" && !utility {
            "Authors service classification and placement; no additional service simulation settings are available."
        } else { "" },
    })
}

/// Field-addressed authoring diagnostics. All checks are read-only, including invalid old data.
pub(crate) fn issues(
    data: &Value,
    catalog: Option<&RuntimeEconomyCatalog>,
    description: &Value,
) -> Vec<Value> {
    let mut issues = Vec::new();
    let mut issue = |section: &str, field: &str, message: &str| {
        issues.push(
            json!({"section": section, "field": field, "message": message, "severity": "error"}),
        );
    };
    if !super::is_valid_asset_id(text(data, "asset_id")) {
        issue(
            "overview",
            "asset_id",
            "Choose a valid dot-separated asset ID.",
        );
    }
    if !super::is_valid_pack_id(text(data, "pack_id")) {
        issue("overview", "pack_id", "Choose a destination pack.");
    }
    if text(data, "display_name").is_empty() {
        issue("overview", "display_name", "Give the asset a display name.");
    }
    if data["mesh_parts"].as_array().is_none_or(Vec::is_empty) {
        issue("model", "mesh_parts", "Import at least one mesh part.");
    }
    if !data["anchors"].as_array().is_some_and(|anchors| {
        anchors.iter().any(|anchor| {
            text(anchor, "anchor_type") == "entrance" && text(anchor, "name") == "main"
        })
    }) {
        issue(
            "site",
            "anchors",
            "Main entrance missing. Add an entrance named main.",
        );
    }
    if kind(data) == "unsupported" {
        issue(
            "overview",
            "type",
            "This asset contract is not supported by the building editor; its data is preserved.",
        );
    }
    if kind(data) == "residential" && data["household_capacity"].as_f64().unwrap_or(0.0) <= 0.0 {
        issue(
            "gameplay",
            "household_capacity",
            "Set a positive household capacity.",
        );
    }
    let selected = text(data, "economy_profile");
    if selected.is_empty() && description["profile_required"] == true {
        issue(
            "gameplay",
            "economy_profile",
            "Select a compatible economy profile.",
        );
    } else if !selected.is_empty() {
        if catalog.is_none() {
            issue(
                "gameplay",
                "economy_profile",
                "Economy catalog unavailable; the authored profile has been preserved.",
            );
        } else if description["selected_profile"].is_null() {
            issue(
                "gameplay",
                "economy_profile",
                "The authored economy profile is missing; choose a replacement or restore the catalog.",
            );
        } else if description["selected_profile"]["compatible"] != true {
            issue(
                "gameplay",
                "economy_profile",
                "The authored economy profile is incompatible with this asset type; it has not been changed.",
            );
        }
    }
    let incompatible: &[&str] = match kind(data) {
        "residential" => &[
            "worker_capacity",
            "service_class",
            "extractor_resource",
            "field_resource",
        ],
        "commercial" | "industrial" => &["service_class", "extractor_resource", "field_resource"],
        "extractor" => &["zone_type", "density", "service_class", "field_resource"],
        "farm" => &[
            "zone_type",
            "density",
            "service_class",
            "extractor_resource",
        ],
        "service" | "explicit" => &["zone_type", "density"],
        _ => &[],
    };
    for &field in incompatible {
        let present = if field == "worker_capacity" {
            data[field].as_f64().is_some_and(|value| value > 0.0)
        } else {
            !matches!(text(data, field), "" | "none")
        };
        if present {
            issues.push(json!({"section": "validate", "field": field, "severity": "error",
                "message": format!("Preserved {field} conflicts with this asset type. Review before clearing."),
                "replacement": null}));
        }
    }
    for (target, field) in [
        ("farm", "field_area_mode"),
        ("extractor", "extractor_area_mode"),
    ] {
        if kind(data) == target && !matches!(text(data, field), "" | "player_polygon") {
            issues.push(json!({"section": "gameplay", "field": field, "severity": "error",
                "message": format!("{field} must use the player-drawn polygon contract. Review correction."),
                "replacement": "player_polygon"}));
        }
    }
    issues
}

/// Build an explicit, reviewable conversion candidate. The caller must confirm before applying.
/// Only type-owned metadata changes; geometry, identity, tags and unrecognized fields survive.
pub(crate) fn conversion(data: &Value, target: &str, subtype: &str) -> Result<Value, String> {
    if !data.is_object() || !TYPES.iter().any(|(id, _)| *id == target) {
        return Err("Unsupported building authoring type".into());
    }
    if target == "service" && !SERVICES.iter().any(|(id, _)| *id == subtype) {
        return Err("Choose a supported service subtype".into());
    }
    let old_kind = kind(data);
    if old_kind == target && (target != "service" || text(data, "service_class") == subtype) {
        return Ok(json!({"document": data, "changes": []}));
    }
    let mut candidate = data.clone();
    let zoned = matches!(target, "residential" | "commercial" | "industrial");
    for key in [
        "zone_type",
        "density",
        "min_zone_width_cells",
        "min_zone_depth_cells",
        "household_capacity",
        "worker_capacity",
        "flat_size_m2",
        "service_class",
        "economy_profile",
        "extractor_resource",
        "extractor_area_mode",
        "field_resource",
        "field_area_mode",
        "asset_set",
    ] {
        candidate[key] = Value::Null;
    }
    candidate["asset_class"] = json!("building");
    candidate["placement_mode"] = json!(if zoned { "zoned_private" } else { "explicit" });
    candidate["level"] = json!(1);
    if zoned {
        candidate["zone_type"] = json!(target);
        candidate["density"] = json!("low");
    }
    match target {
        "residential" => {
            candidate["household_capacity"] = json!(1);
            candidate["flat_size_m2"] = json!(60.0);
        }
        "commercial" | "industrial" => candidate["worker_capacity"] = json!(1),
        "extractor" => {
            candidate["extractor_resource"] = json!("coal");
            candidate["extractor_area_mode"] = json!("player_polygon");
        }
        "farm" => {
            candidate["field_resource"] = json!("grain");
            candidate["field_area_mode"] = json!("player_polygon");
            candidate["household_capacity"] = json!(1);
            candidate["flat_size_m2"] =
                json!(super::asset::BuildingData::DEFAULT_FARMHOUSE_AREA_M2);
        }
        "service" => candidate["service_class"] = json!(subtype),
        _ => {}
    }
    let changes: Vec<_> = candidate
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, value)| data[*key] != **value)
        .map(|(key, value)| json!({"field": key, "before": data[key], "after": value}))
        .collect();
    Ok(json!({"document": candidate, "changes": changes}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::economy::definitions::load_runtime_economy_catalog;

    #[test]
    fn authoring_lod_bands_reuse_manifest_validation() {
        let lod = |min, max| super::super::LodEntry {
            file: "model.glb".into(),
            distance_min_m: min,
            distance_max_m: max,
        };
        assert!(lod_chain_error(&[lod(0.0, Some(35.0)), lod(35.0, None)]).is_ok());
        assert!(lod_chain_error(&[]).is_err());
        assert!(lod_chain_error(&[lod(1.0, None)]).is_err());
        assert!(lod_chain_error(&[lod(0.0, Some(35.0)), lod(12.0, None)]).is_err());
        assert!(lod_chain_error(&[lod(0.0, Some(f32::INFINITY))]).is_err());
    }

    fn preset(target: &str, subtype: &str) -> Value {
        conversion(
            &json!({"asset_class": "building", "mesh_parts": [{"name": "keep"}]}),
            target,
            subtype,
        )
        .unwrap()["document"]
            .clone()
    }

    #[test]
    fn every_preset_is_inferred_from_contract_not_name() {
        for (target, _) in TYPES {
            let mut document = preset(target, "power");
            document["display_name"] = json!("Coal mine");
            document["asset_id"] = json!("building.residential.fake");
            assert_eq!(kind(&document), target);
        }
        assert_eq!(kind(&json!({"asset_class": "vehicle"})), "unsupported");
    }

    #[test]
    fn housing_never_exposes_unrelated_gameplay_or_profiles() {
        let document = preset("residential", "");
        let description = describe(&document, None);
        let fields = description["fields"].as_array().unwrap();
        for forbidden in [
            "worker_capacity",
            "economy_profile",
            "service_class",
            "extractor_resource",
            "field_resource",
        ] {
            assert!(!fields.contains(&json!(forbidden)));
        }
        assert!(fields.contains(&json!("household_capacity")));
    }

    #[test]
    fn editing_resource_does_not_change_authoring_type() {
        for (target, field) in [
            ("farm", "field_resource"),
            ("extractor", "extractor_resource"),
        ] {
            let mut document = preset(target, "");
            document[field] = json!("");
            assert_eq!(kind(&document), target);
            assert!(
                describe(&document, None)["fields"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(field))
            );
        }
    }

    #[test]
    fn catalog_filtering_and_worker_ownership_are_type_specific() {
        let catalog = load_runtime_economy_catalog().unwrap();
        for (target, expected) in [
            ("commercial", "grocery_basic"),
            ("industrial", "food_processor_basic"),
            ("extractor", "coal_mine_basic"),
            ("farm", "grain_farm_basic"),
            ("service", "power_plant_basic"),
        ] {
            let mut document = preset(target, "power");
            let description = describe(&document, Some(&catalog));
            let profiles = description["profiles"].as_array().unwrap();
            assert!(
                profiles.iter().any(|p| p["id"] == expected),
                "{target}: {profiles:?}"
            );
            if target != "extractor" {
                assert!(!profiles.iter().any(|p| p["id"] == "coal_mine_basic"));
            }
            document["economy_profile"] = json!(expected);
            let description = describe(&document, Some(&catalog));
            assert!(
                !description["fields"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("worker_capacity"))
            );
            assert_eq!(description["selected_profile"]["compatible"], true);
        }
    }

    #[test]
    fn diagnostics_point_to_the_task_and_never_modify_data() {
        let mut data = preset("residential", "");
        data["household_capacity"] = json!(0);
        let original = data.clone();
        let issues = issues(&data, None, &describe(&data, None));
        assert!(
            issues
                .iter()
                .any(|issue| issue["section"] == "site" && issue["field"] == "anchors")
        );
        assert!(
            issues
                .iter()
                .any(|issue| issue["section"] == "gameplay"
                    && issue["field"] == "household_capacity")
        );
        assert_eq!(data, original);
    }

    #[test]
    fn missing_or_incompatible_profile_remains_in_document() {
        let catalog = load_runtime_economy_catalog().unwrap();
        let mut document = preset("commercial", "");
        document["economy_profile"] = json!("coal_mine_basic");
        let original = document.clone();
        assert_eq!(
            describe(&document, Some(&catalog))["selected_profile"]["compatible"],
            false
        );
        assert_eq!(document, original);
        document["economy_profile"] = json!("missing");
        assert!(describe(&document, Some(&catalog))["selected_profile"].is_null());
        assert_eq!(document["economy_profile"], "missing");
    }

    #[test]
    fn conversion_is_explicit_loss_report_and_preserves_non_type_data() {
        let mut document = preset("residential", "");
        document["asset_set"] = json!("family");
        document["future_metadata"] = json!({"preserve": true});
        let original = document.clone();
        let result = conversion(&document, "commercial", "").unwrap();
        assert_eq!(document, original);
        assert_eq!(result["document"]["mesh_parts"], original["mesh_parts"]);
        assert_eq!(
            result["document"]["future_metadata"],
            original["future_metadata"]
        );
        assert!(
            result["changes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|change| change["field"] == "asset_set"
                    && change["before"] == "family"
                    && change["after"].is_null())
        );
        assert_eq!(
            conversion(&document, "residential", "").unwrap()["document"],
            original
        );
        assert!(conversion(&document, "road", "").is_err());
        assert!(conversion(&document, "service", "invented").is_err());
    }
}
