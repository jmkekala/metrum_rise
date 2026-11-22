// SPDX-License-Identifier: GPL-3.0-or-later

//! The vehicle sidecar reader and the drafted `[vehicle]` asset, on a fixture shaped as the
//! pilot estate's sidecar (cut to its required lenses). Its `.glb` files are JSON-only GLB
//! containers referencing the maps, as the models build exports them.

use super::*;
use crate::assets::authoring::files::token;
use crate::assets::authoring::model_draft::tests::glb;
use std::fs;

pub(crate) const PACK: &str = "vehicle-test";
pub(crate) const ASSET: &str = "vehicle.civil.estate_compact";
const NAME: &str = "estate_compact";

/// A temporary vehicle export, its sidecar and a packs folder with one empty pack.
pub(crate) struct VehicleFixture {
    pub(crate) root: PathBuf,
    pub(crate) sidecar: Value,
}

fn lens(group: &str, id: u8, side: &str, x: f32, z: f32, facing_z: f32) -> Value {
    json!({"group": group, "lamp": id, "side": side, "position": if facing_z > 0.0 { "front" } else { "rear" },
           "colour": if facing_z > 0.0 { "white" } else { "red" }, "centre": [x, 0.8, z],
           "facing": [0.0, 0.6, 0.8 * facing_z.signum()], "width_m": 0.2, "height_m": 0.1})
}

impl VehicleFixture {
    pub(crate) fn new() -> Self {
        let root = std::env::temp_dir().join(format!("metrum-vehicle-draft-{}", token()));
        let export = root.join("models/export");
        fs::create_dir_all(&export).unwrap();
        fs::create_dir_all(root.join("mods").join(PACK)).unwrap();
        fs::write(
            root.join("mods").join(PACK).join("pack.toml"),
            crate::assets::pack::manifest_toml(PACK, "Vehicle Test", "0.1.0", "Tester", "CC-BY-4.0"),
        )
        .unwrap();
        let maps = ["emission", "normal", "orm", "albedo_plain"];
        for map in maps {
            fs::write(export.join(format!("{NAME}_{map}.png")), map).unwrap();
        }
        fs::write(root.join("models/preview.png"), "preview").unwrap();
        let images: Vec<_> = maps.iter().map(|m| json!({"uri": format!("{NAME}_{m}.png")})).collect();
        for (tier, triangles) in [2944u64, 892, 208, 40].into_iter().enumerate() {
            let model = json!({
                "asset": {"version": "2.0"}, "nodes": [{"mesh": 0}],
                "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "COLOR_0": 1}, "material": 0}]}],
                "accessors": [{"count": triangles * 3}, {"count": triangles * 3}],
                "materials": [{"name": NAME}], "images": images,
            });
            fs::write(export.join(format!("{NAME}_lod{tier}.glb")), glb(&model)).unwrap();
        }
        let mut fixture = Self {
            root,
            sidecar: Value::Null,
        };
        fixture.sidecar = fixture.example();
        fixture.write_sidecar();
        fixture
    }

    pub(crate) fn sidecar_path(&self) -> PathBuf {
        self.root.join(format!("models/export/{NAME}.model.json"))
    }

    pub(crate) fn mods(&self) -> PathBuf {
        self.root.join("mods")
    }

    pub(crate) fn write_sidecar(&self) {
        fs::write(self.sidecar_path(), serde_json::to_string_pretty(&self.sidecar).unwrap()).unwrap();
    }

    pub(crate) fn role(&self) -> Value {
        json!({"pack_id": PACK, "asset_id": ASSET, "vehicle_class": "civil",
               "vehicle_family": "estate", "seats": 5})
    }

    fn digest(&self, relative: &str) -> String {
        crate::assets::archive::sha256(&self.root.join("models/export").join(relative)).unwrap()
    }

