// SPDX-License-Identifier: GPL-2.0-only

//! Asset manifest parsing and validation regression tests.

use super::*;

// ── Building ────────────────────────────────────────────────────────────

const BUILDING_TOML: &str = r#"
asset_id = "building.residential.lowrise_corner"
display_name = "Low-rise Corner Building"
tags = ["residential", "corner"]
asset_set = "lowrise_residential"

[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 3
lot_depth_cells = 3
household_capacity = 12
service_class = "standard"
economy_profile = "residential_basic"

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 4.5]
forward = [0.0, 0.0, 1.0]

[[anchors]]
type = "parking"
position = [-2.5, 0.0, 1.0]
forward = [0.0, 0.0, 1.0]
width_m = 2.5
length_m = 5.0
vehicle_class = "car"

[[site_surfaces]]
material = "concrete"
name = "front_walk"
y_m = 0.01
vertices = [[-0.7, 1.0], [0.7, 1.0], [0.7, 7.0], [-0.7, 7.0]]

[[mesh_parts]]
name = "main"
position = [0.0, 0.0, 0.0]
rotation_degrees = [0.0, 0.0, 0.0]
scale = 1.0

[[mesh_parts.lods]]
file = "lod0.glb"
distance_min_m = 0.0
distance_max_m = 150.0

[[mesh_parts.lods]]
file = "lod1.glb"
distance_min_m = 150.0
distance_max_m = 600.0
"#;

#[test]
fn building_manifest_round_trip() {
    let m = AssetManifest::from_str(BUILDING_TOML).expect("parse failed");
    assert_eq!(m.asset_id, "building.residential.lowrise_corner");
    assert_eq!(m.display_name, "Low-rise Corner Building");
    assert_eq!(m.class().unwrap(), AssetClass::Building);
    let b = m.building.as_ref().unwrap();
    assert_eq!(b.zone_type, Some(ZoneClass::Residential));
    assert_eq!(b.placement_mode, PlacementMode::ZonedPrivate);
    assert_eq!(b.lot_width_cells, 3);
    assert_eq!(b.lot_depth_cells, 3);
    assert_eq!(b.household_capacity, Some(12));
    assert_eq!(b.economy_profile.as_deref(), Some("residential_basic"));
    assert_eq!(m.lods.len(), 0);
    assert_eq!(m.mesh_parts.len(), 1);
    assert_eq!(m.mesh_parts[0].name, "main");
    assert_eq!(m.mesh_parts[0].lods.len(), 2);
    assert_eq!(m.anchors.len(), 2);
    assert_eq!(m.anchors[0].anchor_type, AnchorType::Entrance);
    assert_eq!(m.anchors[0].position, [0.0, 0.0, 4.5]);
    assert_eq!(m.anchors[1].anchor_type, AnchorType::Parking);
    assert_eq!(m.anchors[1].name, "");
    assert_eq!(m.anchors[1].width_m, Some(2.5));
    assert_eq!(m.anchors[1].length_m, Some(5.0));
    assert_eq!(m.anchors[1].vehicle_class.as_deref(), Some("car"));
    assert_eq!(m.site_surfaces.len(), 1);
    assert_eq!(m.site_surfaces[0].material, SiteSurfaceMaterial::Concrete);
    assert_eq!(m.site_surfaces[0].name, "front_walk");
    assert_eq!(m.site_surfaces[0].y_m, 0.01);
    assert_eq!(m.site_surfaces[0].vertices.len(), 4);
}

#[test]
fn building_frontage_forward_defaults_to_main_entrance_forward() {
    let m = BUILDING_TOML
        .parse::<AssetManifest>()
        .expect("parse failed");
    assert_eq!(m.building_frontage_forward(), [0.0, 0.0, 1.0]);
}

