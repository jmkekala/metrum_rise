# Metrum Rise — Docs Guide

This directory stays intentionally flat for now. Each file should have one clear role.

## Core Roles

| File | Role |
|------|------|
| [`project.md`](project.md) | Current dashboard: shipped status, current focus, recent changes, and links to the owning docs. |
| [`roadmap.md`](roadmap.md) | Active tracked work, stable IDs, validated bugs, and later priorities. |
| [`code_audit.md`](code_audit.md) | Cross-system source audit coverage, confirmed findings, and verification evidence (`AUDIT-01`). |
| [`reference.md`](reference.md) | Stable lookup tables, data formats, memory budgets, and vocabulary. |
| [`simulation.md`](simulation.md) | Simulation engine contract: game clock, scheduler, entity handles, timed events, snapshot rules, sim-frame budget and simulation tiers (`SIM-02`). |
| [`entrance_and_exit.md`](entrance_and_exit.md) | Building entrance/exit and trip-planning spec. |
| [`traffic.md`](traffic.md) | Lane-bound vehicle movement, car following, junction traversal, lane changes, overtaking, and traffic debug. |
| [`economy.md`](economy.md) | Current economy specification: money and fiscal policy, households, funding census, labor, production, utilities, freight, bankruptcy, and validation recipes. |
| [`demand.md`](demand.md) | Current growth specification: household admission and removal, private construction, level changes, failed-site clearance, and growth diagnostics. |
| [`zoning.md`](zoning.md) | Zoning system spec. |
| [`building_allocator.md`](building_allocator.md) | Building placement, removal, frontage attachment, and allocator ownership boundaries. |
| [`buildings.md`](buildings.md) | Reserved for a future building type catalog. See `economy.md` and `demand.md` for current building specs. |
| [`asset_editor.md`](asset_editor.md) | Asset-editor workflow and content contract. |
| [`roads.md`](roads.md) | Live road surface / roadbed runtime contract. |
| [`earthworks.md`](earthworks.md) | Shared engineered-ground / cut-fill / local terrain-override contract for roads and terrain pads; building construction lifecycle is owned by `economy.md` / `demand.md`. |
| [`terrain.md`](terrain.md) | Terrain source ingest, chunked terrain runtime, and large-world terrain ownership. |
| [`ui.md`](ui.md) | UI paradigm, surface ownership, style conventions, and migration plan. |
| [`debug.md`](debug.md) | Debug channels, log format, shared log, events, the `MetrumDebug` bridge, the in-game debug console, and `run.sh` debug flags (`DEBUG-01`). |

## Working Rules

- Do not use positional backlog references like `item 30` or `bug B14` in new docs.
- Use stable IDs from [`roadmap.md`](roadmap.md) for active work.
- Keep detailed subsystem behavior in the owning spec, not in [`project.md`](project.md).
- Put retired plans or superseded ledgers in [`archive/`](archive/) instead of leaving them half-live.
- Do not cite local-only paths: `/tmp`, `rust/target/` artifact folders or personal home directories. Record lasting evidence in the owning doc itself. In commands, `$ARTIFACTS` is a caller-chosen local output directory, `$TEST_BINARY` the release library test executable printed by `cargo test --release --lib --no-run`, `$BENCH_BINARY` the benchmark executable printed by `cargo bench --no-run`, and `$HOME` the user's home.

## Archive

- [`archive/project_legacy_2026-04-09.md`](archive/project_legacy_2026-04-09.md) preserves the old monolithic project ledger and numbered backlog for historical reference only. It is not the live planning source anymore.
- [`archive/roads_hardcut_history_2026-05-31.md`](archive/roads_hardcut_history_2026-05-31.md) preserves the old full roadbed hardcut spec for historical reference only. [`roads.md`](roads.md) is the live road contract.
- [`archive/economy_history_2026-10-10.md`](archive/economy_history_2026-10-10.md) and [`archive/demand_history_2026-10-10.md`](archive/demand_history_2026-10-10.md) preserve the economy and demand documents up to 2026-10-10, including the `ECON-14`/`DEM-02` diagnoses, implementation logs and dated measurements. [`economy.md`](economy.md) and [`demand.md`](demand.md) are the live specifications.
