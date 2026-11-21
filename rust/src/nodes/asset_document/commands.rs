// SPDX-License-Identifier: GPL-3.0-or-later

//! Typed authoring commands over lossless document dictionaries; no JSON edit round trips.
//! Validate the whole target set before making a private command snapshot.

use crate::assets::asset::YardPlantKind;
use crate::assets::authoring::edits::{ObjectKind, unique_name};
use godot::classes::Json;
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
        "passage" => resolved.len() == 1 && resolved[0].3 == ObjectKind::Mesh,
        "route" => resolved.len() == 1 && resolved[0].3 == ObjectKind::Loading,
        "move_site" => resolved.is_empty(),
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
            // Yard paving, or a yard planting area, which the editor keeps among the surfaces
            // under its plant kind and export writes out as `[[building.yard_planting]]`.
            surface
                if matches!(surface, "asphalt" | "concrete")
                    || YardPlantKind::from_name(surface).is_some() =>
            {
                let (w, d) = match kind.as_str() {
                    "asphalt" => (2.5, 3.5),
                    "concrete" => (0.7, 3.0),
                    _ => (2.0, 2.0),
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
    } else if action == "move_site" {
        move_site(&mut params, args)?;
        selection = targets.clone();
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
                if !matches!(material.as_str(), "asphalt" | "concrete")
                    && YardPlantKind::from_name(&material).is_none()
                {
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
            "passage" => edit_passages(&mut entry, args)?,
            "route" if text(args, "op") == "arc" => arc_route(&params, *index, &mut entry, args)?,
            "route" => edit_route(&mut entry, args)?,
            _ => return Err("Unsupported edit".into()),
        }
        entries.set(*index, &entry.to_variant());
        params.set(key, entries);
        selection = targets.clone();
    }
    next.set("params", params);
    Ok((next, selection))
}

// Adds, removes or changes one passage of a mesh part (`PARK-19`): `op` is "add" (at the
// optional part-local `position`, opening along -Z), "delete" or "set" (`field` to `value`) of
// passage `passage`. Names stay unique in the part and sizes positive; the lot-axis and bounds
// rules are reported by validation, so an author may pass through an invalid state. O(P) for
// the part's P passages.
fn edit_passages(part: &mut VarDictionary, args: &VarDictionary) -> Result<(), String> {
    let mut passages = array(part, "passages");
    let index = args
        .get("passage")
        .and_then(|v| v.try_to::<i64>().ok())
        .and_then(|v| usize::try_from(v).ok())
        .filter(|&i| i < passages.len());
    match text(args, "op").as_str() {
        "add" => {
            let position = args
                .get("position")
                .and_then(|v| v.try_to::<VarArray>().ok())
                .filter(|v| v.len() == 3)
                .unwrap_or_else(|| vec3([0.0; 3]));
            let name = unique_name("passage", &mut names(&passages));
            let passage = vdict! { "name": name, "position": position, "forward": vec3([0.0, 0.0, -1.0]), "width_m": 5.0, "length_m": 8.0, "clear_height_m": 4.5, "through": false };
            passages.push(&passage.to_variant());
        }
        "delete" => {
            passages.remove(index.ok_or("The passage no longer exists")?);
        }
        "set" => {
            let index = index.ok_or("The passage no longer exists")?;
            let mut passage: VarDictionary =
                passages.at(index).try_to().map_err(|e| e.to_string())?;
            let field = text(args, "field");
            let value = args.get("value").ok_or("Missing value")?;
            match field.as_str() {
                "name" => {
                    let name = value.try_to::<GString>().map_err(|e| e.to_string())?;
                    let name = name.to_string().trim().to_owned();
                    if name.is_empty() {
                        return Err("Name cannot be empty".into());
                    }
                    if passages.iter_shared().enumerate().any(|(i, v)| {
                        i != index
                            && v.try_to::<VarDictionary>()
                                .is_ok_and(|v| text(&v, "name") == name)
                    }) {
                        return Err("That name is already in use".into());
                    }
                    passage.set("name", name);
                }
                "width_m" | "length_m" | "clear_height_m" => {
                    let size = value.try_to::<f64>().map_err(|e| e.to_string())?;
                    if !size.is_finite() || size <= 0.0 {
                        return Err("A passage's sizes must be greater than zero".into());
                    }
                    passage.set(field.as_str(), size);
                }
                "position" | "forward" => {
                    let vector = value.try_to::<VarArray>().map_err(|e| e.to_string())?;
                    if vector.len() != 3
                        || vector
                            .iter_shared()
                            .any(|v| !v.try_to::<f64>().is_ok_and(f64::is_finite))
                    {
                        return Err("Expected three finite numbers".into());
                    }
                    passage.set(field.as_str(), vector);
                }
                "through" => {
                    passage.set("through", value.try_to::<bool>().map_err(|e| e.to_string())?);
                }
                _ => return Err("Unsupported passage field".into()),
            }
            passages.set(index, &passage.to_variant());
        }
        _ => return Err("Unsupported passage edit".into()),
    }
    part.set("passages", passages);
    Ok(())
}

