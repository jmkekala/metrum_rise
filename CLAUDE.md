# Metrum Rise — CLAUDE.md

## Project Overview

Metrum Rise is a city simulation game inspired by Cities: Skylines and SimCity. The long-term goal is to support **≥1,000,000 concurrent autonomous agents** on a 20 km × 20 km map with realistic traffic, zoning, economic, and environmental simulation.

**Architecture:** Rust simulation backend compiled as a GDExtension DLL (`libmetrum_rise.so`), loaded by a Godot 4 frontend that handles rendering and user input.

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Game Engine | Godot 4 (GDScript) |
| Simulation | Rust 2024 edition |
| Godot–Rust bridge | `godot-rust` (gdext 0.4.5), `experimental-threads` feature |
| Parallelism | Rayon 1.10 |
| Serialization | serde + serde_json |
| Benchmarking | criterion 0.5 |

## Project Structure

```
metrum_rise/
├── rust/src/
│   ├── lib.rs                         GDExtension entry point
│   ├── config.rs                      Global constants (map size, grid cell size, lane widths)
│   ├── nodes/simulation_node.rs       Main Godot bridge (1,654 lines — planned for splitting)
│   └── simulation/
│       ├── core/time.rs               Day-clock, speed multiplier
│       ├── terrain/                   Heightmap, raycasting
│       ├── water/                     Shallow-water SWE equations
│       ├── network/                   Road graph, topology, lane types, rendering
│       ├── pathing/                   A*, HPA* (hierarchical pathfinding)
│       ├── grid/                      DataGrid<T>, zoning, pollution, noise, desirability
│       ├── buildings/allocator.rs     BuildingAllocator
│       └── economy/agents.rs          AgentSystem (Structure-of-Arrays FSM, 707 lines)
├── godot/scripts/                     GDScript: UI, input, tool panels, rendering bridges
├── docs/                              Architecture notes, phase plans, benchmarking howto
└── run.sh                             Build script: cargo build → deploy .so → launch Godot
```

## Building and Running

```bash
./run.sh              # Debug build, deploys .so, launches Godot
./run.sh --headless   # Headless mode
cd rust && cargo build --release   # Release build
cd rust && cargo bench             # Criterion benchmarks
```

The compiled library must be at `godot/bin/libmetrum_rise.so`. `run.sh` handles this automatically.

## Performance Philosophy

**This project is performance-first.** The 1M-agent scale target is non-negotiable and must be kept in mind for every decision, including small ones. Correctness without acceptable performance is not a done state.

- Measure before you add. Every new system must have a clear complexity bound. If a proposed implementation would degrade an existing O(1) or O(log N) path to O(N) or worse at city scale, it is not acceptable.
- Reuse before you build. Before writing new data structures, algorithms, or abstractions, check whether an existing one already solves the problem. `DataGrid<T>`, the 512 m chunk index, the SoA agent layout, and Rayon parallelism cover the majority of simulation needs. Adding a fifth grid or a second spatial index for a job that `DataGrid` already handles is a maintenance cost with no benefit.
- Hot-path allocations are bugs. Any allocation inside a per-tick or per-agent loop is a correctness issue at scale, not a style issue.
- Parallelism is the default, not an optimisation. If a system iterates over a flat collection independently per element, it uses `rayon::par_iter`. Single-threaded iteration over large collections is a conscious exception that must be justified.

## Key Design Patterns

- **DataGrid\<T\>** — flat 2-D grid used for terrain, pollution, noise, desirability, and zoning. Prefer this over ad-hoc spatial structures.
- **Structure-of-Arrays (SoA)** — `AgentSystem` uses parallel `Vec<T>` fields indexed by agent ID for cache-friendly bulk iteration.
- **Rayon par_iter()** — grid ticks (pollution, noise, desirability, water) are parallelised. `AgentSystem.tick()` is a near-term parallelisation target.
- **Spatial chunk index** — 512 m chunks used to accelerate road edge queries. Replicate this pattern for any new spatial lookup, not linear scans.

## Bugs and Backlog

`docs/project.md` is the **single source of truth** for the current state of the codebase. It must be kept accurate at all times. Update it whenever:

- **A bug is fixed** — remove the entry from the Known Bugs table entirely. Do not leave it in with a "fixed" note.
- **A backlog item is implemented** — move it from the Backlog section into Implemented Systems with an accurate description of what was built.
- **A system's behaviour changes** — update the relevant Implemented Systems entry. Stale descriptions are as harmful as missing ones.
- **A new bug is discovered** — add it to Known Bugs with a severity tag and the exact file/function where the root cause lives.
- **A new backlog item is identified** — add it to the appropriate milestone section with complexity and dependency notes.

Do not introduce workarounds that mask known bugs. Fix the root cause and remove the entry from the bug table.

`docs/project.md` has a milestone structure: **v0.01 Blockers** (must fix before tagging), **v0.01 Goals** (quality targets including the multi-modal transport foundation), **v0.1** (100k-agent feature milestone), **v0.2** (250k–500k agent scaling baseline + first new transport mode), **v1.0** (1M-agent target). Backlog items belong in the earliest milestone where they become necessary — do not defer performance-critical items to a later milestone just because they are not yet visibly broken.

## Documentation Practices

