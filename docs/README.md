# Metrum Rise — Docs Guide

This directory stays intentionally flat for now. Each file should have one clear role.

## Core Roles

| File | Role |
|------|------|
| [`project.md`](project.md) | Current dashboard: shipped status, current focus, recent changes, and links to the owning docs. |
| [`roadmap.md`](roadmap.md) | Active tracked work, stable IDs, validated bugs, and later priorities. |
| [`reference.md`](reference.md) | Stable lookup tables, bridge API inventory, data formats, and vocabulary. |
| [`entrance_and_exit.md`](entrance_and_exit.md) | Building entrance/exit and trip-planning spec. |
| [`economy.md`](economy.md) | Economy and freight design/spec. |
| [`demand.md`](demand.md) | Demand, growth pressure, and household admission/removal ownership. |
| [`zoning.md`](zoning.md) | Zoning system spec. |
| [`building_allocator.md`](building_allocator.md) | Building placement, removal, frontage attachment, and allocator ownership boundaries. |
| [`buildings.md`](buildings.md) | Reserved for a future building type catalog. See `economy.md` and `demand.md` for current building specs. |
| [`asset_editor.md`](asset_editor.md) | Asset-editor workflow and content contract. |
| [`improved_roads.md`](improved_roads.md) | Road-renderer architecture notes. |
| [`ui.md`](ui.md) | UI paradigm, surface ownership, style conventions, and migration plan. |

## Working Rules

- Do not use positional backlog references like `item 30` or `bug B14` in new docs.
- Use stable IDs from [`roadmap.md`](roadmap.md) for active work.
- Keep detailed subsystem behavior in the owning spec, not in [`project.md`](project.md).
- Put retired plans or superseded ledgers in [`archive/`](archive/) instead of leaving them half-live.

## Archive

- [`archive/project_legacy_2026-04-09.md`](archive/project_legacy_2026-04-09.md) preserves the old monolithic project ledger and numbered backlog for historical reference only. It is not the live planning source anymore.
