// SPDX-License-Identifier: GPL-2.0-only

//! Import-time building bounds using the same scene/root convention as building rendering.

use crate::assets::AssetManifest;
use godot::classes::{
    FbxDocument, FbxState, GltfDocument, GltfState, MeshInstance3D, Node, Node3D,
};
use godot::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub(super) type MeshBoundsCache = HashMap<PathBuf, Result<[[f32; 3]; 2], String>>;

pub(super) fn import_building_bounds(
    manifest: &mut AssetManifest,
    asset_dir: &str,
    cache: &mut MeshBoundsCache,
) -> Result<(), String> {
    for part in &mut manifest.mesh_parts {
        let lod = part.lods.first().ok_or("missing building LOD0")?;
        let path = Path::new(asset_dir).join(&lod.file);
        let bounds = cache
            .entry(path.clone())
            .or_insert_with(|| import_bounds(&path));
        part.imported_bounds = Some(bounds.clone()?);
    }
    Ok(())
}

fn import_bounds(path: &Path) -> Result<[[f32; 3]; 2], String> {
    // Godot import APIs are main-thread-only. This runs once per distinct asset mesh
    // per pack refresh, never in placement, per-building, or simulation tick loops.
    let (mut document, state): (Gd<GltfDocument>, Gd<GltfState>) = if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("fbx"))
    {
        (FbxDocument::new_gd().upcast(), FbxState::new_gd().upcast())
    } else {
        (GltfDocument::new_gd(), GltfState::new_gd())
    };
    let error = document.append_from_file(&GString::from(path.to_string_lossy().as_ref()), &state);
    if error != godot::global::Error::OK {
        return Err(format!(
            "cannot import building bounds '{}': {error:?}",
            path.display()
        ));
    }
    let scene = document
        .generate_scene(&state)
        .ok_or_else(|| format!("no building mesh scene in '{}'", path.display()))?;
    let mut bounds = None;
    collect_bounds(&scene, Transform3D::IDENTITY, true, &mut bounds);
    scene.free();
    let bounds =
        bounds.ok_or_else(|| format!("no building mesh bounds in '{}'", path.display()))?;
    let min = bounds.position;
    let max = bounds.end();
    if !min.is_finite() || !max.is_finite() || max.x <= min.x || max.z <= min.z {
        return Err(format!(
            "invalid building mesh bounds in '{}'",
            path.display()
        ));
    }
    Ok([[min.x, min.y, min.z], [max.x, max.y, max.z]])
}

fn collect_bounds(node: &Gd<Node>, parent: Transform3D, is_root: bool, bounds: &mut Option<Aabb>) {
    let transform = if !is_root {
        node.clone()
            .try_cast::<Node3D>()
            .map_or(parent, |node| parent * node.get_transform())
    } else {
        parent
    };
    if let Ok(instance) = node.clone().try_cast::<MeshInstance3D>()
        && let Some(mesh) = instance.get_mesh()
    {
        let transformed = transform * mesh.get_aabb();
        *bounds = Some(bounds.map_or(transformed, |old| old.merge(transformed)));
    }
    for child in node.get_children().iter_shared() {
        collect_bounds(&child, transform, false, bounds);
    }
}