#[test]
fn building_frontage_forward_defaults_to_legacy_driveway_when_present() {
    let toml = r#"
asset_id = "building.industrial.legacy_driveway"
display_name = "Legacy Driveway"
[building]
placement_mode = "zoned_private"
zone_type = "industrial"
density = "low"
lot_width_cells = 2
lot_depth_cells = 2
worker_capacity = 4

[[anchors]]
type = "entrance"
name = "main"
position = [2.0, 0.0, 3.0]
forward = [-1.0, 0.0, 0.0]

[[anchors]]
type = "driveway"
position = [0.0, 0.0, -10.0]
forward = [0.0, 0.0, 1.0]
width_m = 3.0

[[mesh_parts]]
name = "main"

[[mesh_parts.lods]]
file = "lod0.glb"
distance_min_m = 0.0
"#;
    let m = toml.parse::<AssetManifest>().expect("parse failed");
    assert_eq!(m.building_frontage_forward(), [0.0, 0.0, -1.0]);
}

#[test]
fn building_frontage_forward_can_differ_from_entrance_forward() {
    let toml = BUILDING_TOML
        .replace(
            "lot_depth_cells = 3",
            "lot_depth_cells = 3\nfrontage_forward = [1.0, 0.0, 0.0]",
        )
        .replace(
            "name = \"main\"\nposition = [0.0, 0.0, 4.5]\nforward = [0.0, 0.0, 1.0]",
            "name = \"main\"\nposition = [0.0, 0.0, 4.5]\nforward = [0.0, 0.0, -1.0]",
        );
    let m = toml.parse::<AssetManifest>().expect("parse failed");
    assert_eq!(m.building_frontage_forward(), [1.0, 0.0, 0.0]);
    assert_eq!(m.anchors[0].forward, [0.0, 0.0, -1.0]);
}

#[test]
fn building_rejects_top_level_lods() {
    let toml = r#"
asset_id = "building.residential.bad"
display_name = "Bad"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 2
lot_depth_cells = 2
household_capacity = 1

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 2.0]
forward = [0.0, 0.0, 1.0]

[[lods]]
file = "lod0.glb"
distance_min_m = 0.0
"#;
    assert!(toml.parse::<AssetManifest>().is_err());
}

#[test]
fn building_rejects_unknown_top_level_manifest_field() {
    let toml = BUILDING_TOML.replace(
        "display_name = \"Low-rise Corner Building\"",
        "display_name = \"Low-rise Corner Building\"\nlegacy_field = true",
    );
    let err = toml.parse::<AssetManifest>().unwrap_err();
    assert!(
        err.to_string().contains("unknown field"),
        "expected unknown-field parse error, got: {err}"
    );
}

#[test]
fn building_rejects_unknown_anchor_field() {
    let toml = BUILDING_TOML.replace(
        "vehicle_class = \"car\"",
        "vehicle_class = \"car\"\npurpose = \"resident\"",
    );
    let err = toml.parse::<AssetManifest>().unwrap_err();
    assert!(
        err.to_string().contains("unknown field"),
        "expected unknown-field parse error, got: {err}"
    );
}

#[test]
fn building_rejects_unknown_site_surface_field() {
    let toml = BUILDING_TOML.replace("y_m = 0.01", "y_m = 0.01\nwidth_m = 1.4");
    let err = toml.parse::<AssetManifest>().unwrap_err();
    assert!(
        err.to_string().contains("unknown field"),
        "expected unknown-field parse error, got: {err}"
    );
}

#[test]
fn building_qualified_id() {
    let m = BUILDING_TOML.parse::<AssetManifest>().unwrap();
    assert_eq!(
        m.qualified_id("kenney-city-pack"),
        "kenney-city-pack:building.residential.lowrise_corner"
    );
}

#[test]
fn building_rejects_zero_lot_cells() {
    let toml = r#"
asset_id = "building.residential.bad"
display_name = "Bad"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 0
lot_depth_cells = 3
"#;
    assert!(toml.parse::<AssetManifest>().is_err());
}

