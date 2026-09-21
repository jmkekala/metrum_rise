// SPDX-License-Identifier: GPL-2.0-only

//! Asset registry: runtime catalogue of all loaded content-pack assets.
//!
//! The registry is the single source of truth for simulation-facing asset identity.
//! Godot owns mesh/texture/material loading; Rust owns manifest parsing, validation,
//! footprint dimensions, and stable qualified IDs.
//!
//! Primary key is `"pack_id:asset_id"`. Secondary indices cover:
//! - zone-type placement: all zoned-private building assets for a given zone
//! - family upgrade chains: `(asset_set, level + 1) → qualified_id` for in-place upgrades

use crate::assets::asset::{AssetManifest, PlacementMode, ZoneClass};
use std::collections::HashMap;

/// A single registered asset, combining its manifest with its pack origin.
#[derive(Debug, Clone)]
pub struct AssetEntry {
    /// Fully parsed and validated asset manifest.
    pub manifest: AssetManifest,
    /// Pack that owns this asset (`pack_id` from `pack.toml`).
    pub pack_id: String,
    /// Native filesystem path to the directory containing `asset.toml`.
    /// Used to resolve relative LOD and texture paths at runtime.
    pub asset_dir: String,
}

/// Runtime catalogue of all loaded content-pack assets.
///
/// Build the registry at startup by scanning pack directories and calling [`AssetRegistry::register`]
/// for each successfully parsed [`AssetManifest`]. All simulation systems that need asset
/// dimensions or identity should read from a shared registry rather than maintaining their own maps.
#[derive(Debug, Default, Clone)]
pub struct AssetRegistry {
    revision: u64,
    /// Primary store: qualified_id → entry.
    entries: HashMap<String, AssetEntry>,
    /// Secondary index: `by_zone[ZoneClass as usize]` = sorted list of qualified_ids for
    /// zoned-private building assets of that zone type. Sorted for deterministic placement selection.
    by_zone: [Vec<String>; 5],
    /// Density buckets per zone, queried with borrowed density strings.
    by_zone_density: [HashMap<String, Vec<String>>; 5],
    /// Level → family → qualified_id. Direct tier indexing avoids a second hash lookup.
    /// At most 256 buckets, allocated only through the highest registered tier.
    upgrade_index: Vec<HashMap<String, String>>,
}

