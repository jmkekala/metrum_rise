// SPDX-License-Identifier: GPL-2.0-only

//! The road-type library file (`ROAD-101`): round trip, ids, two handles, and the rules that
//! never lose a player's road (unreadable and newer files untouched, invalid entries kept).

use super::*;
use crate::assets::authoring::files::token;
use std::fs;
use crate::simulation::network::graph::SidewalkSides;
use crate::simulation::network::road_types::RoadAttributes;

const S: RoadPart = RoadPart::Sidewalk;
const F: RoadPart = RoadPart::CarLane { forward: true };
const B: RoadPart = RoadPart::CarLane { forward: false };

fn grass(width_m: f32) -> RoadPart {
    RoadPart::Median(RoadMedian {
        style: MedianStyle::KerbedGrass,
        width_m,
    })
}

// A scratch folder holding one library file, removed afterwards.
struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("metrum-road-types-{}", token()));
        fs::create_dir(&dir).unwrap();
        Self(dir)
    }
    fn path(&self) -> PathBuf {
        self.0.join("road_types.json")
    }
    fn library(&self) -> RoadTypeLibrary {
        RoadTypeLibrary::new(self.path())
    }
    fn json(&self) -> Value {
        serde_json::from_slice(&fs::read(self.path()).unwrap()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn reason<T: std::fmt::Debug>(result: Result<T, RoadTypeRefusal>) -> &'static str {
    result.unwrap_err().reason
}

fn custom(library: &mut RoadTypeLibrary) -> Vec<RoadType> {
    library.types()[BUILTIN_ROAD_TYPES.len()..].to_vec()
}

#[test]
fn a_missing_file_lists_the_builtins_and_is_created_by_the_first_write() {
    let fixture = Fixture::new();
    let mut library = fixture.library();
    let names: Vec<&str> = library.types().iter().map(|ty| ty.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Walkway",
            "2-Lane Road",
            "4-Lane Road",
            "One-Way 1L",
            "One-Way 2L"
        ]
    );
    assert_eq!(library.error(), None);
    assert!(!fixture.path().exists(), "listing never creates the file");
    assert_eq!(
        library.create(" Boulevard ", &[S, B, grass(3.0), F, S], 50, true, true),
        Ok("custom:1".into())
    );
    assert!(fixture.path().exists());
}

#[test]
fn types_round_trip_through_the_file() {
    let fixture = Fixture::new();
    let mut library = fixture.library();
    let divided = [S, B, B, grass(3.5), F, F, S];
    assert_eq!(library.create("Boulevard", &divided, 50, true, true), Ok("custom:1".into()));
    assert_eq!(library.create("Lane", &[S, F, S], 50, true, true), Ok("custom:2".into()));
    let written = custom(&mut library);
    assert_eq!(written[0].name, "Boulevard");
    assert_eq!(written[0].parts, divided);
    assert_eq!(
        written[0].usable(),
        Some(RoadCrossSection {
            fwd_lanes: 2,
            bkw_lanes: 2,
            median: Some(RoadMedian {
                style: MedianStyle::KerbedGrass,
                width_m: 3.5,
            }),
            sidewalks: SidewalkSides::BOTH,
        })
    );
    // Another handle (another game instance, the next session) reads the same types.
    assert_eq!(custom(&mut fixture.library()), written);
    // The file: versioned, pretty, style names as strings, the counter stored.
    let text = fs::read_to_string(fixture.path()).unwrap();
    assert!(text.starts_with("{\n  \"format\": \"metrum-road-types\",\n  \"version\": 2,"));
    let json = fixture.json();
    assert_eq!(json["next_id"], 3);
    assert_eq!(json["types"][0]["parts"][3]["kind"], "median");
    assert_eq!(json["types"][0]["parts"][3]["style"], "grass");
    assert_eq!(json["types"][0]["parts"][3]["width_m"], 3.5);
    assert_eq!(json["types"][1]["parts"][1]["kind"], "car_lane");
    assert_eq!(json["types"][1]["parts"][1]["forward"], true);
}