- **`docs/project.md`** — single source of truth for what is implemented, what is broken, and what is planned. Update it as described above. It must reflect the actual state of the code, not an aspirational state.
- **`docs/analysis.md`** — contains the detailed algorithmic and data-structure analysis: which technologies were selected, why, what alternatives exist, and how each system scales toward 1M agents. It also documents the multi-modal transport compatibility analysis (bicycles, buses, taxis, trains, ships, airplanes). **Update `docs/analysis.md` only when explicitly requested** — it is a reference document, not a living log. When updated, it should be re-exported to `docs/analysis.pdf` via `pandoc docs/analysis.md -o docs/analysis.pdf --pdf-engine=xelatex`.
- **Do not create additional `*.md` files in `docs/`** unless they are truly standalone references that `project.md` links to. Default is to edit `project.md`.
- **Do not create standalone `*.md` files outside `docs/`** (except `CLAUDE.md` and `README`).
- **Severity tags in `docs/project.md`**: `[BLOCKER]` = must fix before v0.01, `[BUG]` = correctness failure, `[v0.01]` / `[v0.1]` / `[v0.2]` / `[v1.0]` = milestone targets. Use these consistently.
- When a bug is fixed, remove it from the Known Bugs table. When a backlog item ships, move it to Implemented Systems.

## AI Behaviour Guidelines

### General Approach

- Read existing code before suggesting changes. Understand the module's role and its interactions with adjacent modules.
- **Before implementing anything new, check whether an existing system already solves the problem.** `DataGrid<T>`, the chunk spatial index, the SoA agent layout, and Rayon cover most needs. Extend these before introducing new structures.
- Prefer targeted, minimal edits. Do not refactor or reorganise code that is not part of the current task.
- Do not add docstrings, comments, or type annotations to code you did not change.
- Do not introduce new dependencies without a clear justification — the crate list is intentionally lean.
- State the complexity bound of any new hot-path code. If it is worse than the existing path, it must be explicitly justified.
- Do not introduce `unsafe` blocks without explicit approval. If unsafe is necessary, explain exactly which invariant you are upholding and why safe alternatives were ruled out.
- Follow the existing error handling pattern in the module. Do not switch between `anyhow`, `thiserror`, `?`-propagation styles, or panic/unwrap approaches unless explicitly asked.
- If a borrow checker conflict arises, explain the ownership issue before proposing a solution. If resolving it requires a structural refactor (splitting a struct, reordering operations, changing ownership), flag it to the user rather than doing it silently — these conflicts often surface real architectural decisions.
- All suggested code must compile. If you are uncertain whether something compiles, say so explicitly rather than presenting it with false confidence.
- Do not add, remove, or modify tests outside the scope of the current task. If new logic clearly needs a test, flag it rather than silently writing one.
- Show changes as minimal diffs, not full file rewrites, unless a full rewrite was explicitly requested.
- Atomic Checklist Updates: When updating docs/project.md or other tracking files, use the smallest possible `TargetContent` to avoid overwriting adjacent tasks. 
- Preserve Pending Task Identifiers: Never remove or modify lines containing [(B1, B2, etc.)] identifiers unless explicitly tasked with those specific items.
- Section Integrity: When refactoring a list (e.g., merging "Technical Debt" into "DONE"), ensure that all uncompleted items are either preserved in their original section or explicitly moved, never deleted.

### Rust Code Style

- Match the existing style: no unnecessary `pub`, no redundant type annotations, no defensive `unwrap`/`expect` chains for unreachable states.
- Parallelism must use Rayon. Do not introduce `std::thread::spawn` for simulation work.
- Avoid allocating inside hot loops. Prefer pre-allocated buffers and SoA patterns consistent with `AgentSystem`.
- All new spatial lookups must use the chunk-based spatial index or `DataGrid`. Linear scans over full collections are not acceptable at simulation scale.

### Godot / GDScript

- GDScript files are thin rendering and input bridges only. Simulation logic belongs in Rust.
- Do not move simulation state or decisions into GDScript. GDScript calls Rust methods; it does not compute game outcomes.

**GDScript documentation rules:**

- Every `.gd` file must have a `##` class-level header block (before `extends`) stating what the script manages and which Rust methods it calls. This is the GDScript equivalent of a Rust `//!` module header.
- GDScript's `##` system does NOT generate navigable HTML for user scripts — it only powers editor tooltips for `@export` and signals. Do not add `##` to individual functions; there is no tooling payoff and the maintenance cost is real.
- Use inline `#` comments for non-obvious geometry, data unpacking (e.g. 12-float transform buffers), and state machine transitions. Skip comments on simple event handlers and UI construction code.
- The canonical reference for which script calls which Rust methods is in `docs/project.md` → Godot Layer. Keep that table updated when methods are added or renamed.

### Testing

- Unit tests live alongside source files as `#[cfg(test)]` modules or separate `*_test.rs` files in the same directory.
- Integration tests go in `rust/tests/`.
- Tests must not depend on Godot being present — keep simulation logic fully testable without the engine.

### Rustdoc

Every new **public** item (`pub struct`, `pub enum`, enum variant, `pub fn`, `pub const`) **must** have a `///` doc comment at the time it is written. `#![warn(missing_docs)]` is enabled in `lib.rs` and will produce a compiler warning for any public item that is missing one.

Rules:
- `///` is for public API items. `//` is for implementation detail inside function bodies.
- Doc comments must add information not already in the signature. "Returns the node count" on `get_node_count()` is not acceptable.
- Every source file and `mod.rs` must have a `//!` module-level header (one paragraph describing the module's role in the simulation).
- Do not add `# Examples` blocks at this stage — field/method contracts take priority.
- Private functions and fields use `//` inline comments only.

To check doc coverage after changes:

```bash
cd rust && cargo doc --no-deps 2>&1 | grep "warning\[missing_docs\]" | wc -l
```

### What Not to Do

- Do not add `println!` or `godot_print!` debug output and leave it in. Use the benchmark suite or a dedicated debug flag.
- Do not add feature flags or backwards-compatibility shims for code that can simply be changed.
- Do not create new `docs/` files for topics already covered in existing documentation — update the existing file instead.
- Do not suggest GPU compute, ECS frameworks, or other large architectural additions — these are explicitly deferred to post-v0.01.
