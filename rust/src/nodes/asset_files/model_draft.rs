// SPDX-License-Identifier: GPL-3.0-or-later

//! The draft command's one Rust call (`TOOLS-33`, docs/asset_editor.md "Draft command"): the
//! existence check, publication through the editor's path, the ledger and the acceptance check,
//! first in the pack copy and, only when that passes and a write was asked for, in the real
//! pack. Engine work arrives as callbacks, so tests run the whole flow without Godot.

use super::publish_state;
use crate::assets::AssetManifest;
use crate::assets::authoring::model_draft::{self as draft, acceptance, sidecar};
use crate::nodes::sim::asset_export::{ExportParams, validated_tomls};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// What only the engine does for a draft.
pub(super) struct Engine<'a> {
    /// R8: write the thumbnail from the models preview (source, destination).
    pub(super) thumbnail: &'a dyn Fn(&Path, &Path) -> Result<(), String>,
    /// Import an installed asset's `LOD0` bounds into its parts.
    pub(super) bounds: &'a dyn Fn(&mut AssetManifest, &Path) -> Result<(), String>,
    /// Whether an image file decodes.
    pub(super) decodes: &'a dyn Fn(&Path) -> bool,
}

/// What `tools/model_draft.py` sends: native paths, the role inputs inline.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    sidecar: PathBuf,
    role: Value,
    /// The packs folder holding the copy of the role's pack.
    mods: PathBuf,
    /// Scratch space for the thumbnail, inside the throwaway profile.
    work: PathBuf,
    /// The real packs folder, given only with `--write`.
    #[serde(default)]
    write_mods: Option<PathBuf>,
    /// The real packs folder `--mods` names, given with or without `--write`: a dry run's report
    /// names where `--write` would publish. Nothing is written there without `write_mods`.
    #[serde(default)]
    target_mods: Option<PathBuf>,
    /// `--check`: run the stale report and the acceptance check on the existing asset as it is in
    /// the pack (the copy of it in `mods`), drafting and writing nothing.
    #[serde(default)]
    check: bool,
    /// `--source-commit`: the models commit the files were copied from, for the ledger.
    #[serde(default)]
    source_commit: Option<String>,
}

/// The exit status a draft leaves (`tools/model_draft.py` passes it on).
const EXIT_OK: i64 = 0;
const EXIT_FAILED: i64 = 1;
const EXIT_STALE: i64 = 2;

enum Step {
    Existing(draft::Existing),
    Drafted {
        summary: Value,
        outcome: acceptance::Outcome,
        asset: PathBuf,
        ledger: PathBuf,
    },
}

/// Run one draft request. Returns the exit status and the JSON report with its text summary.
pub(super) fn run(request: &str, engine: &Engine) -> (i64, Value) {
    let mut asset = None;
    let result = (|| -> Result<(i64, Value), String> {
        let request: Request =
            serde_json::from_str(request).map_err(|e| format!("draft request: {e}"))?;
        // Named from the role as sent, so a refusal of the role itself still names it.
        let label = asset_label(&request.role);
        asset = Some(label.clone());
        let mut inputs = draft::read_inputs(&request.sidecar, request.role)?;
        inputs.source_commit = sidecar::source_commit(request.source_commit)?;
        let mut report = json!({"asset": label});
        if request.check {
            let exit = check_existing(&inputs, &request.mods, engine, &mut report)?;
            return Ok((exit, report));
        }
        let copy = draft_into(&inputs, &request.mods, &request.work, engine)?;
        let mut exit = describe(&copy, &mut report, "copy");
        // Unambiguous for a reader of the JSON: a dry run writes nothing outside the copy, and
        // only a report with a `written` object published into the real pack.
        report["dry_run"] = json!(request.write_mods.is_none());
        report["status"] = json!(match (&copy, exit) {
            (Step::Existing(_), _) => "existing",
            (Step::Drafted { .. }, EXIT_OK) => "drafted",
            _ => "failed",
        });
        if let (Step::Drafted { .. }, EXIT_OK, Some(real)) = (&copy, exit, &request.write_mods) {
            let written = draft_into(&inputs, real, &request.work, engine)?;
            exit = describe(&written, &mut report, "written");
            report["status"] = json!(match written {
                Step::Existing(_) => "existing",
                Step::Drafted { .. } => "written",
            });
        } else if let (Step::Drafted { .. }, EXIT_OK, None, Some(target)) =
            (&copy, exit, &request.write_mods, &request.target_mods)
        {
            // Where `--write` would publish: the real pack, which this run left untouched.
            let pack = target.join(&inputs.role.pack_id);
            report["would_write"] = json!({
                "asset": pack.join("assets").join(&inputs.role.asset_id),
                "ledger": sidecar::ledger_path(&pack, &inputs.role.asset_id),
            });
        }
        Ok((exit, report))
    })();
    let (exit, mut report) = result.unwrap_or_else(|error| {
        let mut report = json!({"status": "refused", "error": error});
        if let Some(asset) = asset {
            report["asset"] = json!(asset);
        }
        (EXIT_FAILED, report)
    });
    report["summary"] = json!(summary(&report));
    (exit, report)
}