#[test]
fn edits_follow_the_name_and_layout_rules() {
    let fixture = Fixture::new();
    let mut library = fixture.library();
    let id = library
        .create("Boulevard", &[S, B, grass(3.0), F, S], 50, true, true)
        .unwrap();
    assert_eq!(reason(library.create("  ", &[S, F, S], 50, true, true)), "name_empty");
    assert_eq!(
        reason(library.create(&"x".repeat(41), &[S, F, S], 50, true, true)),
        "name_too_long"
    );
    assert_eq!(
        reason(library.create("a\tb", &[S, F, S], 50, true, true)),
        "name_invalid_characters"
    );
    assert_eq!(
        reason(library.create("2-lane ROAD", &[S, F, S], 50, true, true)),
        "name_taken"
    );
    assert_eq!(
        reason(library.create("BOULEVARD", &[S, F, S], 50, true, true)),
        "name_taken"
    );
    assert_eq!(
        reason(library.create("Odd", &[S, F, B, S], 50, true, true)),
        "layout_lane_order"
    );
    assert_eq!(
        reason(library.update(&id, "Odd", &[S, F, grass(3.0), S], 50, true, true)),
        "median_requires_both_directions"
    );
    assert_eq!(
        reason(library.update("builtin:road_2l", "X", &[S, F, S], 50, true, true)),
        "road_type_builtin"
    );
    assert_eq!(
        reason(library.delete("builtin:walkway")),
        "road_type_builtin"
    );
    assert_eq!(
        reason(library.update("custom:9", "X", &[S, F, S], 50, true, true)),
        "road_type_unknown"
    );
    assert_eq!(reason(library.delete("custom:9")), "road_type_unknown");
    // The builder's name check gives the write's verdict without writing.
    assert_eq!(reason(library.check_name(" ", "")), "name_empty");
    assert_eq!(reason(library.check_name("boulevard", "")), "name_taken");
    assert_eq!(reason(library.check_name("2-Lane road", &id)), "name_taken");
    assert_eq!(library.check_name("boulevard", &id), Ok(()));
    assert_eq!(library.check_name("Avenue", ""), Ok(()));
    // Another handle's write lists after a reload.
    let avenue = fixture.library().create("Avenue", &[S, B, F, S], 50, true, true).unwrap();
    assert_eq!(library.check_name("Avenue", ""), Ok(()));
    library.reload();
    assert_eq!(reason(library.check_name("avenue", "")), "name_taken");
    assert_eq!(library.delete(&avenue), Ok(()));
    // A type may keep its own name in another case; its id stays.
    assert_eq!(
        library.update(&id, "boulevard", &[S, B, B, grass(4.0), F, F, S], 50, true, true),
        Ok(())
    );
    let edited = &custom(&mut library)[0];
    assert_eq!(
        (edited.id.as_str(), edited.name.as_str()),
        ("custom:1", "boulevard")
    );
    assert_eq!(edited.usable().map(|section| section.fwd_lanes), Some(2));
    assert_eq!(
        library.suggested_name(edited.usable().unwrap()),
        "2+2 Grass 4 m"
    );
    library.create("2+2 Grass 4 m", &[S, B, F, S], 50, true, true).unwrap();
    assert_eq!(
        library.suggested_name(edited.usable().unwrap()),
        "2+2 Grass 4 m (2)"
    );
    assert_eq!(library.delete(&id), Ok(()));
    assert_eq!(custom(&mut library).len(), 1);
    assert_eq!(custom(&mut fixture.library()).len(), 1);
}

#[test]
fn ids_come_from_the_stored_counter_and_are_never_reissued() {
    let fixture = Fixture::new();
    let mut library = fixture.library();
    assert_eq!(library.create("A", &[S, F, S], 50, true, true), Ok("custom:1".into()));
    assert_eq!(library.delete("custom:1"), Ok(()));
    assert_eq!(
        fixture.library().create("B", &[S, F, S], 50, true, true),
        Ok("custom:2".into())
    );
    // A hand-edited counter behind the file's ids never reissues one.
    let mut json = fixture.json();
    json["next_id"] = 1.into();
    fs::write(fixture.path(), serde_json::to_vec(&json).unwrap()).unwrap();
    assert_eq!(
        fixture.library().create("C", &[S, F, S], 50, true, true),
        Ok("custom:3".into())
    );
}

