# Metrum Rise — Roadmap

Active tracked work uses stable IDs instead of positional numbering. This file is the live planning surface for priorities, validated bugs, and later tracks.

Status values:
- `open`
- `in_progress`
- `needs_revalidation`
- `done`
- `parked`

Priority values:
- `P0`
- `P1`
- `P2`

## Active Priorities

| ID         | Status        | Priority | Owner doc                                        | Summary                                                                                                                                                                                                             |
| ------------| ---------------| ----------| --------------------------------------------------| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `CIV-01`   | `open`        | `P1`     | [`economy.md`](economy.md)                       | Add the first service-building / coverage slice so city stability is no longer only conceptual.                                                                                                                     |
| `DEM-01`   | `open`        | `P1`     | [`demand.md`](demand.md)                         | Move immigration admission and displacement ownership fully behind demand-layer outputs instead of allocator/transport leftovers.                                                                                   |
| `ALLOC-01` | `open`        | `P1`     | [`building_allocator.md`](building_allocator.md) | Harden the building allocator against the known spec and ownership limitations documented in [`building_allocator.md`](building_allocator.md) before it becomes the long-term demand-driven growth execution layer. |
| `DOC-01`   | `in_progress` | `P1`     | [`README.md`](README.md)                         | Finish the docs restructure: keep `project.md` dashboard-sized, keep roadmap IDs stable, and retire remaining numbered references in live docs.                                                                     |

## Parked / Watchlist

| ID | Status | Priority | Owner doc | Summary |
|----|--------|----------|-----------|---------|
| `QA-01` | `parked` | `P2` | [`project.md`](project.md) | Old long-run simulation-thread panic. It has not reproduced recently, including at least one overnight run, so it is no longer an active blocker. Promote it back only if it reappears with logs or a reliable repro. |

## Large-World Track

| ID | Status | Priority | Owner doc | Summary |
|----|--------|----------|-----------|---------|
| `WORLD-01` | `open` | `P2` | [`project.md`](project.md) | Distant-region aggregate simulation for world regions outside the active area of interest. |
| `WORLD-02` | `open` | `P2` | [`project.md`](project.md) | Promotion and demotion rules between full-FSM, flow-field, and aggregate tiers. |
| `WORLD-03` | `open` | `P2` | [`project.md`](project.md) | World overview view and aggregate flow inspection for inactive regions. |

## Transport Expansion Track

| ID           | Status | Priority | Owner doc                  | Summary                                                              |
| --------------| --------| ----------| ----------------------------| ----------------------------------------------------------------------|
| `TRANSIT-01` | `open` | `P2`     | [`project.md`](project.md) | Bus support on top of a shared vehicle / waiting-state foundation.   |
| `TRANSIT-02` | `open` | `P2`     | [`project.md`](project.md) | Rail / metro support on isolated `RAIL` routing.                     |
| `TRANSIT-03` | `open` | `P2`     | [`project.md`](project.md) | Ship / ferry support through harbor-linked water routes.             |
| `TRANSIT-04` | `open` | `P2`     | [`project.md`](project.md) | Air / border travel support through airport and border-node routing. |
| `TRANSIT-05` | `open` | `P1`     | [`project.md`](project.md) | Add bicycle support as the next real transport mode on top of the existing multi-modal foundation. |

## Tools And Content

| ID | Status | Priority | Owner doc | Summary |
|----|--------|----------|-----------|---------|
| `TOOLS-01` | `open` | `P2` | [`asset_editor.md`](asset_editor.md) | Add an in-game pack-manager UI for installed content packs. |

## Notes

- The old numbered backlog and bug table are archived in [`archive/project_legacy_2026-04-09.md`](archive/project_legacy_2026-04-09.md).
- New work should never introduce fresh `item N` references. Use stable IDs instead.
- The Criterion microbenchmark suite was expanded to cover `ACCESS_EGRESS` and `ACCESS_INGRESS`. Benchmark deltas against older saved results may therefore reflect a changed suite shape, not only a runtime regression.