#[test]
fn building_rejects_missing_main_entrance() {
    let toml = r#"
asset_id = "building.residential.bad"
display_name = "Bad"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 2
lot_depth_cells = 2
"#;
    assert!(toml.parse::<AssetManifest>().is_err());
}

#[test]
fn building_rejects_secondary_entrance_anchor() {
    let toml = r#"
asset_id = "building.residential.bad"
display_name = "Bad"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 2
lot_depth_cells = 2

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 2.0]
forward = [0.0, 0.0, 1.0]

[[anchors]]
type = "entrance"
name = "rear"
position = [0.0, 0.0, -2.0]
forward = [0.0, 0.0, -1.0]
"#;
    assert!(toml.parse::<AssetManifest>().is_err());
}

#[test]
fn building_rejects_duplicate_main_entrance_anchor() {
    let toml = r#"
asset_id = "building.residential.bad"
display_name = "Bad"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 2
lot_depth_cells = 2

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 2.0]
forward = [0.0, 0.0, 1.0]

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, -2.0]
forward = [0.0, 0.0, -1.0]
"#;
    assert!(toml.parse::<AssetManifest>().is_err());
}

#[test]
fn building_rejects_anchor_outside_lot() {
    let toml = BUILDING_TOML.replace("position = [-2.5, 0.0, 1.0]", "position = [16.0, 0.0, 1.0]");
    let err = toml.parse::<AssetManifest>().unwrap_err();
    assert!(
        err.to_string().contains("outside the building lot bounds"),
        "expected outside-lot validation error, got: {err}"
    );
}

#[test]
fn building_rejects_site_anchor_footprint_outside_lot() {
    let toml = BUILDING_TOML.replace(
        "position = [-2.5, 0.0, 1.0]",
        "position = [-2.5, 0.0, 12.0]",
    );
    let err = toml.parse::<AssetManifest>().unwrap_err();
    assert!(
        err.to_string().contains("site footprint"),
        "expected footprint validation error, got: {err}"
    );
}

#[test]
fn building_rejects_site_surface_vertex_outside_lot() {
    let toml = BUILDING_TOML.replace(
        "vertices = [[-0.7, 1.0], [0.7, 1.0], [0.7, 7.0], [-0.7, 7.0]]",
        "vertices = [[-0.7, 1.0], [16.0, 1.0], [0.7, 7.0], [-0.7, 7.0]]",
    );
    let err = toml.parse::<AssetManifest>().unwrap_err();
    assert!(
        err.to_string().contains("crosses the building lot bounds"),
        "expected site-surface bounds validation error, got: {err}"
    );
}

#[test]
fn building_rejects_site_surface_self_intersection() {
    let toml = BUILDING_TOML.replace(
        "vertices = [[-0.7, 1.0], [0.7, 1.0], [0.7, 7.0], [-0.7, 7.0]]",
        "vertices = [[-2.0, 0.0], [2.0, 0.0], [-2.0, 2.0], [2.0, 2.0], [2.0, 4.0], [-2.0, 4.0]]",
    );
    let err = toml.parse::<AssetManifest>().unwrap_err();
    assert!(
        err.to_string().contains("polygon self-intersects"),
        "expected site-surface self-intersection validation error, got: {err}"
    );
}

#[test]
fn building_accepts_driveway_on_frontage_edge() {
    let toml = r#"
asset_id = "building.residential.good"
display_name = "Good"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 2
lot_depth_cells = 2
frontage_forward = [0.0, 0.0, -1.0]
household_capacity = 1

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, -10.0]
forward = [0.0, 0.0, -1.0]

[[anchors]]
type = "driveway"
position = [0.0, 0.0, -10.0]
forward = [0.0, 0.0, 1.0]
width_m = 3.0

[[mesh_parts]]
name = "main"

[[mesh_parts.lods]]
file = "lod0.glb"
distance_min_m = 0.0
"#;
    toml.parse::<AssetManifest>().unwrap();
}

