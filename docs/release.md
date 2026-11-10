# Release Builds

Release binaries for Linux, Windows and macOS are built, tested and published by GitHub Actions
([`.github/workflows/release.yml`](../.github/workflows/release.yml), `TOOLS-16`). Nobody builds
releases by hand.

## When a release is built

- Every push to `main` that changes more than Markdown or `docs/` builds and publishes a
  release. `dev` and every other branch never do: `dev` changes constantly and is for developers.
- To try a workflow change, add a test branch to the trigger temporarily and push it: it builds,
  exports and smoke-tests, and never publishes (`publish` runs on `main` only). Remove the
  trigger before the change lands. The workflow was tested this way on `ci/release`.
- A failed run can be repeated from the Actions page ("Re-run jobs"); a re-run of a released
  commit keeps its version and replaces the files.

## Versions

- A release is `YYYY.M.N` (`2026.11.1`): the UTC year and month of the build, and N counting that
  month's releases from 1. The tag is `v2026.11.1`. `tools/ci/version.sh` computes it.
- Test runs are labelled `test-<sha7>` and take no number.
- The version is stamped into `application/config/version` for the build only, so the main menu
  and the macOS bundle show it; it is never committed. The library jobs get it as
  `METRUM_VERSION`, and saves record it as `save_meta.game_build`. A local build shows and
  records `dev`. The crate version in `Cargo.toml` is not the game version and stays unchanged.
- Save and world compatibility is not in the number. Both formats load only on an exact match
  (`SAVE_VERSION`, `WORLD_DEFINITION_FORMAT_VERSION`), and the release notes say whether saves
  and worlds from the previous release still load (`tools/ci/release_notes.sh`).

## Pipeline

| Job | Runner | Work |
| --- | --- | --- |
| `version` | Ubuntu 24.04 | the version above |
| `lib-linux` | Ubuntu 22.04 | `libmetrum_rise.so`; the oldest supported image keeps the glibc requirement low |
| `lib-windows` | Ubuntu 24.04 | `metrum_rise.dll`, cross-built with MinGW (`x86_64-pc-windows-gnu`) |
| `lib-macos` | macOS 15 | `libmetrum_rise.dylib`, arm64 and x86_64 merged with `lipo`, macOS 11 minimum |
| `export` | Ubuntu 24.04 | pinned official Godot editor and templates (`tools/ci/install_godot.sh`, checked against Godot's SHA512 sums), import, the three exports, packages with `.sha256` files |
| `smoke-linux`, `smoke-windows` | Ubuntu 24.04 | `tools/ci/smoke.sh` on the unpacked packages; Windows runs under WineHQ's Wine 11.0, pinned (Ubuntu's Wine 9.0 crashes starting Godot 4.7) |
| `publish` | Ubuntu 24.04 | `main` only: a GitHub Release with the packages and notes, published only when complete |

- Rust is pinned to the version developers use (`RUST_VERSION`), Godot to `GODOT_VERSION`.
  Libraries are built `--locked` and without debug info.
- Godot exits 0 even when an import or export logs errors, so the export job fails on any `ERROR`
  line itself.
- Only GitHub's own actions are used, each pinned to a commit.

## Packages

| File | Contents |
| --- | --- |
| `MetrumRise-linux-x86_64.tar.gz` | `MetrumRise/` with the executable, `MetrumRise.pck` and the library |
| `MetrumRise-windows-x86_64.zip` | `MetrumRise/` with `MetrumRise.exe`, `MetrumRise.pck` and the DLL |
| `MetrumRise-macos-universal.zip` | `Metrum Rise.app`, bundle ID `com.metrumrise.game`, ad-hoc signed |

- The macOS bundle ID is fixed: a change makes macOS treat the game as a different app.
- Builds are unsigned. Windows shows SmartScreen; macOS needs approval under Privacy & Security.
- Players keep saves and packs in `user://`, so an update replaces only the game folder.
- About 500 MB compressed per platform; 4K textures in `assets/textures/general` are most of it.
- Extra content packs are shared separately as `.metrum.zip` (`TOOLS-09`/`TOOLS-10`).

## Exported-game rules

An exported game has neither the repository nor raw `res://` files (`TOOLS-15`):

- Runtime Rust embeds shipped data with `include_str!`; `env!("CARGO_MANIFEST_DIR")` paths are for
  tests and benchmarks only.
- A file that scripts read raw (`GLTFDocument.append_from_file`, `Image.load_from_file`,
  `FileAccess`) imports as `keep`; a non-resource file it needs (the walker `.bin` buffers) goes
  in the presets' include filter.
- First-run setup copies `res://bootstrap/` without `.import` sidecars.

## Smoke test

`tools/ci/smoke.sh linux|windows <game-dir> <bootstrap.sha256>` runs each bundled world on a fresh
profile, with `tools/ci/smoke_driver.gd` injected as an autoload through `override.cfg` (release
templates ignore `--script`). The driver starts a new game as the main menu does. A world passes
when:

- the driver reports a real `SimulationNode` in the gameplay scene;
- the log has no `ERROR` or `WARNING` line, except Godot's release-template popup messages
  (`UI-11`, upstream godot#87626);
- the first-run packs and worlds match the export job's `bootstrap.sha256` byte for byte, with no
  Git LFS pointer files.

The smoke jobs check out only `tools/ci`, so a path baked in at compile time cannot resolve. A run
takes well under a minute per platform. Locally, hide the checkout the same way (for example with
`bwrap --tmpfs <checkout>`) and keep Godot on an isolated profile.

Not covered: macOS (no Mac runner smoke test yet), real Windows, GPU rendering, and play beyond
starting a city. A person tries each platform's package before announcing a release.

## When Godot or Rust changes

- New Godot: update `GODOT_VERSION`; when the release includes godot#123998, remove the `UI-11`
  allowance in `smoke.sh` and close `UI-11`.
- New Rust for development: update `RUST_VERSION` in the same change.
