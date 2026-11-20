// SPDX-License-Identifier: GPL-3.0-or-later

//! Derivation rules, the sidecar reader, the no-overwrite rule and the stale report, on a
//! fixture that follows the spec's worked example (`rintamamiestalo_porch`). The fixture's
//! `.glb` files are JSON-only GLB containers: every Rust consumer reads the JSON chunk only.

use super::report::{Line, stale_report};
use super::rules;
use super::sidecar::{Facts, Role, read_ledger, write_ledger};
use super::*;
use crate::assets::asset::geometry::{fits_lot, polygons_overlap};
use crate::assets::authoring::files::token;
use serde_json::json;
use std::fs;

/// A temporary models export, its sidecar, and a packs folder with one empty pack.
pub(crate) struct Fixture {
    pub(crate) root: PathBuf,
    pub(crate) sidecar: Value,
}

pub(crate) const PACK: &str = "draft-test";
pub(crate) const ASSET: &str = "building.residential.house_3";
const NAME: &str = "rintamamiestalo_porch";
const VARIANTS: [&str; 5] = ["red", "yellow", "white", "blue", "green"];

impl Fixture {
    pub(crate) fn new() -> Self {
        let root = std::env::temp_dir().join(format!("metrum-model-draft-{}", token()));
        let export = root.join("models/export");
        fs::create_dir_all(&export).unwrap();
        fs::create_dir_all(root.join("mods").join(PACK)).unwrap();
        fs::write(
            root.join("mods").join(PACK).join("pack.toml"),
            crate::assets::pack::manifest_toml(PACK, "Draft Test", "0.1.0", "Tester", "CC-BY-4.0"),
        )
        .unwrap();
        for (file, content) in [
            (format!("{NAME}_orm.png"), "orm"),
            (format!("{NAME}_normal.png"), "normal"),
            (format!("{NAME}_emission.png"), "emission"),
        ] {
            fs::write(export.join(file), content).unwrap();
        }
        for variant in VARIANTS {
            fs::write(export.join(format!("{NAME}_albedo_{variant}.png")), variant).unwrap();
        }
        fs::write(root.join("models/preview.png"), "preview").unwrap();
        // Triangle counts of the worked example's tiers.
        for (tier, triangles) in [2642u64, 2054, 682, 342].into_iter().enumerate() {
            fs::write(
                export.join(format!("{NAME}_lod{tier}.glb")),
                glb(&model_json(NAME, triangles)),
            )
            .unwrap();
        }
        let mut fixture = Self {
            root,
            sidecar: Value::Null,
        };
        fixture.sidecar = fixture.worked_example();
        fixture.write_sidecar();
        fixture
    }

    pub(crate) fn export(&self) -> PathBuf {
        self.root.join("models/export")
    }
    pub(crate) fn sidecar_path(&self) -> PathBuf {
        self.export().join(format!("{NAME}.model.json"))
    }
    pub(crate) fn mods(&self) -> PathBuf {
        self.root.join("mods")
    }
    pub(crate) fn pack(&self) -> PathBuf {
        self.mods().join(PACK)
    }
    pub(crate) fn write_sidecar(&self) {
        fs::write(self.sidecar_path(), self.sidecar.to_string()).unwrap();
    }

    pub(crate) fn role(&self) -> Value {
        json!({"pack_id": PACK, "asset_id": ASSET, "display_name": "House 3",
               "zone_type": "residential", "density": "low", "level": 1})
    }

    pub(crate) fn inputs(&self) -> Inputs {
        read_inputs(&self.sidecar_path(), self.role()).unwrap()
    }

    fn digest(&self, relative: &str) -> String {
        crate::assets::archive::sha256(&self.export().join(relative)).unwrap()
    }