#[test]
fn building_accepts_driveway_with_legacy_edge_drift() {
    let toml = r#"
asset_id = "building.residential.good"
display_name = "Good"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 2
lot_depth_cells = 2
household_capacity = 1

[[anchors]]
type = "entrance"
name = "main"
position = [4.0, 0.0, -2.7]
forward = [0.0, 0.0, -1.0]

[[anchors]]
type = "driveway"
position = [0.76, 0.0, -9.97]
forward = [0.0, 0.0, 1.0]
width_m = 3.0

[[mesh_parts]]
name = "main"

[[mesh_parts.lods]]
file = "lod0.glb"
distance_min_m = 0.0
"#;
    toml.parse::<AssetManifest>().unwrap();
}

#[test]
fn building_rejects_driveway_away_from_frontage_edge() {
    let toml = r#"
asset_id = "building.residential.bad"
display_name = "Bad"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 2
lot_depth_cells = 2
frontage_forward = [0.0, 0.0, -1.0]
household_capacity = 1

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, -10.0]
forward = [0.0, 0.0, -1.0]

[[anchors]]
type = "driveway"
position = [0.0, 0.0, 0.0]
forward = [0.0, 0.0, 1.0]
width_m = 3.0

[[mesh_parts]]
name = "main"

[[mesh_parts.lods]]
file = "lod0.glb"
distance_min_m = 0.0
"#;
    let err = toml.parse::<AssetManifest>().unwrap_err();
    assert!(
        err.to_string().contains("frontage edge"),
        "expected driveway frontage-edge validation error, got: {err}"
    );
}

#[test]
fn building_rejects_driveway_forward_away_from_lot() {
    let toml = r#"
asset_id = "building.residential.bad"
display_name = "Bad"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 2
lot_depth_cells = 2
frontage_forward = [0.0, 0.0, -1.0]
household_capacity = 1

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, -10.0]
forward = [0.0, 0.0, -1.0]

[[anchors]]
type = "driveway"
position = [0.0, 0.0, -10.0]
forward = [0.0, 0.0, -1.0]
width_m = 3.0

[[mesh_parts]]
name = "main"

[[mesh_parts.lods]]
file = "lod0.glb"
distance_min_m = 0.0
"#;
    let err = toml.parse::<AssetManifest>().unwrap_err();
    assert!(
        err.to_string().contains("point inward"),
        "expected driveway inward-direction validation error, got: {err}"
    );
}

#[test]
fn building_rejects_driveway_with_diagonal_frontage() {
    let toml = r#"
asset_id = "building.residential.bad"
display_name = "Bad"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 2
lot_depth_cells = 2
frontage_forward = [0.70710677, 0.0, 0.70710677]
household_capacity = 1

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 10.0]
forward = [0.0, 0.0, 1.0]

[[anchors]]
type = "driveway"
position = [0.0, 0.0, 10.0]
forward = [0.0, 0.0, -1.0]
width_m = 3.0

[[mesh_parts]]
name = "main"

[[mesh_parts.lods]]
file = "lod0.glb"
distance_min_m = 0.0
"#;
    let err = toml.parse::<AssetManifest>().unwrap_err();
    assert!(
        err.to_string().contains("cardinal"),
        "expected driveway cardinal-frontage validation error, got: {err}"
    );
}

#[test]
fn building_rejects_driveway_footprint_outside_lot() {
    let toml = r#"
asset_id = "building.residential.bad"
display_name = "Bad"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 2
lot_depth_cells = 2
household_capacity = 1

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, -10.0]
forward = [0.0, 0.0, -1.0]

[[anchors]]
type = "driveway"
position = [0.0, 0.0, -10.0]
forward = [0.0, 0.0, 1.0]
width_m = 25.0

[[mesh_parts]]
name = "main"

[[mesh_parts.lods]]
file = "lod0.glb"
distance_min_m = 0.0
"#;
    let err = toml.parse::<AssetManifest>().unwrap_err();
    assert!(
        err.to_string().contains("site footprint"),
        "expected driveway footprint validation error, got: {err}"
    );
}