    fn example(&self) -> Value {
        let file = |relative: String| json!({"file": relative, "sha256": self.digest(&relative)});
        let wheel = |id: &str, part: u8, axle: usize, x: f32, z: f32| {
            json!({"id": id, "part": part, "axle": axle, "side": if x > 0.0 { "left" } else { "right" },
                   "centre": [x, 0.32, z], "radius_m": 0.32, "width_m": 0.207, "steer": axle == 0})
        };
        let mut lamps = Vec::new();
        for (group, id, z, facing) in [
            ("tail_brake", 3, -2.0, -1.0),
            ("indicator_left", 4, 2.0, 1.0),
            ("indicator_right", 5, 2.0, 1.0),
            ("reverse", 6, -2.0, -1.0),
            ("low_beam", 7, 2.0, 1.0),
            ("high_beam", 8, 2.05, 1.0),
            ("daytime", 9, 2.1, 1.0),
        ] {
            lamps.push(lens(group, id, "left", 0.7, z, facing));
            lamps.push(lens(group, id, "right", -0.7, z, facing));
        }
        lamps.push(lens("brake", 2, "centre", 0.0, -1.8, -1.0));
        json!({
            "schema": "metrum-model-facts", "version": 2, "kind": "vehicle",
            "name": NAME, "path": "vehicles/car/estate_compact",
            "source": {"commit": "25b701da0eeab899ae1f705d2eabd761998faf4e", "dirty": false, "blender": "5.3.0"},
            "license": "CC-BY-4.0",
            "bounds": {"min": [-1.01, 0.0, -2.24], "max": [1.01, 1.468, 2.24]},
            "body": {"class": "car", "length_m": 4.48, "width_m": 1.79, "width_mirrors_m": 2.02,
                     "height_m": 1.468, "wheelbase_m": 2.65, "front_overhang_m": 0.86,
                     "rear_overhang_m": 0.97, "ground_clearance_m": 0.19},
            "axles": [{"z": 1.38, "track_m": 1.54, "steer": true, "twin": false},
                      {"z": -1.27, "track_m": 1.54, "steer": false, "twin": false}],
            "wheels": [wheel("front_left", 1, 0, 0.77, 1.38), wheel("front_right", 2, 0, -0.77, 1.38),
                       wheel("rear_left", 3, 1, 0.77, -1.27), wheel("rear_right", 4, 1, -0.77, -1.27)],
            "attributes": {"set": "COLOR_0", "r": "part", "g": "surface", "b": "paint", "version": 1},
            "lamps": lamps,
            "glass": [{"group": "glass_windscreen", "id": 32, "area_m2": 1.199}],
            "cargo": null, "coupling": null, "interior": null,
            "lods": (0..4).map(|tier| {
                let relative = format!("{NAME}_lod{tier}.glb");
                let triangles = [2944, 892, 208, 40][tier];
                json!({"file": relative, "triangles": triangles, "sha256": self.digest(&relative),
                       "wheel_parts": tier < 2, "lamp_ids": tier < 3, "glass_ids": tier < 3})
            }).collect::<Vec<_>>(),
            "textures": {"orm": file(format!("{NAME}_orm.png")), "emission": file(format!("{NAME}_emission.png")),
                         "normal": file(format!("{NAME}_normal.png"))},
            "variants": [{"name": "plain", "albedo": file(format!("{NAME}_albedo_plain.png")), "paint": true}],
            "default_variant": "plain",
            "preview": {"file": "preview.png", "sha256": crate::assets::archive::sha256(&self.root.join("models/preview.png")).unwrap()},
        })
    }

    pub(crate) fn inputs(&self) -> VehicleInputs {
        read_inputs(&self.sidecar_path(), self.sidecar.clone(), self.role()).unwrap()
    }
}

impl Drop for VehicleFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn facts_with(edit: impl FnOnce(&mut Value)) -> Result<VehicleFacts, String> {
    let fixture = VehicleFixture::new();
    let mut sidecar = fixture.sidecar.clone();
    edit(&mut sidecar);
    VehicleFacts::from_value(sidecar)
}

#[test]
fn the_reader_takes_the_pilot_shape_and_is_strict() {
    let facts = facts_with(|_| {}).unwrap();
    assert_eq!((facts.wheels.len(), facts.lamps.len()), (4, 15));
    assert!(is_vehicle(&VehicleFixture::new().sidecar));
    type Edit = fn(&mut Value);
    let refusals: [(&str, Edit); 8] = [
        ("unknown key", |s| s["extra"] = json!(1)),
        ("version 1", |s| s["version"] = json!(1)),
        ("another kind", |s| s["kind"] = json!("building")),
        ("a coupling", |s| s["coupling"] = json!({"hitch": [0, 0, 0]})),
        ("three tiers", |s| {
            s["lods"].as_array_mut().unwrap().pop();
        }),
        ("wheels out of order", |s| s["wheels"][0]["part"] = json!(2)),
        ("another id attribute", |s| s["attributes"]["set"] = json!("_METRUM_IDS")),
        ("a file outside the export", |s| s["textures"]["orm"]["file"] = json!("../orm.png")),
    ];
    for (what, edit) in refusals {
        assert!(facts_with(edit).is_err(), "{what} accepted");
    }
    // A truck without its cargo box.
    assert!(facts_with(|s| s["body"]["class"] = json!("truck")).is_err());
}

#[test]
fn inputs_refuse_a_dirty_source_a_changed_file_and_a_bad_role() {
    let fixture = VehicleFixture::new();
    let read = |sidecar: Value, role: Value| read_inputs(&fixture.sidecar_path(), sidecar, role);
    let mut dirty = fixture.sidecar.clone();
    dirty["source"]["dirty"] = json!(true);
    assert!(read(dirty, fixture.role()).unwrap_err().contains("dirty"));
    let mut role = fixture.role();
    role["vehicle_family"] = json!("spaceship");
    assert!(read(fixture.sidecar.clone(), role).unwrap_err().contains("vehicle_family"));
    fs::write(fixture.root.join("models/export/estate_compact_orm.png"), "changed").unwrap();
    assert!(read(fixture.sidecar.clone(), fixture.role()).unwrap_err().contains("SHA-256"));
}

