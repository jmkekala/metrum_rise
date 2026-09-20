// SPDX-License-Identifier: GPL-2.0-only

//! Typed authoring commands over lossless document dictionaries; no JSON edit round trips.
//! Validate the whole target set before making a private command snapshot.

use crate::assets::authoring::edits::{ObjectKind, unique_name};
use godot::{builtin::vdict, prelude::*};
use std::collections::HashSet;

fn text(entry: &VarDictionary, key: &str) -> String {
    entry
        .get(key)
        .and_then(|v| v.try_to::<GString>().ok())
        .unwrap_or_default()
        .to_string()
}

fn array(entry: &VarDictionary, key: &str) -> VarArray {
    let Some(value) = entry.get(key) else {
        return VarArray::new();
    };
    if let Ok(array) = value.try_to::<VarArray>() {
        return array;
    }
    // Godot distinguishes Array[Dictionary] from Array; projection capture uses both.
    value
        .try_to::<Array<VarDictionary>>()
        .map(|values| values.iter_shared().map(|v| v.to_variant()).collect())
        .unwrap_or_default()
}

fn dictionary_at(params: &VarDictionary, key: &str, index: usize) -> Option<VarDictionary> {
    let value = params.get(key)?;
    if let Ok(values) = value.try_to::<Array<VarDictionary>>() {
        return values.get(index);
    }
    value.try_to::<VarArray>().ok()?.get(index)?.try_to().ok()
}

fn collection(kind: &str) -> Option<&'static str> {
    match kind {
        "mesh" => Some("mesh_parts"),
        "anchor" => Some("anchors"),
        "surface" => Some("site_surfaces"),
        _ => None,
    }
}

fn resolve(
    params: &VarDictionary,
    target: &VarDictionary,
) -> Result<(String, usize, VarDictionary, ObjectKind), String> {
    let kind = text(target, "kind");
    let key = collection(&kind).ok_or("Not an authored object")?;
    let index = target
        .get("index")
        .and_then(|v| v.try_to::<i64>().ok())
        .and_then(|v| usize::try_from(v).ok())
        .ok_or("Invalid target")?;
    let entry = dictionary_at(params, key, index).ok_or("The selected object no longer exists")?;
    let object =
        ObjectKind::parse(&kind, &text(&entry, "anchor_type")).ok_or("Unsupported object type")?;
    Ok((kind, index, entry, object))
}

pub(super) fn capabilities(state: &VarDictionary, targets: &Array<VarDictionary>) -> VarDictionary {
    let params: VarDictionary = state
        .get("params")
        .and_then(|v| v.try_to().ok())
        .unwrap_or_default();
    let mut rotate = !targets.is_empty();
    let mut duplicate = rotate;
    let mut rename = targets.len() == 1;
    for target in targets.iter_shared() {
        let Ok((_, _, _, kind)) = resolve(&params, &target) else {
            return vdict! { "error": "Selection is no longer available" };
        };
        rotate &= kind.rotates();
        duplicate &= kind.duplicates();
        rename &= kind != ObjectKind::Entrance;
    }
    vdict! { "rotate": rotate, "duplicate": duplicate, "rename": rename, "delete": !targets.is_empty() }
}

pub(super) fn prepare(
    state: &VarDictionary,
    action: &str,
    targets: &Array<VarDictionary>,
    args: &VarDictionary,
) -> VarDictionary {
    match edit(state, action, targets, args) {
        Ok((document, selection)) => vdict! { "document": document, "selection": selection },
        Err(error) => vdict! { "error": error },
    }
}