// Changes one loading bay's drive-through (`PARK-21`): `op` "exit" sets the exit Driveway's
// name (`value`; empty clears it, and with it the way out, which needs an exit); "add" inserts
// waypoint `point` (`[x, z]`) into `way` ("in" or "out") before `waypoint`, or at the end when
// absent; "move" sets waypoint `waypoint` of `way` to `point`; "delete" removes it. Whether the
// exit names a gate and the route proves is validation's to report, so an author may pass
// through an invalid state. O(waypoints). `op` "stand" sets how a truck stands in the bay
// (`PARK-23`): `value` "tail_in" (backs in), or "nose_in" or empty for the default, which leaves
// the key out; that a backed-into bay names no exit is validation's to report.
fn edit_route(anchor: &mut VarDictionary, args: &VarDictionary) -> Result<(), String> {
    let op = text(args, "op");
    if op == "stand" {
        match text(args, "value").trim() {
            "tail_in" => anchor.set("stand", "tail_in"),
            "nose_in" | "" => {
                anchor.remove("stand");
            }
            _ => return Err("Unsupported stand".into()),
        }
        return Ok(());
    }
    if op == "exit" {
        let exit = text(args, "value").trim().to_owned();
        if exit.is_empty() {
            anchor.remove("exit");
            anchor.remove("route_out");
        } else {
            anchor.set("exit", exit);
        }
        return Ok(());
    }
    let key = match text(args, "way").as_str() {
        "in" => "route_in",
        "out" if anchor.contains_key("exit") => "route_out",
        "out" => return Err("Name the exit Driveway before adding a way out".into()),
        _ => return Err("Unsupported route".into()),
    };
    let mut points = array(anchor, key);
    let waypoint = args
        .get("waypoint")
        .and_then(|v| v.try_to::<i64>().ok())
        .and_then(|v| usize::try_from(v).ok());
    let point = || {
        let point = array(args, "point");
        let finite = point.len() == 2
            && point
                .iter_shared()
                .all(|v| v.try_to::<f64>().is_ok_and(f64::is_finite));
        finite.then_some(point).ok_or("Expected two finite numbers")
    };
    let most = crate::assets::asset::validation::MAX_ROUTE_POINTS - 3;
    match op.as_str() {
        "add" if points.len() >= most => {
            return Err(format!("A way holds at most {most} waypoints"));
        }
        "add" => {
            let at = waypoint.unwrap_or(points.len()).min(points.len());
            points.insert(at, &point()?.to_variant());
        }
        "move" | "delete" => {
            let index = waypoint
                .filter(|&i| i < points.len())
                .ok_or("The waypoint no longer exists")?;
            if op == "move" {
                points.set(index, &point()?.to_variant());
            } else {
                points.remove(index);
            }
        }
        _ => return Err("Unsupported route edit".into()),
    }
    if points.is_empty() {
        anchor.remove(key);
    } else {
        anchor.set(key, points);
    }
    Ok(())
}