// `<pack_id>:<asset_id>` from role inputs that may not parse.
fn asset_label(role: &Value) -> String {
    let field = |key: &str| {
        role[key]
            .as_str()
            .map_or_else(|| format!("(no {key})"), str::to_owned)
    };
    format!("{}:{}", field("pack_id"), field("asset_id"))
}

// `--check`: the existing asset's stale report and its acceptance check, as it is in the pack, so
// a warning the author has since fixed in the editor (a workplace's parking) is seen to clear.
// Exit status 1 when the check fails, else 2 with a stale line, else 0. Writes nothing.
fn check_existing(
    inputs: &draft::Inputs,
    mods: &Path,
    engine: &Engine,
    report: &mut Value,
) -> Result<i64, String> {
    let (pack_id, asset_id) = (&inputs.role.pack_id, &inputs.role.asset_id);
    let pack = crate::assets::authoring::files::pack_directory(mods, pack_id)?;
    let existing = draft::existing(&pack, inputs).ok_or_else(|| {
        format!(
            "assets/{asset_id} does not exist in pack {pack_id}: nothing to check; run without \
             --check to draft it"
        )
    })?;
    let exit = describe(&Step::Existing(existing), report, "existing");
    let outcome = acceptance::check(
        mods,
        pack_id,
        asset_id,
        &acceptance::Engine {
            bounds: engine.bounds,
            decodes: engine.decodes,
        },
    );
    report["status"] = json!("checked");
    report["check"] = json!(true);
    let exit = if outcome.passed() { exit } else { EXIT_FAILED };
    report["acceptance"] = json!(outcome);
    Ok(exit)
}

// Draft into the role's pack in `mods`: nothing is written when the asset exists.
fn draft_into(
    inputs: &draft::Inputs,
    mods: &Path,
    work: &Path,
    engine: &Engine,
) -> Result<Step, String> {
    let derived = draft::derive(inputs, mods, work)?;
    // The role values pass the export contract before anything else happens.
    let params: ExportParams = serde_json::from_value(derived.document["params"].clone())
        .map_err(|e| format!("draft parameters: {e}"))?;
    validated_tomls(&params)?;
    if let Some(existing) = draft::existing(&derived.pack, inputs) {
        return Ok(Step::Existing(existing));
    }
    let asset_id = &inputs.role.asset_id;
    (engine.thumbnail)(&derived.preview, &derived.thumbnail)?;
    publish_state(derived.document.clone(), &derived.pack, engine.decodes)?;
    let ledger = sidecar::write_ledger(
        &derived.pack,
        asset_id,
        &inputs.sidecar,
        &json!(inputs.role),
        inputs.source_commit.as_deref(),
    )
    .map_err(|e| format!("assets/{asset_id} was published, but its {e}"))?;
    let outcome = acceptance::check(
        mods,
        &inputs.role.pack_id,
        asset_id,
        &acceptance::Engine {
            bounds: engine.bounds,
            decodes: engine.decodes,
        },
    );
    Ok(Step::Drafted {
        summary: derived.summary,
        outcome,
        asset: derived.pack.join("assets").join(asset_id),
        ledger,
    })
}

