//! Godot-Rust bridge helpers for asset management and content registries.
use godot::prelude::*;
use std::path::Path;
use crate::nodes::sim::core::SimCore;

/// Scans a native filesystem directory for content packs and registers all valid assets.
pub fn load_asset_packs(core: &mut SimCore, dir_path: GString, enabled_pack_ids: GString) -> GString {
    use crate::assets::scan_pack_dir;
    let filter_str = enabled_pack_ids.to_string();
    let filter: Vec<&str> = if filter_str.is_empty() {
        vec![]
    } else {
        filter_str.split(',').map(str::trim).collect()
    };
    
    let result = scan_pack_dir(Path::new(&dir_path.to_string()));
    for pack in result.packs {
        if !filter.is_empty() && !filter.contains(&pack.pack.pack_id.as_str()) {
            continue;
        }
        for (asset, asset_dir) in pack.assets {
            core.allocator.registry.register(&pack.pack.pack_id, asset, asset_dir);
        }
    }
    GString::from(result.warnings.join("\n").as_str())
}

/// Returns the native filesystem path to the LOD0 mesh file for a registered asset.
pub fn get_lod0_native_path(core: &SimCore, qualified_id: GString) -> GString {
    let qid = qualified_id.to_string();
    if let Some(entry) = core.allocator.registry.get(&qid) {
        if let Some(lod) = entry.manifest.lods.first() {
            let path = Path::new(&entry.asset_dir).join(&lod.file);
            return GString::from(path.to_string_lossy().as_ref());
        }
    }
    GString::new()
}