impl AssetRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one asset.
    ///
    /// Replaces any existing entry with the same qualified ID. Updates the zone placement
    /// index and, for buildings with an `asset_set`, the upgrade index. If a different
    /// asset is already registered at the same `(asset_set, level)` pair a warning is
    /// printed and the new registration wins.
    ///
    /// The manifest must already be validated through [`AssetManifest`] parsing.
    /// `asset_dir` is the native filesystem path to the directory containing `asset.toml`.
    pub fn register(&mut self, pack_id: &str, manifest: AssetManifest, asset_dir: String) {
        self.revision = self.revision.wrapping_add(1);
        let qid = manifest.qualified_id(pack_id);

        // Replacement must remove the old classification before indexing the new manifest.
        // Only the old zone/density buckets are visited, never the whole registry.
        if let Some(old) = self.entries.get(&qid)
            && let Some(building) = &old.manifest.building
        {
            if let Some(zone) = building.zone_type {
                self.by_zone[zone as usize].retain(|id| id != &qid);
                let densities = &mut self.by_zone_density[zone as usize];
                let density = building.density_key().unwrap_or("low");
                if let Some(ids) = densities.get_mut(density) {
                    ids.retain(|id| id != &qid);
                    if ids.is_empty() {
                        densities.remove(density);
                    }
                }
            }
            if let Some(set) = &old.manifest.asset_set
                && let Some(families) = self.upgrade_index.get_mut(usize::from(building.level))
            {
                if families.get(set) == Some(&qid) {
                    families.remove(set);
                }
                while self.upgrade_index.last().is_some_and(HashMap::is_empty) {
                    self.upgrade_index.pop();
                }
            }
        }

        if let Some(bd) = &manifest.building
            && bd.placement_mode == PlacementMode::ZonedPrivate
            && let Some(zone_type) = bd.zone_type
        {
            // Zone placement index.
            let list = &mut self.by_zone[zone_type as usize];
            if !list.contains(&qid) {
                list.push(qid.clone());
                list.sort_unstable();
            }
            let density_list = self.by_zone_density[zone_type as usize]
                .entry(bd.density_key().unwrap_or("low").to_owned())
                .or_default();
            if !density_list.contains(&qid) {
                density_list.push(qid.clone());
                density_list.sort_unstable();
            }

            // Upgrade index — only populated when the asset belongs to a named family.
            if let Some(set) = &manifest.asset_set {
                let level = usize::from(bd.level);
                if self.upgrade_index.len() <= level {
                    self.upgrade_index.resize_with(level + 1, HashMap::new);
                }
                let families = &mut self.upgrade_index[level];
                if let Some(prev) = families.get(set) {
                    if prev != &qid {
                        // Two assets claim the same family slot — last registration wins.
                        eprintln!(
                            "asset registry: conflict in family '{}' at level {}: \
                             '{}' replaced by '{}'",
                            set, bd.level, prev, qid
                        );
                    }
                }
                families.insert(set.clone(), qid.clone());
            }
        }

        self.entries.insert(
            qid,
            AssetEntry {
                manifest,
                pack_id: pack_id.to_owned(),
                asset_dir,
            },
        );
    }

    /// Returns the qualified ID of the next upgrade tier for the given asset, or `None`.
    ///
    /// Upgrade is possible when the asset has an `asset_set` and a building with
    /// `level + 1` in the same family is registered.
    pub fn next_level(&self, qualified_id: &str) -> Option<&str> {
        let entry = self.entries.get(qualified_id)?;
        let asset_set = entry.manifest.asset_set.as_deref()?;
        let level = entry.manifest.building.as_ref()?.level;
        self.upgrade_index
            .get(usize::from(level.checked_add(1)?))?
            .get(asset_set)
            .map(String::as_str)
    }

    /// Returns the qualified ID of the previous downgrade tier for the given asset, or `None`.
    ///
    /// Downgrade is possible when the asset has an `asset_set` and a building with
    /// `level - 1` in the same family is registered.
    pub fn prev_level(&self, qualified_id: &str) -> Option<&str> {
        let entry = self.entries.get(qualified_id)?;
        let asset_set = entry.manifest.asset_set.as_deref()?;
        let level = entry.manifest.building.as_ref()?.level;
        if level <= 1 {
            return None;
        }
        self.upgrade_index
            .get(usize::from(level - 1))?
            .get(asset_set)
            .map(String::as_str)
    }

    /// Returns one household slot for farms, otherwise the manifest's household capacity.
    ///
    /// Other buildings without declared housing, and unresolved assets, return zero.
    pub fn household_capacity(&self, qualified_id: &str) -> u32 {
        self.entries
            .get(qualified_id)
            .and_then(|entry| entry.manifest.building.as_ref())
            .map(|building| building.effective_household_capacity())
            .unwrap_or(0)
    }

    /// Returns the worker capacity declared by a building asset's manifest.
    ///
    /// Returns `0` if the asset is not a building or has no declared worker capacity.
    pub fn worker_capacity(&self, qualified_id: &str) -> u32 {
        self.entries
            .get(qualified_id)
            .and_then(|entry| entry.manifest.building.as_ref())
            .and_then(|building| building.worker_capacity)
            .unwrap_or(0)
    }

    /// Returns the authored economy-profile reference declared by a building asset.
    pub fn economy_profile(&self, qualified_id: &str) -> Option<&str> {
        self.entries
            .get(qualified_id)
            .and_then(|entry| entry.manifest.building.as_ref())
            .and_then(|building| building.economy_profile.as_deref())
    }

    /// Returns the authored service class for an explicit service building asset.
    pub fn service_class(&self, qualified_id: &str) -> Option<&str> {
        self.entries
            .get(qualified_id)
            .and_then(|entry| entry.manifest.building.as_ref())
            .and_then(|building| building.service_class.as_deref())
            .filter(|service_class| {
                let service_class = service_class.trim();
                !service_class.is_empty() && service_class != "none"
            })
    }

    /// Returns the authored extractor resource id for an explicit industry building asset.
    pub fn extractor_resource(&self, qualified_id: &str) -> Option<&str> {
        self.entries
            .get(qualified_id)
            .and_then(|entry| entry.manifest.building.as_ref())
            .and_then(|building| building.extractor.as_ref())
            .map(|extractor| extractor.resource.trim())
            .filter(|resource| !resource.is_empty())
    }

    /// Returns the authored field resource id for an explicit agricultural building asset.
    pub fn field_resource(&self, qualified_id: &str) -> Option<&str> {
        self.entries
            .get(qualified_id)
            .and_then(|entry| entry.manifest.building.as_ref())
            .and_then(|building| building.field.as_ref())
            .map(|field| field.resource.trim())
            .filter(|resource| !resource.is_empty())
    }

    /// Returns whether the asset is an explicitly placed resource extractor.
    pub fn is_resource_extractor_asset(&self, qualified_id: &str) -> bool {
        self.entries
            .get(qualified_id)
            .and_then(|entry| entry.manifest.building.as_ref())
            .is_some_and(|building| {
                building.placement_mode == PlacementMode::Explicit
                    && self.extractor_resource(qualified_id).is_some()
            })
    }

    /// Returns whether the asset is an explicitly placed field producer.
    pub fn is_field_producer_asset(&self, qualified_id: &str) -> bool {
        self.entries
            .get(qualified_id)
            .and_then(|entry| entry.manifest.building.as_ref())
            .is_some_and(|building| building.is_field_producer())
    }

    /// Returns whether the asset is an explicit industry building that owns a player area.
    pub fn is_industry_area_asset(&self, qualified_id: &str) -> bool {
        self.is_resource_extractor_asset(qualified_id) || self.is_field_producer_asset(qualified_id)
    }

    /// Returns whether the asset is an explicitly placed city-service building.
    pub fn is_city_service_asset(&self, qualified_id: &str) -> bool {
        self.entries
            .get(qualified_id)
            .and_then(|entry| entry.manifest.building.as_ref())
            .is_some_and(|building| {
                building.placement_mode == PlacementMode::Explicit
                    && self.service_class(qualified_id).is_some()
            })
    }

    /// Returns floor area per household; farms default to a 120 m² farmhouse when unspecified.
    pub fn flat_size_m2(&self, qualified_id: &str) -> f32 {
        self.entries
            .get(qualified_id)
            .and_then(|entry| entry.manifest.building.as_ref())
            .map(|building| building.effective_flat_size_m2())
            .unwrap_or(0.0)
    }

    /// Returns the entry for a qualified ID, or `None` if not registered.
    pub fn get(&self, qualified_id: &str) -> Option<&AssetEntry> {
        self.entries.get(qualified_id)
    }

    /// Returns all qualified IDs for building assets of the given zone type.
    ///
    /// `zone_class` is the [`ZoneClass`] discriminant (0 = Residential … 4 = Mixed).
    /// Returns an empty slice if the zone has no registered building assets.
    pub fn buildings_for_zone(&self, zone_class: ZoneClass) -> &[String] {
        &self.by_zone[zone_class as usize]
    }

    /// Returns all qualified IDs for building assets of one `(zone_type, density)` pair.
    pub fn buildings_for_zone_density(&self, zone_class: ZoneClass, density: &str) -> &[String] {
        self.by_zone_density[zone_class as usize]
            .get(density)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Returns the lot footprint in zoning cells `(width, depth)` for an asset.
    ///
    /// Falls back to `(1, 1)` if the asset is unknown or is not a building.
    pub fn lot_size(&self, qualified_id: &str) -> (usize, usize) {
        if let Some(entry) = self.entries.get(qualified_id) {
            if let Some(bd) = &entry.manifest.building {
                return (bd.lot_width_cells as usize, bd.lot_depth_cells as usize);
            }
        }
        (1, 1)
    }

    /// Returns `true` if no assets have been registered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Total number of registered assets across all packs.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Iterates all qualified IDs currently in the registry.
    pub fn qualified_ids(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// Removes all entries and clears all secondary indices.
    pub fn clear(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.entries.clear();
        for list in &mut self.by_zone {
            list.clear();
        }
        for densities in &mut self.by_zone_density {
            densities.clear();
        }
        self.upgrade_index.clear();
    }

    /// Revision of asset registration/removal, including replacement of an existing manifest.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::AssetManifest;
    use crate::assets::asset::{
        Anchor, AnchorType, BuildingData, MeshPart, PlacementMode, ZoneClass,
    };

    fn make_building_manifest(asset_id: &str, zone: ZoneClass, w: u16, d: u16) -> AssetManifest {
        let (household_capacity, worker_capacity) = match zone {
            ZoneClass::Residential => (Some(6), None),
            ZoneClass::Commercial | ZoneClass::Industrial | ZoneClass::Office => (None, Some(4)),
            ZoneClass::Mixed => (Some(4), Some(2)),
        };
        AssetManifest {
            asset_id: asset_id.to_owned(),
            display_name: "Test Building".to_owned(),
            asset_set: None,
            tags: vec![],
            thumbnail: None,
            lods: vec![],
            mesh_parts: vec![MeshPart::single_lod0("main", "lod0.glb")],
            anchors: vec![Anchor {
                anchor_type: AnchorType::Entrance,
                name: "main".to_owned(),
                position: [0.0; 3],
                forward: [0.0, 0.0, 1.0],
                width_m: None,
                length_m: None,
                vehicle_class: None,
            }],
            site_surfaces: vec![],
            building: Some(BuildingData {
                appearance: None,
                flat_size_m2: None,
                placement_mode: PlacementMode::ZonedPrivate,
                zone_type: Some(zone),
                density: Some("low".to_owned()),
                lot_width_cells: w,
                lot_depth_cells: d,
                frontage_forward: None,
                min_zone_width_cells: None,
                min_zone_depth_cells: None,
                level: 1,
                household_capacity,
                worker_capacity,
                service_class: None,
                economy_profile: None,
                extractor: None,
                field: None,
            }),
            prop: None,
            vehicle: None,
            character: None,
        }
    }

    #[test]
    fn register_and_get() {
        let mut reg = AssetRegistry::new();
        let m = make_building_manifest("building.residential.house", ZoneClass::Residential, 3, 2);
        reg.register("base", m, String::new());

        let entry = reg
            .get("base:building.residential.house")
            .expect("should be registered");
        assert_eq!(entry.pack_id, "base");
        assert_eq!(entry.manifest.asset_id, "building.residential.house");
    }

    #[test]
    fn lot_size_returns_manifest_dimensions() {
        let mut reg = AssetRegistry::new();
        reg.register(
            "base",
            make_building_manifest("b.res.big", ZoneClass::Residential, 5, 4),
            String::new(),
        );
        assert_eq!(reg.lot_size("base:b.res.big"), (5, 4));
    }

    #[test]
    fn lot_size_falls_back_to_one_one_for_unknown() {
        let reg = AssetRegistry::new();
        assert_eq!(reg.lot_size("nonexistent:asset"), (1, 1));
    }

    #[test]
    fn buildings_for_zone_returns_sorted_ids() {
        let mut reg = AssetRegistry::new();
        reg.register(
            "base",
            make_building_manifest("b.res.c", ZoneClass::Residential, 1, 1),
            String::new(),
        );
        reg.register(
            "base",
            make_building_manifest("b.res.a", ZoneClass::Residential, 2, 1),
            String::new(),
        );
        reg.register(
            "base",
            make_building_manifest("b.res.b", ZoneClass::Residential, 1, 2),
            String::new(),
        );
        reg.register(
            "base",
            make_building_manifest("b.com.x", ZoneClass::Commercial, 1, 1),
            String::new(),
        );

        let res = reg.buildings_for_zone(ZoneClass::Residential);
        assert_eq!(res, ["base:b.res.a", "base:b.res.b", "base:b.res.c"]);

        let com = reg.buildings_for_zone(ZoneClass::Commercial);
        assert_eq!(com, ["base:b.com.x"]);

        // Zone with no assets returns empty.
        assert!(reg.buildings_for_zone(ZoneClass::Industrial).is_empty());
    }

    #[test]
    fn register_replaces_existing_entry() {
        let mut reg = AssetRegistry::new();
        reg.register(
            "base",
            make_building_manifest("b.res.house", ZoneClass::Residential, 1, 1),
            String::new(),
        );
        reg.register(
            "base",
            make_building_manifest("b.res.house", ZoneClass::Residential, 4, 3),
            String::new(),
        );
        // Only one entry, no duplicates in zone list.
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.buildings_for_zone(ZoneClass::Residential).len(), 1);
        assert_eq!(reg.lot_size("base:b.res.house"), (4, 3));
    }

    #[test]
    fn replacement_removes_old_zone_density_and_family_membership() {
        let mut reg = AssetRegistry::new();
        let mut house = make_building_manifest("building", ZoneClass::Residential, 2, 2);
        house.asset_set = Some("old".to_owned());
        reg.register("base", house.clone(), String::new());
        let revision = reg.revision();
        let mut replacement = house;
        replacement.asset_set = Some("new".to_owned());
        let building = replacement.building.as_mut().unwrap();
        building.zone_type = Some(ZoneClass::Commercial);
        building.density = Some("high".to_owned());
        building.level = 2;
        reg.register("base", replacement.clone(), String::new());
        assert_ne!(reg.revision(), revision);
        assert!(reg.buildings_for_zone(ZoneClass::Residential).is_empty());
        assert!(
            reg.buildings_for_zone_density(ZoneClass::Residential, "low")
                .is_empty()
        );
        assert!(
            reg.upgrade_index
                .iter()
                .all(|families| !families.contains_key("old"))
        );
        assert_eq!(
            reg.buildings_for_zone_density(ZoneClass::Commercial, "high"),
            ["base:building"]
        );
        assert_eq!(
            reg.upgrade_index
                .get(2)
                .and_then(|families| families.get("new"))
                .map(String::as_str),
            Some("base:building")
        );

        replacement.building = None;
        reg.register("base", replacement, String::new());
        assert!(reg.buildings_for_zone(ZoneClass::Commercial).is_empty());
        assert!(
            reg.buildings_for_zone_density(ZoneClass::Commercial, "high")
                .is_empty()
        );
        assert!(reg.upgrade_index.is_empty());
    }

    #[test]
    fn len_and_is_empty() {
        let mut reg = AssetRegistry::new();
        assert!(reg.is_empty());
        reg.register(
            "base",
            make_building_manifest("b.res.x", ZoneClass::Residential, 1, 1),
            String::new(),
        );
        assert!(!reg.is_empty());
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn upgrade_chain_handles_endpoints_and_replaced_family_slots() {
        let mut reg = AssetRegistry::new();
        for level in [1, 2, 254, 255] {
            let mut manifest =
                make_building_manifest(&format!("tier_{level}"), ZoneClass::Residential, 2, 2);
            manifest.asset_set = Some("family".to_owned());
            manifest.building.as_mut().unwrap().level = level;
            manifest.validate().unwrap();
            reg.register("base", manifest, String::new());
        }
        assert_eq!(reg.next_level("base:tier_1"), Some("base:tier_2"));
        assert_eq!(reg.prev_level("base:tier_2"), Some("base:tier_1"));
        assert_eq!(reg.prev_level("base:tier_1"), None);
        assert_eq!(reg.next_level("base:tier_2"), None);
        assert_eq!(reg.next_level("base:tier_254"), Some("base:tier_255"));
        assert_eq!(reg.prev_level("base:tier_255"), Some("base:tier_254"));
        assert_eq!(reg.next_level("base:tier_255"), None);

        let mut alternative = reg.get("base:tier_2").unwrap().manifest.clone();
        alternative.asset_id = "alternative".to_owned();
        reg.register("base", alternative, String::new());
        let mut replaced = reg.get("base:tier_2").unwrap().manifest.clone();
        replaced.asset_set = Some("other".to_owned());
        reg.register("base", replaced, String::new());
        assert_eq!(reg.next_level("base:tier_1"), Some("base:alternative"));
        assert_eq!(reg.prev_level("base:tier_2"), None);
    }

    #[test]
    #[ignore = "isolated release benchmark for allocation-free registry lookups"]
    fn benchmark_asset_registry_lookups() {
        use std::hint::black_box;
        use std::time::Instant;

        for family_count in [128, 2048] {
            let mut registry = AssetRegistry::new();
            let mut queries = Vec::with_capacity(family_count);
            for family in 0..family_count {
                for level in [1, 2] {
                    let mut manifest = make_building_manifest(
                        &format!("family_{family}_tier_{level}"),
                        ZoneClass::Residential,
                        2,
                        2,
                    );
                    manifest.asset_set = Some(format!("family-{family}"));
                    manifest.building.as_mut().unwrap().level = level;
                    manifest.validate().unwrap();
                    registry.register("base", manifest, String::new());
                }
                queries.push((
                    format!("base:family_{family}_tier_1"),
                    format!("base:family_{family}_tier_2"),
                ));
            }
            for (low, high) in &queries {
                assert_eq!(registry.next_level(low), Some(high.as_str()));
                assert_eq!(registry.prev_level(high), Some(low.as_str()));
            }
            let ids = registry.buildings_for_zone_density(ZoneClass::Residential, "low");
            assert_eq!(ids.len(), family_count * 2);
            assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));

            let mut samples = Vec::with_capacity(11);
            for sample in 0..14 {
                let start = Instant::now();
                for _ in 0..64 {
                    for (low, high) in &queries {
                        black_box(registry.next_level(black_box(low)));
                        black_box(registry.prev_level(black_box(high)));
                        black_box(registry.buildings_for_zone_density(
                            black_box(ZoneClass::Residential),
                            black_box("low"),
                        ));
                    }
                }
                let ns_per_triplet =
                    start.elapsed().as_secs_f64() * 1.0e9 / (queries.len() * 64) as f64;
                if sample >= 3 {
                    samples.push(ns_per_triplet);
                }
            }
            samples.sort_by(f64::total_cmp);
            eprintln!(
                "asset_registry_lookup families={family_count} assets={} median_ns_per_triplet={:.3}",
                registry.len(),
                samples[samples.len() / 2]
            );
        }
    }

    #[test]
    fn clear_empties_registry_and_indices() {
        let mut reg = AssetRegistry::new();
        reg.register(
            "base",
            make_building_manifest("b.res.x", ZoneClass::Residential, 1, 1),
            String::new(),
        );
        reg.clear();
        assert!(reg.is_empty());
        assert!(reg.buildings_for_zone(ZoneClass::Residential).is_empty());
    }
}