#[test]
fn two_handles_writing_in_turn_keep_both_entries() {
    let fixture = Fixture::new();
    let (mut first, mut second) = (fixture.library(), fixture.library());
    assert_eq!(first.types().len(), BUILTIN_ROAD_TYPES.len());
    assert_eq!(second.types().len(), BUILTIN_ROAD_TYPES.len());
    assert_eq!(first.create("One", &[S, F, S], 50, true, true), Ok("custom:1".into()));
    // The second handle re-reads before writing: it sees the first's type and its id.
    assert_eq!(reason(second.create("ONE", &[S, F, S], 50, true, true)), "name_taken");
    assert_eq!(second.create("Two", &[S, F, F, S], 50, true, true), Ok("custom:2".into()));
    assert_eq!(first.delete("custom:2"), Ok(()));
    let names: Vec<String> = custom(&mut fixture.library())
        .into_iter()
        .map(|ty| ty.name)
        .collect();
    assert_eq!(names, ["One"]);
}

#[test]
fn an_unparsable_file_is_left_untouched_and_refuses_writes() {
    for bytes in [
        &b"{\"format\": \"metrum-road-types\", \"version\": 1, \"types\": [ oops"[..],
        b"{\"format\": \"something-else\", \"version\": 1}",
        b"",
    ] {
        let fixture = Fixture::new();
        fs::write(fixture.path(), bytes).unwrap();
        let mut library = fixture.library();
        assert_eq!(library.types().len(), BUILTIN_ROAD_TYPES.len());
        assert_eq!(
            library.error().map(|error| error.reason),
            Some("library_unreadable")
        );
        assert_eq!(
            reason(library.create("A", &[S, F, S], 50, true, true)),
            "library_unreadable"
        );
        assert_eq!(
            reason(library.update("custom:1", "A", &[S, F, S], 50, true, true)),
            "library_unreadable"
        );
        assert_eq!(reason(library.delete("custom:1")), "library_unreadable");
        assert_eq!(
            fs::read(fixture.path()).unwrap(),
            bytes,
            "the file is byte-identical"
        );
        // Fixed by hand, it is writable again.
        fs::remove_file(fixture.path()).unwrap();
        assert_eq!(library.create("A", &[S, F, S], 50, true, true), Ok("custom:1".into()));
    }
}

#[test]
fn a_newer_version_is_read_only() {
    let fixture = Fixture::new();
    let bytes = br#"{"format": "metrum-road-types", "version": 3, "next_id": 2, "future": 1,
        "types": [{"id": "custom:1", "name": "Later", "parts": [{"kind": "sidewalk"},
        {"kind": "car_lane", "forward": true}, {"kind": "sidewalk"}]}]}"#;
    fs::write(fixture.path(), bytes).unwrap();
    let mut library = fixture.library();
    assert_eq!(
        library.error().map(|error| error.reason),
        Some("library_newer_version")
    );
    let listed = custom(&mut library);
    assert_eq!(listed[0].name, "Later");
    assert!(listed[0].usable().is_some(), "its types still draw");
    assert_eq!(
        reason(library.create("A", &[S, F, S], 50, true, true)),
        "library_newer_version"
    );
    assert_eq!(
        reason(library.update("custom:1", "A", &[S, F, S], 50, true, true)),
        "library_newer_version"
    );
    assert_eq!(reason(library.delete("custom:1")), "library_newer_version");
    assert_eq!(fs::read(fixture.path()).unwrap(), bytes);
}

