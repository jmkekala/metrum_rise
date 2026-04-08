# Metrum Rise — CLAUDE.md

## Project Overview

Metrum Rise is a large-scale city simulation game. The long-term goal is to support **≥1,000,000 total population across simulation tiers**. Full-FSM simulation is reserved for the active area of interest; distant parts of the same world may run at coarser aggregate fidelity. World scale is flexible, with 20 km × 20 km as the default working scale.

**Architecture:** Rust simulation backend compiled as a GDExtension DLL (`libmetrum_rise.so`), loaded by a Godot 4 frontend that handles rendering and user input.

## Tech Stack

Current stack summary. For exact versions, check `rust/Cargo.toml` and the Godot project/runtime.

- Godot 4.x frontend, Rust simulation backend, `godot-rust` / gdext bridge
- Rayon for parallelism
- SQLite (`rusqlite`) plus serde-based data formats
- Criterion for benchmarks
- Blender for modeling

## Project Structure

- `rust/src/simulation/` contains the core simulation systems: network, pathing, buildings, economy, grids, and save/load.
- `rust/src/nodes/` contains the Godot bridge and runtime-facing APIs.
- `rust/src/assets/` contains asset packs, registry, and validation.
- `godot/scripts/` contains thin UI/input/render bridges.
- `docs/` contains the implementation ledger and subsystem specs.

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

- Determinism is the default. Given the same save state, inputs, and tick sequence, the simulation should produce the same results. Any intentional randomness must be explicit, controlled, and justified; cosmetic variance should not silently change simulation outcomes.
- Measure before you add. Every new system must have a clear complexity bound. If a proposed implementation would degrade an existing O(1) or O(log N) path to O(N) or worse at city scale, it is not acceptable.
- Reuse before you build. Before writing new data structures, algorithms, or abstractions, check whether an existing one already solves the problem. `DataGrid<T>`, the road-edge `rstar` R-tree, the 16 m node lookup grid, the 512 m building/routing chunk indices, the SoA agent layout, and Rayon parallelism cover the majority of simulation needs. Adding another spatial structure when one of those already answers the query is a maintenance cost with no benefit.
- Hot-path allocations are bugs. Any allocation inside a per-tick or per-agent loop is a correctness issue at scale, not a style issue.
- Parallelism is the default, not an optimisation. If a system iterates over a flat collection independently per element, it uses `rayon::par_iter`. Single-threaded iteration over large collections is a conscious exception that must be justified.

## Bugs and Backlog

- `docs/project.md` is the implementation ledger: shipped behavior, known bugs, and milestone backlog. Keep it accurate.
- Update it whenever a bug is fixed, a system changes materially, or backlog status changes.
- Do not hide bugs behind workarounds. Fix the root cause and update the ledger.
- Put backlog work in the earliest milestone where it becomes necessary; do not defer performance-critical work just because it is not yet visibly broken.

## Documentation Practices

- **`docs/project.md`** — current implementation ledger for what is shipped, what is broken, and what is planned next. Keep it summary-first: status, bugs, backlog, and links out to subsystem docs. It must reflect the actual state of the code, not an aspirational state.
- **Subsystem docs in `docs/`** — files such as `entrance_and_exit.md`, `economy.md`, `demand.md`, and `zoning.md` own the detailed design/spec contracts for their respective systems. When behavior changes, update both the subsystem spec and the relevant status note in `project.md`.
- **`CLAUDE.md` / `AGENTS.md`** — contributor guidance, architectural invariants, workflow rules, and sharp edges for coding agents. Keep this file focused on stable guidance rather than exhaustive feature-by-feature implementation history.
- **Do not create additional `*.md` files in `docs/`** unless they are truly standalone references that `project.md` links to. Prefer updating the owning subsystem document rather than piling more detail into `project.md`.
- **Do not create standalone `*.md` files outside `docs/`** (except `CLAUDE.md` and `README`).

## AI Behaviour Guidelines

### General Approach