// The arc helper (`PARK-21`): inserts into `way` ("in" or "out") of loading bay `index` the
// waypoints of a turn of `radius` metres and `angle` degrees to `side` ("left" or "right", as a
// driver sees it), after waypoint `waypoint` (absent: the last), from the heading of the leg
// arriving there. Rust lays the chords (`route_arc`), every bend under 15°; this inserts them
// as one edit. O(anchors + waypoints + chords).
fn arc_route(
    params: &VarDictionary,
    index: usize,
    anchor: &mut VarDictionary,
    args: &VarDictionary,
) -> Result<(), String> {
    let way_in = match text(args, "way").as_str() {
        "in" => true,
        "out" if anchor.contains_key("exit") => false,
        "out" => return Err("Name the exit Driveway before adding a way out".into()),
        _ => return Err("Unsupported route".into()),
    };
    let number = |key: &str| {
        args.get(key)
            .and_then(|v| v.try_to::<f64>().ok())
            .filter(|v| v.is_finite())
            .ok_or(format!("Expected a number for {key}"))
    };
    let left = match text(args, "side").as_str() {
        "left" => true,
        "right" => false,
        _ => return Err("An arc turns left or right".into()),
    };
    let after = args
        .get("waypoint")
        .and_then(|v| v.try_to::<i64>().ok())
        .and_then(|v| usize::try_from(v).ok());
    let json: serde_json::Value =
        serde_json::from_str(&Json::stringify(&params.to_variant()).to_string())
            .map_err(|e| e.to_string())?;
    let (at, arc) = crate::simulation::parking::validation::route_arc(
        &json,
        index,
        way_in,
        after,
        number("radius")? as f32,
        number("angle")? as f32,
        left,
    )?;
    let key = if way_in { "route_in" } else { "route_out" };
    let mut points = array(anchor, key);
    for (k, [x, z]) in arc.into_iter().enumerate() {
        // Rounded to 0.1 mm as `arc_waypoints` laid them, without the f32's tail.
        let mm = |v: f32| (f64::from(v) * 10_000.0).round() / 10_000.0;
        let point: VarArray = varray![mm(x), mm(z)];
        points.insert(at + k, &point.to_variant());
    }
    anchor.set(key, points);
    Ok(())
}

// "Move whole site" (`TOOLS-40`): every part, access point, surface vertex and waypoint by
// `dx`, `dz` (a Driveway along its street edge), as one edit; `site_move::move_site` decides and
// refuses a move that leaves the lot. `part_bounds` is the parts' imported bounds, six floats
// each. Only the moved keys are written back, so every other value stays as authored.
// O(parts + anchors + vertices + waypoints).
fn move_site(params: &mut VarDictionary, args: &VarDictionary) -> Result<(), String> {
    let number = |key: &str| {
        args.get(key)
            .and_then(|v| v.try_to::<f64>().ok())
            .ok_or(format!("Expected a number for {key}"))
    };
    let bounds = args
        .get("part_bounds")
        .and_then(|v| v.try_to::<PackedFloat32Array>().ok())
        .unwrap_or_default();
    let json: serde_json::Value =
        serde_json::from_str(&Json::stringify(&params.to_variant()).to_string())
            .map_err(|e| e.to_string())?;
    let moved = crate::assets::authoring::site_move::move_site(
        &json,
        bounds.as_slice(),
        number("dx")?,
        number("dz")?,
    )?;
    let points = |points: &[[f64; 2]]| -> VarArray {
        points
            .iter()
            .map(|&[x, z]| varray![x, z].to_variant())
            .collect()
    };
    let mut write = |key: &str, edit: &mut dyn FnMut(usize, &mut VarDictionary)| {
        let mut entries = array(params, key);
        for index in 0..entries.len() {
            if let Ok(mut entry) = entries.at(index).try_to::<VarDictionary>() {
                edit(index, &mut entry);
                entries.set(index, &entry.to_variant());
            }
        }
        params.set(key, entries);
    };
    write("mesh_parts", &mut |i, part| {
        part.set("position", vec3(moved.parts[i]));
    });
    write("anchors", &mut |i, anchor| {
        let moved = &moved.anchors[i];
        anchor.set("position", vec3(moved.position));
        for (key, route) in [("route_in", &moved.route_in), ("route_out", &moved.route_out)] {
            if let Some(route) = route {
                anchor.set(key, points(route));
            }
        }
    });
    write("site_surfaces", &mut |i, surface| {
        surface.set("vertices", points(&moved.surfaces[i]));
    });
    Ok(())
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
