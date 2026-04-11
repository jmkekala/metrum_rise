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
    /// Primary store: qualified_id → entry.
    entries: HashMap<String, AssetEntry>,
    /// Secondary index: `by_zone[ZoneClass as usize]` = sorted list of qualified_ids for
    /// zoned-private building assets of that zone type. Sorted for deterministic placement selection.
    by_zone: [Vec<String>; 5],
    /// Secondary index: `(zone_type, density)` → sorted list of qualified_ids.
    by_zone_density: HashMap<(ZoneClass, String), Vec<String>>,
    /// Upgrade index: `(asset_set, level) → qualified_id`.
    /// Lets the runtime find the next-tier asset without forward pointers in the manifest.
    upgrade_index: HashMap<(String, u8), String>,
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
    /// The manifest must already be validated (produced by [`AssetManifest::from_str`]).
    /// `asset_dir` is the native filesystem path to the directory containing `asset.toml`.
    pub fn register(&mut self, pack_id: &str, manifest: AssetManifest, asset_dir: String) {
        let qid = manifest.qualified_id(pack_id);

        if let Some(bd) = &manifest.building {
            if bd.placement_mode != PlacementMode::ZonedPrivate {
                self.entries.insert(
                    qid,
                    AssetEntry {
                        manifest,
                        pack_id: pack_id.to_owned(),
                        asset_dir,
                    },
                );
                return;
            }
            // Zone placement index.
            let Some(zone_type) = bd.zone_type else {
                self.entries.insert(
                    qid,
                    AssetEntry {
                        manifest,
                        pack_id: pack_id.to_owned(),
                        asset_dir,
                    },
                );
                return;
            };
            let zi = zone_type as usize;
            if zi < self.by_zone.len() {
                let list = &mut self.by_zone[zi];
                if !list.contains(&qid) {
                    list.push(qid.clone());
                    list.sort_unstable();
                }
            }
            let density_key = (zone_type, bd.density_key().unwrap_or("low").to_owned());
            let density_list = self.by_zone_density.entry(density_key).or_default();
            if !density_list.contains(&qid) {
                density_list.push(qid.clone());
                density_list.sort_unstable();
            }

            // Upgrade index — only populated when the asset belongs to a named family.
            if let Some(set) = &manifest.asset_set {
                let key = (set.clone(), bd.level);
                if let Some(prev) = self.upgrade_index.get(&key) {
                    if prev != &qid {
                        // Two assets claim the same family slot — last registration wins.
                        eprintln!(
                            "asset registry: conflict in family '{}' at level {}: \
                             '{}' replaced by '{}'",
                            set, bd.level, prev, qid
                        );
                    }
                }
                self.upgrade_index.insert(key, qid.clone());
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
            .get(&(asset_set.to_owned(), level + 1))
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
            .get(&(asset_set.to_owned(), level - 1))
            .map(String::as_str)
    }

    /// Returns the residential capacity declared by a building asset's manifest.
    ///
    /// Returns `0` if the asset is not a building or has no declared residential capacity.
    pub fn resident_capacity(&self, qualified_id: &str) -> u32 {
        self.entries
            .get(qualified_id)
            .and_then(|entry| entry.manifest.building.as_ref())
            .and_then(|building| building.residents_capacity)
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

    /// Returns the occupant capacity declared by a building asset's manifest.
    ///
    /// For residential assets this is `residents_capacity`; for commercial/industrial/office
    /// assets this is `worker_capacity`; for mixed assets this is the sum of both.
    /// Returns `0` if the asset is not a building or has no declared capacity.
    pub fn capacity(&self, qualified_id: &str) -> u32 {
        self.resident_capacity(qualified_id) + self.worker_capacity(qualified_id)
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
        let zi = zone_class as usize;
        if zi < self.by_zone.len() {
            &self.by_zone[zi]
        } else {
            &[]
        }
    }

    /// Returns all qualified IDs for building assets of one `(zone_type, density)` pair.
    pub fn buildings_for_zone_density(&self, zone_class: ZoneClass, density: &str) -> &[String] {
        self.by_zone_density
            .get(&(zone_class, density.to_owned()))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Returns the lot footprint in zoning grid cells `(width, depth)` for an asset.
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
        self.entries.clear();
        for list in &mut self.by_zone {
            list.clear();
        }
        self.by_zone_density.clear();
        self.upgrade_index.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::AssetManifest;
    use crate::assets::asset::{BuildingData, LodEntry, PlacementMode, ZoneClass};

    fn make_building_manifest(asset_id: &str, zone: ZoneClass, w: u16, d: u16) -> AssetManifest {
        let (residents_capacity, worker_capacity) = match zone {
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
            lods: vec![LodEntry {
                file: "lod0.glb".to_owned(),
                distance_min_m: 0.0,
                distance_max_m: None,
            }],
            anchors: vec![],
            building: Some(BuildingData {
                placement_mode: PlacementMode::ZonedPrivate,
                zone_type: Some(zone),
                density: Some("low".to_owned()),
                lot_width_cells: w,
                lot_depth_cells: d,
                min_zone_width_cells: None,
                min_zone_depth_cells: None,
                level: 1,
                residents_capacity,
                worker_capacity,
                service_class: None,
                economy_profile: None,
                preview_scale: None,
            }),
            prop: None,
            vehicle: None,
            character: None,
            pivot_offset: None,
        }
    }

    #[test]
    fn register_and_get() {
        let mut reg = AssetRegistry::new();
        let m = make_building_manifest("building.residential.house", ZoneClass::Residential, 3, 2);
        reg.register("base", m.clone(), String::new());

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
