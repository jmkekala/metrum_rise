# Metrum Rise — Project Dashboard

This file is the live dashboard for current state, priorities, and links to the owning docs. It is intentionally summary-first.

The old monolithic ledger and numbered backlog are archived in [`archive/project_legacy_2026-04-09.md`](archive/project_legacy_2026-04-09.md). That archive is historical reference only, not the current planning source.

## Snapshot

- **Scale target**: at least 1,000,000 total population across simulation tiers in one large world.
- **Simulation model**: full-FSM simulation stays inside the active area of interest; distant world regions are expected to degrade to coarser flow-field or aggregate simulation.
- **Current focus**: keep the playable small-to-medium city slice correct, deterministic, and scalable while the docs and planning model are cleaned up.

## Shipped Foundations

- **Road network and routing**: modular `RegionGraph`, lane system, CCH pathfinding, road rendering, border nodes, and roadway editing are all live.
- **Zoning and buildings**: world-space zoning grid, occupancy tracking, building placement, vacancy indexing, and no-build edge flags are live.
- **Entrance-aware movement**: the building entrance/exit rewrite is implemented through the exact-plan system described in [`entrance_and_exit.md`](entrance_and_exit.md), including the Phase 1–6 and Phase 8 slices already verified against the live code.
- **Benchmark coverage**: the Criterion suite now measures the live access phases through `ACCESS_EGRESS` and `ACCESS_INGRESS` in addition to pure `NETWORK` and idle scaling. Treat comparisons against older benchmark runs as a fresh baseline unless the benchmark shape is identical.
- **Economy foundation**: household records, building-centric daily economy, freight jobs, `OWA` fallback, and exact entrance-side freight ETA are live. See [`economy.md`](economy.md).
- **Demand telemetry**: demand is rebuilt from live pressure rather than blindly spawning private buildings, and now owns immigration/displacement cleanup direction. See [`demand.md`](demand.md).
- **Persistence and runtime**: SQLite save/load, background simulation thread, render snapshots, debug flags, asset editor, and economy editor are live.

## Current Priorities

For active tracked work, use [`roadmap.md`](roadmap.md).

- `QA-01`: revalidate and root-cause the old long-run sim-thread panic.
- `CIV-01`: add service-building coverage so city stability is not only conceptual.
- `MOB-01`: ship bicycle support as the next transport mode.
- `DEM-01`: finish moving immigration and displacement ownership behind the demand layer.
- `DOC-01`: finish replacing old numbered backlog references in live docs.

`QA-01` is now parked in [`roadmap.md`](roadmap.md): the old long-run sim-thread panic has not reproduced recently, including at least one overnight run, so it is no longer treated as an active blocker.

## System Ownership

| Area | Owning doc |
|------|------------|
| Current status / priorities | [`project.md`](project.md), [`roadmap.md`](roadmap.md) |
| Stable constants / bridge API / formats | [`reference.md`](reference.md) |
| Entrance / exit / trip attachment | [`entrance_and_exit.md`](entrance_and_exit.md) |
| Economy / freight / household runtime | [`economy.md`](economy.md) |
| Demand / immigration pressure / displacement ownership | [`demand.md`](demand.md) |
| Zoning | [`zoning.md`](zoning.md) |
| Asset-editor workflow and pack contract | [`asset_editor.md`](asset_editor.md) |
| Road-renderer notes | [`improved_roads.md`](improved_roads.md) |

## Recent Structural Changes

- `project.md` was reduced from a monolithic implementation ledger into this dashboard.
- `roadmap.md` now owns active tracked work through stable IDs instead of positional numbering.
- `README.md` now serves as the docs index and ownership map.
- The legacy numbered backlog and bug table were preserved in the archive rather than kept half-live in the dashboard.
- `rust/benches/agent_benchmark.rs` now includes access-phase microbenchmarks for `ACCESS_EGRESS` and `ACCESS_INGRESS`, so old Criterion result history is no longer strictly apples-to-apples with the updated suite.

## Reference

- Stable technical lookup data: [`reference.md`](reference.md)
- Live work tracker: [`roadmap.md`](roadmap.md)
- Historical numbered ledger: [`archive/project_legacy_2026-04-09.md`](archive/project_legacy_2026-04-09.md)
