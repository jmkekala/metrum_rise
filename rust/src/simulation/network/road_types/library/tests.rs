// SPDX-License-Identifier: GPL-2.0-only

//! The road-type library file (`ROAD-101`): round trip, ids, two handles, and the rules that
//! never lose a player's road (unreadable and newer files untouched, invalid entries kept).

use super::*;
use crate::assets::authoring::files::token;
use std::fs;

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
        library.create(" Boulevard ", &[S, B, grass(3.0), F, S]),
        Ok("custom:1".into())
    );
    assert!(fixture.path().exists());
}

#[test]
fn types_round_trip_through_the_file() {
    let fixture = Fixture::new();
    let mut library = fixture.library();
    let divided = [S, B, B, grass(3.5), F, F, S];
    assert_eq!(library.create("Boulevard", &divided), Ok("custom:1".into()));
    assert_eq!(library.create("Lane", &[S, F, S]), Ok("custom:2".into()));
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
        })
    );
    // Another handle (another game instance, the next session) reads the same types.
    assert_eq!(custom(&mut fixture.library()), written);
    // The file: versioned, pretty, style names as strings, the counter stored.
    let text = fs::read_to_string(fixture.path()).unwrap();
    assert!(text.starts_with("{\n  \"format\": \"metrum-road-types\",\n  \"version\": 1,"));
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
        .create("Boulevard", &[S, B, grass(3.0), F, S])
        .unwrap();
    assert_eq!(reason(library.create("  ", &[S, F, S])), "name_empty");
    assert_eq!(
        reason(library.create(&"x".repeat(41), &[S, F, S])),
        "name_too_long"
    );
    assert_eq!(
        reason(library.create("a\tb", &[S, F, S])),
        "name_invalid_characters"
    );
    assert_eq!(
        reason(library.create("2-lane ROAD", &[S, F, S])),
        "name_taken"
    );
    assert_eq!(
        reason(library.create("BOULEVARD", &[S, F, S])),
        "name_taken"
    );
    assert_eq!(
        reason(library.create("Odd", &[S, F, B, S])),
        "layout_lane_order"
    );
    assert_eq!(
        reason(library.update(&id, "Odd", &[S, F, grass(3.0), S])),
        "median_requires_both_directions"
    );
    assert_eq!(
        reason(library.update("builtin:road_2l", "X", &[S, F, S])),
        "road_type_builtin"
    );
    assert_eq!(
        reason(library.delete("builtin:walkway")),
        "road_type_builtin"
    );
    assert_eq!(
        reason(library.update("custom:9", "X", &[S, F, S])),
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
    let avenue = fixture.library().create("Avenue", &[S, B, F, S]).unwrap();
    assert_eq!(library.check_name("Avenue", ""), Ok(()));
    library.reload();
    assert_eq!(reason(library.check_name("avenue", "")), "name_taken");
    assert_eq!(library.delete(&avenue), Ok(()));
    // A type may keep its own name in another case; its id stays.
    assert_eq!(
        library.update(&id, "boulevard", &[S, B, B, grass(4.0), F, F, S]),
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
    library.create("2+2 Grass 4 m", &[S, B, F, S]).unwrap();
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
    assert_eq!(library.create("A", &[S, F, S]), Ok("custom:1".into()));
    assert_eq!(library.delete("custom:1"), Ok(()));
    assert_eq!(
        fixture.library().create("B", &[S, F, S]),
        Ok("custom:2".into())
    );
    // A hand-edited counter behind the file's ids never reissues one.
    let mut json = fixture.json();
    json["next_id"] = 1.into();
    fs::write(fixture.path(), serde_json::to_vec(&json).unwrap()).unwrap();
    assert_eq!(
        fixture.library().create("C", &[S, F, S]),
        Ok("custom:3".into())
    );
}

#[test]
fn two_handles_writing_in_turn_keep_both_entries() {
    let fixture = Fixture::new();
    let (mut first, mut second) = (fixture.library(), fixture.library());
    assert_eq!(first.types().len(), BUILTIN_ROAD_TYPES.len());
    assert_eq!(second.types().len(), BUILTIN_ROAD_TYPES.len());
    assert_eq!(first.create("One", &[S, F, S]), Ok("custom:1".into()));
    // The second handle re-reads before writing: it sees the first's type and its id.
    assert_eq!(reason(second.create("ONE", &[S, F, S])), "name_taken");
    assert_eq!(second.create("Two", &[S, F, F, S]), Ok("custom:2".into()));
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
            reason(library.create("A", &[S, F, S])),
            "library_unreadable"
        );
        assert_eq!(
            reason(library.update("custom:1", "A", &[S, F, S])),
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
        assert_eq!(library.create("A", &[S, F, S]), Ok("custom:1".into()));
    }
}

#[test]
fn a_newer_version_is_read_only() {
    let fixture = Fixture::new();
    let bytes = br#"{"format": "metrum-road-types", "version": 2, "next_id": 2, "future": 1,
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
        reason(library.create("A", &[S, F, S])),
        "library_newer_version"
    );
    assert_eq!(
        reason(library.update("custom:1", "A", &[S, F, S])),
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
    assert_eq!(library.create("New", &[S, F, F, S]), Ok("custom:10".into()));
    let json = fixture.json();
    let kept = json["types"].as_array().unwrap();
    assert_eq!(kept.len(), entries.len() + 1);
    assert_eq!(
        kept[..entries.len()],
        original["types"].as_array().unwrap()[..]
    );
    // An unusable entry can be fixed in place; its duplicate id resolves to the first entry.
    assert_eq!(
        library.update("custom:3", "Wrong way", &[S, B, F, S]),
        Ok(())
    );
    assert!(custom(&mut library)[2].usable().is_some());
    assert_eq!(library.delete("custom:1"), Ok(()));
    assert_eq!(custom(&mut library)[5].name, "Copy");
}