fn edit(
    state: &VarDictionary,
    action: &str,
    targets: &Array<VarDictionary>,
    args: &VarDictionary,
) -> Result<(VarDictionary, Array<VarDictionary>), String> {
    let params: VarDictionary = state
        .get("params")
        .and_then(|v| v.try_to().ok())
        .ok_or("Open an asset first")?;
    let mut seen = HashSet::new();
    let resolved = targets
        .iter_shared()
        .map(|target| {
            let item = resolve(&params, &target)?;
            if !seen.insert((item.0.clone(), item.1)) {
                return Err("Duplicate selection target".into());
            }
            Ok(item)
        })
        .collect::<Result<Vec<_>, String>>()?;
    let allowed = match action {
        "duplicate" => !resolved.is_empty() && resolved.iter().all(|v| v.3.duplicates()),
        "rename" => resolved.len() == 1 && resolved[0].3 != ObjectKind::Entrance,
        "delete" => !resolved.is_empty(),
        "material" | "insert_vertex" | "delete_vertex" => {
            resolved.len() == 1 && resolved[0].3 == ObjectKind::Yard
        }
        "create" => resolved.is_empty() && text(&params, "asset_class") == "building",
        _ => false,
    };
    if !allowed {
        return Err("This action is not available for the complete selection".into());
    }
    let mut next = state.duplicate_deep();
    let mut params: VarDictionary = next
        .get("params")
        .and_then(|v| v.try_to().ok())
        .ok_or("Missing asset parameters")?;
    let mut selection = Array::new();
    if action == "create" {
        let kind = text(args, "kind");
        let (key, mut entry) = match kind.as_str() {
            "mesh" => {
                let path = text(args, "path");
                if !std::path::Path::new(&path).is_file() {
                    return Err("Mesh file does not exist".into());
                }
                let filename = std::path::Path::new(&path)
                    .file_name()
                    .and_then(|v| v.to_str())
                    .ok_or("Invalid mesh filename")?;
                let lod = vdict! { "file": filename, "distance_min_m": 0.0 };
                let mut lods = VarArray::new();
                lods.push(&lod.to_variant());
                let mut sources = array(&next, "sources");
                let part_count = array(&params, "mesh_parts").len();
                while sources.len() < part_count {
                    sources.push(&VarArray::new().to_variant());
                }
                let mut paths = VarArray::new();
                paths.push(&path.to_variant());
                sources.push(&paths.to_variant());
                next.set("sources", sources);
                (
                    "mesh_parts",
                    vdict! { "name": filename, "position": vec3([0.0; 3]), "rotation_degrees": vec3([0.0; 3]), "scale": 1.0, "pivot_offset": vec3([0.0; 3]), "lods": lods },
                )
            }
            "entrance" | "driveway" | "parking" | "loading_bay" => {
                if kind == "entrance"
                    && array(&params, "anchors").iter_shared().any(|v| {
                        v.try_to::<VarDictionary>()
                            .is_ok_and(|v| text(&v, "anchor_type") == "entrance")
                    })
                {
                    return Err("Select the existing main entrance instead".into());
                }
                let mut entry = vdict! { "anchor_type": kind.clone(), "name": if kind == "entrance" { "main" } else { kind.as_str() }, "position": vec3([0.0; 3]), "forward": params.get("frontage_forward").unwrap_or(vec3([0.0, 0.0, 1.0]).to_variant()) };
                if kind != "entrance" {
                    entry.set(
                        "width_m",
                        match kind.as_str() {
                            "parking" => 2.5,
                            "loading_bay" => 3.5,
                            _ => 3.0,
                        },
                    );
                    entry.set(
                        "vehicle_class",
                        if kind == "loading_bay" {
                            "freight"
                        } else {
                            "car"
                        },
                    );
                }
                if kind == "parking" || kind == "loading_bay" {
                    entry.set("length_m", if kind == "parking" { 5.0 } else { 8.0 });
                }
                ("anchors", entry)
            }
            "asphalt" | "concrete" => {
                let (w, d) = if kind == "asphalt" {
                    (2.5, 3.5)
                } else {
                    (0.7, 3.0)
                };
                let vertices = [[-w, -d], [w, -d], [w, d], [-w, d]]
                    .into_iter()
                    .map(|v| VarArray::from_iter(v.map(|n| n.to_variant())).to_variant())
                    .collect::<VarArray>();
                (
                    "site_surfaces",
                    vdict! { "name": kind.clone(), "material": kind.clone(), "y_m": 0.01, "vertices": vertices },
                )
            }
            _ => return Err("Unsupported creation type".into()),
        };
        let mut entries = array(&params, key);
        if kind != "entrance" {
            let mut names = names(&entries);
            entry.set("name", unique_name(&text(&entry, "name"), &mut names));
        }
        selection.push(&vdict! { "kind": match key { "mesh_parts" => "mesh", "anchors" => "anchor", _ => "surface" }, "index": entries.len() as i64 });
        entries.push(&entry.to_variant());
        params.set(key, entries);
    } else if action == "duplicate" || action == "delete" {
        for (kind, key) in [
            ("mesh", "mesh_parts"),
            ("anchor", "anchors"),
            ("surface", "site_surfaces"),
        ] {
            let mut indices = resolved
                .iter()
                .filter(|v| v.0 == kind)
                .map(|v| v.1)
                .collect::<Vec<_>>();
            // Never normalize unrelated or omitted collections during another domain's edit.
            if indices.is_empty() {
                continue;
            }
            let mut entries = array(&params, key);
            let mut sources = array(&next, "sources");
            indices.sort_unstable();
            let mut used = names(&entries);
            if action == "delete" {
                indices.reverse();
            }
            for index in indices {
                if action == "delete" {
                    entries.remove(index);
                    if kind == "mesh" && index < sources.len() {
                        sources.remove(index);
                    }
                } else {
                    let mut entry = entries
                        .at(index)
                        .try_to::<VarDictionary>()
                        .map_err(|e| e.to_string())?
                        .duplicate_deep();
                    entry.set("name", unique_name(&text(&entry, "name"), &mut used));
                    selection.push(&vdict! { "kind": kind, "index": entries.len() as i64 });
                    entries.push(&entry.to_variant());
                    if kind == "mesh" {
                        while sources.len() < entries.len() - 1 {
                            sources.push(&VarArray::new().to_variant());
                        }
                        let paths = sources.get(index).unwrap_or(VarArray::new().to_variant());
                        sources.push(&paths);
                    }
                }
            }
            params.set(key, entries);
            if kind == "mesh" {
                next.set("sources", sources);
            }
        }
    } else {
        let (kind, index, _, _) = &resolved[0];
        let key = collection(kind).ok_or("Invalid object kind")?;
        let mut entries = array(&params, key);
        let mut entry: VarDictionary = entries.at(*index).try_to().map_err(|e| e.to_string())?;
        match action {
            "rename" => {
                let name = text(args, "name").trim().to_owned();
                if name.is_empty() {
                    return Err("Name cannot be empty".into());
                }
                if entries.iter_shared().enumerate().any(|(i, v)| {
                    i != *index
                        && v.try_to::<VarDictionary>()
                            .is_ok_and(|v| text(&v, "name") == name)
                }) {
                    return Err("That name is already in use".into());
                }
                entry.set("name", name);
            }
            "material" => {
                let material = text(args, "material");
                if !matches!(material.as_str(), "asphalt" | "concrete") {
                    return Err("Unsupported surface material".into());
                }
                entry.set("material", material);
            }
            "insert_vertex" | "delete_vertex" => {
                let mut vertices = array(&entry, "vertices");
                let vertex = args
                    .get("vertex")
                    .and_then(|v| v.try_to::<i64>().ok())
                    .and_then(|v| usize::try_from(v).ok())
                    .ok_or("Invalid vertex")?;
                if vertex >= vertices.len() {
                    return Err("Vertex no longer exists".into());
                }
                if action == "delete_vertex" {
                    vertices.remove(vertex);
                } else {
                    let point = array(args, "point");
                    vertices.insert(vertex + 1, &point.to_variant());
                }
                let points = vertices
                    .iter_shared()
                    .map(|v| {
                        let v: VarArray = v.try_to().map_err(|e| e.to_string())?;
                        Ok([
                            v.get(0)
                                .and_then(|v| v.try_to::<f32>().ok())
                                .ok_or("Invalid X")?,
                            v.get(1)
                                .and_then(|v| v.try_to::<f32>().ok())
                                .ok_or("Invalid Z")?,
                        ])
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                if !crate::assets::asset::geometry::is_valid(&points) {
                    return Err(
                        "A yard needs at least three vertices and a non-intersecting polygon"
                            .into(),
                    );
                }
                entry.set("vertices", vertices);
            }
            _ => return Err("Unsupported edit".into()),
        }
        entries.set(*index, &entry.to_variant());
        params.set(key, entries);
        selection = targets.clone();
    }
    next.set("params", params);
    Ok((next, selection))
}

fn names(entries: &VarArray) -> HashSet<String> {
    entries
        .iter_shared()
        .filter_map(|v| v.try_to::<VarDictionary>().ok())
        .map(|v| text(&v, "name"))
        .collect()
}

fn vec3(value: [f64; 3]) -> VarArray {
    value.into_iter().map(|v| v.to_variant()).collect()
}
