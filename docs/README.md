# Metrum Rise — Docs Guide

This directory stays intentionally flat for now. Each file should have one clear role.

New here? Start with [`README2-ONBOARDING.md`](README2-ONBOARDING.md).

## Core Roles

| File | Role |
|------|------|
| [`project.md`](project.md) | Current dashboard: shipped status, current focus, recent changes, and links to the owning docs. |
| [`roadmap.md`](roadmap.md) | Active tracked work, stable IDs, validated bugs, and later priorities. |
| [`reference.md`](reference.md) | Stable lookup tables, data formats, memory budgets, and vocabulary. |
| [`entrance_and_exit.md`](entrance_and_exit.md) | Building entrance/exit and trip-planning spec. |
| [`traffic.md`](traffic.md) | Lane-bound vehicle movement, car following, junction traversal, lane changes, overtaking, and traffic debug. |
| [`economy.md`](economy.md) | Economy and freight design/spec. |
| [`demand.md`](demand.md) | Demand, growth pressure, and household admission/removal ownership. |
| [`zoning.md`](zoning.md) | Zoning system spec. |
| [`building_allocator.md`](building_allocator.md) | Building placement, removal, frontage attachment, and allocator ownership boundaries. |
| [`buildings.md`](buildings.md) | Reserved for a future building type catalog. See `economy.md` and `demand.md` for current building specs. |
| [`asset_editor.md`](asset_editor.md) | Asset-editor workflow and content contract. |
| [`roads.md`](roads.md) | Live road surface / roadbed runtime contract. |
| [`earthworks.md`](earthworks.md) | Shared engineered-ground / cut-fill / local terrain-override contract for roads and terrain pads; building construction lifecycle is owned by `economy.md` / `demand.md`. |
| [`terrain.md`](terrain.md) | Terrain source ingest, chunked terrain runtime, large-world terrain ownership, world generation, and mineral deposits. |
| [`ui.md`](ui.md) | UI paradigm, surface ownership, style conventions, and migration plan. |
| [`narrative.md`](narrative.md) | Setting, the arc from first town to space launch, the political layer, tone, and the events that carry them. |
| [`region.md`](region.md) | The regional tier: tiles, founding and the blessing, expansion, the density law, settlements that grow themselves, incorporation, and national parks. |
| [`services.md`](services.md) | Emergency response, health, education, civic amenity, the utility networks, crime and policing, and border patrol. Coverage and response; `economy.md` owns their money. |
| [`transit.md`](transit.md) | Public transport modes, why tram/subway/train are one rail network, and freight that travels visibly. Nothing built yet. |
| [`simulation_layers.md`](simulation_layers.md) | The physical and living layers being ported: the shared wind/water/fire field, minerals, flora, fauna, agent minds, disasters, and how 20M agents stay affordable. |

## What This Contribution Changed

Every document below was written or edited as part of the design and lane work
described in the root [`readme.md`](../readme.md). Lines added by that work carry
a `` or `[C]` marker until they have been reviewed; the markers are a
review aid and are removed once a section is accepted.

### New documents

| File | Why it exists |
|------|---------------|
| [`narrative.md`](narrative.md) | The technical docs said nothing about theme, setting, story, or tone. A search for any of those returned no design statement. |
| [`region.md`](region.md) | The regional tier had no home: founding, expansion, the density law, incorporation, and national parks were homeless. |
| [`services.md`](services.md) | Services were one line naming `always_on_service`. Nothing owned how a service reaches the people it serves. |
| [`transit.md`](transit.md) | The transit modes existed only in design notes. Records that tram, subway, and train are one rail network before three systems get built. |
| [`simulation_layers.md`](simulation_layers.md) | The physical and living layers being ported from an external engine, and the ordering: the shared field first, because everything rides on it. |

### Documents extended

| File | What was added |
|------|----------------|
| [`roads.md`](roads.md) | The lane model: an edge is an ordered cross-section rather than two counts. Intersection control at mod-level granularity, road roles, and three forbidden regressions naming the formulas that must not return. |
| [`zoning.md`](zoning.md) | Alleys entire, build granularity including subletting, gridless parcels, districts, water frontage, and parking supply. |
| [`terrain.md`](terrain.md) | World generation, where the checklist seeds the generator rather than auditing it, and mineral deposits derived from how the rock formed. |
| [`economy.md`](economy.md) | The industry scaffold, the tycoon layers, the two money pools, and foreign capital. |
| [`traffic.md`](traffic.md) | Routing feedback: congestion has to reach the router, or the traffic problem does not exist inside the simulation. |
| [`ui.md`](ui.md) | The road builder as a cross-section editor, post-placement editing, and the tilt-shift camera. |
| [`building_allocator.md`](building_allocator.md) | Frontage roles. The code existed since the alley work and the document said nothing about it. |
| [`asset_editor.md`](asset_editor.md) | The editor as a first class system, saving any building to a library, plop dials, and interiors generated once. |
| [`project.md`](project.md) | The scale target. |
| [`entrance_and_exit.md`](entrance_and_exit.md) | A note on what the agent memory budget actually bounds, since the arithmetic is only correct for the active set. |
| [`README.md`](README.md) | This file: the new documents above, and this section. |

### Outside `docs/`

| File | What changed |
|------|--------------|
| [`readme.md`](../readme.md) | Where the 2.5D engine came from and what has been contributed here. |
| [`CLAUDE.md`](../CLAUDE.md) | The scale target, in both the overview and the performance section, which disagreed with each other. `AGENTS.md` is a symlink to it. |
| `UPSTREAM_ISSUE_crash_rs.md` | A report that `rust/src/debug/crash.rs` was missing and the crate did not build. Upstream supplied the real file in `87059cb`, so this is now answered. |

Three documents were touched only by the upstream merge and carry no changes
from this work: `demand.md`, `reference.md`, and `roadmap.md`.

## Working Rules

- Do not use positional backlog references like `item 30` or `bug B14` in new docs.
- Use stable IDs from [`roadmap.md`](roadmap.md) for active work.
- Keep detailed subsystem behavior in the owning spec, not in [`project.md`](project.md).
- Put retired plans or superseded ledgers in [`archive/`](archive/) instead of leaving them half-live.

## Archive

- [`archive/project_legacy_2026-04-09.md`](archive/project_legacy_2026-04-09.md) preserves the old monolithic project ledger and numbered backlog for historical reference only. It is not the live planning source anymore.
- [`archive/roads_hardcut_history_2026-05-31.md`](archive/roads_hardcut_history_2026-05-31.md) preserves the old full roadbed hardcut spec for historical reference only. [`roads.md`](roads.md) is the live road contract.