#[test]
fn the_drafted_asset_carries_the_facts_and_passes_the_pack_load_checks() {
    let fixture = VehicleFixture::new();
    let mut inputs = fixture.inputs();
    inputs.role.paints = Some(vec![RolePaint {
        colour: [0.1, 0.2, 0.3],
        weight: 2.0,
    }]);
    let derived = derive(&inputs, &fixture.mods(), &fixture.root.join("work")).unwrap();
    let manifest = AssetManifest::from_str(&derived.asset_toml).unwrap();
    let vehicle = manifest.vehicle.as_ref().unwrap();
    assert_eq!(manifest.display_name, "Estate Compact");
    assert_eq!(vehicle.body_class, BodyClass::Car);
    assert_eq!(vehicle.seats, 5);
    assert_eq!(vehicle.wheels[1].centre, [-0.77, 0.32, 1.38]);
    assert_eq!(vehicle.lamps.len(), 15);
    assert_eq!(vehicle.paints[0].colour, [0.1, 0.2, 0.3]);
    assert_eq!(vehicle.normal.as_deref(), Some("estate_compact_normal.png"));
    // A 4.48 m car switches tiers at the contract's bands, rounded to whole metres.
    let starts: Vec<_> = manifest.lods.iter().map(|lod| lod.distance_min_m).collect();
    assert_eq!(starts, [0.0, 30.0, 100.0, 398.0]);
    assert_eq!(tier_starts(7.9), [0.0, 53.0, 176.0, 702.0]);
    // Tiers as models, every map and the thumbnail as extras.
    assert_eq!(derived.models.len(), 4);
    let extras: Vec<_> = derived.extras.iter().map(|(file, _)| file.as_str()).collect();
    assert_eq!(
        extras,
        [
            "estate_compact_orm.png",
            "estate_compact_emission.png",
            "estate_compact_normal.png",
            "estate_compact_albedo_plain.png",
            super::super::rules::THUMBNAIL_FILE,
        ]
    );
}

// The pilot estate's doors as models commit 0f4757e states them (docs/vehicles.md "Doors").
fn estate_doors() -> Value {
    json!([
        {"side": "left", "row": "front", "centre": [0.895, 0.0, 0.3675], "width_m": 1.135},
        {"side": "right", "row": "front", "centre": [-0.895, 0.0, 0.3675], "width_m": 1.135},
        {"side": "left", "row": "rear", "centre": [0.895, 0.0, -0.58], "width_m": 0.76},
        {"side": "right", "row": "rear", "centre": [-0.895, 0.0, -0.58], "width_m": 0.76},
        {"side": "rear", "row": "cargo", "centre": [0.0, 0.0, -2.24], "width_m": 1.15}
    ])
}

#[test]
fn version_3_carries_doors_into_the_asset_and_version_2_still_reads() {
    let fixture = VehicleFixture::new();
    // Version 2 has no doors, and its asset none.
    let derived = derive(&fixture.inputs(), &fixture.mods(), &fixture.root.join("work")).unwrap();
    let manifest = AssetManifest::from_str(&derived.asset_toml).unwrap();
    assert!(manifest.vehicle.unwrap().doors.is_empty());
    assert!(facts_with(|s| s["doors"] = estate_doors()).is_err(), "doors in version 2 accepted");
    assert!(facts_with(|s| s["version"] = json!(3)).is_err(), "version 3 without doors accepted");
    let v3 = |s: &mut Value| {
        s["version"] = json!(3);
        s["doors"] = estate_doors();
    };
    let facts = facts_with(v3).unwrap();
    let inputs = VehicleInputs { facts, ..fixture.inputs() };
    let derived = derive(&inputs, &fixture.mods(), &fixture.root.join("work")).unwrap();
    let manifest = AssetManifest::from_str(&derived.asset_toml).unwrap();
    let doors = manifest.vehicle.unwrap().doors;
    assert_eq!(doors.len(), 5);
    assert_eq!((doors[1].side, doors[1].row), (crate::assets::DoorSide::Right, crate::assets::DoorRow::Front));
    assert_eq!(doors[1].centre, [-0.895, 0.0, 0.3675]);
    assert_eq!((doors[4].side, doors[4].row, doors[4].width_m), (crate::assets::DoorSide::Rear, crate::assets::DoorRow::Cargo, 1.15));
    // The pack-load checks: unmirrored front doors, a missing front door, list order, a door
    // off its side, one above the ground.
    type Edit = fn(&mut Value);
    let refusals: [(&str, Edit); 5] = [
        ("unmirrored", |s| s["doors"][1]["centre"][2] = json!(0.4)),
        ("one front door", |s| {
            s["doors"].as_array_mut().unwrap().remove(1);
        }),
        ("out of order", |s| s["doors"].as_array_mut().unwrap().swap(0, 2)),
        ("off its side", |s| s["doors"][0]["centre"][0] = json!(-0.895)),
        ("above the ground", |s| s["doors"][2]["centre"][1] = json!(0.3)),
    ];
    for (what, edit) in refusals {
        let facts = facts_with(|s| {
            v3(s);
            edit(s);
        })
        .unwrap();
        let inputs = VehicleInputs { facts, ..fixture.inputs() };
        assert!(derive(&inputs, &fixture.mods(), &fixture.root.join("work")).is_err(), "{what} accepted");
    }
}