#[test]
fn building_rejects_zero_anchor_forward() {
    let toml = BUILDING_TOML.replace("forward = [0.0, 0.0, 1.0]", "forward = [0.0, 0.0, 0.0]");
    let err = toml.parse::<AssetManifest>().unwrap_err();
    assert!(
        err.to_string().contains("must be a non-zero unit vector"),
        "expected forward-vector validation error, got: {err}"
    );
}

#[test]
fn building_rejects_non_unit_anchor_forward() {
    let toml = BUILDING_TOML.replace("forward = [0.0, 0.0, 1.0]", "forward = [0.0, 0.0, 2.0]");
    let err = toml.parse::<AssetManifest>().unwrap_err();
    assert!(
        err.to_string().contains("must be a non-zero unit vector"),
        "expected forward-vector validation error, got: {err}"
    );
}

#[test]
fn building_rejects_invalid_anchor_vehicle_class() {
    let toml = BUILDING_TOML.replace("vehicle_class = \"car\"", "vehicle_class = \"hovercraft\"");
    let err = toml.parse::<AssetManifest>().unwrap_err();
    assert!(
        err.to_string().contains("invalid vehicle_class"),
        "expected vehicle-class validation error, got: {err}"
    );
}

// ── Prop ────────────────────────────────────────────────────────────────

const PROP_TOML: &str = r#"
asset_id = "prop.street.bench_01"
display_name = "Street Bench"
tags = ["street_furniture"]

[prop]
category = "street_furniture"
bounding_size_m = [1.5, 0.9, 0.6]
snap_mode = "edge"
terrain_behavior = "flat_ground"

[[lods]]
file = "bench.glb"
distance_min_m = 0.0
"#;

#[test]
fn prop_manifest_round_trip() {
    let m = PROP_TOML.parse::<AssetManifest>().expect("parse failed");
    assert_eq!(m.class().unwrap(), AssetClass::Prop);
    let p = m.prop.as_ref().unwrap();
    assert_eq!(p.category, "street_furniture");
    assert_eq!(p.snap_mode, SnapMode::Edge);
    assert_eq!(p.terrain_behavior, TerrainBehavior::FlatGround);
    assert_eq!(p.bounding_size_m, [1.5, 0.9, 0.6]);
    assert_eq!(m.lods[0].distance_max_m, None);
}

// ── Vehicle ─────────────────────────────────────────────────────────────

const VEHICLE_TOML: &str = r#"
asset_id = "vehicle.civil.sedan_compact"
display_name = "Compact Sedan"
tags = ["civil", "sedan"]

[vehicle]
vehicle_class = "civil"
vehicle_family = "sedan"
length_m = 4.5
width_m = 1.8
height_m = 1.5

[[vehicle.color_variants]]
name = "red"
albedo_file = "textures/sedan_red.png"

[[vehicle.color_variants]]
name = "blue"
albedo_file = "textures/sedan_blue.png"

[[anchors]]
type = "wheel"
name = "front_left"
position = [-0.85, 0.0, 1.5]
forward = [0.0, 0.0, 1.0]

[[lods]]
file = "lod0.glb"
distance_min_m = 0.0
distance_max_m = 40.0
"#;

#[test]
fn vehicle_manifest_round_trip() {
    let m = VEHICLE_TOML.parse::<AssetManifest>().expect("parse failed");
    assert_eq!(m.class().unwrap(), AssetClass::Vehicle);
    let v = m.vehicle.as_ref().unwrap();
    assert_eq!(v.vehicle_class, VehicleClass::Civil);
    assert_eq!(v.vehicle_family, VehicleFamily::Sedan);
    assert_eq!(v.length_m, 4.5);
    assert_eq!(v.color_variants.len(), 2);
    assert_eq!(v.color_variants[0].name, "red");
    assert_eq!(m.anchors[0].anchor_type, AnchorType::Wheel);
}