- Read existing code before suggesting changes. Understand the module's role and its interactions with adjacent modules.
- **Before implementing anything new, check whether an existing system already solves the problem.** Extend existing simulation systems and indices before introducing new structures.
- Prefer targeted, minimal edits. Do not refactor or reorganise code that is not part of the current task.
- Do not introduce new dependencies without a clear justification.
- State the complexity bound of any new hot-path code. If it is worse than the existing path, it must be explicitly justified.
- Do not introduce `unsafe` blocks without explicit approval. If unsafe is necessary, explain exactly which invariant you are upholding and why safe alternatives were ruled out.
- Follow the existing error handling pattern in the module. Do not switch between `anyhow`, `thiserror`, `?`-propagation styles, or panic/unwrap approaches unless explicitly asked.
- If a borrow checker conflict arises, explain the ownership issue before proposing a solution. If resolving it requires a structural refactor (splitting a struct, reordering operations, changing ownership), flag it to the user rather than doing it silently — these conflicts often surface real architectural decisions.
- All suggested code must compile. If you are uncertain whether something compiles, say so explicitly rather than presenting it with false confidence.
- Do not add, remove, or modify tests outside the scope of the current task. If new logic clearly needs a test, flag it rather than silently writing one.
- Show changes as minimal diffs, not full file rewrites, unless a full rewrite was explicitly requested.
- When updating tracking docs, make the smallest safe edit and preserve unrelated pending items.

### Rust Code Style

- Match the existing style: no unnecessary `pub`, no redundant type annotations, no defensive `unwrap`/`expect` chains for unreachable states.
- Parallelism must use Rayon. Do not introduce `std::thread::spawn` for simulation work.
- Avoid allocating inside hot loops. Prefer pre-allocated buffers and SoA patterns consistent with `AgentSystem`.
- All new spatial lookups must use the existing spatial system that fits the query. Linear scans over full collections are not acceptable at simulation scale.

### Simulation Safety Notes

These are the stable, high-level sharp edges worth remembering. Keep detailed subsystem rules in the code and subsystem docs, not here.

- Use high-level mutators for live simulation systems. Avoid direct external mutation of the road graph, agent SoA storage, building occupancy bookkeeping, or other indexed state unless you are intentionally working inside that subsystem.
- Treat topology edits as multi-system changes. When roads, lanes, buildings, or indices are remapped, the dependent adjacency, caches, and cross-system references must stay in sync in the same change.
- Treat `AgentSystem` and `BuildingAllocator` as tightly coupled indexed stores. Swap-remove and remap behavior means IDs are not casually stable; prefer lifecycle helpers over open-coded field mutation.
- Keep benchmarks and isolated tests away from full gameplay constructors unless you are explicitly measuring that end-to-end path. Setup helpers can trigger zoning work, cache rebuilds, path planning, or other expensive side effects that hide the cost you meant to measure.
- When touching routing or entrance/exit behavior, update both the authoritative state and the derived caches/plans that consume it. If a value is derived, make the rebuild path explicit rather than relying on accidental later repair.

### Godot / GDScript

- GDScript files are thin rendering and input bridges only. Simulation logic belongs in Rust.
- Do not move simulation state or decisions into GDScript. GDScript calls Rust methods; it does not compute game outcomes.
- Every `.gd` file must have a `##` class-level header block before `extends`.
- Use inline `#` comments for non-obvious geometry, packed data formats, and state transitions. Skip comments on obvious handlers and boilerplate UI setup.

### Testing

- Unit tests live alongside source files as `#[cfg(test)]` modules or separate `*_test.rs` files in the same directory.
- Integration tests go in `rust/tests/`.
- Tests must not depend on Godot being present — keep simulation logic fully testable without the engine.
- For isolated graph, routing, or performance tests, prefer minimal subsystem setup over full gameplay placement/spawn flows unless those side effects are explicitly part of the test.

### Rustdoc

Every new **public** item (`pub struct`, `pub enum`, enum variant, `pub fn`, `pub const`) **must** have a `///` doc comment at the time it is written. `#![warn(missing_docs)]` is enabled in `lib.rs` and will produce a compiler warning for any public item that is missing one.

Rules:
- `///` is for public API items. `//` is for implementation detail inside function bodies.
- Doc comments must add information not already obvious from the signature.
- Every source file and `mod.rs` must have a `//!` module-level header.
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
