// SPDX-License-Identifier: GPL-2.0-only

//! The Godot test guard's refusal (`TOOLS-19` part 2, docs/debug.md "Godot Test Guard"): a test
//! script, benchmark matrix or probe started in the player's real profile stops before it writes
//! anything, with an error and a non-zero exit.
//!
//! The run is a test when its command line names a script under `res://tests/` or
//! `res://scripts/benchmarks/` (`--script` / `-s`), or a benchmark user argument. Its profile is
//! the player's when the project keeps its own name (an isolated run's copy is renamed:
//! `tools/godot_isolated.py`) and Godot's data directory is the platform's default one (an
//! absolute `XDG_DATA_HOME` moves it; Godot ignores a relative one). What cannot be decided (no
//! home directory) refuses. Godot's own log opens before extensions load, and an editor import
//! runs no test, so neither is covered here; the launcher covers both.

use std::path::{Path, PathBuf};

/// The project's own name (`application/config/name`); the user data directory of the player's
/// profile is named after it.
pub const PLAYER_PROJECT_NAME: &str = "Metrum Rise";

// Script folders whose scripts are tests, benchmarks or probes.
const TEST_SCRIPT_DIRS: [&str; 2] = ["tests/", "scripts/benchmarks/"];
// User arguments (after `--`) that start a benchmark matrix or probe.
const BENCHMARK_ARGS: [&str; 3] = [
    "--gameplay-road-benchmark",
    "--benchmark",
    "--generate-benchmark",
];

/// What the refusal reads of the process, gathered once at startup.
pub struct LaunchFacts<'a> {
    /// Every command-line argument (`OS.get_cmdline_args()` and the user arguments after `--`).
    pub args: &'a [String],
    /// `application/config/name` as the project settings hold it (overrides applied).
    pub project_name: &'a str,
    /// Godot's data directory (`OS.get_data_dir()`), under which `app_userdata` lives.
    pub data_dir: &'a str,
    /// The platform's default data directory, `None` when it cannot be derived.
    pub default_data_dir: Option<PathBuf>,
}

/// The platform's default data directory for `os_name` (`OS.get_name()`): the one Godot uses
/// when nothing moves it. `None` without a home directory (or `APPDATA` on Windows).
pub fn default_data_dir(os_name: &str, env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let home = || {
        env("HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
    };
    match os_name {
        "Windows" => env("APPDATA")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from),
        "macOS" => home().map(|home| home.join("Library/Application Support")),
        _ => home().map(|home| home.join(".local/share")),
    }
}

// The script a `--script <path>` / `-s <path>` names, without its `res://`.
fn test_script(args: &[String]) -> Option<&str> {
    args.windows(2).find_map(|pair| {
        if pair[0] != "--script" && pair[0] != "-s" {
            return None;
        }
        let path = pair[1].strip_prefix("res://").unwrap_or(&pair[1]);
        TEST_SCRIPT_DIRS
            .iter()
            .any(|dir| path.starts_with(dir))
            .then_some(pair[1].as_str())
    })
}

// Whether two directories are the same, compared canonical where both exist.
fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a.components().eq(b.components()),
    }
}

/// Why this run must not start (`None`: it may). A test, benchmark or probe in the player's
/// profile is refused; so is one whose profile cannot be told apart from it.
pub fn refusal(facts: &LaunchFacts) -> Option<String> {
    let what = test_script(facts.args)
        .map(|script| format!("test script {script}"))
        .or_else(|| {
            BENCHMARK_ARGS
                .iter()
                .find(|arg| facts.args.iter().any(|given| given == *arg))
                .map(|arg| format!("benchmark argument {arg}"))
        })?;
    if facts.project_name != PLAYER_PROJECT_NAME {
        return None;
    }
    let real = match &facts.default_data_dir {
        Some(default) => same_dir(Path::new(facts.data_dir), default),
        None => true,
    };
    real.then(|| {
        format!(
            "refusing to run the {what} in the player's profile ({}/godot/app_userdata/{}): run \
             it through tools/godot_isolated.py (TOOLS-19, docs/debug.md \"Godot Test Guard\")",
            facts.data_dir, PLAYER_PROJECT_NAME
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    fn facts<'a>(args: &'a [String], name: &'a str, data: &'a str) -> LaunchFacts<'a> {
        LaunchFacts {
            args,
            project_name: name,
            data_dir: data,
            default_data_dir: Some(PathBuf::from("/home/p/.local/share")),
        }
    }

    #[test]
    fn a_test_in_the_players_profile_is_refused() {
        let test = args(&[
            "--headless",
            "--script",
            "res://tests/graded_tile_renderer_test.gd",
        ]);
        let refused = refusal(&facts(&test, "Metrum Rise", "/home/p/.local/share"));
        assert!(refused.unwrap().contains("graded_tile_renderer_test"));
        let short = args(&["-s", "tests/x_test.gd"]);
        assert!(refusal(&facts(&short, "Metrum Rise", "/home/p/.local/share/")).is_some());
        let bench = args(&["--path", "godot", "--", "--gameplay-road-benchmark"]);
        assert!(refusal(&facts(&bench, "Metrum Rise", "/home/p/.local/share")).is_some());
        let probe = args(&[
            "--script",
            "res://scripts/benchmarks/road_terrain_replay.gd",
        ]);
        assert!(refusal(&facts(&probe, "Metrum Rise", "/home/p/.local/share")).is_some());
    }

    #[test]
    fn an_isolated_or_player_run_starts() {
        let test = args(&["--script", "res://tests/graded_tile_renderer_test.gd"]);
        // A renamed project copy, or an absolute data home elsewhere.
        assert!(
            refusal(&facts(
                &test,
                "MetrumGodotTests-abc",
                "/home/p/.local/share"
            ))
            .is_none()
        );
        assert!(refusal(&facts(&test, "Metrum Rise", "/tmp/profile/data")).is_none());
        // The player's own run, and a script outside the test folders.
        let play = args(&["--path", "godot"]);
        assert!(refusal(&facts(&play, "Metrum Rise", "/home/p/.local/share")).is_none());
        let tool = args(&["--script", "res://scripts/tools/x.gd"]);
        assert!(refusal(&facts(&tool, "Metrum Rise", "/home/p/.local/share")).is_none());
    }

    #[test]
    fn an_unknown_home_refuses() {
        let test = args(&["--script", "res://tests/x_test.gd"]);
        let mut unknown = facts(&test, "Metrum Rise", "/somewhere");
        unknown.default_data_dir = None;
        assert!(refusal(&unknown).is_some());
    }

    #[test]
    fn default_data_dirs_follow_the_platform() {
        let env = |name: &str| match name {
            "HOME" => Some("/home/p".to_string()),
            "APPDATA" => Some("C:/Users/p/AppData/Roaming".to_string()),
            _ => None,
        };
        let linux = default_data_dir("Linux", env);
        assert_eq!(linux, Some(PathBuf::from("/home/p/.local/share")));
        let mac = default_data_dir("macOS", env);
        assert_eq!(
            mac,
            Some(PathBuf::from("/home/p/Library/Application Support"))
        );
        let windows = default_data_dir("Windows", env);
        assert_eq!(windows, Some(PathBuf::from("C:/Users/p/AppData/Roaming")));
        assert_eq!(default_data_dir("Linux", |_| None), None);
    }
}
