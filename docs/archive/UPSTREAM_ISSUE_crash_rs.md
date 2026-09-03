<!--
   =========================================================================
    MANIFEST
   =========================================================================
    script_name: UPSTREAM_ISSUE_crash_rs.md
    script_path: UPSTREAM_ISSUE_crash_rs.md
    module_name: upstream_issue_crash_rs
    version: 0.1.0
    description: Bug report prepared for the upstream repository:
             rust/src/debug.rs declares mod crash and re-exports from it,
             but rust/src/debug/crash.rs was never committed, so a clean
             clone fails to build with E0583. Written as a standalone
             report with the reproduction, the three independent checks
             that rule out a local cause, and the compiler output verbatim,
             so a maintainer can confirm it without access to this working
             copy.
    kind: module
    spec: none
    internal_dependencies: []
    external_dependencies: []
    features: [upstream-issue, build-failure, missing-module]
    api_version: metrum-v1.0.0
    last_updated: 2026-08-24
   =========================================================================
-->

# `rust/src/debug/crash.rs` is missing from the repository, so `main` does not build

## Summary

`rust/src/debug.rs` declares `mod crash;` and re-exports eight items from it,
but `rust/src/debug/crash.rs` is not in the repository. `cargo build` fails at
`main` with E0583 on every platform.

This is not a local or platform-specific problem. Verified three ways:

- `git ls-files rust/src/debug/` returns nothing, so the directory was never
  tracked.
- `rust/.gitignore` contains only `/target`, so nothing excluded it.
- The GitHub tree API for `main` lists 23 Rust files with `debug` in the path
  and no `crash.rs` among them; searching the whole tree for `crash` returns an
  empty set.

Most likely an uncommitted local file. `run.sh` documents
`METRUM_CRASH_DIAGNOSTICS=1` as defaulting on under `--release` and writing
panic dumps to `logs/`, so the feature clearly exists in a working copy.

## Reproduction

```
git clone https://github.com/jmkekala/metrum_rise
cd metrum_rise/rust
cargo build --release
```

Result, after all 217 dependencies build successfully:

```
error[E0583]: file not found for module `crash`
  --> src\debug.rs:17:1
   |
17 | mod crash;
   | ^^^^^^^^^^
   |
   = help: to create the module `crash`, create file "src\debug\crash.rs"
error: could not compile `metrum_rise` (lib) due to 1 previous error
```

Reproduced on `x86_64-pc-windows-msvc` with Rust 1.96.1 at commit `fed286a`.
Nothing in the error is platform-dependent.

## The interface the missing file must satisfy

Reconstructing enough of a stub to compile past E0583 made the compiler state
the exact signatures, which may be useful when restoring the real file. All of
these come from type errors, not from guesswork:

```rust
pub static CRASH_DIAGNOSTICS_ENABLED: AtomicBool;
pub fn is_crash_diagnostics_enabled() -> bool;

pub(crate) enum CrashCommand {
    Undo,
    Bulldoze,
    SetSpeed { speed: f32 },
    SetCameraAabb { x_min: f32, x_max: f32, z_min: f32, z_max: f32 },
    AddRoad { point_count: usize, fwd_lanes: i32, bkw_lanes: i32, snap_to_existing_roads: bool },
}

pub(crate) struct CrashSimSnapshot {
    pub day_index: u32,
    pub minute_of_day: u16,
    pub speed_multiplier: f32,
    pub agent_count: usize,
    pub pathfind_count: u32,      // u32, not usize
    pub building_count: usize,
    pub household_count: usize,
    pub road_node_count: usize,
    pub road_edge_count: usize,
    pub road_generation: u64,
    pub pending_demand_spawns: usize,
    pub last_agent_tick_us: u64,
    pub last_tick_duration_ms: f64,
    pub terrain_dirty: bool,
    pub water_dirty: bool,
    pub network_dirty: bool,
}

pub(crate) fn init();
pub(crate) fn record_crash_phase(phase: &'static str, snapshot: CrashSimSnapshot);
pub(crate) fn record_crash_command(command: CrashCommand, snapshot: CrashSimSnapshot);
pub(crate) fn record_crash_frame(
    summary: CrashSimSnapshot,
    active_ms: f64, command_ms: f64, lock_wait_ms: f64, lock_held_ms: f64,
    snapshot_ms: f64, snapshot_write_ms: f64,
    elapsed_minutes: u16,          // u16, not u32
    pending_spawns_executed: usize,
    hourly_ticks: usize,           // usize, not u32
    daily_ticks: usize,            // usize, not u32
    commands_processed: usize,
);
pub(crate) fn flush_crash_diagnostics(phase: &str);  // &str, not &'static str
```

Four details worth noting, because each one produced a compile error when
guessed the obvious way:

- `CrashSimSnapshot.pathfind_count` is `u32`, since it comes from
  `core.agents.pathfind_count.load(Ordering::Relaxed)`.
- `record_crash_frame` takes `elapsed_minutes: u16`, and `hourly_ticks` and
  `daily_ticks` as `usize`.
- `flush_crash_diagnostics` takes `&str` rather than `&'static str`, because
  `run_sim_phase<T>(phase: &str, ...)` passes a borrowed name; requiring
  `'static` produces E0521.
- `debug.rs:95` calls `crash::init()`, so the module also owns an initialiser
  that the re-export list does not mention.

## Suggested fix

Commit the real `rust/src/debug/crash.rs`. If it is not readily recoverable, a
no-op implementation of the interface above compiles and lets the tree build,
with crash diagnostics inert until the real recorder returns.

Happy to open a PR with whichever you prefer.