// Record one step under `key`; returns its exit status.
fn describe(step: &Step, report: &mut Value, key: &str) -> i64 {
    match step {
        Step::Existing(existing) => {
            let (stale, info): (Vec<_>, Vec<_>) =
                existing.lines.iter().partition(|line| line.stale);
            report["stale"] = json!(stale);
            report["info"] = json!(info);
            report["doors"] = json!(existing.doors);
            if stale.is_empty() {
                EXIT_OK
            } else {
                EXIT_STALE
            }
        }
        Step::Drafted {
            summary,
            outcome,
            asset,
            ledger,
        } => {
            let mut draft = summary.clone();
            // The doors are the report's, for a draft and an existing asset alike.
            report["doors"] = draft
                .as_object_mut()
                .and_then(|draft| draft.remove("doors"))
                .unwrap_or_default();
            report["draft"] = draft;
            report["acceptance"] = json!(outcome);
            // `copy` names the throwaway copy the draft was checked in; `written` the real pack.
            report[key] = json!({"asset": asset, "ledger": ledger, "passed": outcome.passed()});
            if outcome.passed() {
                EXIT_OK
            } else {
                EXIT_FAILED
            }
        }
    }
}

// The report as text for a terminal.
fn summary(report: &Value) -> String {
    let status = report["status"].as_str().unwrap_or_default();
    let dry_run = report["dry_run"] == json!(true);
    let mut lines = vec![format!(
        "{}: {status}{}",
        report["asset"].as_str().unwrap_or("draft"),
        if dry_run && !matches!(status, "refused" | "checked") {
            " (dry run: nothing written)"
        } else {
            ""
        }
    )];
    if let Some(error) = report["error"].as_str() {
        lines.push(format!("  refused: {error}"));
    }
    let draft = &report["draft"];
    if draft.is_object() {
        lines.push(format!(
            "  lot {} x {} cells; part at {}; entrance at {} facing {}",
            draft["lot_cells"][0],
            draft["lot_cells"][1],
            draft["part_position"],
            draft["entrance"]["position"],
            draft["entrance"]["forward"]
        ));
        lines.push(format!("  capacity: {}", capacity(draft)));
        if let Some(commit) = draft["source_commit"].as_str() {
            lines.push(format!(
                "  source: files from models commit {commit}; the sidecar was built at {}",
                text(&draft["sidecar_commit"])
            ));
        }
        let free = &draft["free_space"];
        lines.push(format!(
            "  free space: front {} m, back {} m, left {} m, right {} m",
            free["front_m"], free["back_m"], free["left_m"], free["right_m"]
        ));
    }
    for (kind, key) in [("FAIL", "failures"), ("warn", "warnings")] {
        for entry in report["acceptance"][key].as_array().into_iter().flatten() {
            lines.push(format!("  {kind}: {}", entry.as_str().unwrap_or_default()));
        }
    }
    for (prefix, key) in [("", "stale"), ("info: ", "info")] {
        for line in report[key].as_array().into_iter().flatten() {
            lines.push(format!(
                "  {prefix}{} [{}]: {}",
                line["check"].as_str().unwrap_or_default(),
                line["field"].as_str().unwrap_or_default(),
                line["message"].as_str().unwrap_or_default()
            ));
        }
    }
    lines.extend(door_lines(&report["doors"]));
    if status == "checked" {
        lines.push("  checked the asset as it is in the pack; nothing was written".into());
    } else if report["stale"].as_array().is_some_and(Vec::is_empty) {
        lines.push("  the asset exists and no line marks it stale; nothing was published".into());
    }
    if report["written"].is_object() {
        lines.push(format!(
            "  written: {}; ledger {}",
            text(&report["written"]["asset"]),
            text(&report["written"]["ledger"])
        ));
    } else if dry_run && status == "drafted" {
        lines.push(match report["would_write"].as_object() {
            Some(target) => format!(
                "  dry run: nothing was written; --write would publish {} and its ledger {}",
                text(&target["asset"]),
                text(&target["ledger"])
            ),
            None => "  dry run: nothing was written".into(),
        });
    } else if dry_run && status == "failed" {
        lines.push(
            "  dry run: nothing was written; --write would publish nothing until the acceptance check passes"
                .into(),
        );
    }
    lines.join("\n")
}