    // The spec's worked example, with the fixture files' real digests.
    fn worked_example(&self) -> Value {
        let file = |relative: String| json!({"file": relative, "sha256": self.digest(&relative)});
        let lods: Vec<Value> = [2642, 2054, 682, 342]
            .iter()
            .enumerate()
            .map(|(tier, triangles)| {
                let relative = format!("{NAME}_lod{tier}.glb");
                json!({"file": relative, "triangles": triangles, "sha256": self.digest(&relative)})
            })
            .collect();
        let variants: Vec<Value> = VARIANTS
            .iter()
            .map(|id| {
                let albedo = format!("{NAME}_albedo_{id}.png");
                json!({"id": id, "albedo": albedo, "sha256": self.digest(&albedo)})
            })
            .collect();
        let preview =
            crate::assets::archive::sha256(&self.root.join("models/preview.png")).unwrap();
        json!({
            "schema": "metrum-model-facts",
            "version": 1,
            "name": NAME,
            "path": "buildings/residential/rintamamiestalo_porch",
            "source": {"commit": "9c93f65ae0f3e78d40c9c86f3b65cc290965c35a", "dirty": false, "blender": "5.2.0"},
            "license": "CC-BY-4.0",
            "bounds": {"min": [-5.35, 0.0, -6.84], "max": [5.35, 9.562, 6.84]},
            "footprint": [[-5.0, -5.1], [5.0, -5.1], [5.0, 2.9], [0.3, 2.9], [0.3, 5.1], [-2.7, 5.1], [-2.7, 2.9], [-5.0, 2.9]],
            "storeys": [{"floor_y_m": 0.70, "area_m2": 86.6}, {"floor_y_m": 3.80, "area_m2": 58.0}],
            "dwellings": 1,
            "doors": [
                {"id": "front", "kind": "entrance", "position": [-1.2, 0.70, 5.1], "facing": [0, 0, 1], "width_m": 0.90, "height_m": 2.10, "main": true},
                {"id": "back", "kind": "service", "position": [1.5, 0.70, -5.1], "facing": [0, 0, -1], "width_m": 0.90, "height_m": 2.10, "main": false}
            ],
            "lods": lods,
            "textures": {
                "orm": file(format!("{NAME}_orm.png")),
                "normal": file(format!("{NAME}_normal.png")),
                "emission": file(format!("{NAME}_emission.png")),
            },
            "variants": variants,
            "default_variant": "red",
            "preview": {"file": "../preview.png", "sha256": preview},
            "hero": false
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// One mesh of `triangles` triangles in one material, its maps beside it as the models
/// export names them.
pub(crate) fn model_json(name: &str, triangles: u64) -> Value {
    let images = ["albedo_red", "orm", "normal", "emission"]
        .map(|map| json!({"uri": format!("{name}_{map}.png")}));
    json!({
        "asset": {"version": "2.0"},
        "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1, "material": 0}]}],
        "accessors": [{"count": 24}, {"count": triangles * 3}],
        "materials": [{"name": name, "pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}}],
        "textures": [{"source": 0}, {"source": 1}, {"source": 2}, {"source": 3}],
        "images": images,
    })
}

/// A GLB container holding only its JSON chunk.
pub(crate) fn glb(json: &Value) -> Vec<u8> {
    let mut chunk = json.to_string().into_bytes();
    while !chunk.len().is_multiple_of(4) {
        chunk.push(b' ');
    }
    let total = 20 + chunk.len() as u32;
    let mut bytes = Vec::new();
    for word in [0x46546c67u32, 2, total, chunk.len() as u32, 0x4e4f534a] {
        bytes.extend(word.to_le_bytes());
    }
    bytes.extend(chunk);
    bytes
}

fn facts_with(edit: impl FnOnce(&mut Value)) -> Result<Facts, String> {
    let fixture = Fixture::new();
    let mut sidecar = fixture.sidecar.clone();
    edit(&mut sidecar);
    Facts::from_value(sidecar)
}

fn close(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-9)
}

#[test]
fn the_reader_takes_the_worked_example() {
    let fixture = Fixture::new();
    let inputs = fixture.inputs();
    assert_eq!(inputs.facts.main_door().id, "front");
    assert_eq!(inputs.facts.lods.len(), 4);
    assert_eq!(inputs.role.asset_id, ASSET);
    assert_eq!(inputs.export, fixture.export());
}

#[test]
fn the_reader_is_strict() {
    let refused = |edit: fn(&mut Value), expected: &str| {
        let error = facts_with(edit).unwrap_err();
        assert!(error.contains(expected), "{expected}: {error}");
    };
    // A newer file is refused for its version, not for the keys it adds.
    refused(
        |s| {
            s["version"] = json!(2);
            s["future"] = json!(true);
        },
        "version 2",
    );
    refused(|s| s["schema"] = json!("other"), "schema");
    refused(|s| s["extra"] = json!(1), "unknown field");
    refused(|s| s["doors"][1]["main"] = json!(true), "exactly one door");
    refused(|s| s["doors"][0]["main"] = json!(false), "exactly one door");
    refused(|s| s["doors"][1]["id"] = json!("front"), "repeated");
    refused(
        |s| s["doors"][0]["facing"] = json!([0, 0, 2]),
        "unit facing",
    );
    refused(
        |s| s["lods"][0]["sha256"] = json!("ABC"),
        "64 lowercase hex",
    );
    refused(
        |s| s["lods"][0]["file"] = json!("../x.glb"),
        "inside the export folder",
    );
    refused(
        |s| s["preview"]["file"] = json!("/abs/preview.png"),
        "preview path",
    );
    refused(
        |s| s["footprint"] = json!([[0, 0], [1, 1]]),
        "simple polygon",
    );
    refused(|s| s["bounds"]["min"][1] = json!(0.5), "ground");
    refused(
        |s| s["default_variant"] = json!("purple"),
        "default_variant",
    );
    refused(|s| s["source"]["commit"] = json!("9c93f65"), "40-hex");
    refused(
        |s| s["storeys"][1]["floor_y_m"] = json!(0.1),
        "lowest first",
    );
    // Door kinds: required, from the closed list, and the main door is an entrance.
    refused(
        |s| {
            s["doors"][1].as_object_mut().unwrap().remove("kind");
        },
        "kind",
    );
    refused(
        |s| s["doors"][1]["kind"] = json!("garage"),
        "unknown variant",
    );
    refused(
        |s| s["doors"][0]["kind"] = json!("service"),
        "kind entrance",
    );
    // No fallback picks a main door the builder did not tag (Q7).
    refused(|s| s["doors"][0]["main"] = json!(false), "exactly one door");
    // Positive shoelace area in [x, z], any start point.
    refused(
        |s| {
            s["footprint"].as_array_mut().unwrap().reverse();
        },
        "positive area",
    );
    assert!(
        facts_with(|s| {
            s["footprint"].as_array_mut().unwrap().rotate_left(3);
            // Real-data shapes: a balcony door at its upper floor's height, shed and goods
            // doors beside the unit doors, an upper storey larger than the footprint.
            let door = |id: &str, kind: &str, y: f64| {
                json!({"id": id, "kind": kind, "position": [0.0, y, -5.1],
                       "facing": [0, 0, -1], "width_m": 0.8, "height_m": 1.2, "main": false})
            };
            let doors = s["doors"].as_array_mut().unwrap();
            doors.push(door("balcony", "balcony", 3.8));
            doors.push(door("shed", "store", 0.0));
            doors.push(door("goods", "vehicle", 0.0));
            s["storeys"][1]["area_m2"] = json!(120.0);
        })
        .is_ok()
    );
}

#[test]
fn inputs_refuse_dirty_sources_changed_files_and_bad_roles() {
    let mut fixture = Fixture::new();
    let role = fixture.role();
    fs::write(fixture.export().join(format!("{NAME}_orm.png")), "changed").unwrap();
    let error = read_inputs(&fixture.sidecar_path(), role.clone())
        .err()
        .unwrap();
    assert!(error.contains("differs"), "{error}");
    fixture.sidecar = fixture.worked_example();
    fixture.sidecar["source"]["dirty"] = json!(true);
    fixture.write_sidecar();
    let error = read_inputs(&fixture.sidecar_path(), role.clone())
        .err()
        .unwrap();
    assert!(error.contains("dirty"), "{error}");
    fixture.sidecar["source"]["dirty"] = json!(false);
    fixture.write_sidecar();
    for (key, value, expected) in [
        ("zone_type", json!("service"), "zoned building"),
        ("level", json!(0), "level"),
        ("asset_id", json!("Bad Id"), "asset id"),
        ("economy_profile", json!("grocery_basic"), "economy_profile"),
        ("colour", json!("red"), "unknown field"),
        ("lot_cells", json!([2, 0]), "at least 1"),
        // A workplace without a profile never spawns: refused up front.
        ("zone_type", json!("commercial"), "needs an economy_profile"),
    ] {
        let mut role = role.clone();
        role[key] = value;
        let error = read_inputs(&fixture.sidecar_path(), role).err().unwrap();
        assert!(error.contains(expected), "{key}: {error}");
    }
}

#[test]
fn r1_lot_cells_cover_the_bounds_with_the_editor_tolerance() {
    assert_eq!(rules::lot_cells(10.70), 2);
    assert_eq!(rules::lot_cells(13.68), 2);
    assert_eq!(rules::lot_cells(20.0), 2);
    assert_eq!(rules::lot_cells(20.009), 2);
    assert_eq!(rules::lot_cells(20.02), 3);
    assert_eq!(rules::lot_cells(0.5), 1);
    // The evidence table: rowhouse_1, the grocery and the factory (one cell less deep than
    // its hand value, which added a yard cell).
    assert_eq!(rules::lot_cells(36.78), 4);
    assert_eq!([rules::lot_cells(35.0), rules::lot_cells(21.0)], [4, 3]);
    assert_eq!([rules::lot_cells(42.60), rules::lot_cells(28.25)], [5, 3]);
    let fixture = Fixture::new();
    assert_eq!(rules::tight_lot(&fixture.inputs().facts), [2, 2]);
}

// The worked example resized to `width` × `depth` metres with `dwellings`.
fn sized(width: f64, depth: f64, dwellings: u32) -> Facts {
    let fixture = Fixture::new();
    let mut facts = fixture.inputs().facts;
    facts.bounds.min = [-width / 2.0, 0.0, -depth / 2.0];
    facts.bounds.max = [width / 2.0, 9.0, depth / 2.0];
    facts.dwellings = dwellings;
    facts
}

#[test]
fn r1_gives_a_detached_house_its_yard_and_anything_else_the_tight_lot() {
    // `rintamamiestalo_gable_entry`: 9.08 m wide needs 3 m each side, so 2 cells across.
    assert_eq!(rules::lot(&sized(9.08, 14.98, 1), None).unwrap(), [2, 2]);
    // The other three houses stay 2 × 2.
    for (width, depth) in [(10.70, 14.08), (10.70, 13.68), (10.70, 12.44)] {
        assert_eq!(rules::lot(&sized(width, depth, 1), None).unwrap(), [2, 2]);
    }
    // The yard margins with the 0.01 m slack: 14.01 + 6 m fits 2 cells, 14.03 does not.
    assert_eq!(rules::lot(&sized(14.01, 10.0, 1), None).unwrap(), [2, 2]);
    assert_eq!(rules::lot(&sized(14.03, 16.02, 1), None).unwrap(), [3, 3]);
    // A terrace and a shop keep the tight fit.
    assert_eq!(rules::lot(&sized(9.08, 14.98, 4), None).unwrap(), [1, 2]);
    assert_eq!(rules::lot(&sized(35.0, 21.0, 0), None).unwrap(), [4, 3]);
}

#[test]
fn r1_uses_an_ordered_lot_as_is_and_refuses_one_the_model_does_not_fit() {
    let shop = sized(35.0, 21.0, 0);
    assert_eq!(rules::lot(&shop, Some([5, 4])).unwrap(), [5, 4]);
    assert_eq!(rules::lot(&shop, Some([4, 3])).unwrap(), [4, 3]);
    let error = rules::lot(&shop, Some([4, 2])).unwrap_err();
    assert!(error.contains("[4, 3]"), "{error}");
    // An ordered lot wins over the house yard, too.
    assert_eq!(
        rules::lot(&sized(9.08, 14.98, 1), Some([1, 2])).unwrap(),
        [1, 2]
    );
    let fixture = Fixture::new();
    let mut role = fixture.role();
    role["lot_cells"] = json!([3, 4]);
    let inputs = read_inputs(&fixture.sidecar_path(), role.clone()).unwrap();
    let derived = derive(&inputs, &fixture.mods(), &fixture.root).unwrap();
    let params = &derived.document["params"];
    assert_eq!(
        [&params["lot_width_cells"], &params["lot_depth_cells"]],
        [3, 4]
    );
    // R2 stays back-flush in the ordered lot: z = −20 + 6.84.
    let z = params["mesh_parts"][0]["position"][2].as_f64().unwrap();
    assert!((z + 13.16).abs() < 1e-9, "{z}");
    role["lot_cells"] = json!([1, 1]);
    let inputs = read_inputs(&fixture.sidecar_path(), role).unwrap();
    let error = derive(&inputs, &fixture.mods(), &fixture.root)
        .err()
        .unwrap();
    assert!(error.contains("too small"), "{error}");
}

#[test]
fn r2_places_the_part_centred_across_and_on_the_back_edge() {
    let fixture = Fixture::new();
    let facts = fixture.inputs().facts;
    // `house_3` by hand: z −3.16, the same.
    assert!(close(rules::part_position(&facts, 2), [0.0, 0.0, -3.16]));
    let mut offset = facts.clone();
    offset.bounds.min[0] = -4.0;
    assert!(close(
        rules::part_position(&offset, 3),
        [-0.675, 0.0, -8.16]
    ));
    assert_eq!(
        rules::free_space(&facts, [2, 2], rules::part_position(&facts, 2)),
        json!({"front_m": 6.32, "back_m": 0.0, "left_m": 4.65, "right_m": 4.65})
    );
}

#[test]
fn r4_puts_the_entrance_on_the_main_door_wall_plane_at_ground_level() {
    let fixture = Fixture::new();
    let facts = fixture.inputs().facts;
    let part = rules::part_position(&facts, 2);
    let entrance = rules::entrance(part, facts.main_door());
    let position: [f64; 3] = serde_json::from_value(entrance["position"].clone()).unwrap();
    assert!(close(position, [-1.2, 0.0, 1.94]));
    assert_eq!(entrance["forward"], json!([0.0, 0.0, 1.0]));
    assert_eq!(entrance["anchor_type"], "entrance");
    assert_eq!(entrance["name"], "main");
}

#[test]
fn r5_flat_size_rounds_to_ten_and_caps_one_dwelling_at_80() {
    let fixture = Fixture::new();
    let mut facts = fixture.inputs().facts;
    // 0.8 × 144.6 = 115.7 → 120, capped at 80 for one dwelling.
    assert_eq!(rules::flat_size_m2(&facts), Some(80.0));
    let plate = |facts: &mut Facts, per_dwelling: f64, dwellings: u32| {
        facts.dwellings = dwellings;
        facts.storeys = vec![super::sidecar::Storey {
            floor_y_m: 0.0,
            area_m2: per_dwelling * f64::from(dwellings),
        }];
        rules::flat_size_m2(facts)
    };
    // The terraces: 102 and 120 m² wall plates per dwelling.
    assert_eq!(plate(&mut facts, 102.0, 6), Some(80.0));
    assert_eq!(plate(&mut facts, 120.0, 4), Some(100.0));
    // Halves round away from zero: 0.8 × 106.25 = 85 → 90.
    assert_eq!(plate(&mut facts, 106.25, 2), Some(90.0));
    assert_eq!(plate(&mut facts, 60.0, 1), Some(50.0));
    assert_eq!(plate(&mut facts, 2.0, 1), None);
    assert_eq!(plate(&mut facts, 100.0, 0), None);
}

#[test]
fn r6_bands_follow_the_tier_count() {
    assert_eq!(
        rules::lod_bands(4).unwrap(),
        [
            (0.0, Some(35.0)),
            (35.0, Some(70.0)),
            (70.0, Some(120.0)),
            (120.0, None)
        ]
    );
    assert_eq!(rules::lod_bands(1).unwrap(), [(0.0, None)]);
    assert!(rules::lod_bands(5).is_err());
    assert!(rules::lod_bands(0).is_err());
}

#[test]
fn r7_one_scheme_per_variant_sorted_by_id() {
    assert_eq!(rules::scheme_name("dark_red"), "Dark Red");
    assert_eq!(rules::scheme_name("red"), "Red");
    let fixture = Fixture::new();
    let facts = fixture.inputs().facts;
    let materials = vec![NAME.to_owned(); 4];
    let appearance = rules::appearance(&facts, "part.glb", &materials);
    let ids: Vec<_> = appearance["schemes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["blue", "green", "red", "white", "yellow"]);
    assert_eq!(appearance["default_scheme"], "red");
    assert_eq!(appearance["spawn"], "random_scheme");
    assert_eq!(
        appearance["schemes"][2]["overrides"][0],
        json!({"part": "part.glb", "materials": materials,
               "albedo": format!("colours/{NAME}_albedo_red.png")})
    );
    let parsed: crate::assets::asset::BuildingAppearance =
        serde_json::from_value(appearance).unwrap();
    assert_eq!(parsed.schemes[0].name, "Blue");
}

#[test]
fn r7_stops_a_tier_with_other_than_one_material() {
    let fixture = Fixture::new();
    let mut model = model_json(NAME, 342);
    model["materials"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "glass"}));
    let path = fixture.export().join(format!("{NAME}_lod3.glb"));
    fs::write(&path, glb(&model)).unwrap();
    let mut fixture = fixture;
    fixture.sidecar["lods"][3]["sha256"] = json!(crate::assets::archive::sha256(&path).unwrap());
    fixture.write_sidecar();
    let error = derive(&fixture.inputs(), &fixture.mods(), &fixture.root)
        .err()
        .unwrap();
    assert!(error.contains("exactly one named material"), "{error}");
}

#[test]
fn r8_crops_the_preview_about_its_centre_to_four_by_three() {
    assert_eq!(rules::thumbnail_crop([1600, 1000]), [133, 0, 1333, 1000]);
    assert_eq!(rules::thumbnail_crop([1024, 768]), [0, 0, 1024, 768]);
    assert_eq!(rules::thumbnail_crop([800, 800]), [0, 100, 800, 600]);
}

#[test]
fn r9_and_the_document_carry_the_presets_and_role_inputs_only() {
    let fixture = Fixture::new();
    let derived = derive(&fixture.inputs(), &fixture.mods(), &fixture.root).unwrap();
    let params = &derived.document["params"];
    assert_eq!(params["window_brightness"], 3.0);
    assert_eq!(params["placement_mode"], "zoned_private");
    assert_eq!(params["frontage_forward"], json!([0.0, 0.0, 1.0]));
    assert_eq!(params["thumbnail"], "thumbnail.webp");
    assert_eq!(params["household_capacity"], 1);
    assert_eq!(params["flat_size_m2"], 80.0);
    assert_eq!(params["pack_name"], "Draft Test");
    for absent in [
        "asset_set",
        "economy_profile",
        "min_zone_width_cells",
        "yard_hedge",
    ] {
        assert!(params[absent].is_null(), "{absent}");
    }
    assert_eq!(params["site_surfaces"], json!([]));
    let part = &params["mesh_parts"][0];
    assert_eq!(part["name"], format!("{NAME}_lod0.glb"));
    assert_eq!(part["pivot_offset"], json!([0.0, 0.0, 0.0]));
    assert_eq!(
        part["lods"][3],
        json!({"file": format!("{NAME}_lod3.glb"), "distance_min_m": 120.0})
    );
    assert_eq!(derived.document["sources"][0].as_array().unwrap().len(), 4);
    assert_eq!(
        derived.document["colour_sources"][format!("colours/{NAME}_albedo_blue.png")],
        json!(fixture.export().join(format!("{NAME}_albedo_blue.png")))
    );
    assert_eq!(derived.preview, fixture.export().join("../preview.png"));
    let mut role = fixture.role();
    role["asset_set"] = json!("veterans");
    let inputs = read_inputs(&fixture.sidecar_path(), role).unwrap();
    let derived = derive(&inputs, &fixture.mods(), &fixture.root).unwrap();
    assert_eq!(derived.document["params"]["asset_set"], "veterans");
    // The same inputs derive the same document.
    let again = derive(&inputs, &fixture.mods(), &fixture.root).unwrap();
    assert_eq!(derived.document, again.document);
}

#[test]
fn a_missing_pack_is_a_refusal() {
    let fixture = Fixture::new();
    let mut role = fixture.role();
    role["pack_id"] = json!("absent");
    let inputs = read_inputs(&fixture.sidecar_path(), role).unwrap();
    assert!(derive(&inputs, &fixture.mods(), &fixture.root).is_err());
}

#[test]
fn anything_at_the_asset_path_blocks_the_draft() {
    let fixture = Fixture::new();
    let inputs = fixture.inputs();
    let target = fixture.pack().join("assets").join(ASSET);
    assert!(existing(&fixture.pack(), &inputs).is_none());
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "a file").unwrap();
    assert!(existing(&fixture.pack(), &inputs).is_some());
    fs::remove_file(&target).unwrap();
    fs::create_dir(&target).unwrap();
    let lines = existing(&fixture.pack(), &inputs).unwrap();
    assert_eq!(lines[0].field, "asset_id");
    fs::remove_dir(&target).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(fixture.root.join("nowhere"), &target).unwrap();
        assert!(existing(&fixture.pack(), &inputs).is_some());
    }
}

#[test]
fn the_ledger_round_trips_through_the_strict_reader() {
    let fixture = Fixture::new();
    assert_eq!(read_ledger(&fixture.pack(), ASSET).unwrap(), None);
    let path = write_ledger(&fixture.pack(), ASSET, &fixture.sidecar, &fixture.role()).unwrap();
    assert_eq!(
        path,
        fixture
            .pack()
            .join("model_sources")
            .join(format!("{ASSET}.json"))
    );
    assert_eq!(
        read_ledger(&fixture.pack(), ASSET).unwrap(),
        Some(Facts::from_value(fixture.sidecar.clone()).unwrap())
    );
    fs::write(&path, "{\"sidecar\": {}, \"role\": {}}").unwrap();
    assert!(read_ledger(&fixture.pack(), ASSET).is_err());
    assert!(Role::parse(fixture.role()).is_ok());
}

// `house_3` as the owner's pack holds it, with a level-1 hand entrance and asphalt walk.
fn hand_manifest(entrance: [f32; 3], surface: &str) -> crate::assets::AssetManifest {
    let schemes: String = VARIANTS
        .iter()
        .map(|id| {
            format!(
                "\n[[building.appearance.schemes]]\nid = \"{id}\"\nname = \"{id}\"\n\
                 [[building.appearance.schemes.overrides]]\npart = \"{NAME}_lod0.glb\"\n\
                 materials = [\"{NAME}\"]\nalbedo = \"colours/{NAME}_albedo_{id}.png\"\n"
            )
        })
        .collect();
    let [x, y, z] = entrance;
    crate::assets::AssetManifest::from_str(&format!(
        "asset_id = \"{ASSET}\"\ndisplay_name = \"House 3\"\n[building]\n\
         placement_mode = \"zoned_private\"\nzone_type = \"residential\"\ndensity = \"low\"\n\
         lot_width_cells = 2\nlot_depth_cells = 2\nlevel = 1\nhousehold_capacity = 1\n\
         flat_size_m2 = 80\n[[mesh_parts]]\nname = \"{NAME}_lod0.glb\"\n\
         position = [-0.036335707, 0, -3.1599998]\n[[mesh_parts.lods]]\n\
         file = \"{NAME}_lod0.glb\"\ndistance_min_m = 0\n[[anchors]]\ntype = \"entrance\"\n\
         name = \"main\"\nposition = [{x}, {y}, {z}]\nforward = [0, 0, 1]\n\
         [[site_surfaces]]\nmaterial = \"asphalt\"\nname = \"walk\"\nvertices = {surface}\n\
         [building.appearance]\ndefault_scheme = \"red\"\n{schemes}"
    ))
    .unwrap()
}

fn published(fixture: &Fixture, facts: &Facts) -> BTreeMap<String, String> {
    let mut files: BTreeMap<String, String> = facts
        .hashed_files()
        .into_iter()
        .filter(|(file, _)| !file.starts_with(".."))
        .map(|(file, digest)| (file.to_owned(), digest.to_owned()))
        .collect();
    for variant in &facts.variants {
        files.insert(rules::colour_path(&variant.albedo), variant.sha256.clone());
    }
    // The hand asset publishes LOD0 only.
    files.retain(|file, _| !file.contains("_lod") || file.ends_with("_lod0.glb"));
    let _ = fixture;
    files
}

fn checks(lines: &[Line]) -> Vec<&str> {
    lines.iter().map(|line| line.check).collect()
}

#[test]
fn the_stale_report_names_each_difference_and_its_editor_field() {
    let fixture = Fixture::new();
    let facts = fixture.inputs().facts;
    let walk = "[[-3.57, 3], [1.43, 3], [1.43, 10], [-3.57, 10]]";
    // The hand house: anchor 0.37 m from the door, walk in front of the porch.
    let manifest = hand_manifest([-1.14, 0.0, 1.58], walk);
    let mut files = published(&fixture, &facts);
    let lines = stale_report(&facts, None, &manifest, &files);
    // Tiers 1-3 exist in the model only.
    assert_eq!(checks(&lines), ["files", "files", "files"], "{lines:?}");
    assert!(lines.iter().all(|line| line.field == "mesh_parts"));

    // A changed texture, an entrance off the door, a walk under the porch, a missing scheme
    // and one more dwelling.
    files.insert(format!("{NAME}_orm.png"), "0".repeat(64));
    let mut moved = facts.clone();
    moved.dwellings = 2;
    moved.variants.push(super::sidecar::Variant {
        id: "grey".into(),
        albedo: format!("{NAME}_albedo_grey.png"),
        sha256: "1".repeat(64),
    });
    let manifest = hand_manifest(
        [-3.0, 0.0, 1.58],
        "[[-3.57, 0], [1.43, 0], [1.43, 10], [-3.57, 10]]",
    );
    let lines = stale_report(&moved, Some(&facts), &manifest, &files);
    let found = checks(&lines);
    for expected in [
        "files",
        "facts",
        "entrance",
        "site_layout",
        "schemes",
        "capacity",
    ] {
        assert!(found.contains(&expected), "{expected}: {lines:?}");
    }
    assert!(
        lines
            .iter()
            .any(|l| l.check == "files" && l.field == "appearance")
    );
    assert!(
        lines
            .iter()
            .any(|l| l.check == "capacity" && l.field == "household_capacity")
    );

    // With a ledger, the door's own move is reported from 0.05 m.
    let mut door_moved = facts.clone();
    door_moved.doors[0].position[0] += 0.06;
    let manifest = hand_manifest([-1.14, 0.0, 1.58], walk);
    let lines = stale_report(&door_moved, Some(&facts), &manifest, &files);
    assert!(
        lines
            .iter()
            .any(|l| l.check == "entrance" && l.message.contains("moved"))
    );
    assert!(
        lines
            .iter()
            .any(|l| l.check == "facts" && l.message.contains("door 'front'"))
    );

    // Bounds that no longer fit the lot through the present part.
    let mut grown = facts.clone();
    grown.bounds.max[0] = 12.0;
    let lines = stale_report(&grown, None, &manifest, &files);
    assert!(lines.iter().any(|l| l.check == "lot_fit"));
}

#[test]
fn triangles_come_from_the_index_counts() {
    let fixture = Fixture::new();
    let path = fixture.export().join(format!("{NAME}_lod0.glb"));
    assert_eq!(acceptance::glb_triangles(&path).unwrap(), 2642);
}

#[test]
fn lot_fit_and_overlap_geometry() {
    assert!(fits_lot(
        [[-10.0, -10.0], [10.005, 10.0]],
        [10.0, 10.0],
        0.01
    ));
    assert!(!fits_lot(
        [[-10.0, -10.0], [10.02, 10.0]],
        [10.0, 10.0],
        0.01
    ));
    let square = [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]];
    let inner = [[0.5, 0.5], [1.0, 0.5], [1.0, 1.0]];
    let apart = [[3.0, 3.0], [4.0, 3.0], [4.0, 4.0]];
    assert!(polygons_overlap(&square, &inner));
    assert!(polygons_overlap(&inner, &square));
    assert!(!polygons_overlap(&square, &apart));
    let mut part = crate::assets::MeshPart::single_lod0("p", "p.glb");
    part.rotation_degrees = [0.0, 90.0, 0.0];
    part.position = [1.0, 0.0, 0.0];
    let plan = part.plan_bounds([[-2.0, 0.0, -1.0], [2.0, 3.0, 1.0]]);
    for (actual, expected) in plan.iter().flatten().zip([0.0, -2.0, 2.0, 2.0]) {
        assert!((actual - expected).abs() < 1e-5, "{plan:?}");
    }
}
