// SPDX-License-Identifier: GPL-3.0-or-later

//! Godot-Rust bridge helpers for asset management and content registries.

mod mesh_bounds;

use crate::assets::asset::ZoneClass;
use crate::assets::{AssetManifest, ScanResult, scan_pack_dir};
use crate::debug_log;
use crate::nodes::sim::core::SimCore;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::definitions::load_runtime_economy_catalog;
use crate::simulation::zoning::load_builtin_profile_registry;
use godot::prelude::*;
use std::path::Path;

/// Replaces the registry with selected packs, or all packs when no selection is supplied.
pub fn load_asset_packs(
    core: &mut SimCore,
    dir_path: GString,
    enabled_pack_ids: Option<&PackedStringArray>,
) -> GString {
    let mut result = scan_enabled_packs(&dir_path, enabled_pack_ids);
    let mut bounds_cache = mesh_bounds::MeshBoundsCache::new();
    let mut entries = Vec::new();
    for pack in result.packs {
        for (mut asset, asset_dir) in pack.assets {
            if let Err(error) =
                mesh_bounds::import_building_bounds(&mut asset, &asset_dir, &mut bounds_cache)
            {
                result.warnings.push(format!(
                    "skipping '{}': {error}",
                    asset.qualified_id(&pack.pack.pack_id)
                ));
                continue;
            }
            entries.push((pack.pack.pack_id.clone(), asset, asset_dir));
        }
    }
    core.replace_asset_registry(entries);
    let reg = &core.allocator.registry;
    debug_log!(
        "spawn",
        "asset_packs loaded: total={} \
         res_low={} com_low={} ind_low={} \
         res_med={} com_med={} ind_med={}",
        reg.len(),
        reg.buildings_for_zone_density(ZoneClass::Residential, "low")
            .len(),
        reg.buildings_for_zone_density(ZoneClass::Commercial, "low")
            .len(),
        reg.buildings_for_zone_density(ZoneClass::Industrial, "low")
            .len(),
        reg.buildings_for_zone_density(ZoneClass::Residential, "medium")
            .len(),
        reg.buildings_for_zone_density(ZoneClass::Commercial, "medium")
            .len(),
        reg.buildings_for_zone_density(ZoneClass::Industrial, "medium")
            .len(),
    );
    GString::from(result.warnings.join("\n").as_str())
}

/// Import one asset's `LOD0` bounds into its parts, as a pack load does (the acceptance check
/// of `TOOLS-33`). Main thread only: Godot's importers run here.
pub(crate) fn import_asset_bounds(
    manifest: &mut AssetManifest,
    asset_dir: &Path,
) -> Result<(), String> {
    mesh_bounds::import_building_bounds(
        manifest,
        &asset_dir.to_string_lossy(),
        &mut mesh_bounds::MeshBoundsCache::new(),
    )
}

// The packs a load registers: every pack without a selection, none for an empty one.
fn scan_enabled_packs(
    dir_path: &GString,
    enabled_pack_ids: Option<&PackedStringArray>,
) -> ScanResult {
    if enabled_pack_ids.is_some_and(PackedStringArray::is_empty) {
        return ScanResult::default();
    }
    let mut result = scan_pack_dir(Path::new(&dir_path.to_string()));
    if let Some(ids) = enabled_pack_ids {
        result
            .packs
            .retain(|pack| ids.contains(pack.pack.pack_id.as_str()));
    }
    result
}

/// Whether the selected packs in `dir_path` stock any zoning profile, by the allocator's spawn
/// filter, without a world: the new-game dialog's no-building-pack notice. One manifest scan
/// (`O(packs + assets)` file reads) plus the filter; the meshes are not imported, so an asset
/// whose mesh a load would skip still counts here.
pub fn enabled_packs_stock_zones(dir_path: GString, enabled_pack_ids: &PackedStringArray) -> bool {
    // A throwaway allocator: the filter reads only its registry.
    let mut allocator = BuildingAllocator::new();
    for pack in scan_enabled_packs(&dir_path, Some(enabled_pack_ids)).packs {
        for (asset, asset_dir) in pack.assets {
            allocator
                .registry
                .register(&pack.pack.pack_id, asset, asset_dir);
        }
    }
    let profiles = load_builtin_profile_registry()
        .unwrap_or_else(|err| panic!("could not load built-in zoning profiles: {err}"));
    let catalog = load_runtime_economy_catalog()
        .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
    !allocator
        .stocked_zone_profiles(&profiles, &catalog)
        .is_empty()
}

impl SimCore {
    /// Replaces the registry with `(pack_id, manifest, asset_dir)` entries and refreshes every
    /// building site, cell lot and entrance that depends on the catalog, and which zoning
    /// profiles have stock.
    ///
    /// Treat every load as an authoritative registry refresh: asset editor moves, pack disables
    /// and deleted folders must remove stale qualified IDs too.
    pub(crate) fn replace_asset_registry(
        &mut self,
        entries: impl IntoIterator<Item = (String, AssetManifest, String)>,
    ) {
        self.allocator.registry.clear();
        for (pack_id, manifest, asset_dir) in entries {
            self.allocator
                .registry
                .register(&pack_id, manifest, asset_dir);
        }
        self.rebuild_vehicle_catalogue();
        self.allocator
            .refresh_building_sites_after_asset_reload(self.zoning.config.zone_cell_m);
        self.zoning.invalidate_cell_lot_assets();
        self.rebuild_building_entrances_internal();
        self.publish_reloaded_building_site_changes();
        self.stocked_zone_profiles = self
            .allocator
            .stocked_zone_profiles(&self.zoning.profiles, self.demand.runtime_catalog());
    }
}

/// Returns the number of renderable building mesh parts for a registered asset.
pub fn get_building_mesh_part_count(core: &SimCore, qualified_id: GString) -> i32 {
    let qid = qualified_id.to_string();
    core.allocator
        .registry
        .get(&qid)
        .map(|entry| entry.manifest.mesh_parts.len() as i32)
        .unwrap_or(0)
}