#[test]
fn invalid_entries_are_listed_unusable_and_kept_on_write() {
    let fixture = Fixture::new();
    let lane = r#"{"kind": "car_lane", "forward": true}"#;
    let side = r#"{"kind": "sidewalk"}"#;
    let entries = [
        format!(r#"{{"id": "custom:1", "name": "Good", "parts": [{side}, {lane}, {side}]}}"#),
        format!(
            r#"{{"id": "custom:2", "name": "Mauve", "parts": [{side}, {{"kind": "median", "style": "mauve", "width_m": 2.0}}, {side}]}}"#
        ),
        format!(
            r#"{{"id": "custom:3", "name": "Wrong way", "parts": [{side}, {lane}, {{"kind": "car_lane", "forward": false}}, {side}]}}"#
        ),
        r#"{"id": "custom:4", "name": "Tram", "parts": [{"kind": "tram_track"}]}"#.to_string(),
        r#"{"id": "custom:5", "name": "No parts"}"#.to_string(),
        format!(r#"{{"id": "custom:6", "name": "good", "parts": [{side}, {lane}, {side}]}}"#),
        format!(r#"{{"id": "custom:1", "name": "Copy", "parts": [{side}, {lane}, {side}]}}"#),
        format!(
            r#"{{"id": "custom:7", "name": "4-LANE road", "parts": [{side}, {lane}, {side}]}}"#
        ),
        format!(r#"{{"id": "custom:8", "name": "", "parts": [{side}, {lane}, {side}]}}"#),
        format!(
            r#"{{"id": "custom:9", "name": "Wide", "parts": [{side}, {{"kind": "car_lane", "forward": false}}, {{"kind": "median", "style": "grass", "width_m": 40.0}}, {lane}, {side}]}}"#
        ),
    ];
    let text = format!(
        r#"{{"format": "metrum-road-types", "version": 1, "next_id": 10, "types": [{}]}}"#,
        entries.join(", ")
    );
    fs::write(fixture.path(), &text).unwrap();
    let original: Value = serde_json::from_str(&text).unwrap();
    let mut library = fixture.library();
    assert_eq!(
        library.error(),
        None,
        "bad entries do not make the file unreadable"
    );
    let reasons: Vec<(String, &str)> = custom(&mut library)
        .iter()
        .map(|ty| {
            (
                ty.name.clone(),
                ty.verdict
                    .as_ref()
                    .err()
                    .map_or("", |refusal| refusal.reason),
            )
        })
        .collect();
    let expected = [
        ("Good", ""),
        ("Mauve", "unknown_median_style"),
        ("Wrong way", "layout_lane_order"),
        ("Tram", "entry_unreadable"),
        ("No parts", "entry_unreadable"),
        ("good", "name_taken"),
        ("Copy", "road_type_duplicate_id"),
        ("4-LANE road", "name_taken"),
        ("", "name_empty"),
        ("Wide", "median_width_out_of_range"),
    ];
    let expected: Vec<(String, &str)> = expected
        .iter()
        .map(|(name, reason)| (name.to_string(), *reason))
        .collect();
    assert_eq!(reasons, expected);
    // A write keeps every entry as it was, the bad ones included.
    assert_eq!(library.create("New", &[S, F, F, S], 50, true, true), Ok("custom:10".into()));
    let json = fixture.json();
    let kept = json["types"].as_array().unwrap();
    assert_eq!(kept.len(), entries.len() + 1);
    assert_eq!(
        kept[..entries.len()],
        original["types"].as_array().unwrap()[..]
    );
    // An unusable entry can be fixed in place; its duplicate id resolves to the first entry.
    assert_eq!(
        library.update("custom:3", "Wrong way", &[S, B, F, S], 50, true, true),
        Ok(())
    );
    assert!(custom(&mut library)[2].usable().is_some());
    assert_eq!(library.delete("custom:1"), Ok(()));
    assert_eq!(custom(&mut library)[5].name, "Copy");
}

#[test]
fn speed_zoning_and_sidewalks_round_trip_and_version_1_reads_with_defaults() {
    let fixture = Fixture::new();
    // A version 1 file: no speed, no zoning, sidewalks as parts.
    let v1 = br#"{"format": "metrum-road-types", "version": 1, "next_id": 2, "types": [
        {"id": "custom:1", "name": "Old", "parts": [{"kind": "sidewalk"},
        {"kind": "car_lane", "forward": false}, {"kind": "car_lane", "forward": true},
        {"kind": "sidewalk"}]}]}"#;
    fs::write(fixture.path(), v1).unwrap();
    let mut library = fixture.library();
    assert_eq!(library.error(), None);
    let old = &custom(&mut library)[0];
    assert_eq!((old.speed_kmh, old.zoning), (50, true));
    assert_eq!(old.usable().map(|section| section.sidewalks), Some(SidewalkSides::BOTH));
    // A rural road: no sidewalk parts, 80 km/h, no zoning; a highway-like 120 km/h.
    assert_eq!(
        library.create("Rural", &[B, F], 80, false, true),
        Ok("custom:2".into())
    );
    assert_eq!(
        library.create("Fast", &[B, B, grass(3.0), F, F], 120, false, true),
        Ok("custom:3".into())
    );
    // The write stores version 2 and keeps the version 1 entry as it was.
    let json = fixture.json();
    assert_eq!(json["version"], 2);
    let original: Value = serde_json::from_slice(v1).unwrap();
    assert_eq!(json["types"][0], original["types"][0]);
    assert_eq!(json["types"][1]["speed_kmh"], 80);
    assert_eq!(json["types"][1]["zoning"], false);
    let listed = custom(&mut fixture.library());
    let rural = &listed[1];
    assert_eq!((rural.speed_kmh, rural.zoning), (80, false));
    assert_eq!(
        rural.usable(),
        Some(RoadCrossSection {
            fwd_lanes: 1,
            bkw_lanes: 1,
            median: None,
            sidewalks: SidewalkSides::NONE,
        })
    );
    let attributes = RoadAttributes::new(rural.speed_kmh, SidewalkSides::NONE, rural.zoning);
    assert_eq!(
        attributes.speed_ms.to_bits(),
        (80.0_f32 * crate::config::KMH_TO_MPS).to_bits()
    );
    assert_eq!(listed[2].speed_kmh, 120);
    // Speeds off the range or the 10 km/h grid are refused; a walkway's is ignored.
    for speed in [0, 10, 55, 130, -50, i64::from(u16::MAX) + 50] {
        assert_eq!(
            reason(library.create("Odd", &[S, F, S], speed, true, true)),
            "speed_out_of_range",
            "{speed}"
        );
    }
    assert_eq!(
        reason(library.update("custom:2", "Rural", &[B, F], 125, false, true)),
        "speed_out_of_range"
    );
    assert_eq!(
        library.create("Path", &[RoadPart::Footpath], 999, true, true),
        Ok("custom:4".into())
    );
    assert_eq!(custom(&mut library)[3].speed_kmh, 0);
    // An entry with a speed out of range is listed unusable and kept as it was.
    let mut json = fixture.json();
    json["types"][1]["speed_kmh"] = 45.into();
    fs::write(fixture.path(), serde_json::to_vec(&json).unwrap()).unwrap();
    let mut library = fixture.library();
    let rural = &custom(&mut library)[1];
    assert_eq!(
        rural.verdict.as_ref().err().map(|refusal| refusal.reason),
        Some("speed_out_of_range")
    );
    assert_eq!(rural.usable(), None);
    library.create("New", &[S, F, F, S], 50, true, true).unwrap();
    assert_eq!(fixture.json()["types"][1]["speed_kmh"], 45);
}

// `ROAD-101` RB3e: street lights came within version 2, without a version change. An entry
// without the field lists lit and stays as it was through another write; a type created or
// updated with them off keeps that; built-ins are lit.
#[test]
fn a_version_2_entry_without_street_lights_reads_lit() {
    let fixture = Fixture::new();
    let older = br#"{"format": "metrum-road-types", "version": 2, "next_id": 2, "types": [
        {"id": "custom:1", "name": "Older", "parts": [{"kind": "car_lane", "forward": false},
        {"kind": "car_lane", "forward": true}], "speed_kmh": 80, "zoning": false}]}"#;
    fs::write(fixture.path(), older).unwrap();
    let mut library = fixture.library();
    assert_eq!(library.error(), None);
    let listed = &custom(&mut library)[0];
    assert!(listed.usable().is_some());
    assert!(listed.street_lights);
    assert!(library.types().iter().filter(|ty| ty.builtin).all(|ty| ty.street_lights));
    assert_eq!(
        library.create("Dark", &[B, B, F, F], 100, false, false),
        Ok("custom:2".into())
    );
    let json = fixture.json();
    assert_eq!(json["version"], 2);
    let original: Value = serde_json::from_slice(older).unwrap();
    assert_eq!(json["types"][0], original["types"][0], "kept as it was");
    assert_eq!(json["types"][1]["street_lights"], false);
    assert!(!custom(&mut fixture.library())[1].street_lights);
    assert_eq!(
        library.update("custom:1", "Older", &[B, F], 80, false, false),
        Ok(())
    );
    assert_eq!(fixture.json()["types"][0]["street_lights"], false);
    assert!(!custom(&mut fixture.library())[0].street_lights);
}
