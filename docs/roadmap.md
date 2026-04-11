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

Kind values:
- `feature`
- `bug`
- `hardening`
- `refactor`
- `docs`

## Active Priorities

| ID         | Kind        | Status        | Priority | Owner doc                                        | Problem                                                                                                                   | Exit criteria                                                                                                      |
| ------------| -------------| ---------------| ----------| --------------------------------------------------| ---------------------------------------------------------------------------------------------------------------------------| --------------------------------------------------------------------------------------------------------------------|
| `CIV-01`   | `feature`   | `open`        | `P1`     | [`economy.md`](economy.md)                       | City stability still lacks the first real service-building and coverage slice.                                            | At least one service-building class affects live city stability through a documented and verified coverage path.   |
| `DEM-01`   | `hardening` | `open`        | `P1`     | [`demand.md`](demand.md)                         | Immigration admission and displacement ownership still leaks through allocator and transport leftovers.                   | Demand-layer outputs fully own immigration and displacement decisions with old cross-system leftovers removed.     |
| `ALLOC-01` | `hardening` | `open`        | `P1`     | [`building_allocator.md`](building_allocator.md) | The building allocator still has known spec and ownership limitations before it can become the long-term growth executor. | Documented allocator limitations are either removed or explicitly bounded in the owning spec and code.             |
| `DOC-01`   | `docs`      | `in_progress` | `P1`     | [`README.md`](README.md)                         | The docs restructure is not finished and some live references still point back to old numbered backlog habits.            | `project.md` stays dashboard-sized, roadmap IDs remain stable, and remaining numbered live references are retired. |

## Code Quality / Technical Debt

| ID        | Kind        | Status | Priority | Owner doc                                      | Problem                                                                                                                                                                | Exit criteria                                                                                                                                           |
| -----------| -------------| --------| ----------| ------------------------------------------------| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------| ---------------------------------------------------------------------------------------------------------------------------------------------------------|
| `CODE-01` | `refactor`  | `open` | `P1`     | [`entrance_and_exit.md`](entrance_and_exit.md) | `rust/src/simulation/economy/agents/tick.rs` mixes trip planning, local-access geometry, unsafe SoA movement, congestion, and cache maintenance in one hotspot module. | Agent trip planning, movement, and congestion/cache responsibilities live in smaller focused modules with current behavior and test coverage preserved. |
| `CODE-02` | `refactor`  | `open` | `P2`     | [`asset_editor.md`](asset_editor.md)           | `godot/scripts/asset_editor.gd` combines editor shell UI, economy-catalog loading, import/export flow, and viewport interaction in one oversized bridge script.        | The asset editor shell is split into smaller focused bridges/helpers without changing the current authoring workflow or content contract.               |
| `CODE-03` | `refactor`  | `open` | `P2`     | [`economy.md`](economy.md)                     | `rust/src/simulation/economy/definitions.rs` combines authored-economy schema, validation, sandbox simulation, and export/index IO in one module.                      | Authored economy schema, validation, sandbox execution, and export/index generation are separated into focused modules with the same external behavior. |

## Validated Bugs

| ID      | Kind  | Status   | Priority | Owner doc                  | Problem                                                                                                                                     | Exit criteria                                                                                                               |
| ---------| -------| ----------| ----------| ----------------------------| ---------------------------------------------------------------------------------------------------------------------------------------------| -----------------------------------------------------------------------------------------------------------------------------|
| `QA-01` | `bug` | `parked` | `P2`     | [`project.md`](project.md) | There are historical reports of a long-run simulation-thread panic, but it has not reproduced recently enough to keep as an active blocker. | A reliable repro plus logs promotes this back to active work, or a sustained revalidation window closes it with confidence. |

### `QA-01`

- Evidence: historical long-run panic reports exist, but at least one overnight run completed without reproducing the issue.
- Revalidation trigger: new logs, a reproducible save, or a fresh report from current code.

## Large-World Track

| ID | Kind | Status | Priority | Owner doc | Problem | Exit criteria |
|----|------|--------|----------|-----------|---------|---------------|
| `WORLD-01` | `feature` | `open` | `P2` | [`project.md`](project.md) | Regions outside the active area of interest still lack aggregate simulation. | Inactive regions run at aggregate fidelity with documented behavior and integration points. |
| `WORLD-02` | `feature` | `open` | `P2` | [`project.md`](project.md) | Promotion and demotion rules between full-FSM, flow-field, and aggregate tiers are not defined in live code. | Tier transitions are implemented and documented with deterministic promotion/demotion rules. |
| `WORLD-03` | `feature` | `open` | `P2` | [`project.md`](project.md) | There is no world overview for inspecting inactive-region aggregate state. | The world overview can inspect aggregate inactive-region state and flows at a useful gameplay level. |

## Transport Expansion Track

| ID | Kind | Status | Priority | Owner doc | Problem | Exit criteria |
|----|------|--------|----------|-----------|---------|---------------|
| `TRANSIT-01` | `feature` | `open` | `P2` | [`project.md`](project.md) | Bus support is not yet layered onto the shared vehicle and waiting-state foundation. | Bus routing, waiting, and vehicle flow run on the shared transport foundation in live gameplay. |
| `TRANSIT-02` | `feature` | `open` | `P2` | [`project.md`](project.md) | Rail and metro support are still missing from isolated `RAIL` routing. | Rail routing and the first rail gameplay loop are implemented on isolated `RAIL` paths. |
| `TRANSIT-03` | `feature` | `open` | `P2` | [`project.md`](project.md) | Harbor-linked ship and ferry transport does not exist yet. | Water transport is supported through harbor-linked water routes with live routing behavior. |
| `TRANSIT-04` | `feature` | `open` | `P2` | [`project.md`](project.md) | Air and border travel support are still absent. | Airport and border-node travel paths are implemented with live routing support. |
| `TRANSIT-05` | `feature` | `open` | `P1` | [`project.md`](project.md) | Bicycle support is still the next missing real transport mode on top of the multi-modal foundation. | Bicycles are supported as a live transport mode with routing, movement, and gameplay-facing behavior. |
| `TRANSIT-06` | `feature` | `open` | `P1` | [`project.md`](project.md) | Pedestrian and bicycle movement still lacks bounded local steering, and future high-density station crowds need a dedicated waiting model. | Active pedestrians and bicycles use deterministic corridor-local steering, while dense passenger waiting uses a separate slot/queue model instead of naive all-to-all crowd steering. |

- Steering reference for `TRANSIT-06`: Craig W. Reynolds, [*Steering Behaviors For Autonomous Characters*](https://www.red3d.com/cwr/steer/gdc99/). Treat it as a local-movement reference rather than a replacement for exact route planning or dense waiting-state simulation.

## Tools And Content

| ID | Kind | Status | Priority | Owner doc | Problem | Exit criteria |
|----|------|--------|----------|-----------|---------|---------------|
| `TOOLS-01` | `feature` | `open` | `P2` | [`asset_editor.md`](asset_editor.md) | Installed content packs still lack an in-game pack-manager UI. | Players can view and manage installed content packs through an in-game pack-manager UI. |

## Notes

- The old numbered backlog and bug table are archived in [`archive/project_legacy_2026-04-09.md`](archive/project_legacy_2026-04-09.md).
- New work should never introduce fresh `item N` references. Use stable IDs instead.
- Keep roadmap rows short: describe the current problem and a visible exit condition, then put subsystem detail in the owner doc.
- The Criterion microbenchmark suite was expanded to cover `ACCESS_EGRESS` and `ACCESS_INGRESS`. Benchmark deltas against older saved results may therefore reflect a changed suite shape, not only a runtime regression.