// Every sidecar door in the lot frame, one line each, so the author lays bays and walks to them
// without working the part's transform out by hand.
fn door_lines(doors: &Value) -> Vec<String> {
    let Some(doors) = doors.as_array().filter(|doors| !doors.is_empty()) else {
        return Vec::new();
    };
    let mut lines =
        vec!["  doors in the lot frame (position [x, y, z], y the threshold height):".to_owned()];
    for door in doors {
        lines.push(format!(
            "    {} '{}'{} at {} facing {}, {} x {} m",
            text(&door["kind"]),
            text(&door["id"]),
            if door["main"] == json!(true) {
                " (main)"
            } else {
                ""
            },
            door["position"],
            door["facing"],
            door["width_m"],
            door["height_m"]
        ));
    }
    lines
}

// A JSON string as text, anything else as JSON.
fn text(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

// The derived capacities: households and flat size for housing, the profile (which supplies the
// workers) otherwise.
fn capacity(draft: &Value) -> String {
    let mut parts = Vec::new();
    if !draft["household_capacity"].is_null() {
        parts.push(format!("households {}", draft["household_capacity"]));
    }
    if !draft["flat_size_m2"].is_null() {
        parts.push(format!("flat size {} m²", draft["flat_size_m2"]));
    }
    if let Some(profile) = draft["economy_profile"].as_str() {
        parts.push(format!("economy_profile {profile}"));
    }
    if parts.is_empty() {
        "none".into()
    } else {
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::authoring::model_draft::tests::{ASSET, Fixture, PACK};
    use std::fs;

    // The engine without Godot: the preview copied as the thumbnail, the sidecar's bounds as
    // the imported bounds, every image decoding.
    fn run_fixture(fixture: &Fixture, role: Value, write: bool) -> (i64, Value) {
        run_with(fixture, role, write, json!({}))
    }

    // `extra` adds request fields (`check`, `source_commit`).
    fn run_with(fixture: &Fixture, role: Value, write: bool, extra: Value) -> (i64, Value) {
        let bounds = [[-5.35, 0.0, -6.84], [5.35, 9.5625, 6.84]];
        let engine = Engine {
            thumbnail: &|preview, output| {
                fs::create_dir_all(output.parent().unwrap()).map_err(|e| e.to_string())?;
                fs::copy(preview, output)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            },
            bounds: &|manifest, _| {
                for part in &mut manifest.mesh_parts {
                    part.imported_bounds = Some(bounds);
                }
                Ok(())
            },
            decodes: &|path| path.is_file(),
        };
        let copy = fixture.root.join("copy");
        if !copy.join(PACK).exists() {
            fs::create_dir_all(copy.join(PACK)).unwrap();
            fs::copy(
                fixture.pack().join("pack.toml"),
                copy.join(PACK).join("pack.toml"),
            )
            .unwrap();
        }
        let mut request = json!({
            "sidecar": fixture.sidecar_path(), "role": role, "mods": copy,
            "work": fixture.root.join("work"),
            "write_mods": if write { json!(fixture.mods()) } else { Value::Null },
            "target_mods": fixture.mods(),
        });
        for (key, value) in extra.as_object().into_iter().flatten() {
            request[key] = value.clone();
        }
        run(&request.to_string(), &engine)
    }

    fn snapshot(folder: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files = Vec::new();
        let mut stack = vec![folder.to_owned()];
        while let Some(folder) = stack.pop() {
            for entry in fs::read_dir(folder).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    files.push((path.clone(), fs::read(path).unwrap()));
                }
            }
        }
        files.sort();
        files
    }

    #[test]
    fn a_draft_publishes_into_the_copy_and_writes_the_real_pack_only_when_asked() {
        let fixture = Fixture::new();
        let real = fixture.pack();
        let before = snapshot(&real);
        let (exit, report) = run_fixture(&fixture, fixture.role(), false);
        assert_eq!(
            (exit, report["status"].as_str()),
            (0, Some("drafted")),
            "{report:#}"
        );
        assert_eq!(
            snapshot(&real),
            before,
            "a draft without --write leaves the pack alone"
        );
        let copy = fixture.root.join("copy").join(PACK);
        let asset = copy.join("assets").join(ASSET);
        let text = fs::read_to_string(asset.join("asset.toml")).unwrap();
        let manifest = AssetManifest::from_str(&text).unwrap();
        let building = manifest.building.as_ref().unwrap();
        assert_eq!([building.lot_width_cells, building.lot_depth_cells], [2, 2]);
        assert_eq!(building.household_capacity, Some(1));
        assert_eq!(manifest.mesh_parts[0].lods.len(), 4);
        assert_eq!(building.appearance.as_ref().unwrap().schemes.len(), 5);
        assert!(asset.join("thumbnail.webp").is_file());
        assert!(
            asset
                .join("colours/rintamamiestalo_porch_albedo_blue.png")
                .is_file()
        );
        assert!(
            copy.join("model_sources")
                .join(format!("{ASSET}.json"))
                .is_file()
        );
        assert!(
            report["acceptance"]["failures"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let summary = report["summary"].as_str().unwrap();
        assert!(summary.contains("free space: front 6.32 m"), "{summary}");
        assert!(summary.contains("facing [0.0,0.0,1.0]"), "{summary}");
        // Every door in the lot frame: the worked example's part stands at z -3.16.
        assert!(
            summary.contains("entrance 'front' (main) at [-1.2,0.7,1.94] facing [0.0,0.0,1.0], 0.9 x 2.1 m"),
            "{summary}"
        );
        assert!(summary.contains("service 'back' at [1.5,0.7,-8.26]"), "{summary}");
        assert_eq!(report["doors"][1]["id"], "back", "{report:#}");
        assert!(report["draft"].get("doors").is_none());
        assert!(
            summary.contains("capacity: households 1, flat size 80.0 m²"),
            "{summary}"
        );
        // A dry run says so, and names the real pack only as where `--write` would publish.
        assert_eq!(report["dry_run"], json!(true));
        assert!(report.get("written").is_none(), "{report:#}");
        let target = real.join("assets").join(ASSET);
        assert_eq!(report["would_write"]["asset"], json!(target));
        assert!(
            summary.starts_with(&format!("{PACK}:{ASSET}: drafted (dry run: nothing written)")),
            "{summary}"
        );
        assert!(
            summary.contains(&format!(
                "dry run: nothing was written; --write would publish {}",
                target.display()
            )),
            "{summary}"
        );

        let (exit, report) = run_fixture(&fixture, fixture.role(), true);
        // The copy holds the asset from the first run, so nothing is written anywhere.
        assert_eq!(
            (exit, report["status"].as_str()),
            (0, Some("existing")),
            "{report:#}"
        );
        assert_eq!(snapshot(&real), before);
        fs::remove_dir_all(fixture.root.join("copy")).unwrap();
        let (exit, report) = run_fixture(&fixture, fixture.role(), true);
        assert_eq!(
            (exit, report["status"].as_str()),
            (0, Some("written")),
            "{report:#}"
        );
        assert_eq!(report["dry_run"], json!(false));
        assert!(report.get("would_write").is_none(), "{report:#}");
        let summary = report["summary"].as_str().unwrap();
        assert!(
            summary.contains(&format!("written: {}", real.join("assets").join(ASSET).display())),
            "{summary}"
        );
        assert!(!summary.contains("dry run"), "{summary}");
        assert!(real.join("assets").join(ASSET).join("asset.toml").is_file());
        assert!(
            real.join("model_sources")
                .join(format!("{ASSET}.json"))
                .is_file()
        );
    }

    #[test]
    fn an_existing_asset_is_never_overwritten_and_gets_the_stale_report() {
        let mut fixture = Fixture::new();
        assert_eq!(run_fixture(&fixture, fixture.role(), true).0, 0);
        fs::remove_dir_all(fixture.root.join("copy")).unwrap();
        let installed = snapshot(&fixture.pack());
        // Unchanged model: nothing to report, nothing written.
        let (exit, report) = run_fixture(&fixture, fixture.role(), true);
        assert_eq!(
            (exit, report["status"].as_str()),
            (0, Some("existing")),
            "{report:#}"
        );
        assert_eq!(report["stale"], json!([]));
        // An existing asset's report lists the doors through its present part too.
        assert_eq!(report["doors"][0]["position"], json!([-1.2, 0.7, 1.94]));
        // The rebuilt model moved its main door and gained a dwelling.
        fixture.sidecar["doors"][0]["position"][0] = json!(-2.0);
        fixture.sidecar["dwellings"] = json!(2);
        fixture.write_sidecar();
        fs::remove_dir_all(fixture.root.join("copy")).unwrap();
        let (exit, report) = run_fixture(&fixture, fixture.role(), true);
        assert_eq!(
            (exit, report["status"].as_str()),
            (2, Some("existing")),
            "{report:#}"
        );
        let checks: Vec<_> = report["stale"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line["check"].as_str().unwrap())
            .collect();
        for expected in ["facts", "entrance", "capacity"] {
            assert!(checks.contains(&expected), "{expected}: {report:#}");
        }
        assert_eq!(
            snapshot(&fixture.pack()),
            installed,
            "the report writes nothing"
        );
    }

    #[test]
    fn information_lines_leave_the_exit_status_and_refusals_name_the_asset() {
        let mut fixture = Fixture::new();
        // The copy keeps the asset from this run.
        assert_eq!(run_fixture(&fixture, fixture.role(), false).0, 0);
        // A mistyped id that hits the level-1 asset: role lines only, exit 0.
        let mut role = fixture.role();
        role["level"] = json!(2);
        let (exit, report) = run_fixture(&fixture, role, false);
        assert_eq!(
            (exit, report["status"].as_str()),
            (0, Some("existing")),
            "{report:#}"
        );
        assert_eq!(report["stale"], json!([]));
        assert_eq!(report["info"][0]["field"], "level", "{report:#}");
        let summary = report["summary"].as_str().unwrap();
        assert!(
            summary.contains("info: role [level]: the role gives level 2"),
            "{summary}"
        );
        // A refused role still names its pack and asset.
        fixture.sidecar["hero"] = json!(true);
        fixture.write_sidecar();
        let (exit, report) = run_fixture(&fixture, fixture.role(), false);
        assert_eq!(exit, 1, "{report:#}");
        let label = format!("{PACK}:{ASSET}");
        assert_eq!(report["asset"], json!(label));
        assert!(
            report["summary"]
                .as_str()
                .unwrap()
                .starts_with(&format!("{label}: refused")),
            "{report:#}"
        );
    }

    #[test]
    fn refusals_write_nothing() {
        let mut fixture = Fixture::new();
        fixture.sidecar["source"]["dirty"] = json!(true);
        fixture.write_sidecar();
        let before = snapshot(&fixture.root);
        let (exit, report) = run_fixture(&fixture, fixture.role(), true);
        assert_eq!(
            (exit, report["status"].as_str()),
            (1, Some("refused")),
            "{report:#}"
        );
        assert!(report["error"].as_str().unwrap().contains("dirty"));
        let after: Vec<_> = snapshot(&fixture.root)
            .into_iter()
            .filter(|(path, _)| !path.starts_with(fixture.root.join("copy")))
            .collect();
        assert_eq!(after, before);
        assert!(!fixture.root.join("copy").join(PACK).join("assets").exists());
        // A role the export contract refuses: an unsupported density.
        fixture.sidecar["source"]["dirty"] = json!(false);
        fixture.write_sidecar();
        let mut role = fixture.role();
        role["density"] = json!("towering");
        let (exit, _) = run_fixture(&fixture, role, true);
        assert_eq!(exit, 1);
        assert!(!fixture.root.join("copy").join(PACK).join("assets").exists());
    }

    #[test]
    fn a_workplace_needs_a_profile_and_passes_with_a_parking_warning() {
        let mut fixture = Fixture::new();
        fixture.sidecar["dwellings"] = json!(0);
        // A model defect the check reports: a main door 1.9375 m high on a 0.375 m plinth. A
        // low balcony door is no pedestrian door and is not sized.
        fixture.sidecar["doors"][0]["height_m"] = json!(1.9375);
        fixture.sidecar["doors"][0]["position"][1] = json!(0.375);
        fixture.sidecar["doors"][1]["kind"] = json!("balcony");
        fixture.sidecar["doors"][1]["height_m"] = json!(1.5);
        fixture.write_sidecar();
        let mut role = fixture.role();
        role["zone_type"] = json!("commercial");
        role["asset_id"] = json!("building.commercial.shop");
        // Without a profile the role is refused before anything is written.
        let (exit, report) = run_fixture(&fixture, role.clone(), false);
        assert_eq!(
            (exit, report["status"].as_str()),
            (1, Some("refused")),
            "{report:#}"
        );
        assert!(
            report["error"]
                .as_str()
                .unwrap()
                .contains("economy_profile")
        );
        assert!(!fixture.root.join("copy").join(PACK).join("assets").exists());
        role["economy_profile"] = json!("grocery_basic");
        let (exit, report) = run_fixture(&fixture, role.clone(), false);
        assert_eq!(exit, 1, "{report:#}");
        let failures = report["acceptance"]["failures"].to_string();
        assert!(
            failures.contains("door 'front' height_m 1.9375"),
            "{failures}"
        );
        assert!(!failures.contains("door 'back'"), "{failures}");
        fixture.sidecar["doors"][0]["height_m"] = json!(2.1);
        fixture.write_sidecar();
        role["asset_id"] = json!("building.commercial.grocery");
        let (exit, report) = run_fixture(&fixture, role, false);
        assert_eq!(exit, 0, "{report:#}");
        assert!(
            report["acceptance"]["warnings"]
                .to_string()
                .contains("parking")
        );
        assert!(report["draft"]["household_capacity"].is_null());
        // R5: the profile supplies the workers; the draft writes no number of its own (`TOOLS-38`).
        assert!(report["draft"].get("worker_capacity").is_none(), "{report:#}");
        let published = fs::read_to_string(
            fixture
                .root
                .join("copy")
                .join(PACK)
                .join("assets/building.commercial.grocery/asset.toml"),
        )
        .unwrap();
        assert!(published.contains("economy_profile = \"grocery_basic\""));
        assert!(!published.contains("worker_capacity"), "{published}");
    }

    #[test]
    fn check_runs_the_acceptance_check_on_the_existing_asset_and_writes_nothing() {
        let mut fixture = Fixture::new();
        fixture.sidecar["dwellings"] = json!(0);
        fixture.write_sidecar();
        let mut role = fixture.role();
        role["zone_type"] = json!("commercial");
        role["asset_id"] = json!("building.commercial.grocery");
        role["economy_profile"] = json!("grocery_basic");
        // Nothing to check before the asset exists.
        let (exit, report) = run_with(&fixture, role.clone(), false, json!({"check": true}));
        assert_eq!((exit, report["status"].as_str()), (1, Some("refused")), "{report:#}");
        assert!(report["error"].as_str().unwrap().contains("nothing to check"));
        // Drafted into the copy, the workplace warns that it has no parking.
        assert_eq!(run_fixture(&fixture, role.clone(), false).0, 0);
        let copy = fixture.root.join("copy");
        let before = snapshot(&copy);
        let (exit, report) = run_with(&fixture, role.clone(), false, json!({"check": true}));
        assert_eq!((exit, report["status"].as_str()), (0, Some("checked")), "{report:#}");
        assert!(report["acceptance"]["warnings"].to_string().contains("parking"), "{report:#}");
        let summary = report["summary"].as_str().unwrap();
        assert!(summary.contains("warn: a workplace without a parking anchor"), "{summary}");
        assert!(summary.contains("nothing was written") && !summary.contains("dry run"), "{summary}");
        assert_eq!(report["doors"][0]["id"], "front");
        assert_eq!(snapshot(&copy), before, "a check writes nothing");
        // The author adds parking in the editor: the warning clears.
        let manifest = copy.join(PACK).join("assets/building.commercial.grocery/asset.toml");
        let mut text = fs::read_to_string(&manifest).unwrap();
        text.push_str("\n[[anchors]]\ntype = \"parking\"\nname = \"staff\"\nposition = [7.5, 0, 9]\nforward = [0, 0, -1]\nwidth_m = 2.5\nlength_m = 5\nvehicle_class = \"car\"\n");
        fs::write(&manifest, text).unwrap();
        let (exit, report) = run_with(&fixture, role.clone(), false, json!({"check": true}));
        assert_eq!(exit, 0, "{report:#}");
        assert!(!report["acceptance"]["warnings"].to_string().contains("parking"), "{report:#}");
        // A failing check fails the command.
        fs::remove_file(copy.join(PACK).join("model_sources/building.commercial.grocery.json")).unwrap();
        let (exit, report) = run_with(&fixture, role, false, json!({"check": true}));
        assert_eq!((exit, report["status"].as_str()), (1, Some("checked")), "{report:#}");
        assert!(report["summary"].as_str().unwrap().contains("FAIL: the asset has no ledger entry"));
    }

    #[test]
    fn a_source_commit_goes_into_the_ledger_and_the_stale_report_names_a_different_one() {
        let fixture = Fixture::new();
        let commit = |c: &str| json!({"source_commit": c});
        let (exit, report) = run_with(&fixture, fixture.role(), false, commit("034DC4"));
        assert_eq!((exit, report["status"].as_str()), (1, Some("refused")), "{report:#}");
        let (exit, report) = run_with(&fixture, fixture.role(), true, commit("034dc4c"));
        assert_eq!(exit, 0, "{report:#}");
        let summary = report["summary"].as_str().unwrap();
        assert!(summary.contains("source: files from models commit 034dc4c"), "{summary}");
        let ledger = fs::read_to_string(fixture.pack().join(format!("model_sources/{ASSET}.json"))).unwrap();
        assert!(ledger.contains("\"source_commit\": \"034dc4c\""), "{ledger}");
        fs::remove_dir_all(fixture.root.join("copy")).unwrap();
        // The same commit: no line; another: an information line, exit unchanged. The fresh copy
        // drafts again; the real pack holds the asset, so the write reports instead.
        let (exit, report) = run_with(&fixture, fixture.role(), true, commit("034dc4c"));
        assert_eq!(
            (exit, report["status"].as_str(), report["info"].clone()),
            (0, Some("existing"), json!([])),
            "{report:#}"
        );
        fs::remove_dir_all(fixture.root.join("copy")).unwrap();
        let (exit, report) = run_with(&fixture, fixture.role(), true, commit("b5660bb"));
        assert_eq!(exit, 0, "{report:#}");
        assert_eq!(report["info"][0]["field"], "source_commit", "{report:#}");
        assert!(report["summary"].as_str().unwrap().contains("the ledger records 034dc4c"));
    }

    #[test]
    fn a_higher_level_needs_its_level_below() {
        let fixture = Fixture::new();
        let mut role = fixture.role();
        role["level"] = json!(2);
        role["asset_id"] = json!("building.residential.house_3_level_2");
        let (exit, report) = run_fixture(&fixture, role.clone(), false);
        assert_eq!(exit, 1, "{report:#}");
        assert!(
            report["acceptance"]["failures"]
                .to_string()
                .contains("level 1")
        );
        assert_eq!(run_fixture(&fixture, fixture.role(), false).0, 0);
        let (exit, report) = run_fixture(&fixture, role, false);
        assert_eq!(exit, 0, "{report:#}");
    }
}