#[test]
fn vehicle_rejects_zero_dimensions() {
    let toml = r#"
asset_id = "vehicle.civil.bad"
display_name = "Bad"
[vehicle]
vehicle_class = "civil"
vehicle_family = "sedan"
length_m = 0.0
width_m = 1.8
height_m = 1.5
"#;
    assert!(toml.parse::<AssetManifest>().is_err());
}

// ── Character ────────────────────────────────────────────────────────────

const CHARACTER_TOML: &str = r#"
asset_id = "character.pedestrian.adult_male_01"
display_name = "Adult Male Pedestrian"
tags = ["pedestrian", "adult"]

[character]
archetype_family = "adult_male"
age_group = "adult"
body_type = "average"

[[character.skin_variants]]
name = "default"
albedo_file = "textures/skin_default.png"

[[character.skin_variants]]
name = "summer"
albedo_file = "textures/skin_summer.png"
"#;

#[test]
fn character_manifest_round_trip() {
    let m = CHARACTER_TOML
        .parse::<AssetManifest>()
        .expect("parse failed");
    assert_eq!(m.class().unwrap(), AssetClass::Character);
    let c = m.character.as_ref().unwrap();
    assert_eq!(c.archetype_family, ArchetypeFamily::AdultMale);
    assert_eq!(c.age_group.as_deref(), Some("adult"));
    assert_eq!(c.skin_variants.len(), 2);
    assert_eq!(c.skin_variants[1].name, "summer");
}

// ── Validation ────────────────────────────────────────────────────────────

#[test]
fn rejects_no_class_section() {
    let toml = r#"
asset_id = "building.residential.no_class"
display_name = "No Class"
"#;
    assert!(toml.parse::<AssetManifest>().is_err());
}

#[test]
fn rejects_two_class_sections() {
    let toml = r#"
asset_id = "building.residential.two_classes"
display_name = "Two Classes"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 3
lot_depth_cells = 3
[prop]
category = "street_furniture"
bounding_size_m = [1.0, 1.0, 1.0]
snap_mode = "free"
terrain_behavior = "flat_ground"
"#;
    assert!(toml.parse::<AssetManifest>().is_err());
}

#[test]
fn explicit_building_manifest_round_trip() {
    let toml = r#"
asset_id = "building.service.water_tower"
display_name = "Water Tower"

[building]
placement_mode = "explicit"
lot_width_cells = 4
lot_depth_cells = 4
service_class = "water"
worker_capacity = 6

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 2.0]
forward = [0.0, 0.0, 1.0]

[[mesh_parts]]
name = "main"

[[mesh_parts.lods]]
file = "water_tower.glb"
distance_min_m = 0.0
"#;
    let manifest = toml
        .parse::<AssetManifest>()
        .expect("explicit parse failed");
    let building = manifest.building.as_ref().unwrap();
    assert_eq!(building.placement_mode, PlacementMode::Explicit);
    assert_eq!(building.zone_type, None);
    assert_eq!(building.density, None);
    assert_eq!(building.service_class.as_deref(), Some("water"));
}

#[test]
fn explicit_building_rejects_zone_fields() {
    let toml = r#"
asset_id = "building.service.bad_explicit"
display_name = "Bad Explicit"
[building]
placement_mode = "explicit"
zone_type = "residential"
density = "low"
lot_width_cells = 2
lot_depth_cells = 2

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 1.0]
forward = [0.0, 0.0, 1.0]
"#;
    assert!(toml.parse::<AssetManifest>().is_err());
}

#[test]
fn rejects_invalid_asset_id() {
    let toml = r#"
asset_id = "Bad.Asset.ID"
display_name = "Bad"
[prop]
category = "test"
bounding_size_m = [1.0, 1.0, 1.0]
snap_mode = "free"
terrain_behavior = "flat_ground"
"#;
    assert!(toml.parse::<AssetManifest>().is_err());
}
