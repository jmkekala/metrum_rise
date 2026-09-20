# Asset Editor / Importer Design Spec

## Purpose

Metrum Rise needs a dedicated asset-authoring tool for first-party content and modders. The tool lets creators import and validate game-ready assets without starting a live city simulation. It remains consistent with the performance constraints of the main game: the importer can do expensive offline work, but the shipped runtime assets must be cheap to load and render.

The editor is not a replacement for Blender or other DCC tools. It is a constrained packaging, validation, preview, and metadata-authoring tool that turns external art assets into Metrum Rise content packs.

## Document Conventions

This document is standalone. It is written to be usable by another engineer or another AI without prior conversational context.

Interpretation rules:

- Sections outside `Later / Ecosystem Extensions` are the active v1 specification unless a section explicitly says otherwise.
- `Current repository state` means descriptive information about the present codebase, not a v1 requirement.
- `Later / Ecosystem Extensions` means non-blocking future design material that does not gate the first implementation pass.
- `must` means required for the v1 contract.
- `may` means allowed but optional.

Terminology note:

- `lot_width_cells` and `lot_depth_cells` are authored building footprint dimensions in zoning cells; they do not imply a cadastral parcel system.
- `plot` in this document means an editor preview footprint or buildable rectangle.
- `category` in this document means asset-catalog grouping unless a section explicitly talks about `zone_type`.

## V1 Design Constraints

The first implementation must stay narrow. The asset editor is a packaging, validation, preview, and metadata-authoring tool, not a general-purpose content pipeline for every asset type or every possible runtime behavior.

Core v1 constraints:

- The editor must be a separate tool. Modders need a stable preview sandbox, not a live city with traffic, demand, and simulation noise.
- The preview sandbox uses a `500 m x 500 m` map. That size is sufficient for scale validation, lighting checks, prop placement, and vehicle/character reference scenes.
- Asset metadata must be authored outside raw model files. Building footprint size, jobs, asset category, pack membership, author, and licensing are manifest data, not mesh data.
- Pack enable-disable support is required. The content system must support curation at the pack level from the beginning.

V1 non-goals and hard limits:

- Asset reference spaces are class-specific. Cells are appropriate for zoned buildings and some parks, but not for vehicles or characters. Vehicles need lane-width and turning-radius references; characters need sidewalk, doorway, and camera-distance references.
- Roads are out of scope for the first importer. Road assets are network templates with lane, sidewalk, marking, junction, and topology metadata, not ordinary imported meshes.
- Runtime skeletal crowd animation is out of scope. The editor may ingest rigged source assets, but shipped character runtime content must be static, billboarded, or VAT-baked.
- `3 x 3` is only a preset, not a design limit. Building assets must declare explicit `lot_width_cells` and `lot_depth_cells`.
- The fixed `ZONING_DEPTH` storage model is not acceptable for the long-term asset pipeline. It must be replaced by dynamic zoning extents so building size is limited by painted zoning and map space rather than a compile-time constant.

## Product Shape

Build a separate `asset_editor` executable or launch mode inside the same Godot + Rust project family.

Rationale:

- Godot already gives scene importing, material preview, cameras, lighting, gizmos, thumbnails, and export workflows.
- The runtime and the editor can share the same asset schema and validation rules.
- This keeps the content pipeline consistent with the main game instead of creating a second rendering stack.

Runtime shape:

- Launch into a stripped-down sandbox scene.
- No demographics, no active `AgentSystem`, no demand simulation, no immigration.
- Small world sandbox, for example `WorldConfig::editor_sandbox()`.
- A few preview templates:
  - Flat studio scene
  - Zoned roadside scene
  - Sidewalk + lane scene
  - Day/night lighting scene

The editor follows an in-engine preview model while still keeping explicit metadata files instead of hiding footprint metadata and asset data inside scene files.

## Scope Split

V1 scope includes four authoring targets:

1. Zoned buildings
2. Props / environment details
3. Vehicles
4. Character source assets for offline baking

V1 excludes:

- Full road authoring
- Junction authoring
- Rail/ship/air infrastructure authoring
- Runtime skeletal crowd animation
- Arbitrary simulation scripting inside assets

## Canonical Asset Format

Use `glTF 2.0`, preferably binary `.glb`, as the canonical 3D asset format.

Rationale:

- Godot recommends `glTF 2.0` for 3D scenes.
- It preserves meshes, materials, UVs, skeletons, and animation clips better than older interchange formats.
- It is more pipeline-friendly than OBJ and less awkward than keeping FBX as the canonical shipped format.

Import policy:

- First-class supported source: `.glb`
- Acceptable source for authoring convenience: `.fbx`, but convert it to `.glb` during import or baking
- Do not make raw `.fbx` the canonical packaged format

Texture inputs:

- Required: base color / albedo
- Optional but recommended: normal
- Optional: ORM packed texture or separate roughness / metallic / AO
- Optional: emission
- Optional: opacity mask

Packaged runtime outputs:

- `.glb` for static meshes
- `.png` or engine-imported textures for standard materials
- `.exr` only where VAT baking needs float texture data

## Runtime Ownership

Meshes and textures do not "live in Rust".

Responsibility split:

- Rust owns asset manifests, metadata validation, gameplay categories, footprint rules, pack enable-disable state, and any simulation-facing IDs.
- Godot runtime owns mesh loading, texture loading, material setup, thumbnails, and actual rendering.

Rationale:

- Rust is the simulation/backend layer, not the rendering asset system.
- Godot already has the runtime APIs for loading and instantiating meshes, textures, and scenes.
- Keeping binary art assets in Rust would make modding harder, not easier.

Architecture split:

- Rust reads `pack.toml` and `asset.toml`.
- Godot reads referenced `.glb`, `.png`, `.exr`, and related runtime assets.
- The renderer requests only the metadata and asset IDs it needs from Rust.

## Modding And Deployment

For a moddable shipped game, custom assets do not require import into the Godot project under `res://`.

Current repository state:

- Buildings load raw model files from the enabled packs in `user://mods/`. Explicit pack reload
  replaces both the Rust registry and normal/deserted rendering meshes, including changed files
  under an existing asset ID and removed parts. Disabling every pack clears both owners.
- The asset editor scans all installed packs once per browser refresh, independently of the
  gameplay selection. Gameplay passes an explicit pack-ID array; the authoring API requests
  all packs through the same registry loader.
- Cars load their bundled `.glb` mesh and texture sources at runtime so fresh project-cache
  launches do not depend on `godot/.godot/imported` files; VAT pedestrians already use runtime
  VAT assets, still from project-bundled paths.

Building polling (`AUDIT-01-F5`) uses one refresh per 30 process frames in both windowed and
headless execution. Asset-instance setup owns the aligned packed requests; periodic refresh
reuses them. The all-city Rust transform scans remain a separate audit item.

Matched headless release-extension measurements use
`godot --headless --path godot --script res://tests/building_asset_reload_test.gd -- --benchmark-building-poll`,
CPU affinity 0, `RAYON_NUM_THREADS=24`, `METRUM_DEBUG=0`, and five alternating process pairs.
An empty world with one generated asset containing 2 / 128 / 1,024 mesh parts measures completed
request/bridge/upload calls, excluding busy replies, fixture import and node setup. Medians change
from **12.05 / 272.85 / 2,498.85 µs** to **9.90 / 183.85 / 1,598.35 µs**. These are boundary costs,
not populated-city rendering acceptance. Raw logs, matched results and exact source/extension
identities are under `/tmp/metrum-full-audit/building-poll-*`.

Shipped design:

- Built-in first-party content can remain bundled with the game.
- V1 canonical mod install location: `user://mods/`
- Release bootstrap content may be bundled under `res://bootstrap/mods/`; startup copies each
  missing top-level pack into `user://mods/` and never merges into an existing user pack.
- Fresh profiles seed `user://active_packs.cfg` with the bundled `kenney` pack enabled. Once the
  player saves pack selection, even an empty enabled list is preserved as intentional.

```text
user://mods/
```

- Each mod pack contains raw packaged assets plus human-authored manifest files.
- The shipped game scans enabled packs at startup and loads them at runtime.
- No Godot Editor import database is required for user mod packs.

Distinction:

- Godot runtime is required, because the game renders through Godot.
- Godot Editor is not required for mod users.

## Toolchain Requirements

The shipped game and shipped asset editor do not require source code, the Rust toolchain, or a separate Godot installation.

Distribution model:

- `metrum_rise_game` executable
- `metrum_rise_asset_editor` executable
- bundled precompiled Rust dynamic library
- bundled Godot runtime

Distribution consequences:

- Players do not need Rust installed.
- Players do not need the Godot Editor installed.
- Players do not need the game source code.

The asset editor is a normal shipped application, not a project that the user opens manually in Godot.

## Pack Distribution And Installation

Exported asset packs must be easy to share as ordinary files.

Distribution model:

- Canonical installed form: unpacked folder in `user://mods/`
- Common share form: `.zip` archive containing exactly one pack root folder
- Optional nicer share extension later: something like `.mrpack.zip`, but plain `.zip` should work from day one

Editor outputs:

- `Export Runtime Pack`: writes the normal unpacked runtime pack folder
- `Export Share Archive`: writes a zip archive of the runtime pack for distribution
- Default export flow: export the runtime pack folder first, then optionally generate the share archive from that exact folder

Do not make zip the only exported artifact. The unpacked runtime pack should remain the canonical installable form.

Player workflow:

1. Creator exports a runtime pack folder from the asset editor.
2. Creator shares that pack as:
   - the folder itself, or
   - a `.zip` made from that folder
3. Another player installs it by:
   - dropping the folder into `user://mods/`, or
   - importing/selecting the `.zip` in the game or asset editor, which then unpacks it into `user://mods/`
4. The game validates `pack.toml`, asset manifests, `checksums.sha256`, and `pack.index.bin` when present.
5. The player enables or disables the pack from the content/mod manager.

Runtime rule:

- Do not make the game stream mods directly from random zip files as the primary path.
- Use zip as a transport/distribution format, then unpack to the normal mod folder structure.

Rationale:

- An unpacked folder is easier to inspect, debug, patch, and hand-edit.
- It keeps the mod format open instead of hiding content behind a container.
- Checksum verification, cache generation, and asset-path resolution are simpler and more robust on normal extracted files.

Install location:

- canonical v1 install path: `user://mods/`

Conflict rules:

- Each installed pack is keyed by `pack_id`.
- Installing a new version of the same `pack_id` triggers an update/replace prompt instead of creating duplicate parallel installs accidentally.
- The content manager shows:
  - pack name
  - version
  - author
  - enabled/disabled state
  - validation warnings

Export contents:

- `pack.toml`
- `pack.index.bin`
- `assets/...`
- thumbnails and any baked outputs required by the assets

## Integrity, Corruption, And Authenticity

The export pipeline supports pack hashing by default.

Pack hashing is useful distribution hardening, but it is not a blocker for starting the first editor/importer implementation. It is an early follow-up layer after the core v1 manifest, scanner, and preview flow exist.

But it is important to separate three different goals:

1. Detect accidental corruption
2. Detect local modification after install
3. Prove authorship / prevent undetected tampering by a third party

V1 pack hashing uses SHA-256 only for integrity checks. SHA-256 is valid for goals 1 and 2 and must not be treated as proof of authorship or tamper-resistant authenticity for goal 3. Authenticity guarantees require a signature scheme and a trusted distribution path for the expected public key or signed manifest.

Hashing design:

- Use normal `.zip` as the default share archive format. It is universal, easy to handle, and good enough for the first shipping version.
- Every exported share archive must have a sibling SHA-256 sidecar file named `<archive_filename>.sha256`. Example:
  - archive: `kenney_city_pack-1.0.0.zip`
  - sidecar: `kenney_city_pack-1.0.0.zip.sha256`
- Every exported pack folder must contain a per-file checksum manifest named exactly `checksums.sha256`.

Verification flow:

- On archive import:
  - compute archive SHA-256
  - compare against a provided `.sha256` sidecar or trusted catalog entry if present
  - unpack into a temporary directory
  - verify the unpacked file set against `checksums.sha256`
  - only then move/install into the real `user://mods/` directory
- After install:
  - the game can re-check `checksums.sha256` to detect local corruption or manual edits
  - if files changed, mark the pack as `modified` instead of silently trusting the old cache

V1 status labels in the content manager:

- `Valid`
- `Modified`
- `Corrupt`
- `Missing dependencies`

First integrity milestone:

- ship zip export
- ship archive SHA-256 sidecar generation
- ship per-file `checksums.sha256`
- verify on import and on load

Later integrity milestone:

- add optional digital signatures for authorship verification and third-party tamper detection
- sign the pack or the checksum manifest with `Ed25519`
- if signatures are added later, the content manager can also show:
  - hash valid but unsigned
  - signature verified
  - signature invalid

That gives the project the best of both worlds:

- modders can zip and share packs freely
- players can install them with one action
- the actual installed format remains a transparent folder tree

## Manifest Format

The exported asset configuration is written directly into the output pack folder as visible text files.

Canonical format:

- Pack-level metadata must be stored in `pack.toml`.
- Asset-level metadata must be stored in `asset.toml`.
- TOML is the canonical manifest format for v1.
- Generated cache or index files are optional derived data.
- Cache or index files must be fully regenerable from the TOML manifests.
- The game and the asset editor must load a pack correctly even when cache or index files are missing.

## Later / Ecosystem Extensions

The following sections are useful design direction, but they are not blockers for the first implementation pass. V1 can start without them as long as the v1 manifest and scanner contract above stays stable.

### Compatibility And Versioning

Packs use explicit compatibility metadata.

Pack-level compatibility fields:

- `schema_version` is an integer.
- `content_api_version` is an integer.
- `pack.version` uses semantic versioning: `MAJOR.MINOR.PATCH`.
- `compatibility.min_game_version` uses semantic versioning.
- `compatibility.max_game_version_exclusive` is optional and uses semantic versioning.
- `compatibility.max_tested_game_version` is optional and uses semantic versioning.
- `dependencies` use explicit version bounds, not free-form range strings.

Runtime behavior:

- Reject a pack before registration when `schema_version` is incompatible.
- Reject a pack before registration when `content_api_version` is incompatible.
- Require `current_game_version >= min_game_version`.
- If `max_game_version_exclusive` exists, require `current_game_version < max_game_version_exclusive`.
- If `max_tested_game_version` exists and `current_game_version > max_tested_game_version`, show a warning and allow override behavior defined by the content manager.
- Resolve dependency compatibility against the installed dependency pack version.
- Mark the pack unresolved when a required dependency is missing or outside its declared version bounds.

Version comparison:

- Parse semantic versions before comparing them.
- Do not compare version strings lexicographically.

Canonical TOML shape:

```toml
[compatibility]
min_game_version = "0.3.0"
max_game_version_exclusive = "0.4.0"
max_tested_game_version = "0.3.7"

[[dependencies]]
pack_id = "base_civilian_materials"
min_version = "1.2.0"
max_version_exclusive = "2.0.0"
optional = false
```

Save compatibility:

- Save files use stable fully-qualified asset IDs in the form `pack_id:asset_id`.
- Later versions may add `[[asset_redirects]]` entries to `pack.toml`.
- `from` and `to` in `[[asset_redirects]]` use fully-qualified asset IDs.
- Each redirect maps one old ID to one canonical new ID.
- Multiple old IDs may redirect to the same canonical new ID.
- Redirects do not support wildcards, pattern rules, or one-to-many targets.

Canonical redirect TOML shape:

```toml
[[asset_redirects]]
from = "kenney_city_pack:building.residential.lowrise_corner"
to = "kenney_city_pack:building.residential.lowrise_corner_brick"
reason = "rename"

[[asset_redirects]]
from = "old_city_pack:vehicle.police.cruiser"
to = "community_vehicle_pack:vehicle.police.patrol_cruiser"
reason = "pack_migration"
```

Load behavior:

1. Try exact `pack_id:asset_id` lookup first.
2. If missing, consult the global redirect table built from all installed `[[asset_redirects]]` entries.
3. Follow redirects with cycle detection and a short maximum chain length such as `8`.
4. The final resolved target must exist as a real installed asset.
5. If resolution succeeds, use the canonical target and mark the reference as redirected.
6. If resolution fails or loops, keep the asset unresolved and surface a clear warning.

Cross-pack redirect rule:

- Cross-pack redirects are primarily a save-compatibility tool, not a normal catalog/spawn registration mechanism.
- If `pack_a` redirects an obsolete ID to `pack_b:asset_id`, but `pack_b` is missing or incompatible, do not silently treat that as a healthy dependency chain for new content.
- New spawn/catalog registration should expose only real currently installed assets, not obsolete redirected aliases.
- For old saves or already-placed content, unresolved cross-pack redirects should keep the simulation entry stable and render a placeholder representation with a clear warning, rather than crashing or deleting state.

Save behavior:

- After a successful redirect, the next save should write the final canonical asset ID, not the obsolete one.

Limitation:

- A true split cannot be solved automatically. If one old asset becomes several new assets, the creator must choose one canonical replacement target or leave the old ID unresolved with a warning.

### Building Site Anchors

V1 keeps the generic entrance/exit runtime intentionally simple: every building has one required
`entrance` anchor named `main` for pedestrian access and entrance-cache derivation. The editor
stores and edits that entrance in the same anchor list as the optional site-layout anchors:

- `driveway`: vehicle connector pinned to the road-facing plot edge and pointing inward into the
  lot; requires `width_m`
- `parking`: car stop/stand position inside the lot; requires `width_m` and `length_m`
- `loading_bay`: freight/service stop position inside the lot; requires `width_m` and `length_m`
- All building anchor positions must remain inside the authored lot footprint. `driveway` anchors
  must lie on the frontage edge; the editor lets them slide along that edge and derives their
  inward direction from the building frontage. For `driveway`, `parking`, and `loading_bay`, the
  whole authored footprint/rectangle must remain inside the lot; the anchor handle alone being
  inside is not enough. Driveway footprint length is derived by the editor from `width_m` in v1.
  All anchor `forward` vectors must be finite non-zero unit vectors.
- `entrance/main` may be removed from an incomplete draft; runtime export requires it. Dragging an eligible anchor moves it; right-click dragging or the
  shared yaw field rotates its `forward`, except for driveway anchors whose direction follows the
  frontage edge.

Runtime use in v1:

- `entrance/main` remains the only anchor consumed by the live entrance/exit system.
- `driveway`, `parking`, and `loading_bay` anchors are semantic site-layout metadata only. They do
  not create asphalt, concrete, paths, pads, yards, parking markings, loading markings, or other
  visuals by themselves.
- Building yard polygons are authored explicitly through `[[site_surfaces]]`. Live gameplay treats
  them as material/layout regions on the flat building-site support plane.
- Driveway anchors are the preferred runtime connection points used to choose a single flat site
  height from the road/world surface. They still do not create surfaces by themselves; authored
  `[[site_surfaces]]` polygons provide the local material/layout regions on the flat lot.
- Site surfaces do not rewrite source terrain, and they do not imply trip planning, vehicle
  parking, freight stop targeting, queueing, or capacity in v1.
- Do not add prop sockets in v1. Decorative attachment points belong to a later visual-variation
  feature, not to baseline site-layout tooling.

### Building Site Surfaces

V1 yard visuals are authored, not inferred. The asset editor is the source of truth for local
asphalt, concrete, walkways, parking pads, service pads, and driveway aprons.

Rules:

- Asset manifests and editor export payloads are strict schemas. Unknown fields are rejected
  instead of silently preserved or repaired; when the schema changes, authored assets must be
  re-exported into the current shape.
- Building assets may define zero or more `[[site_surfaces]]` polygons.
- A site surface has `material`, optional `name`, local vertical offset `y_m`, and local-space
  polygon `vertices = [[x, z], ...]` in winding order.
- Site surfaces must fit fully inside the authored lot rectangle.
- Site surface polygons must have at least three vertices, non-zero area, and no self-intersection.
- Site surfaces are authored material regions. Gameplay partitions the actual foundation/graded
  terrain triangles; it does not draw a separate horizontal polygon or apply the editor's `y_m`
  preview offset. These regions never become independent terrain-cut footprints.
- The flat terrain-ownership footprint derives from mesh-part support, entrance landings and
  authored yard regions. Yard vertices are clamped to the lot's 2 m inset before joining the support
  hull; the remaining edge strips grade to roads, neighboring pads and terrain. Structural mesh
  support is not shrunk. Driveway/parking/loading anchors alone do not expand the pad.
- Site surfaces do not imply access, parking capacity, freight capacity, service eligibility,
  pedestrian paths, or vehicle routing.
- Anchors may sit on top of site surfaces, but anchors never create surfaces by themselves.
- If an asset exports no site surfaces, the live site still has a flat support ground plane, but no
  authored asphalt/concrete material regions.
- The editor can create rectangular starting surfaces, then authors can move the whole polygon,
  drag vertices, right-click an edge to add a vertex, and right-click an existing vertex to delete it
  while preserving the minimum three-vertex polygon.
- Painted decals, curbs, markings, and per-material texture selection are later extensions.

### Flat-Site Authoring

The building authoring view is WYSIWYG for the local flat lot:

- The editor preview shows a flat lot plane with the authored `lot_width_cells` and
  `lot_depth_cells`, not an abstract infinite grid as the main authoring reference.
- The lot boundary is the runtime site footprint.
- Unpainted preview lot areas use the same grass terrain material as the surrounding ground,
  independent of UI theme. Authored yard surfaces cover only their polygons.
- Mesh parts, anchors, and `[[site_surfaces]]` share the same local coordinate system.
- Authored site-surface materials preview on the flat lot as the runtime site client will render
  them.
- The editor does not choose the world height of the lot. Runtime placement chooses the height from
  road/driveway connection and neighboring fixed sites through the shared placement contract in
  [`earthworks.md`](earthworks.md).
- Zoning previews and parcel edits remain visual/legal intent only. They must not preview terrain
  deformation as if a site already existed.

### Editor Workspace

- The exported pack is the portable runtime artifact.
- The local editor workspace is optional editor-only metadata stored outside the exported runtime pack.
- The game does not require workspace files to load or validate a pack.

Canonical workspace rules:

- workspace format: `TOML`
- workspace directory: `user://asset_editor/workspaces/<workspace_id>/`
- main workspace file name: `workspace.toml`

Workspace contents:

- original source file references
- import recipes and bake settings
- LOD generation settings
- thumbnail camera settings
- editor-only notes and draft state
- autosave / recovery data

Default location:

- store workspaces in `user://asset_editor/workspaces/<workspace_id>/`
- store the editable workspace file at `user://asset_editor/workspaces/<workspace_id>/workspace.toml`
- keep autosaves, recovery files, and any editor-only caches alongside it, not inside the exported pack
- use this sibling layout:

```text
user://asset_editor/workspaces/<workspace_id>/
  workspace.toml
  autosave.toml
  recovery/
  cache/
```

Identity rules:

- `workspace_id` should be a local UUID generated by the editor
- the workspace stores the target `pack_id`
- the workspace stores the current exported pack root path
- the workspace stores editor-only source file references and bake settings
- do not use `pack_id` alone as the workspace folder name, because a creator may have multiple local branches/copies of the same pack

Workspace schema:

- `workspace_id`
- `pack_id`
- `pack_root_path`
- `last_opened_utc`
- `source_files`
- `import_recipes`
- `bake_settings`
- `thumbnail_settings`
- `editor_notes`
- `dirty_state`

Autosave and recovery rules:

- `workspace.toml` is the canonical editable workspace state
- `autosave.toml` is overwritten periodically while the editor is open
- `recovery/` stores crash-recovery snapshots only when needed
- `cache/` stores rebuildable editor-only caches that may be deleted safely
- recover from `autosave.toml` only when it is newer than `workspace.toml` and the previous session ended unexpectedly

Sharing rule:

- do not require the workspace file for the game to load the runtime pack
- do not include editor-only local source paths in the shared runtime pack
- do not include autosaves, recovery files, or editor-only notes in the shared runtime pack
- allow the editor to reopen a plain exported runtime pack even when the workspace file is missing, but warn that some rebake/rebuild features may be unavailable

Editor behavior:

- opening an exported pack should try to find an existing local workspace that points at that pack path
- if none exists, the editor should create a new external workspace automatically
- moving, copying, or re-zipping an exported pack must not invalidate the pack
- if multiple workspaces point at the same exported pack path, the editor should let the user choose which local branch/workspace to open
- if a workspace points at a pack path that no longer exists, mark it as stale instead of deleting it automatically

Lifecycle rules:

- stale workspaces remain reopenable so the user can relink them to a moved pack folder
- the editor provides a cleanup screen for stale workspaces, old autosaves, and orphaned caches
- cleanup is explicit user action, not silent background deletion

Later workspace extension rules:

- published runtime packs remain free of local paths and editor-only machine state
- cross-machine authoring handoff, if implemented, uses an explicit `Export Editable Bundle` feature
- `Export Editable Bundle` is a creator-facing handoff artifact and is not the normal mod install or share format

### Manifest Caches At Scale

- TOML remains the source of truth.
- Generated cache or index files are derived data for large mod libraries.
- Direct TOML scanning remains the fallback mode when cache data is missing, stale, or disabled for debugging.

Cache rules:

- Generate a per-pack index file named `pack.index.bin` at export or install time.
- Build startup registry data from per-pack indices instead of reparsing every `asset.toml`.
- Invalidate a per-pack index when pack version, file size/timestamp, or content hash changes.
- Never treat cache data as authoritative over TOML.
- Regenerate cache data whenever it disagrees with the source manifests.
- If the runtime deduplicates byte-identical textures or material payloads, record those content hashes in `pack.index.bin` at export or install time.
- Recompute missing or stale hashes only for the affected pack.
- Do not re-hash the entire mod library on every boot.

### Shared Resources Across Packs

- Resource sharing within the same pack is allowed.
- Cross-pack resource sharing is explicit and is not the default authoring mode.
- Self-contained packs remain the default pack model.

Cross-pack sharing rules:

- Cross-pack resource access is allowed only through declared dependency packs in `pack.toml`.
- Cross-pack resource lookup uses logical asset or resource IDs.
- Cross-pack resource lookup must not use relative filesystem paths into another pack.
- Library packs may expose shared textures, materials, or reusable prop assets.
- The editor validates that every referenced shared resource exists before export.

Placed library prop export modes:

- `embed` is the default mode.
- `embed` bakes the placed prop geometry and materials into the exported asset and removes the external pack dependency.
- `reference` stores the referenced prop asset ID plus local transform.
- `reference` requires an explicit dependency on the source library pack.

Disallowed patterns:

- arbitrary references into another pack folder
- hidden implicit dependencies
- exported assets that depend on unrelated packs without a declared dependency

Optimization rules:

- If internal deduplication is implemented, deduplicate only byte-identical texture files and material payloads.
- Use a recorded content hash from `pack.index.bin` or an install-time cache entry as the deduplication key.
- Compute or refresh deduplication hashes during export, install, or per-pack reindex.
- Do not rescan and rehash the full installed pack library on every startup only to build deduplication state.

Missing dependency behavior:

- Resolve dependency availability before normal asset registration.
- If a required dependency pack is missing or incompatible, mark the dependent pack unresolved and exclude it from normal spawn and catalog registration.
- A missing referenced prop must not crash the game.
- If an optional child prop is unresolved, load the parent asset, skip the child prop, and surface a warning.
- If a required render resource is unresolved, use a placeholder mesh or placeholder material.
- Use a high-visibility missing-material checker pattern for unresolved material references.
- If an existing save references already-placed assets from an unresolved pack, keep the simulation entry stable and render a placeholder representation until the dependency issue is fixed.

## Authoring Requirements

Supported authoring workflows:

- `packaged_asset` input is `.glb` plus textures plus metadata.
- `packaged_asset` requires only the shipped asset editor.
- `packaged_asset` supports any DCC tool that exports a compliant `.glb` matching the Metrum Rise asset contract.
- `advanced_source` input is raw source data from the reference DCC workflow or animated interchange files such as `.fbx`.
- `advanced_source` uses a conversion or bake pipeline bundled with the shipped asset editor.
- User-installed Blender, Python, Godot Editor, or Rust is not part of the supported authoring workflow.
- If a feature requires external tooling that is not bundled with the shipped editor, that feature is outside the supported workflow until the tooling is bundled.
- Native project-file support for non-Blender DCC tools is outside the v1 contract unless added explicitly later.

V1 workflow assignment:

- Buildings use `packaged_asset`.
- Props use `packaged_asset`.
- Vehicles use `packaged_asset`.
- Character VAT authoring uses `advanced_source`.
- Character VAT baking runs through tooling bundled with the shipped editor.

## Runtime Safety Boundaries

First-party and community asset packs are treated as data, not executable code.

V1 safety rules:

- no arbitrary GDScript in packs
- no native DLL/SO plugins in packs
- no arbitrary gameplay scripting in packs
- no unrestricted custom shader code as the normal mod surface

Material model:

- packs describe materials through a fixed manifest/schema
- the engine maps that schema onto its built-in material/shader set
- if advanced material features are added later, they are exposed as explicit supported flags, not as "run any shader file the pack includes"

Editor/runtime boundary:

- The shipped asset editor is a standalone Godot application, not the Godot Editor.
- The asset pipeline does not rely on `EditorPlugin`, `EditorScenePostImport`, or other editor-only extension points.

Bundled tooling rules:

- VAT baking does not rely on Godot Editor import hooks.
- The shipped editor does not depend on user-installed Blender or ad-hoc external Python scripts as part of the supported modder workflow.
- Advanced bake or decimation steps run through bundled tooling only.
- Bundled bake tooling runs in one of these forms:
  - integrated runtime code inside the shipped Godot application
  - a Rust bridge or backend bundled with that application
  - a bundled helper executable or library invoked by the editor
- Implement bundled bake logic in the existing Rust bridge unless a required bake step cannot be supported there.
- Use a separate helper executable or library only for bake steps that cannot be implemented inside the shipped application or Rust bridge.
- Automatic LOD generation is optional offline tooling.
- Automatic LOD generation is not required for importing content.
- Author-supplied LOD meshes remain the baseline import path.

V1 editor architecture:

- the shipped editor handles preview, validation, manifests, thumbnails, and pack assembly
- advanced baking or decimation uses bundled internal tooling only
- auto-generated LODs are optional outputs and are not required for importing content

Legacy tool retirement rules:

- Existing `tools/bake_vat_blend.py` / `tools/bake_vat.py` are transitional developer tooling and validation references until the self-contained replacement reaches parity. Do not delete them first and then discover the replacement disagrees on output format, orientation, or precision.
- The replacement does not need byte-for-byte float identity with the legacy output.
- The legacy Blender VAT baker supports same-file source clips, explicit action selection,
  target-height normalization, stable vertex-ID UV output, and vertex-color material baking for
  low-cost runtime crowd meshes.
- Retire the legacy tools only when all of the following are true:
  - the previewed walk cycle is visually indistinguishable at expected gameplay camera distances
  - exported VAT textures and rest meshes preserve the same orientation and vertex-ID contract
  - the baked per-channel vertex delta stays within the defined tolerance threshold, default `±0.01 m`, unless the format contract is intentionally changed

## Coordinate Conventions

Imported and exported assets use one canonical local basis.

Current repository state:

- The simulation ground plane is `XZ`; `Y` is vertical/up.
- Many simulation helpers store planar positions as `(x, z)` in `Vector2`. Some legacy field names such as `center_y` and `pos_y` actually mean world `Z`, not vertical `Y`.
- Building transforms use `facing_dir` as the transform's local `+Z` axis, so building meshes must face local `+Z`.
- The pedestrian GLTF/VAT pipeline also expects the imported mesh to face local `+Z`. Existing code comments note that Blender-facing `-Y` becomes `+Z` after the current GLTF import path.
- Vehicles are a legacy exception today: the civilian car loader applies a `180°` yaw on load and the runtime car basis currently uses `basis_z = -fwd`.

Canonical local basis:

- Use a right-handed local asset basis.
- `+Y` = up
- `+Z` = front / forward
- `+X` = right

Building assets should use canonical `+Z` frontage when practical. The exported
`[building].frontage_forward` vector is authoritative for runtime building alignment, so imported
meshes whose frontage is not yet canonical can be represented without pack-specific runtime fixes.

Vehicle compatibility rule:

- Newly imported vehicle assets normalize to canonical `+Z` forward during import and export.
- The hidden runtime compatibility rotation applies only to legacy bundled vehicle content that has not been reauthored yet.

Per-asset pivot rules:

- Buildings: origin at ground level, centered on the footprint.
- Characters: origin at ground level between the feet.
- Vehicles: origin at ground level on the vehicle centerline.
- Props: origin at the exact attachment point or ground-contact point used for placement.

Source authoring rules:

- Blender `4.5 LTS` or newer is the reference source-authoring environment.
- Other DCC tools are supported through compliant `.glb` export that matches the Metrum Rise asset contract.
- Native project files from non-Blender DCC tools are outside the v1 contract.
- Blender source authoring uses Blender's default `Z`-up world orientation.
- Assets that must arrive facing `+Z` in packaged output face Blender `-Y` before glTF export.
- Export canonical source assets as `.glb`.
- Hidden renderer-specific rotations are not part of the asset contract.

Editor validation rules:

- Show an axis gizmo and a front arrow on every imported mesh.
- Provide a one-click `Set Front From Current View` action.
- `Set Front From Current View` sets only the initial frontage guess for building assets and snaps
  that guess to the nearest cardinal 90-degree direction.
- Provide one import-time orientation correction step and bake the corrected result into the packaged asset.
- Show the placement pivot explicitly so creators can confirm the mesh sits on the ground plane instead of floating or sinking.
- Provide a legacy vehicle compatibility override until the runtime vehicle renderer is normalized to the same `+Z` convention as buildings and pedestrians.

Frontage persistence rule:

- The exported asset stores an explicit front direction or entrance anchor after the creator confirms it.

## Texture Orientation

Texture orientation is explicit and deterministic.

Current repository state:

- Standard building and vehicle `.glb` assets are loaded through Godot's GLTF pipeline and currently do not apply any general-purpose texture Y-flip in gameplay code.
- Ordinary `.glb` imports currently keep mesh UVs and texture images as authored.
- The VAT pedestrian pipeline is a special case. Its baked `.exr` texture already includes explicit row reversal in the tooling to match the shader's expected sampling convention, so that path must keep its dedicated handling.

Importer/editor rules:

- Do not silently auto-flip all imported textures.
- Validate albedo, normal, ORM, emission, and mask textures in the preview scene.
- Provide a per-texture vertical flip override.
- Use the vertical flip override only when preview validation shows an orientation mismatch.
- Store every flip override in asset metadata.

Editor validation tools:

- A debug material mode that overlays a numbered UV test pattern.
- A "front/top" orientation test texture so upside-down or mirrored imports are obvious immediately.
- Side-by-side material preview for albedo, normal, and packed maps.
- A checker that warns when a normal map appears to be inverted or authored in the wrong convention.

Pipeline rules:

- Ordinary static assets use `.glb` with authored UV maps.
- Exported `.glb` material orientation is validated in the asset editor before packaging.
- VAT EXR textures are pipeline-generated data.
- Manual texture-flip controls do not change VAT EXR orientation handling.

## Anchors And Bounds

Assets use explicit anchors and bounds for placement, culling, selection, and agent interaction.

Rules:

- Every asset has explicit render bounds.
- The editor auto-generates initial render bounds and the exported asset stores explicit bounds data.
- Runtime selection and culling bounds may differ from the visual mesh and do not depend on exact triangle detail.
  Editor point-picking instead intersects the visible imported LOD's triangles; box selection uses projected bounds.
- Shadow proxies are separate from visual bounds.
- Anchor metadata uses one canonical `[[anchors]]` array-of-tables shape rather than a mix of one-off fields like `entrance_anchor`, `parking_slot_1`, etc.
- Single-anchor assets still use `[[anchors]]` with exactly one entry.

Anchor requirements by asset class:

- Buildings require exactly one `entrance` anchor named `main` that marks the main door or primary access point used by the generic entrance/exit system.
- Buildings may define optional `driveway`, `parking`, and `loading_bay` anchors. These are authored
  site-layout metadata in v1. They do not generate visual yard surfaces; vehicle parking and
  freight stop behavior remain later runtime hooks.
- Buildings may define optional `[[site_surfaces]]` polygons for authored yard materials: asphalt
  and concrete. Live gameplay renders/queries these polygons as material/layout regions on the flat
  building-site support plane.
- Vehicles may define optional `wheel` anchors and `light` anchors for wheel positions and light-marker positions.
- Props use their exported origin as the placement point and do not require a separate anchor in v1.
- Characters use the exported feet-center origin as the placement point and do not require additional anchors in v1.

Canonical anchor TOML shape:

```toml
[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 4.5]
forward = [0.0, 0.0, 1.0]

[[anchors]]
type = "parking"
position = [-2.5, 0.0, 1.0]
forward = [0.0, 0.0, 1.0]
width_m = 2.5
length_m = 5.0
vehicle_class = "car"

[[anchors]]
type = "loading_bay"
position = [4.0, 0.0, -3.0]
forward = [0.0, 0.0, -1.0]
width_m = 3.5
length_m = 8.0
vehicle_class = "freight"

[[site_surfaces]]
material = "concrete"
name = "front_walk"
y_m = 0.01
vertices = [[-0.7, 1.0], [0.7, 1.0], [0.7, 8.0], [-0.7, 8.0]]

[[site_surfaces]]
material = "asphalt"
name = "driveway_pad"
y_m = 0.01
vertices = [[2.25, -2.0], [5.75, -2.0], [5.75, 6.0], [2.25, 6.0]]
```

Built-in anchor types:

- `entrance`
- `driveway`
- `parking`
- `loading_bay`
- `wheel`
- `light`

All asset classes use the same `[[anchors]]` table shape, including single-anchor cases.

Built-in site surface materials:

- `asphalt`
- `concrete`

## LOD Strategy

LOD covers authoring, packaging, and runtime selection.

Current repository state:

- Most bundled `.glb` and `.fbx` assets currently rely on Godot import settings with `meshes/generate_lods = true`.
- The roadmap also expects Godot-side distance-based switching on `GeometryInstance3D` / `MultiMeshInstance3D`.

The current repo path is not the shipped moddable-pack contract.

LOD packaging rules:

- The shipped game consumes packaged LOD outputs from the exported pack.
- The exported pack does not require a Godot Editor import step on the end user's machine.

LOD authoring rules:

- `LOD0` is always the highest-detail imported mesh.
- `LOD1`, `LOD2`, and farther representations come from author-supplied meshes or editor-generated simplifications approved in the editor.

LOD review rules:

- Skyline assets, landmark buildings, and silhouette-sensitive vehicles use author-supplied or author-reviewed LOD meshes.
- Auto-generated LODs are allowed only after preview approval in the editor.

LOD enforcement rules:

- `LOD0` is the imported primary mesh and is never replaced by an editor-generated simplification.
- Every editor-generated LOD tier starts in a draft state.
- A draft generated LOD tier is not exportable.
- The editor exports a generated LOD tier only after the creator previews it and marks it approved.
- The editor marks skyline assets, landmark buildings, and silhouette-sensitive vehicles as review-required assets.
- A review-required asset does not export while any generated LOD tier remains unapproved.

LOD rules by asset class:

- Buildings export one or more required `[[mesh_parts]]` entries. Each mesh part requires its
  own `LOD0` under `[[mesh_parts.lods]]`; additional farther tiers are also nested under that
  same part when they exist. Top-level `[[lods]]` is not valid for building assets.
- Building mesh part geometry must fit inside `lot_width_cells` x `lot_depth_cells` after applying
  its authored position, Y rotation, scale, and pivot. This is an editor/export invariant based on
  the imported mesh bounds. Manifest parsing remains engine-independent. Runtime pack loading also
  imports LOD0 bounds through Godot and caches them in the asset registry for structural support;
  it no longer estimates support size from part scale. No new authored field, asset re-export or save
  conversion is required. Import failures produce a pack warning and skip the asset.
- Building part transforms use the editor's positive-Y yaw convention: +X rotates toward -Z.
  Runtime rendering and structural support share one part transform (including scale and pivot),
  and rendering reuses the allocator's frontage basis. Rotated parts must not be mirrored relative
  to the editor or their support footprints.
- Vehicles export with required `LOD0`. Additional farther tiers use ordered `[[lods]]` entries when they exist.
- Props export with required `LOD0`. Props may add `LOD1` and `LOD2`, or cull after `LOD0` or `LOD1`.
- Characters do not use ordinary mesh `[[lods]]`. Character runtime tiers are defined separately:
  - near: optional hero representation
  - mid: required VAT or another crowd-friendly mesh path in v1
  - far: optional billboard or another cheap representation when implemented

Editor responsibilities:

- import multiple LOD meshes when the author provides them
- generate draft simplified meshes offline when requested
- preview distance switching in the sandbox scene
- validate triangle and material counts per LOD tier
- accept `LOD0`-only building, vehicle, and prop assets as valid v1 imports
- warn when a building, vehicle, or prop asset exports only `LOD0` with no additional approved LOD tiers
- keep draft generated LOD tiers in editor-only state until approval
- export only approved LOD files and metadata into the pack
- preview switch behavior with the same discrete thresholds and hysteresis used at runtime

Runtime responsibilities:

- switch representations automatically by camera-distance bands in meters
- never run heavy mesh simplification at gameplay load time
- treat LOD selection as a rendering concern, not simulation metadata
- use discrete switching plus hysteresis

LOD pop-control rules:

- Adjacent LOD tiers preserve the large silhouette, roofline, wheelbase, and other high-contrast shapes.
- Runtime cross-fade blending is not the general LOD policy.
- A future cheap dither or cross-fade mode is reserved for rare skyline or landmark exceptions only.

Packaged metadata:

- store an ordered LOD list in `asset.toml`
- include only approved LOD tiers in exported `asset.toml`
- each tier references its exported file and intended range
- use asset-class default switch distances
- allow per-asset overrides only when silhouette or scale requires them

## Default Performance Budgets

Performance budgets target shipping hardware, not the development machine.

GPU shipping targets:

- GPU floor: `8 GB VRAM`
- GPU recommended: `12-16 GB VRAM`

System RAM is a separate discussion. This section covers GPU memory and render cost.

Budget constraints:

- The total-population target does not imply `1,000,000` fully detailed close-up characters on screen at once.
- VAT character cost scales with visible vertex count.
- Asset budgets must hold even without perfect per-agent occlusion behind buildings.
- Buildings, cars, and props use instancing and LOD, but careless materials and texture sizes still waste VRAM before triangle count becomes the first bottleneck.

Authoring budget rules:

- A material budget refers to render material slots, not to the number of visible real-world surface types in the art.
- Target `1` render material slot per asset.
- Use `2` render material slots only when the second slot resolves a real UV or shading constraint that cannot be handled cleanly in one slot.
- More than `2` materials triggers a warning.
- Use atlases and shared materials aggressively within a pack.
- Use these default texture sizes:
  - buildings: `1024`, with `2048` reserved for landmarks or unusually large skyline assets
  - vehicles: `512`, with `1024` reserved for buses, service trucks, or close-range hero assets
  - props: `256-512`
  - characters: shared `512-1024` atlases per archetype family
- Compression, mipmaps, and byte-identical texture deduplication are mandatory for shipped packs.
- Use masked or alpha-tested detail unless true transparency is required.

Shadow policy:

- Treat shadows as a separate geometry budget.
- Ordinary assets may still cast close-range shadows while rendering `LOD0` visually, but the shadow caster uses a dedicated shadow proxy or `LOD1` or `LOD2` instead of full `LOD0`, unless the asset is marked as a hero or landmark exception.
- Traffic, characters, small props, and foliage disable shadow casting before their final visible LOD or cull distance and may remain visibly rendered after shadows are disabled.
- Traffic impostor tiers and billboard-based render tiers do not cast shadows.
- Shipped mod packs store explicit shadow-caster mesh assets referenced by the pack manifest.
- A shadow-caster mesh may be author-supplied or generated offline by the editor.
- The shipped runtime does not depend on Godot import-time `create_shadow_meshes`.

Default LOD tiers:

### Buildings

- New building authoring targets four tiers. Provisional ordinary bands:
  `LOD0` 8,000–12,000; `LOD1` 2,500–4,000; `LOD2` 800–1,500;
  `LOD3` 200–500 triangles. Landmark ceilings: 20,000 / 6,000 / 2,500 / 700.
  Ceilings are limits, not targets; simple buildings should not add geometry to meet a floor.
- The editor preserves variable-length authored chains, including valid single-tier parts.
  Automatic inspection uses the shared engine screen-size policy (`TOOLS-05`, below).
  Existing manifest metre bands remain serialized unchanged; appended tiers still receive the
  schema's provisional 35 / 70 / 120 m bands. Those fields are not editable preview controls
  and do not influence automatic selection. Schema migration follows threshold calibration.
- Review thresholds with actual asset bounds, camera projection/FOV, resolution and viewing angles.
  Runtime culling, farther skyline representations and shadow budgets require separate measurements.
  Current gameplay still renders building LOD0; editor support does not implement runtime switching.
- Authoring rules:
  - model the silhouette, roofline, and major recesses
  - bake windows, facade repetition, and small trim into textures instead of geometry
  - use `1` material, maximum `2`, for normal zoned content
  - towers and landmarks remain readable across large city views; low-rise filler buildings transition earlier

### Vehicles

- Ordinary civilian car:
  - `LOD0`: `400-1,200` triangles
  - `LOD1`: `120-350` triangles
  - `LOD2`: `24-80` triangles
  - optional `LOD3`: `2-8` triangles as an unlit color block, light sprite, or traffic impostor
- Large service vehicle, truck, or bus:
  - `LOD0`: `800-1,800` triangles
  - `LOD1`: `200-500` triangles
  - `LOD2`: `40-120` triangles
  - optional `LOD3`: `4-16` triangles as a coarse impostor or light-only marker
- Default camera-distance switch bands for traffic vehicles:
  - `LOD0`: `0-40 m`
  - `LOD1`: `40-120 m`
  - `LOD2`: `120-300 m`
  - optional `LOD3`: `300-500 m`
  - beyond `500 m`: cull unless a dedicated traffic-visibility pass justifies going farther
- Authoring rules:
  - do not model interior dashboards, seats, and undercarriage detail for ordinary traffic cars
  - mirrors, wheel arches, grille detail, and trim collapse aggressively in lower tiers
  - use `1` material for ordinary traffic vehicles
  - keep a far-distance impostor instead of hard disappearance on long straight roads

### Props

- Small prop (bench, bin, bollard, mailbox, planter):
  - `LOD0`: `50-300` triangles
  - `LOD1`: `12-80` triangles
  - optional `LOD2`: `4-20` triangles
- Medium prop (lamp post, kiosk, statue base, playground piece):
  - `LOD0`: `200-800` triangles
  - `LOD1`: `40-200` triangles
  - optional `LOD2`: `8-40` triangles
- Default camera-distance switch and cull bands for props:
  - small props: `LOD0` `0-25 m`, `LOD1` `25-80 m`, then `LOD2` `80-120 m` when present, otherwise cull after `LOD1`
  - medium props: `LOD0` `0-40 m`, `LOD1` `40-120 m`, then `LOD2` `120-250 m` when present, otherwise cull after `LOD1`
- Authoring rules:
  - props are numerous, so culling discipline matters more than making each one pretty in isolation
  - favor shared atlases and repeated materials over unique texture sets

### Foliage

Trees and foliage use a separate budget class.

- Small shrub / flower bed:
  - `LOD0`: `20-120` triangles
  - `LOD1`: `4-24` triangles
  - optional `LOD2`: billboard or card cluster
- Street tree / park tree:
  - `LOD0`: `150-600` triangles
  - `LOD1`: `24-120` triangles
  - `LOD2`: `2-12` triangles as a billboard or impostor
- Default camera-distance switch and cull bands for foliage:
  - shrub / flower bed: `LOD0` `0-15 m`, `LOD1` `15-40 m`, then `LOD2` `40-100 m` when present, otherwise cull after `LOD1`
  - street tree: `LOD0` `0-30 m`, `LOD1` `30-120 m`, `LOD2` `120-600 m`
- Authoring rules:
  - overdraw is the primary foliage bottleneck
  - use masked cutouts and early billboard transitions over dense leaf geometry
  - keep foliage atlased and repetitive; do not give ordinary trees unique 2K material sets
  - far foliage casts no shadow, and near foliage uses simplified shadow proxies

### Characters

For VAT characters, the hot-path cost is per visible vertex sample in the shader. Validate both triangles and vertices.

- Default character runtime-tier budget:
  - VAT near tier: `200-500` triangles, roughly `300-700` vertices
  - optional VAT mid tier: `80-180` triangles, roughly `120-260` vertices
  - optional billboard far tier: `2` triangles as a crowd sprite
- Default camera-distance switch bands for characters:
  - VAT near tier: `0-35 m`
  - VAT mid tier: `35-90 m` when present, otherwise keep the near tier active until the far tier or cull distance
  - billboard far tier: beyond `90 m` when present
- Authoring rules:
  - if the runtime still has only one shipped VAT mesh tier for pedestrians, enforce the lean end of the `LOD0` budget
  - keep one material per archetype family and vary look through shared atlases or palette swaps
  - no per-character unique normal/ORM stacks; the crowd system benefits much more from shared data than from micro-detail

Validation defaults:

- warn when an asset exceeds the ordinary budget for its class
- hard-error only on extreme outliers or missing required tiers
- `hero_asset = true` or equivalent override is allowed for landmarks, special service vehicles, and showcase content
- show estimated VRAM cost in the editor based on mesh, textures, mip chain, and material count

## Asset Classes

### Zoned Building

Building asset data:

- One or more visual mesh parts
- Materials/textures
- Metadata:
  - `placement_mode`
  - `zone_type` and `density` when `placement_mode = "zoned_private"`
  - `lot_width_cells`
  - `lot_depth_cells`
  - `household_capacity`, `worker_capacity`, `flat_size_m2`, or combinations depending on `placement_mode` and
    `zone_type` when zoned
  - `service_class`
  - `economy_profile` reference
  - `asset_set`
  - `author`
  - `license`
  - `tags`
  - `mesh_parts` with local position, Y rotation, scale, pivot offset, and LOD references

Constraints:

- `lot_width_cells` and `lot_depth_cells` are first-class authored data and are not inferred from mesh size.
- The legacy `model_metadata.json` path has been replaced. `BuildingAllocator` now holds an `AssetRegistry` keyed by `pack_id:asset_id`, populated from `pack.toml` + `asset.toml` manifests via `scan_pack_dir`. Lot dimensions are read directly from `BuildingData.lot_width_cells` / `lot_depth_cells` in the manifest; no visual-scale multiplier is applied.
- If the importer offers "guess lot size from mesh", that guess is an editor suggestion only.
- The exported manifest stores explicit `lot_width_cells` and `lot_depth_cells`.
- `economy_profile` is a reference to an existing live economy profile. The asset editor should present this as a list or suggestion source from current economy data rather than expecting the importer to invent a new profile name manually.
- The shipped game/editor should include or load a baseline economy profile catalog for asset creators. When that local catalog is outdated, the editor should warn clearly and let the creator refresh to a newer profile list or newer game/editor build instead of inventing new profile names.
- The editor validates the mesh against the lot footprint and does not use free visual scaling to hide a footprint mismatch.
- The editor emits metadata that maps directly onto the runtime `width_cells`, `depth_cells`, and `asset_id` fields on `Building`. The old numeric `variant: u8` field has been removed; placement and rendering are now keyed by the stable qualified `asset_id` string.
- Before shipping arbitrary-size lots beyond `u8` bounds, upgrade footprint-related runtime fields to `u16` or `usize`.
- The current building renderer uses the building's `facing_dir` as local `+Z`, so authored building meshes must face `+Z` in packaged asset space.
- Do not rely on heavy import-time per-model scale tweaking until runtime building scale handling is normalized.
- Building exports require at least one `[[mesh_parts]]` entry, and each part requires explicit
  `LOD0` in `[[mesh_parts.lods]]`.
- Additional lower-detail building tiers are exported as ordered farther `[[mesh_parts.lods]]`
  entries for the same part when they exist.
- The editor may warn when a building part exports only `LOD0`, but `LOD0`-only parts are valid in v1.
- Draft fallback LODs generated by the editor require creator preview approval before export.

Building workflow:

1. Import one or more building mesh parts
2. Set initial frontage from current camera view if the imported front is not already correct
3. Choose `placement_mode`
4. If `placement_mode = "zoned_private"`, choose `zone_type` and `density`
5. Set `lot_width_cells` and `lot_depth_cells`
6. Snap the mesh into a preview footprint rectangle of that exact size
7. Enter capacity and asset-category metadata
8. Run validation for footprint overflow, frontage orientation, and sidewalk clearance

Runtime placement note:

- The allocator-side build-site contract is owned by [`building_allocator.md`](building_allocator.md).
- The zoning storage and paint-model contract is owned by [`zoning.md`](zoning.md).
- For asset authoring, the stable runtime-facing rule is simple: `lot_width_cells` and
  `lot_depth_cells` define the required contiguous footprint that the live allocator must find at a
  legal roadside build site.
- The asset editor does not own zoning storage layout, roadside scan order, or allocator tie-break
  rules.

### Props / Parks / Detail Objects

Version-one prop placement model:

- Props are explicitly authored standalone placeables.
- They are previewed and exported as individual assets, not as procedural "repeat this along a road" rules.
- Roadside generators, fence splines, and other procedural prop systems are later features, not part of the first importer contract.

Required metadata:

- `category`
- `bounding_size_m`
- `snap_mode`
- `terrain_behavior`
- `asset_set`
- `tags`

Concrete v1 enums:

- `snap_mode` = `free`, `grid`, `edge`, `surface`
- `terrain_behavior` = `flat_ground`, `conform_to_surface`, `hang_from_surface`

V1 parks use the prop asset contract plus grouping metadata.

### Vehicle

V1 vehicle assets are static meshes.

Required metadata:

- `vehicle_class`
- `vehicle_family`
- `length_m`
- `width_m`
- `height_m`
- `asset_set`
- `color_variants`
- `lod` references

Optional metadata:

- wheel marker nodes for future wheel-spin or steering visuals; the current runtime uses one rigid vehicle transform and does not animate wheels independently
- light marker nodes

V1 excludes skeletal rigs for normal traffic cars.

Vehicle LOD rules:

- Vehicle exports require `LOD0`.
- Additional lower-detail vehicle tiers are exported as ordered farther `[[lods]]` entries when they exist.
- The editor warns when a vehicle exports only `LOD0`.
- Dense traffic assets may add a very cheap far-distance representation.
- Wheel, mirror, antenna, grille, and cabin details are good candidates to collapse or bake away in lower tiers.
- The editor previews vehicles at lane width and normal gameplay camera heights.

Taxonomy:

- `vehicle_class` = broad gameplay bucket such as `civil`, `police`, `fire`, `ambulance`, `utility`, `bus`
- `vehicle_family` = physical form factor such as `sedan`, `suv`, `van`, `truck`
- V1 simulation logic keys off `vehicle_class`.
- Different vehicles within the same class stay distinct through `asset_id`, `display_name`, dimensions, and visuals rather than a separate `service_role` field.

### Character Source Asset

Character import follows the crowd-rendering contract.

V1 editor support:

- import rest mesh
- import skeleton
- import source clips
- preview clips inside the editor
- bake runtime output

Runtime output rules:

- V1 runtime output is a VAT-ready rest mesh plus baked animation textures.
- V1 runtime packs store baked VAT outputs only and do not include character source clips or source meshes.
- Runtime VAT rest meshes are normalized around a feet-centred origin at their authored target
  height, with shipped adult pedestrians currently baked to `1.8 m`.
- A far-distance SDF billboard descriptor is a later crowd LoD tier, not part of the primary v1 runtime path.
- The bake path is self-contained from the modder's point of view: opening the shipped editor and pressing bake is enough.
- The bake path does not depend on user-managed external tooling.

Character sharing rules:

- Share data within an archetype family.
- Do not force sharing across archetypes with different proportions or silhouettes.

Character archetypes:

- `adult_male`
- `adult_female`
- `child`

Archetype sharing rules:

- Within one archetype family, multiple characters can share the same rest mesh, VAT animation textures, and skeleton source.
- Visual variety inside one archetype family comes from swappable skin or clothing textures, palette variants, and metadata.
- Different archetype families share a texture atlas only when they are authored to the same UV/layout contract.
- Otherwise, use separate atlases for `adult_male`, `adult_female`, and `child`.

V1 archetype rules:

- `adult_male` and `adult_female` use one or a few shared body archetypes.
- Each archetype exposes several albedo or skin variants without duplicating VAT data.
- `child` is a separate archetype family with separate rest meshes and animation bakes.

Animation sharing rules:

- Share source animation clips across archetypes when rigs are compatible or retargeted.
- Shared source animation clips do not imply shared runtime VAT outputs.
- Archetypes with different rest meshes, proportions, or vertex layouts use separate baked runtime data.

Supported character metadata:

- `archetype_family`
- `age_group`
- `body_type`
- `skin_variants`
- `shared_rest_mesh`
- `shared_vat_animation`

Supported source clips in v1:

- `walk` required if using VAT pedestrians
- `idle` optional

V1 does not require a larger animation library than `walk` plus optional `idle`.

## Roads Are A Separate Editor

Road authoring is a separate later tool mode.

A road asset is not just:

- a mesh
- a texture
- a width value

Road assets also require:

- lane definitions
- sidewalk presence and width
- allowed transit mask
- junction clip behavior
- markings
- spline generation rules
- build cost / category
- topology compatibility

Road authoring remains outside the first importer milestone.

## User Experience

### Context menus and explicit manipulation — `TOOLS-07`

One on-demand context menu serves the preview, mesh/access/yard lists and library. Right-click
only opens menus; middle-drag/wheel retain camera navigation and Alt+left-click cycles overlaps.
The depth-aware picker excludes occluded targets. Right-clicking an unselected object selects it
and opens its inspector; clicking a selected object preserves the complete mixed selection.
Empty-space menus preserve selection but offer creation/view actions, never unrelated deletion.
Captured document generations invalidate stale popups and file-pickers. R/Delete respect text,
library and modal focus. Toolbar deletion uses the same complete-selection command.

| Target | Context actions |
|---|---|
| Mesh | Properties, Frame, Rename, Duplicate, Rotate, LOD, Create, Delete entire part/all LOD references |
| Parking/loading | Properties, Frame, Rename, Duplicate, Rotate, Create, Delete |
| Main entrance | Properties, Frame, Rotate, Reset to frontage, Create, Delete; no rename/duplicate |
| Driveway | Properties, Frame, Rename, Duplicate, Create, Delete; no free rotation |
| Yard/edge/vertex | Properties, Frame, Rename, Duplicate, Material; valid edge insertion/vertex deletion; Create, Delete |
| Scale reference / comparison | Frame and reset/hide reference, or clear comparison; preview-only, outside history |
| Library asset | Open, editable copy, comparison, show folder, copy ID, confirmed Move to Trash |
| Library pack/category/empty space | Pack-bound/category-prefilled New asset; pack folder/refresh; empty-space New asset/Create pack/Refresh |

Mixed selections expose only shared supported edits: nothing silently omits unsupported members.
The LOD submenu names the captured replacement tier and actual next/last tiers; preview offers
Automatic plus every imported tier. Removing a part never removes its model files.

Rotate offers interactive/R, left/right 90° and reverse 180°. Pointer movement starts without a
popup-to-preview jump and displays the angle and confirmation/cancellation hint. Each object
rotates around its own pivot with relative angle differences preserved, not a group orbit.
Existing lot clamps apply in the same edit. Left-click confirms exactly one undo entry without
selecting/dragging through; Escape, focus loss, another modal or document replacement cancels.
Right-click cancels the provisional operation before opening the next menu.

Create is available over yards as well as empty ground: mesh, unique main entrance (or Select
main entrance), driveway, parking, loading, asphalt and concrete. A missing ground intersection
disables location-based actions with an explanation; no open document offers New asset instead.
Placement captures the clicked **ground** point, including across a mesh picker, never a roof.
Driveways remain frontage-pinned and inward-facing. Duplicate reuses placement, retains complete
LOD/source/unknown metadata, yard polygons/materials and anchor dimensions, assigns deterministic
unique names and preserves group offsets. Cancellation leaves the document untouched; confirmation
is one undoable command. Object duplication does not copy source files.

Library Open and editable Copy use the unsaved-change guard. Copy asks for a new identity/name
and existing writable destination pack, preserves attribution and creates independent files under
`user://asset_editor/copies/`; the document remains unpublished until explicit export. Original
pack author/licence metadata and any inherited credits are retained under `attribution/`, referenced
by the draft's optional `supporting_sources` map and included in staged export. Runtime manifests
are unchanged, and destination pack authorship does not replace original model credits. Trash checks
validated user-root containment, symlinks, writability, the active publication origin and source
references retained by the working document, savepoint and undo/redo history. Confirmation names
the asset/ID/path and warns that existing saves may reference it. The target is revalidated before
native recoverable Trash; failure never falls back to permanent deletion and only success refreshes
the library. Recovery uses system Trash, not document Undo. Pack removal is not provided.

Rust owns lossless command preparation, capabilities, naming, angle/lot/polygon rules, document
history and file validation/copying. GDScript owns popup/dialog presentation, pointer projection
and transient preview transforms. Old library/vertex popups, right-drag state and duplicate
create/delete implementations are removed. Appending/removing parts retains surviving preview
resources and LOD state. Menus do not import meshes, rebuild mesh BVHs or snapshot the whole
document. Availability/rotation calculations are O(selected objects); yard placement additionally
visits selected vertices. Immutable document copies occur at operation/commit boundaries, never
per pointer motion. Selection outlines rebuild once per group update, not once per member.
Existing affected-domain guide rebuilds remain O(domain geometry), measured separately from
transform calculations; mesh-only gestures skip anchor/yard rebuilds. File planning is O(F log F)
for deterministic ordering; copying is O(files + bytes), unrelated to city size.

Hide/isolate/lock, group-pivot rotation, general clipboard and bulk pack operations remain outside
this milestone; no placeholder entries are shown.

Acceptance — 2026-09-20:

- `cargo test --manifest-path rust/Cargo.toml --lib assets::authoring`: 19 passed.
  Release build, `cargo fmt --check`, `cargo doc --no-deps` (no warnings) and `git diff --check` pass.
- All six headless suites pass: `asset_document_test`, `asset_authoring_test`,
  `asset_workspace_test`, `asset_layout_test`, `asset_selection_test`, `asset_editor_preview_test`.
  Coverage includes real input, cancellation/focus/modal/document changes, group pivots/offsets,
  visible-tier relinking, stale dialogs, full-chain undo, copied credits/export, source/undo
  protections, symlinks, read-only destinations, failed Trash and synthetic native Trash recovery.
- Rendered selection/workspace suites pass on stock Godot 4.7.2, X11 / Forward+, RX 7900 XTX:
  light/dark root and nested menus, keyboard navigation, 960×640 screen-edge placement,
  rotation feedback, depth-aware picking, library rows, independent copy and export.
- Unprofiled release comparison: HEAD `42a0ed85` versus this change, i9-12900K,
  Rust 1.98.1, `RAYON_NUM_THREADS=4`, three fresh isolated processes per fixture/build.
  Runs are serial, with no concurrent builds or rendered checks. Existing synthetic workloads:
  selection = two meshes/one anchor/one yard (10,000 idle + 1,000 query iterations);
  workspace = 100 metadata edits + 10,000 idle updates; preview = one part/four LODs,
  40 cached switches + 2,000 automatic transitions. Median of the three process means:

| Measurement | Baseline | Context menus |
|---|---:|---:|
| Idle picking (µs) | 0.730 | 0.744 |
| Pointer query (µs) | 27.818 | 27.658 |
| Metadata edit (µs) | 280.170 | 292.380 |
| Idle workspace update (µs) | 1.616 | 1.549 |
| Cached LOD switch (ms) | 0.002 | 0.002 |
| Automatic LOD transition (µs) | 10.257 | 10.193 |

Metadata notifications add about 12 µs per document edit; idle/picking/LOD costs remain comparable.
Measured queries and warmed LOD transitions perform zero imports/BVH rebuilds. New-operation
timing separates transform calculation from outline/UI rebuild: one/two selected meshes cost
3.423/6.583 µs for transforms and 38.627/66.531 µs for rebuilds; capability lookup is
1.859/2.445 µs. Group outlines rebuild once per update; pointer motion takes no document snapshots.

Reproduce with a release `.so` and a fresh `XDG_DATA_HOME`/`XDG_CONFIG_HOME` for **each** process:
`RAYON_NUM_THREADS=4 godot --headless --path <project> --script res://tests/asset_selection_test.gd -- --asset-editor --benchmark-asset-selection`;
substitute `asset_workspace_test.gd` / `--benchmark-asset-workspace` or
`asset_editor_preview_test.gd` / `--benchmark-asset-preview`. Render checks use
`--display-driver x11 --rendering-method forward_plus` instead of `--headless`, with
`--capture-selection=<directory>` or `--capture-workspace=<directory>`.
Local artifacts: `/tmp/metrum-context.16ONJ5/`; `complete-*.log`, `rust-complete.log`,
`rustdoc-complete.log`, `render-accepted-*`, `accept-bench-baseline-*.log` and
`accepted-current-*.log`. Baseline/current library SHA-256 prefixes: `ca964f35eb0f` /
`fe522773cce90`. Timed production diff: `production.diff` (`2590f3844968`), plus new-file
hashes in `new-source-sha256.txt` (`af397e428b45`). Earlier exploratory runs are not acceptance data.

Start flow:

1. Choose `New asset…` or open a published asset from the collapsible library. Resume unfinished work through `File → Open Draft`.
2. New asset asks for building type, service subtype when applicable, name, destination pack and optional model.
3. Work freely between Overview, Model, Site, Gameplay, and Validate & export. Save incomplete drafts separately from runtime export.
4. Use the main toolbar's `Export asset…` (also in File) to reveal Validate & export, even with the inspector collapsed. Review the destination and resolve issues, then confirm with `Export runtime asset…`. Opening the review never writes asset files.

The editor does not use a one-asset-only wizard. One pack may contain and edit multiple assets in one session.

V1 editor shell:

- one shared editor shell scene
- one center viewport with shared camera controls
- one building inspector; additional asset classes remain later work
- working preview controls, not selectable placeholders for unimplemented scenes

Editor layout:

- Top: Library, New asset and Export asset are the primary asset actions. Draft open/save remain in File with Ctrl+L / Ctrl+S; they do not occupy toolbar or welcome buttons.
- Center: dominant 3D preview with Lighting & quality controls in a popup; framing respects the visible pane.
- Left: collapsible asset library, hidden initially.
- Right: one task at a time. Model shows the selected part's LOD chain directly, above expandable Placement; there is no Model subsection dropdown or read-only source-material summary. Site separates Footprint & frontage, Access points and Surfaces.
- Bottom: collapsible diagnostics, hidden initially. Addressed validation is in Validate & export, not buried in the log.

Current preview (`TOOLS-04`):

- Flat, shadow-receiving terrain beneath the preview with a 10 m × 10 m grid aligned
  to the lot cells, a selectable 1.8 m scale reference and explicit asset comparison ghost.
  The terrain is preview-only, below the lot surface, and never enters exported assets.
  `WorldMaterials.flat_terrain_material()` reuses the game's terrain shader and cached grass
  albedo/height textures with a constant zero heightmap and no water. World-space grass detail
  and day/night shading come from that shader; the grid is a transparent overlay.
  One static two-triangle plane uses two material passes: O(1) geometry/storage,
  no per-frame CPU updates or allocations, and constant work per visible fragment.
  Enable Scale reference, then click its figure or 10-logical-pixel circular handle and left-drag.
  Movement is free on the ground XZ plane, including across yards/anchors/meshes and outside the lot;
  it preserves the grab offset, does not snap to geometry, and never changes the asset or undo history.
  The reference turns cyan when selected. Its handle stays visible and pickable over other objects;
  the figure itself uses triangle picking and normal depth testing. Empty-space clicks only deselect.
  Its toolbar switch uses an opaque, padded button background in every state, independent of
  scene lighting; light/dark theme changes preserve readable text, including checked-hovered text.
  Escape, focus loss, a modal or hiding the reference cancels a drag. Hiding/showing, lot/frontage/theme
  changes and asset undo preserve placement; preview rebuilds do not regenerate its geometry/pick cache.
- Day/dusk/night presets and an editable hour use the shared gameplay day-cycle lighting.
  Changing the UI theme does not change the selected lighting.
- Window emission defaults to **Automatic (time of day)**: on when the shared day-cycle sun
  is at/below the horizon, off above it. Day/dusk/night presets and manual hour changes update
  it immediately, including meshes imported later and newly selected LODs. Authored default,
  force off and force on remain explicit overrides that survive clock changes; reference
  intensity is adjustable in Automatic and Force on.
  Overrides affect preview-owned materials only; textures/factors in exported files stay unchanged.
  Materials remain authored in the source model; the editor does not expose a material inspector.
  The reference tint is linear RGB `[1.0, 0.55, 0.23]`. Only materials with an emission texture
  participate. Surface emission does not promise light spill onto nearby geometry.
- Automatic screen-size LOD inspection for every part, temporary per-part tier inspection, global preview
  quality presets and explicit camera framing. The visible Model list offers Automatic and each
  LOD file; clicking a tier previews it immediately. `Add LOD1…` (then LOD2, LOD3, etc.) appends
  the next mesh to this part and previews it without saving/exporting. `Replace LODn…` replaces
  or relinks the selected tier (the actually visible tier in Automatic); other tiers, placement and authored bands
  are preserved. Removing the last tier is explicit; LOD0 cannot be removed independently.
  All source/chain edits support undo/redo. The main readout shows the active LOD and triangle count.
  Projected asset size and engine-owned calibration boundaries are under LOD details, collapsed
  by default. The size tooltip explains its stable LOD0-bounds reference; the diagnostic updates
  live even while collapsed. Framing remains in the viewport toolbar.
  Viewport/part selection highlights the rendered tier without disabling Automatic; the highlight
  and replacement target follow zoom/quality transitions. Clicking a LOD row previews it,
  even if already highlighted. Row clicks and Add/Replace hold that tier only until the camera
  moves or its projection changes (zoom, orbit, pan or frame selection); then all inspected parts,
  including unselected ones, resume Automatic with reset transition history. The readout explains
  this temporary state; the Automatic row also exits inspection immediately. Idle, selection and
  lighting changes preserve inspection. Failed automatic switches keep inspection on the retained visible
  mesh; explicitly selecting a failed tier keeps that tier selected for source repair.
  Switching tiers preserves transforms, origins, camera and LOD0 placement bounds.
  This uses the shared selection policy; gameplay building switching is not integrated yet.
- Roadside, lane/sidewalk and traffic comparison scenes are deferred and are not exposed as controls.

Planned quality-of-life features (not shipped):

- autosave of editor state
- recovery of unsaved drafts after crash or forced close
- whole-pack revalidation/rebuild (current-asset revalidation is available)

Thumbnail generation rules:

- Overview captures the visible preview pane, excluding editor chrome. This is an explicit authoring command with undo/redo.
- Captures omit anchor/yard guides and labels, frontage/lot/grid lines, selection/hover handles,
  the scale reference and comparison assets. The model, terrain and authored yard surfaces remain.
  Helper visibility and interaction are restored after the captured frame, before saving the image.
- Captures live beside editor drafts, are packaged as `thumbnail.png` on runtime export, and survive draft and published-asset reopening.
- Capture requires a rendered window; headless validation/export still works with existing thumbnail files.
- Standardized per-class catalog-thumbnail rigs remain later work; current captures use the author's preview camera and lighting.

V1 inspector and viewport contract:

- The editor shell uses resizable left browser, central viewport, right inspector, and bottom log
  panes. Fixed-width side panels are not acceptable because real content packs can contain hundreds
  of assets and long authored IDs.
- Dense editor shells support dark and light UI themes from a top-right chrome switch. The selected
  mode is a local editor preference and applies to editor-owned dialogs such as mesh import.
- Resizable editor UI state is local and persistent. The asset editor stores its window size and
  position, browser/inspector/log split sizes, and editor-owned dialog positions/sizes/splits so
  restarts preserve the working layout. Restored dialog geometry must be clamped to the current
  application viewport so a saved layout cannot reopen editor-owned windows outside the app.
- The asset browser presents a searchable, deterministic hierarchy rather than one flat list. The
  baseline grouping is pack, then asset category derived from the registered asset ID, then the
  individual asset. Individual asset rows display the authored `display_name`; the full asset ID is
  retained as item metadata/tooltip and remains searchable. Single click selects an asset browser
  row; double click or keyboard activation loads it into the inspector.
- Mesh import uses a project-native picker with folder navigation, current-folder filtering, and a
  live 3D preview of the selected GLB/GLTF/FBX file before it is accepted into the asset. The
  preview loader uses the same import path as the final building preview so the picker cannot show
  a materially different model from the exported asset.
- Building mode inspector edits shared asset fields (`asset_id`, `display_name`, `thumbnail`, `asset_set`, `tags`, optional attribution), building fields (`placement_mode`, conditional `zone_type` and `density`, `service_class`, `economy_profile`, `lot_width_cells`, `lot_depth_cells`, `frontage_forward`, `min_zone_width_cells`, `min_zone_depth_cells`, `household_capacity`, `worker_capacity`, `flat_size_m2`, extractor resource metadata, field resource metadata), mesh part files/transforms, optional material paths, the required `entrance/main` anchor, and optional `driveway`/`parking`/`loading_bay` site anchors.
- Normal viewport selection is **All objects**, independent of the current inspector section.
  Click a visible mesh, anchor or yard directly; no toolbar/task change is required first.
  A plain click opens Model, Site / Access points or Site / Surfaces respectively, expanding a
  collapsed inspector. Optional explicit Meshes-only / Anchors-only / Yards-only filters remain;
  inspector navigation never changes the filter. All includes the scale reference and comparison ghost;
  these preview-only helpers select exclusively and never join authored transform groups. Authored
  objects support mixed selection; a deliberately filtered-out object cannot
  intercept a click. Anchor handles use a 12-logical-pixel radius, selected-yard vertices 8 pixels,
  independent of zoom and UI scale. Parking/loading/driveway rectangles and the visible billboard
  labels for anchors/yards are also pickable; text uses its rendered bounds, not its ground projection.
- Point-picking uses actual 3D mesh triangles, authored yard/anchor heights and camera clipping.
  Lot/frontage/site guides, labels, selection brackets and hover outlines use normal mesh depth
  testing. Hidden anchor/yard handles are omitted when their centres are occluded; hidden targets
  never take an ordinary click, including with explicit selection filters. Visible site labels,
  vertex handles and anchor guides retain priority over the yard beneath them; remaining hits
  within a layer are nearest-first with deterministic ties. The scale-reference manipulation
  handle remains the deliberate always-visible exception. Hover outlines and a target name show
  what will be picked.
  **Alt+click** cycles through overlapping eligible objects, including occluded ones; moving the
  pointer more than 3 logical pixels or changing the camera/filter resets the cycle.
- Click selects without moving. A transform starts only after 6 logical pixels of movement;
  Escape, focus loss or a modal dialog cancels it. A completed authored-object drag is one undo command; a cancelled
  drag restores preview geometry and does not undo a preceding inspector edit. Only the selected
  yard exposes vertex and edge editing, with an 8-pixel edge target for its context menu.
- Building mesh parts can be moved on the X/Z plane by left-dragging the part in the preview
  viewport and rotated around Y with Rotate / R followed by horizontal pointer movement, with a light snap when the
  rotation is close to a 90-degree cardinal angle; the clicked part becomes the selected part and
  the inspector transform fields mirror the live manipulation. Selected mesh parts use corner
  handles; the current hover target additionally has a bounding-box outline. The editor clamps the transformed X/Z footprint of
  every mesh part to the authored lot rectangle during explicit import/move/rotate/scale actions.
  Opening a document and changing its lot dimensions never silently repair geometry; validation
  reports overflow, and export rejects parts that cannot fit inside the lot.
- Dragging an empty area in the building preview draws a selection rectangle and selects eligible
  mesh parts/anchors plus at most one yard whose projected bounds intersect it, respecting the
  active filter. Holding `Shift` while left-dragging forces rectangle selection even when the drag
  begins over a mesh part or anchor. Holding `Ctrl` while clicking toggles individual mesh parts,
  the main entrance, site anchors, or one yard into the current selection. Dragging a selected object
  moves the whole selected mesh/anchor/yard group on the X/Z plane; Rotate / R rotates the complete eligible selection. The
  main entrance can be moved or removed in a draft; runtime export requires a valid `entrance/main`.
- Model/Site provide `Delete selection`; keyboard Delete and context menus use the same command.
  Removal deletes the complete authored selection from the draft and preview, never source files.
- Site / Access points provides `Entrance`, `Driveway`, `Parking`, and `Loading Bay` add actions plus a remove action
  for the selected site anchor. Site anchors are selectable in the list or viewport, movable on the
  X/Z plane with left-drag, and (except frontage-bound driveways) rotatable around Y with Rotate / R
  using the same light cardinal snap as meshes. Delete respects the active editing context.
- The task selector stays above independently scrolling task pages. Model and Site disclose one
  relevant sub-section at a time. Changing tasks does not resize the inspector sidebar.
- Overview uses a single `Choose / manage pack…` command plus a compact selected-pack summary instead of
  separate free-form pack fields. The command opens a pack menu listing installed
  `user://mods/*/pack.toml` packs and a `Create New Pack...` action. Creating a pack writes a
  minimal `pack.toml` immediately; asset files are still added on export.
- If an existing loaded asset is exported to a different pack, the editor must stop for an explicit
  retarget choice: `Copy`, `Move`, or `Cancel`. `Copy` writes/updates the target pack and leaves the
  original asset folder untouched. `Move` writes/updates the target pack, then deletes only the
  original asset folder after the target export and file copies succeed. A failed or incomplete
  export must never delete the original.
- If `placement_mode = "zoned_private"`, the zoning-choice controls in building mode should load their available categories and density-band combinations from the shipped zoning-profile registry rather than from hardcoded editor-only lists.
- If `placement_mode = "explicit"`, the zoning-choice controls are hidden and the building is authored outside the painted-zoning path.
- Building mode viewport shows the lot rectangle, frontage arrows,
  entrance anchor gizmo, site-anchor previews, orientation validation, and footprint overflow
  warnings. Frontage and access points share filled, tapered arrows with a split-tone head and soft
  dark keyline. The frontage edge is a continuous violet accent with cell ticks and at most three
  compact arrows; the lot uses a neutral fine outline with corner brackets. Anchor type colors,
  low-opacity fills and dashed unselected borders remain distinct; selected yards/anchors use a
  warm highlight. Labels retain their text size with lighter outlining, and screen handles have
  softer strokes without shrinking hit targets. All guides remain unshaded/depth-independent and
  readable in both themes during day/night preview. These are preview-only visuals; authored
  frontage, footprints, materials, picking priority, dragging and Alt+click are unchanged.
- The comparison ghost is explicit, not automatic. Right-clicking an asset browser row opens an
  asset context menu with `Use as comparison`; that asset remains as the viewport comparison until it is
  replaced or cleared.
- The comparison ghost uses the selected asset's first mesh part and authored part scale.
  Loading another asset into the inspector must not replace the ghost. Assets from packs with
  different source-unit conventions must compare in exported/game-space meters.
- Comparison materials retain source textures, shading, depth testing and opacity, with a subtle
  cool albedo tint on preview-owned copies. Opaque walls/roofs stay solid; authored glass/cutouts
  retain their transparency. No unlit translucent blueprint override is applied. Supported windows
  follow the same automatic/manual emission controls as the edited asset, including loading during
  Night. Neither tint nor emission changes source resources, exports or document history.
  Material setup walks N scene nodes and S surfaces in O(N + S) on load; emission changes reuse the
  existing preview-material controller and cached overrides in O(E) for E emission surfaces, with
  O(1) unchanged-state checks. Textures remain shared; clearing/replacing releases material state.
- In the All selection filter, the comparison ghost can be repositioned by dragging it with the left mouse
  button.
- The following prop/vehicle/character modes are future contracts, not selectable placeholders in the current building editor.
- Prop mode inspector edits shared asset fields (`asset_id`, `display_name`, `thumbnail`, `asset_set`, `tags`, optional attribution), prop fields (`category`, `bounding_size_m`, `snap_mode`, `terrain_behavior`), and optional material paths.
- Prop mode viewport shows ground contact, snap target, pivot, authored bounds, and orientation validation.
- Vehicle mode inspector edits shared asset fields (`asset_id`, `display_name`, `thumbnail`, `asset_set`, `tags`, optional attribution), vehicle fields (`vehicle_class`, `vehicle_family`, `length_m`, `width_m`, `height_m`, `color_variants`), optional material paths, optional `wheel`/`light` anchors, and `[[lods]]`.
- Vehicle mode viewport shows lane width, parking-bay reference, turning-circle reference, optional wheel/light anchor gizmos, forward arrow, and orientation validation.
- Character mode inspector edits shared asset fields (`asset_id`, `display_name`, `thumbnail`, `asset_set`, `tags`, optional attribution), character fields (`archetype_family`, `age_group`, `body_type`), source clip paths, bake settings, and baked runtime outputs.
- Character mode viewport plays `walk` and optional `idle`, shows a bake-status panel, and previews the result against sidewalk and doorway references.

## Cells And Reference Areas

Reference systems are class-specific.

For buildings:

- show the zoning-cell reference
- use the current game cell size
- present a resizable plot rectangle, not a fixed `3 x 3` box
- edit explicit `lot_width_cells` and `lot_depth_cells`
- preview frontage, road-facing direction, and sidewalk clearance
- set the current camera-facing side as the initial frontage guess
- always show a front arrow and support explicit frontage override
- provide presets such as `1x1`, `2x2`, `3x3`, `4x4`, `5x5`, `10x10`
- support manual width and depth entry within the currently painted zoning and map bounds

For cars:

- show lane width, parking bay, and a few turning circles
- do not use building-style lot cells as the main reference

For characters:

- show sidewalk width, a standard doorway, bench, crosswalk, and camera-distance silhouettes
- again, do not use building-style lot cells as the main reference

## Metadata And Pack Files

The pipeline uses explicit TOML manifests as the human-authored source of truth. The exported folder keeps manifests visible and editable.

This section is the canonical v1 implementation contract for manifests, IDs, and scanner behavior. If a later design note conflicts with this section, this section wins for the first implementation pass.

V1 deliberately does not require:

- cross-pack dependencies
- `[[asset_redirects]]`
- workspace files
- signature verification
- cross-pack resource references

### V1 Pack Root And Scanner Rules

The installed runtime form is one unpacked pack folder:

```text
user://mods/<pack_id>/
  pack.toml
  checksums.sha256
  pack.index.bin           # optional derived cache
  assets/
```

Scanner rules:

- A directory counts as a pack root only if it contains `pack.toml`.
- A directory counts as an asset root only if it contains `asset.toml`.
- The scanner walks `assets/` recursively and registers every asset root it finds.
- In v1, all file references inside `asset.toml` must be relative paths from that asset root.
- In v1, referenced files must stay inside that asset root. `..` path traversal is invalid.
- Folder names are for human browsing only. Runtime identity comes from `pack_id` and `asset_id`, not from a folder path alone.
- The scanner validates the canonical category-first layout under `assets/buildings/`, `assets/props/`, `assets/vehicles/`, and `assets/characters/`.
- `pack.index.bin` is optional derived data and must not be required for pack discovery.
- `checksums.sha256` is part of the exported pack format but is not the pack-root detection key.

Canonical v1 asset-root paths:

- buildings: `assets/buildings/<building_group>/<asset_slug>/asset.toml`
- props: `assets/props/<category>/<asset_slug>/asset.toml`
- vehicles: `assets/vehicles/<vehicle_class>/<asset_slug>/asset.toml`
- characters: `assets/characters/<archetype_family>/<asset_slug>/asset.toml`

`<building_group>` is a human-browsing folder bucket only. It does not define placement legality.
For ordinary zoned private buildings, the recommended bucket is the authored `zone_type`. For
explicitly placed buildings, use another stable grouping such as `service`, `utility`, or
`landmark`.

Canonical v1 pack structure:

```text
user://mods/
  kenney_city_pack/
    pack.toml
    checksums.sha256
    pack.index.bin
    assets/
      buildings/
        residential/
          lowrise_corner/
            asset.toml
            mesh.glb
            mesh_lod1.glb
            mesh_lod2.glb
            albedo.png
            normal.png
            thumbnail.png
      props/
        street_furniture/
          bench_wood/
            asset.toml
            mesh.glb
            thumbnail.png
      vehicles/
        police/
          police_cruiser/
            asset.toml
            mesh.glb
            mesh_lod1.glb
            thumbnail.png
      characters/
        adult_male/
          casual_male_a/
            asset.toml
            rest.glb
            walk.exr
            thumbnail.png
```

The repository may mirror the same pack subtree under `content/packs/` for development, but the shipped runtime scanner targets `user://mods/`.

### V1 `pack_id` And `asset_id` Rules

`pack_id` rules:

- lower-case ASCII only
- allowed characters: `a-z`, `0-9`, `_`, `-`
- must match: `^[a-z0-9][a-z0-9_-]*$`
- must be unique among installed packs

`asset_id` rules:

- lower-case ASCII only
- segments are separated by `.`
- each segment may contain `a-z`, `0-9`, `_`, `-`
- must match: `^[a-z0-9][a-z0-9_-]*(\\.[a-z0-9][a-z0-9_-]*)+$`
- must be unique within one pack

Canonical v1 `asset_id` shapes:

- building: `building.<building_group>.<slug>`
- prop: `prop.<category>.<slug>`
- vehicle: `vehicle.<vehicle_class>.<slug>`
- character: `character.<archetype_family>.<slug>`

Examples:

- `building.residential.lowrise_corner`
- `prop.street_furniture.bench_wood`
- `vehicle.police.police_cruiser`
- `character.adult_male.casual_male_a`

Editor behavior:

- The editor generates the initial `asset_id` from the asset class, category, and display name slug.
  The display-name slug is lower-case ASCII; every run of spaces, punctuation, hyphens, underscores,
  or other non-alphanumeric characters collapses to one `_`, with no leading or trailing `_`.
- The author can edit that generated ID before first export.
- Save and export both hard-fail on duplicate `asset_id` values inside one pack.
- In v1, renaming an exported asset is a breaking change. Redirects are a later feature, not part of the first implementation contract.

The globally unique runtime key is always `pack_id:asset_id`.

### V1 `pack.toml` Schema

Required fields:

- `schema_version`: integer, must equal `1`
- `pack_id`: string, must follow the `pack_id` grammar above
- `display_name`: string
- `version`: string, semantic version `MAJOR.MINOR.PATCH`
- `author`: string
- `license`: string

Optional fields:

- `description`: string
- `enabled_by_default`: boolean, default `true`

Not part of v1 `pack.toml`:

- `content_api_version`
- `[compatibility]`
- `[[dependencies]]`
- `[[asset_redirects]]`

### V1 Common `asset.toml` Fields

Required fields for every asset:

- `asset_id`: string, must follow the `asset_id` grammar above
- `asset_class`: enum, one of `building`, `prop`, `vehicle`, `character`
- `display_name`: string

Optional common fields:

- `thumbnail`: relative path to preview image
- `asset_set`: lower-case slug for content grouping. For buildings, the current runtime still uses this field name for upgrade-family identity, but the intended clearer building-side concept is `upgrade_family`
- `tags`: array of strings

Optional `[attribution]` table:

- `author`: string
- `license`: string
- `source`: string URL or free-form source note

Required `[orientation]` table for exported assets:

- `up_axis`: enum, must be `+Y`
- `forward_axis`: enum, must be `+Z`
- `origin`: enum

Allowed `origin` values in v1:

- buildings: `footprint_center`
- props: `placement_anchor`
- vehicles: `vehicle_centerline`
- characters: `feet_center`

Optional `[materials]` table for static-mesh assets:

- `albedo`: relative path
- `normal`: relative path
- `orm`: relative path
- `emission`: relative path
- `opacity_mask`: relative path
- `flip_v_albedo`: boolean, default `false`
- `flip_v_normal`: boolean, default `false`
- `flip_v_orm`: boolean, default `false`
- `flip_v_emission`: boolean, default `false`
- `flip_v_opacity_mask`: boolean, default `false`

Optional `[[anchors]]` table:

- `type`: enum, one of `entrance`, `driveway`, `parking`, `loading_bay`, `wheel`, `light`
- `name`: optional string. Building `entrance/main` requires `name = "main"`; parking and loading
  anchors normally omit names and are shown by deterministic type/index labels in the editor.
- `position`: `[f32, f32, f32]`
- `forward`: `[f32, f32, f32]`; the anchor-local access/gizmo direction. Manifest validation
  rejects non-finite, zero, or non-unit vectors.
- `width_m`: optional positive float. Required for building `driveway`, `parking`, and
  `loading_bay` anchors.
- `length_m`: optional positive float. Required for building `parking` and `loading_bay` anchors.
  `driveway` anchors derive their v1 editor footprint length from `width_m`.
- `vehicle_class`: optional string, baseline values `car`, `freight`, or `service`. If present,
  validation rejects any other value.

Optional `[[site_surfaces]]` table for building visual yard polygons:

- `material`: enum, one of `asphalt` or `concrete`
- `name`: optional editor label
- `y_m`: optional finite offset for the flat asset-editor preview, default `0.0`; gameplay paving
  is a material partition on physical ground, not an elevated overlay or platform
- `vertices`: at least three `[x, z]` pairs in asset-local metres, in winding order
- `vertices` must define a finite, non-self-intersecting polygon fully inside the authored lot

Optional `[[lods]]` table for non-building mesh assets:

- `name`: string
- `mesh`: relative path
- `min_distance_m`: `f32`
- `max_distance_m`: `f32`

LOD rules:

- `lods` are ordered from near to far.
- `min_distance_m` and `max_distance_m` must be monotonically increasing.
- `lod0` is the highest-detail mesh.
- If `lods` is omitted for non-building assets, the runtime uses the asset's primary mesh only.
  Building assets do not use top-level `[[lods]]`; they use `[[mesh_parts.lods]]`.

### V1 Building `asset.toml` Schema

Required fields:

- one `[building]` table
- at least one `[[mesh_parts]]` table
- each `[[mesh_parts]]` table must include at least one `[[mesh_parts.lods]]` entry for `LOD0`
- `placement_mode`: enum, one of `zoned_private`, `explicit`
- `lot_width_cells`: integer, `>= 1`
- `lot_depth_cells`: integer, `>= 1`

Conditional zoning fields:

- `zone_type`: required when `placement_mode = "zoned_private"`; enum, one of `residential`,
  `commercial`, `industrial`
- `density`: required when `placement_mode = "zoned_private"`; enum, one of `low`, `medium`,
  `high`
- `zone_type` and `density` must be omitted when `placement_mode = "explicit"`

Optional fields:

- `mesh_parts.name`: editor label for the part
- `mesh_parts.position`: `[f32, f32, f32]`, local metres from building origin
- `mesh_parts.rotation_degrees`: `[f32, f32, f32]`; the v1 runtime supports Y rotation for building parts
- `mesh_parts.scale`: uniform part scale, default `1.0`
- `mesh_parts.pivot_offset`: optional `[f32, f32, f32]` mesh pivot correction
- `mesh_parts.lods.file`: relative path to the part mesh file
- `mesh_parts.lods.distance_min_m` / `distance_max_m`: existing schema's ordered metre bands,
  retained on save but ignored by the screen-size preview; pending calibrated-policy migration
- `service_class`: enum, one of `none`, `police`, `fire`, `healthcare`, `education`, `power`, `water`, `waste`, `transit`, `parks`, `government`; default `none`. Non-`none` service classes are valid only for `placement_mode = "explicit"` in baseline `v1`.
- `economy_profile`: reference to an authored economy profile. Utility service assets require a resolved utility profile; the starter mappings are `power -> power_plant_basic`, `water -> water_plant_basic`, and `waste -> wastewater_treatment_basic` (`waste` is the asset-side service class for sewage treatment). Ordinary zoned commercial service assets such as barbers and pharmacies select reusable commercial profiles such as `personal_service_small` or `health_essentials_small`; they do not set `service_class` or define barber/pharmacy-specific economy fields.
- The incoming small Machinery factory uses `placement_mode = "zoned_private"`, `zone_type = "industrial"`, `density = "low"`, `level = 1`, and `economy_profile = "machinery_factory_basic"`. Leave `service_class`, field and extractor metadata unset. Choose lot dimensions that fit the mesh; provide the usual `entrance/main` and freight-capable road access/site anchors. The profile supplies four jobs and consumes 12 Steel, 4 Metals and 2 Machinery per day to produce 42 Machinery (40 net). No factory-specific script or new asset schema is needed. Steel/Metals/Machinery imports work before upstream assets exist; see [`economy.md`](economy.md#machinery-upkeep-econ-09).
- `[building.extractor]`: optional explicit extraction contract with `resource` and `area_mode = "player_polygon"`.
- `[building.field]`: optional explicit agricultural field contract with `resource` and `area_mode = "player_polygon"`; the economy profile must be a `field_producer` that outputs the same resource, and its authored daily output is interpreted per 10,000 m2 of committed field area.
- Field/extractor economy profiles use `worker_capacity_area_m2` for staffing independently of hectare-based output. It defaults to 10,000 m2; the grain farm uses one worker per 100,000 m2. The economy editor exposes this as “Area for Worker Capacity (m²)”.
- `min_zone_width_cells`: integer, default `lot_width_cells`
- `min_zone_depth_cells`: integer, default `lot_depth_cells`
- `household_capacity`: integer, `>= 0`. Defines the number of distinct household slots (families). Required for residential. Explicit field-producing farms always provide exactly one slot; the editor displays this as a fixed value.
- `worker_capacity`: integer, `>= 0`. Defines the total staffing capacity. Required for commercial/industrial. Note: if an `economy_profile` is selected, this value is read authoritatively from the profile and cannot be overridden at the asset level.
- `flat_size_m2`: float, `>= 0.0`. The average interior living area per household. Used to derive compatible starter household size from a baseline area, adult-weighted members, and lighter child-weighted extra members. Farmhouses preserve this value through export/re-import, defaulting to 120 m2 at runtime when unspecified. Authoring loads retain raw optional values rather than replacing them with effective runtime defaults. Profile-derived staffing and the fixed one-family farm contract do not erase dormant authored capacity metadata.

Placement-mode interpretation:

- `placement_mode = "zoned_private"` means the building participates in painted zoning legality,
  demand-driven private spawn, rezoning, and ordinary upgrade-family rules
- `placement_mode = "explicit"` means the building is placed directly by player, scenario, or
  explicit city systems and does not participate in painted-zoning legality, demand-owned private
  spawn, or rezoning
- future city-owned service or utility buildings and landmarks belong to `placement_mode =
  "explicit"`

Optional upgrade fields:

- `level`: integer `>= 1`, default `1`. Declares which growth tier this asset represents within its family.
- `upgrade_family`: string, recommended for ordinary zoned private buildings. Current runtime compatibility note: this is still stored as top-level `asset_set` in the implemented schema today

Building families and upgrade levels:

- `upgrade_family` is the intended family key for buildings. In the current runtime and file format this still uses the field name `asset_set`.
- `upgrade_family` is only meaningful for `placement_mode = "zoned_private"` in baseline `v1`.
- Assets with the same `upgrade_family` must share the same `placement_mode`, `zone_type`,
  `density`, `lot_width_cells`, and `lot_depth_cells`; together those define one closed upgrade
  family.
- `level` must be unique within a family. Two assets with the same `upgrade_family` and `level` are a conflict; the runtime logs a warning. The second asset loaded wins.
- A building at level N upgrades to level N+1 in the same family when the runtime finds a registered asset with the same `upgrade_family` and `level = N+1`. No pointer in the manifest is required.
- Each family member is independently authorable. Creating a level-2 variant later never requires editing the level-1 file.
- A building with no `upgrade_family` belongs to no family and never upgrades. This is valid for true one-off buildings and landmarks, but it is risky as an accidental omission on ordinary zoned private buildings.
- Runtime family and zone/density queries borrow their string keys. The upgrade index uses
  direct tier buckets (at most 256 for the `u8` tier domain); tier 255 has no successor.
  These are the existing registry indices, not additional copies of asset identity.
  Audit acceptance (`AUDIT-01-AS1/AS2`): five alternating unprofiled release process pairs per
  CPU, pinned separately to CPUs 0 and 16, `RAYON_NUM_THREADS=24`. Each fixture registers
  128 or 2,048 families with two validated tiers; setup and correctness assertions are outside
  timing. Three warmups precede 11 samples of 64 sweeps, measuring upgrade, downgrade and
  zone/density lookup as one triplet. Median ns/triplet for 128 / 2,048 families:
  CPU 0 `103.339 / 148.413 → 75.215 / 123.192`; CPU 16
  `193.128 / 263.371 → 145.682 / 213.586`. This measures registry queries only.
  Command: `assets::registry::tests::benchmark_asset_registry_lookups --exact --ignored --nocapture`
  on the matched release test executables. Raw rows and executable/source identities are in
  `/tmp/metrum-full-audit/asset-registry-pinned-matched-bench.json`,
  `asset-registry-before-identity.json` and `asset-registry-tier-index-after-identity.json`.

- `lot_width_cells` and `lot_depth_cells` must be identical for all members of a family. The footprint does not change on upgrade; only the mesh and capacities change.
- `household_capacity`, `worker_capacity`, and `flat_size_m2` are tier-specific. A level-2 building may house more households or provide larger flats than a level-1 building of the same family.
- Cross-density change is not an ordinary family upgrade. If gameplay later wants a building to move
  into a different density band, that must happen through rezoning plus redevelopment or
  replacement rather than by crossing density inside one `upgrade_family`.

Recommended editor behavior:

- auto-fill `upgrade_family` when a new building asset is created instead of leaving it blank
- preserve the same `upgrade_family` when creating a higher-level variant from an existing building
- warn when a normal zoned private building has no `upgrade_family`
- require `upgrade_family` when `level > 1`
- keep an explicit way to clear the field for true one-off buildings or landmarks that should never upgrade

#### Zoning Registry Integration

The building inspector should not own hardcoded zoning-choice lists. It should consume the shipped
zoning-profile registry from [`zoning/profiles.toml`](../zoning/profiles.toml) and use the
validated runtime registry order defined in [`zoning.md`](zoning.md).

Deterministic editor rules:

- if `placement_mode = "zoned_private"`, load the shipped zoning-profile registry rather than
  maintaining editor-only `zone_type` or density lists
- present zoning choices in the same deterministic order used by the runtime UI:
  top-level `ZoneType` grouping, then `(ui_order, id)` inside each group
- if `placement_mode = "zoned_private"`, derive the available `zone_type` and `density` authoring
  choices from that loaded registry instead of from hardcoded defaults
- in baseline `v1`, the shipped registry exposes only `residential`, `commercial`, and
  `industrial`; any old editor-side `office` or `mixed` zoning controls should be removed rather
  than preserved as dead options
- if `placement_mode = "zoned_private"`, still write the asset's baseline `zone_type` and
  `density` fields into `asset.toml`; the editor does not write `ZoneProfileId` into building
  assets
- if `placement_mode = "zoned_private"`, validate authored `zone_type` and `density` against the
  loaded registry before export
- if `placement_mode = "explicit"`, do not write or validate zoning fields against the painted
  zoning-profile registry
- if the editor later shows compatible `ZoneProfile`s for a building, that compatibility view must
  be derived from the same loaded registry plus the zoning legality rules, not from a second editor
  heuristic
- if later site-specific filters such as corner-capable assets are surfaced in the UI, they should
  appear as explicit derived compatibility information rather than as hidden overrides of
  `zone_type` or `density`

#### Capacity estimation

The asset editor auto-suggests capacity values when a mesh is scaled. The formulas are:

```
floors         = max(1, round(scaled_height / 3.5))        # ~3.5 m per storey
res_floors     = max(1, round(scaled_height × 0.65 / 3.5)) # residential: 35% height discount for roof pitch
footprint = scaled_width × scaled_depth           # m²
```

m² per person/worker by zone and density:

| Zone | Low | Medium | High |
|------|-----|--------|------|
| residential | 30 m²/person | 20 m²/person | 12 m²/person |
| commercial | 20 m²/worker | 15 m²/worker | 10 m²/worker |
| industrial | 25 m²/worker | 19 m²/worker | 13 m²/worker |

`level` does not affect the suggestion — capacity scaling by level is deferred until the wealth/money system is implemented.

These are starting-point estimates only. Adjust before export to reflect the intended simulation density.

Building rules:

- `placement_mode = "zoned_private"` requires both `zone_type` and `density`.
- `placement_mode = "explicit"` forbids `zone_type` and `density`.
- `placement_mode = "zoned_private"` and `zone_type = "residential"` requires
  `household_capacity` and must not use `worker_capacity`.
- `placement_mode = "zoned_private"` and `zone_type = "commercial"` or `industrial` requires
  `worker_capacity`.
- `density` is independent of `zone_type` when `placement_mode = "zoned_private"`. A
  `residential / high` building is a high-density apartment; `residential / low` is a detached
  house. The zoning system uses `zone_type + density` together as the baseline
  placement-legality keys.
- `level` is a growth-tier field inside an upgrade family. It does not make an otherwise illegal zone-type or density combination legal.
- `upgrade_family` must not cross `zone_type` or `density` for `placement_mode = "zoned_private"`.
- `placement_mode = "explicit"` buildings should omit `upgrade_family` in baseline `v1`.
- shared asset `tags` may later act as additional zoning or build-site filters when a `ZoneProfile` explicitly requires them, but tags do not override `zone_type` or `density`.
- The asset editor should validate `zone_type` and `density` against the loaded shipped
  zoning-profile registry only when `placement_mode = "zoned_private"` so content authoring stays
  aligned with the live zoning data rather than with hardcoded editor defaults.
- `service_class = "none"` is the default for ordinary zoned private buildings.
- `placement_mode = "zoned_private"` must not export a non-`none` `service_class`.
- `placement_mode = "explicit"` may use `service_class = "none"` for landmarks or a non-`none`
  value for explicit service or utility buildings.
- Explicit utility service classes (`power`, `water`, `waste`) require an `economy_profile` that
  resolves to a utility producer or processor for the corresponding runtime utility service. The
  `waste` asset-side service class corresponds to runtime `utility_service = "sewage"`.
- `placement_mode = "zoned_private"` must not export `[building.extractor]` or `[building.field]`.
- `placement_mode = "explicit"` may export either `[building.extractor]` or `[building.field]`, but not both.
- Explicit extractor assets require an `economy_profile` that resolves to an `extractor` profile outputting the same resource.
- Explicit field assets require an `economy_profile` that resolves to a `field_producer` profile outputting the same resource.
- Exactly one `[[anchors]]` entry with `type = "entrance"` and `name = "main"` is required.
- `[building].frontage_forward` defines the asset-local frontage direction used by building
  placement, rendering, and entrance-cache derivation. Older assets that omit it fall back first to
  an authored driveway's road-facing edge direction, then to the `main` entrance anchor's `forward`
  vector.
- Additional building-side site points use `type = "driveway"`, `type = "parking"`, or
  `type = "loading_bay"`, not a second generic `entrance` anchor.
- Driveway anchors require cardinal `[building].frontage_forward`, must lie on that frontage edge,
  and must point inward into the lot.
- Driveway, parking, and loading-bay footprints must fit fully inside the authored lot rectangle.
- In v1, driveway, parking, and loading-bay anchors are not rendered as ground treatment. Authored
  `[[site_surfaces]]` polygons own asphalt, concrete, walkways, parking pads, loading pads, and
  driveway-apron visuals in both the asset editor preview and live gameplay.
- The generic entrance/exit runtime uses only the `main` entrance anchor and does not interpret
  site-anchor capacity, queue behavior, parking, or freight stop behavior yet.
- Every `[[site_surfaces]]` polygon must fit fully inside the authored lot rectangle.
- In the normal case, `min_zone_*` equals the footprint size.
- `min_zone_*` reserves room for future yard or setback support without changing the core format.
- `employment_type` is not part of the v1 building schema. If job-category metadata is needed later, add it as a later extension.

### V1 Prop `asset.toml` Schema

Required fields:

- `asset_class = "prop"`
- `mesh`: relative path to mesh
- `category`: lower-case slug
- `bounding_size_m`: `[f32, f32, f32]`
- `snap_mode`: enum, one of `free`, `grid`, `edge`, `surface`
- `terrain_behavior`: enum, one of `flat_ground`, `conform_to_surface`, `hang_from_surface`

Prop rules:

- V1 props are explicitly placed standalone assets, not procedural road rules.
- V1 does not support "place every N meters along this road edge" or similar generator behavior.
- `surface` snap is for authored placement on a visible surface in the preview scene, not arbitrary runtime attachment to other packs.

### V1 Vehicle `asset.toml` Schema

Required fields:

- `asset_class = "vehicle"`
- `mesh`: relative path to `LOD0` mesh
- `vehicle_class`: enum, one of `civil`, `police`, `fire`, `ambulance`, `utility`, `bus`
- `vehicle_family`: enum, one of `sedan`, `suv`, `van`, `truck`, `bus`
- `length_m`: `f32`, `> 0`
- `width_m`: `f32`, `> 0`
- `height_m`: `f32`, `> 0`

Optional fields:

- `color_variants`: array of strings

Vehicle rules:

- Newly imported vehicles must be baked into the canonical `+Z` forward convention at export time.
- The hidden legacy `180°` runtime yaw correction is for built-in compatibility content only, not new imported assets.
- Wheel and light markers use optional `[[anchors]]` entries with `type = "wheel"` and `type = "light"`.

### V1 Character `asset.toml` Schema

Required fields:

- `asset_class = "character"`
- `archetype_family`: enum, one of `adult_male`, `adult_female`, `child`
- `age_group`: enum, one of `adult`, `child`
- `body_type`: lower-case slug

Required `[runtime_vat]` table:

- `rest_mesh`: relative path to baked runtime rest mesh
- `animation_texture`: relative path to baked VAT texture
- `frame_count`: integer, `>= 1`
- `fps`: `f32`, `> 0`

Character rules:

- VAT bake authoring requires a source `walk` clip in v1.
- The baked output must be self-contained from the author's point of view: opening the shipped editor and pressing bake is enough.
- The exported runtime character asset stores baked runtime outputs only.
- Character source inputs are editor-only authoring data and are not part of the runtime pack.
- Runtime skeletal playback is out of scope. The shipped output is the baked VAT representation, not the source skeleton.

Example `pack.toml`:

```toml
schema_version = 1
pack_id = "kenney_city_pack"
display_name = "Kenney City Pack"
version = "1.0.0"
author = "Metrum Rise Team"
license = "CC0-1.0"
description = "Low-poly starter city assets."
enabled_by_default = true
```

Example building `asset.toml`:

```toml
asset_id = "building.residential.lowrise_corner"
display_name = "Residential Lowrise Corner"
thumbnail = "thumbnail.png"
asset_set = "kenney"

[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 3
lot_depth_cells = 3
frontage_forward = [0.0, 0.0, 1.0]
min_zone_width_cells = 3
min_zone_depth_cells = 3
household_capacity = 6
flat_size_m2 = 85.0
tags = ["lowrise", "starter", "suburban"]

[attribution]
author = "Kenney"
license = "CC0-1.0"
source = "https://kenney.nl/"

[orientation]
up_axis = "+Y"
forward_axis = "+Z"
origin = "footprint_center"

[materials]
albedo = "albedo.png"
normal = "normal.png"
flip_v_albedo = false
flip_v_normal = false

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 4.5]
forward = [0.0, 0.0, 1.0]

[[mesh_parts]]
name = "main"
position = [0.0, 0.0, 0.0]
rotation_degrees = [0.0, 0.0, 0.0]
scale = 1.0
pivot_offset = [0.0, 0.0, 0.0]

[[mesh_parts.lods]]
file = "mesh.glb"
distance_min_m = 0.0
distance_max_m = 75.0

[[mesh_parts.lods]]
file = "mesh_lod1.glb"
distance_min_m = 75.0
distance_max_m = 250.0

[[mesh_parts.lods]]
file = "mesh_lod2.glb"
distance_min_m = 250.0
distance_max_m = 1000.0

[[mesh_parts]]
name = "side_building"
position = [12.0, 0.0, 0.0]
rotation_degrees = [0.0, 90.0, 0.0]
scale = 1.0

[[mesh_parts.lods]]
file = "side_building.glb"
distance_min_m = 0.0
```

The runtime derives the fully qualified identifier from the pack and asset manifests:

```text
kenney_city_pack:building.residential.lowrise_corner
```

## Validation Rules

Implemented manifest checks reject non-finite mesh-part position/rotation/pivot values, non-finite
or negative living area, and non-finite or non-positive vehicle dimensions. Prop bounds must be
finite and nonnegative, preserving planar props with zero-height bounds. Building mesh parts and
declared non-building LOD lists share file/range/order validation; character source manifests may
still omit LODs. These checks run before registration and rendering.

The validator is strict. Invalid content fails before it enters a playable build.

Hard errors:

- duplicate `pack_id` among installed packs
- duplicate `asset_id` inside one pack
- invalid `pack_id` or `asset_id` grammar
- referenced file escapes the asset root or uses `..`
- missing required textures or generated outputs
- lot size outside supported runtime bounds
- missing `placement_mode` on buildings
- missing `zone_type` on `placement_mode = "zoned_private"` buildings
- missing `density` on `placement_mode = "zoned_private"` buildings
- `zone_type` or `density` present on `placement_mode = "explicit"` buildings
- `[building.extractor]` or `[building.field]` present on `placement_mode = "zoned_private"` buildings
- both `[building.extractor]` and `[building.field]` present on the same building
- one `upgrade_family` spanning multiple `zone_type` values
- one `upgrade_family` spanning multiple `density` values
- one `upgrade_family` spanning multiple footprint sizes
- `upgrade_family` present on `placement_mode = "explicit"` buildings
- missing canonical mesh file
- invalid enum value for asset class, placement mode, zone type, prop snap mode, prop terrain behavior, vehicle taxonomy, or character archetype fields
- invalid axes / origin conventions
- character VAT authoring input lacks required `walk` source clip

Warnings:

- excessive triangle counts
- too many materials
- no additional approved LOD tiers beyond `LOD0`
- LOD distance ranges overlap or leave gaps
- bounding box exceeds declared footprint
- thumbnail missing
- no license / attribution metadata

## Implementation Plan

### Phase 0: Schema, Scanner, And Registry

- Lock the v1 `pack.toml` / `asset.toml` schema, scanner rules, and `asset_id` grammar in both Rust and the editor UI.
- Add a pack registry and enable-disable list.
- Add runtime loading that reads manifests, not hardcoded directory assumptions.
- Redesign zoning storage so plot size is bounded by painted area, not by a fixed global `ZONING_DEPTH`.

### Phase 1: Building Importer

- Create the separate editor scene/executable.
- Support `.glb` import, thumbnail generation, metadata editing, variable lot-size authoring, lot-size validation, and pack saving.
- Hook building metadata into the existing variant and footprint systems.
- Replace fixed-depth zoning assumptions in storage, obstruction passes, rendering, and spawning so authored lot dimensions and runtime lot dimensions agree.
- Fix stale runtime `3x3` assumptions and building scale handling so authored lot dimensions and rendered dimensions agree.

### Phase 2: Prop And Vehicle Importer

- Add explicit prop placement authoring using the v1 `snap_mode` and `terrain_behavior` contract.
- Add vehicle-class templates and lane-scale preview scenes.
- Support static meshes, color variants, thumbnails, pack membership, and import-time vehicle orientation normalization.

### Phase 3: Character Source Bake Pipeline

- Add character source import and clip preview.
- Add self-contained offline baking to the current runtime format:
  - VAT outputs from source clips
- Export baked runtime outputs into the runtime pack only.
- Leave room for a future optional far-distance crowd tier:
  - SDF billboard descriptor
- Keep the shipped runtime asset free of skeleton cost.

### Later

- Add in-game content-manager polish, compatibility metadata, redirect handling, workspace support, manifest caches, cross-pack library resources, and signing if/when the moddable ecosystem needs them.
- Define road assets as lane/topology/material templates, not as ordinary imported meshes.

## Implementation Summary

### Task-oriented building authoring — TOOLS-06

Implemented. This replaces the all-fields importer workflow while retaining the existing mesh loader,
screen-size LOD policy and staged package publisher. The shipped contract is:

- With no document open, a welcome screen offers New asset and Browse asset library; File retains Open Draft.
  There is no dummy building, visible lot, inspector, validation count or automatic modal;
  document-only menu actions are disabled. Cancelling creation/open leaves the welcome state intact.
- New asset asks for Residential, Commercial, Industrial, Resource extractor, Farm,
  Service/utility (with subtype), or Other explicitly placed building, then name, destination
  pack and optional model. These are presets of existing Rust contracts, not runtime categories.
  Unsupported asset classes and unimplemented simulation options are not offered.
- Freely navigable **Overview / Model / Site / Gameplay / Validate & export** tasks.
  Overview owns identity, type, pack, thumbnail and completion. Model owns parts, transforms,
  and LOD inspection. Site owns footprint, frontage, entrances, access and surfaces.
  Gameplay displays only applicable fields; Advanced contains uncommon but relevant fields only.
- New/opened assets start in Model; Overview is an explicitly selected metadata task. Empty Model
  invites import rather than showing an empty parts list. Selection-only removal/framing actions
  and Clear comparison are hidden until applicable. Selected parts expose the LOD list and next-tier
  add/selected-tier replacement controls immediately, with Placement collapsed
  below. Pack management uses a dialog.
- Direct all-object selection opens the corresponding settings; optional filters are never tied
  to inspector tasks. Cached triangle-accurate mesh picking, clickable site labels, screen-sized
  handles, hover feedback, Alt+click cycling and cancellable thresholded gestures replace ground
  footprint picking. Visible anchor guides win over underlying yards; yard previews honor authored height.
- The viewport takes remaining space around a resizable, collapsible inspector: 420 logical UI
  pixels by default, bounded to 340–600 and available viewport space (normally at most 45% of
  the right split). Library and diagnostics open on demand. The window minimum is 960×640 logical
  pixels; UI scaling also scales that minimum. Existing text padding and font sizes are retained.
- `workspace_layout.gd` owns event-driven pane sizing, separate from document/session logic.
  It waits for container sorting and normalizes visually clamped divider offsets before resizing.
  Saved widths are bounded; transient startup/hidden-pane measurements never overwrite preferences.
  Intentional inspector width/collapse survive reopening and temporary window constraints.
  **View → Reset layout** restores defaults, expands the inspector and closes library/diagnostics.
- Residential never exposes mining/farm/service/employment controls. Rust filters profiles by
  executable behavior, resource and utility compatibility. Profile staffing is derived/read-only,
  including explicit-work-area staffing density; authors cannot edit a competing worker value.
- Widget-independent documents, saveable incomplete drafts, open/save/save-as, bounded grouped
  undo/redo (including geometry and type changes), and unsaved-change guards for document replacement,
  leaving the editor and closing the window. A successful runtime export establishes the clean
  revision, so New/Open/Close do not request a draft save until further edits. Undo/redo compares
  against that revision; rejected exports leave changes unsaved. Export remains separate from
  draft save and never writes or changes the associated draft path.
- Inline validation has field/task destinations, including a missing main entrance. Export uses
  the same Rust validation as preflight and remains staged/rollback-safe. Toolbar/File Export asset
  opens the destination/validation review and expands a collapsed inspector; publishing still requires
  the review's explicit export action. Draft commands remain secondary File actions with existing shortcuts.
- Type conversion previews every changed field and requires confirmation. Loading or hiding fields
  never clears authored data. Unsupported/unresolved metadata remains in drafts and must be surfaced
  for explicit resolution rather than silently normalized on export. Preview-only state stays out
  of asset documents. Existing thumbnails, complete LOD chains and precise transforms survive.

`assets/authoring.rs` and `AssetAuthoringPolicy` supply a single cached inspection for
capabilities and addressed diagnostics, plus explicit conversion previews. Rust
`assets/authoring/document.rs` owns savepoints and 100-command history through
`AssetAuthoringDocument`; immutable revisions are shared, with typed deep copies only at the
Godot boundary. Undo/redo and transaction setup are O(1). `assets/authoring/files.rs` and
`AssetAuthoringFiles` own versioned, object-disabled drafts, pack creation, dependency discovery,
staging, publication and source-asset removal. Snapshot work is O(document size) per authoring command, not per frame; capability
queries are O(P log P + P log R + ports) for P profiles and R resources, independent of city size.
`authoring_session.gd` coordinates dialogs and Rust commands; `document_adapter.gd` projects geometry without
repairing it, and `task_panels.gd` builds the task-specific views. Metadata-only edits do not reload
models or rebuild preview geometry. Pointer motion updates preview state; release records one
command. The idle camera/LOD check is O(1); changed camera/geometry LOD work is O(document parts).
Missing LOD0 sources reserve their part index and can be explicitly relinked without changing
placement. Invalid hidden profile/capacity/classification bindings have review-and-confirm
corrections; unknown fields remain in drafts and block unsupported runtime export.

Precommit audit (2026-09-20): mesh/access lists use the actual multi-selection signal; plain
clicks clear unrelated selections and Ctrl+click preserves/toggles the selected group. Added real
list-click regressions. LOD edits retain unknown per-tier metadata and omitted fields; relinking
initializes absent source lists reversibly. Incomplete yards and malformed numeric/vector draft
values render safe display defaults without changing the document. Publication preserves dormant
anchor dimensions and rejects unknown placement modes, orphaned area metadata and profiles
inapplicable to the asset type. The unused direct-write SimulationNode exporter is removed:
runtime publication has one staged file-service implementation.

Fresh audit verification: `cargo test` passes **1,833 tests (63 ignored)**; `cargo check --benches`,
`cargo doc --no-deps`, release build, formatting and shell/diff checks pass without warnings.
All six asset-editor headless suites pass using the isolated project created by `run.sh --test`.
Adjacent machinery, camera-save/load, selection-gesture, UI-settings, day-cycle and building-asset
reload suites pass. Four X11 / Forward+ rendered suites (selection, layout, workspace and preview)
exit successfully; inspected captures cover task spacing and direct access-point selection.
Logs/captures: `/tmp/metrum-precommit.Dl9Msd/`; headless asset logs:
`/tmp/metrum-asset-tests.7xPBx4/`. Deployed release library SHA-256:
`5a5ddeb006068a5818dbf23394922352a29008e1457543693ce14918aab6a852`.

Two sequential unprofiled release-extension runs use Godot 4.7.2, four Rayon workers and separate
XDG profiles. Commands: `godot --headless --path <isolated-project> --script
res://tests/asset_{document,selection,workspace}_test.gd -- --asset-editor
--benchmark-asset-{document,selection,workspace}` (one matching script/flag pair per process).
The existing two-part/one-anchor/one-yard fixture measures 0.743 / 0.731 µs idle,
22.883 / 23.764 µs per query; with reference enabled, queries take 28.546 / 29.010 µs and
locked drag motion 8.784 / 8.816 µs. Idle queries and motion BVH rebuilds remain zero.
Document undo averages 0.870 / 0.860 µs with 100 entries and 0.900 / 0.890 µs with 1,000 entries;
workspace metadata edits take 298.280 / 293.530 µs without model imports. Workload iteration
counts are logged in `bench-{document,selection,workspace}-{1,2}.log` in the audit directory.
These are current-build editor CPU measurements, not a matched before/after or city/GPU comparison.
`RAYON_NUM_THREADS=4 cargo bench --bench asset_lod_benchmark` measures 37.610 ns for projection
plus selection and 4.066 ns for selection alone (`lod-benchmark.log`); prior Criterion percentage
comparisons are not treated as matched baselines. Editor/fixture source SHA-256:
`4e76b81dbee6d11cfa130c9b542ea4d724ee42c61f8a7e5a75e5f4c89bcd8cf7`
(`sha256sum godot/scripts/editors/asset_editor/*.gd godot/scripts/editors/asset_editor.gd godot/scripts/core/editor_camera_input.gd godot/scripts/renderers/building_preview.gd godot/tests/asset_*test.gd | sha256sum`).

Initial authoring verification (2026-09-19): seven `assets::authoring` and nineteen `asset_export::tests` Rust tests;
headless `asset_document_test.gd`, `asset_authoring_test.gd`, `asset_workspace_test.gd`, and
`asset_editor_preview_test.gd`. The real workspace test covers all seven presets' publication/reopen,
all ten service subtypes' applicability, compatible profiles, conversion cancel/confirm/undo/redo,
incomplete draft reopen, geometry and vertex history, save/discard/cancel/failure guards, WM-close
protection, exact metadata preservation, source relinking and issue navigation. The four-tier
preview/package regression remains in the suite. `run.sh --test` gives asset regressions a unique
project identity plus temporary XDG data/config paths, and fails on engine/script errors as well
as nonzero exit status. The unique identity also separates macOS fixtures from actual user data;
the runner prints their location. The runner was verified on Linux; macOS execution remains untested.
The adjacent camera save/load and machinery bridge checks also pass. One concurrent debug machinery
run printed PASS then crashed on shutdown; an isolated debug rerun and three sequential release
reruns exited cleanly. Its cause is unconfirmed; the final asset regressions run sequentially.

Native Wayland / Forward+ rendered checks on Radeon RX 7900 XTX cover dark/light task layouts,
Model sub-sections, bounded creation/pack/preview dialogs, thumbnail capture/undo/draft/publication/reopen, unclipped chrome and pane-centered
framing at 1280×900 and 960×640. Artifacts: `/tmp/metrum-authoring.DVzbwb/captures/` and sibling logs.

Two matched unprofiled release measurements used the generated one-part box fixture, Godot 4.7.2,
`RAYON_NUM_THREADS=4`, 100 metadata commands and 10,000 idle camera/LOD updates. Command:
`godot --headless --path godot --script res://tests/asset_workspace_test.gd -- --asset-editor --benchmark-asset-workspace`
with isolated `XDG_DATA_HOME` / `XDG_CONFIG_HOME`. Metadata commands averaged 261.220 / 259.010 µs;
idle updates 1.560 / 1.556 µs, with zero model imports and zero idle per-part policy evaluations.
Field edits style only the changed validation subtree, not every editor dialog.
Release library SHA-256: `b50cba91e17a35d7d0257d77bd263e934593421d1b3a89ba5f88442507ab726b`.
Editor/fixture source digest: `6bca33a1f49e61e9699d2123354bc92069b3b0fc549d1953a0a52468e07f9055`
(`sha256sum godot/scripts/editors/asset_editor/*.gd godot/scripts/editors/asset_editor.gd godot/scripts/core/editor_camera_input.gd godot/tests/asset_workspace_test.gd | sha256sum`).
Logs: `/tmp/metrum-authoring.DVzbwb/workspace-final-release-{1,2}.log`. These measure editor command/idle
cost only, not city-scale rendering or GPU performance. Roadside/traffic scenes, autosave/recovery,
standardized thumbnail rigs and gameplay LOD integration remain separate work.

Startup/layout follow-up verification (2026-09-19): all five targeted headless asset regressions
pass, including the new `asset_layout_test.gd` in `run.sh --test`. It covers a fresh welcome,
cancelled dialogs, import-ready Model, selection/comparison actions, library activation into a
real published fixture, 1,699-pixel stale preferences, drag limits, collapsed/resized restoration,
Reset layout, idle preference stability, and 960×640 / 1440×900 / 3063×1751 layouts at 100/150/200%.
The same regression passes rendered on native Wayland / Forward+ in both themes; settled captures
are at `/tmp/metrum-layout.zFcGpF/verified-captures/`, with sibling test logs. Fixtures are generated
in isolated user profiles. Layout work is O(1) per coalesced UI layout event and adds no idle loop.
Two sequential, unprofiled release runs of the existing workspace benchmark command above used
separate isolated profiles and `RAYON_NUM_THREADS=4`, with the same release library hash above.
Metadata edits averaged 276.770 / 275.820 µs; idle updates 1.590 / 1.564 µs; imports and idle
per-part policy evaluations remained zero. Logs: `/tmp/metrum-layout.zFcGpF/acceptance-{1,2}.log`.
Follow-up source digest: `beab756bd646191b6e97cefde14c6b52bb8e821d4b2d5697436c9ffcba8b3148`
(`sha256sum godot/scripts/editors/asset_editor/*.gd godot/scripts/editors/asset_editor.gd godot/scripts/core/editor_camera_input.gd godot/scripts/ui/top_menu.gd godot/tests/asset_workspace_test.gd godot/tests/asset_layout_test.gd | sha256sum`).
The earlier Rust checks and timing/capture artifacts describe the original authoring build;
this follow-up changes only editor UI/workflow and its regressions, not simulation or asset contracts.

Export discoverability verification (2026-09-20): all six headless asset suites pass with the existing
extension. The layout suite exercises toolbar/File export for incomplete and valid assets, collapsed
inspector recovery, non-publishing review, disabled empty-state export and retained File draft commands.
Rendered X11 / Forward+ layout checks pass in both themes at 100/150/200% UI scale; logs and captures:
`/tmp/metrum-export-navigation.V3RSSv/` (`*-accepted.log`, `rendered-layout.log`, `captures/`).
This is event-driven UI navigation using existing validation; no new idle work or simulation changes.

LOD workflow verification (2026-09-20): all six headless asset suites and rendered X11 / Forward+
preview/layout suites pass. Generated fixtures exercise visible Add LOD1–3, immediate preview,
selected-tier replacement preserving other sources/transforms/bands, undo/redo, removal and automatic
selection. Layout assertions keep Add LOD on-screen at 960×640 through 3063×1751 and 100/150/200% UI
scale. Short lists fit their rows; longer lists scroll after five rows. Artifacts:
`/tmp/metrum-lod-workflow.Guc0R7/` (`*-accepted.log`, `accepted-render-*.log`, `accepted-*-captures/`).

Matched sequential unprofiled measurements used the existing one-part synthetic-box workspace
benchmark, Godot 4.7.2 and `RAYON_NUM_THREADS=4`, with separate isolated XDG profiles and no renderer
running concurrently. Command: `godot --headless --path /tmp/metrum-reference-label.wfAwYu/project
--script res://tests/asset_workspace_test.gd -- --asset-editor --benchmark-asset-workspace`.
For 100 metadata edits / 10,000 idle updates, before means were 303.770 / 290.700 µs per edit and
1.621 / 1.594 µs idle; after means were 290.900 / 324.760 µs per edit and 1.562 / 1.585 µs idle.
All runs had zero reimports and zero idle per-part policy evaluations. List rebuilds remain O(LODs)
on selection/document changes; fitting their visible height is O(1), with no new idle work.
Logs: `before-{1,2}.log`, `accepted-after-{1,2}.log` in the artifact directory. Unchanged release
library SHA-256: `5a5ddeb006068a5818dbf23394922352a29008e1457543693ce14918aab6a852`;
workflow source/test digest: `c828b0e156cdb66705ef8db039995d626b4f8a0d1b90d1ded54005c928abb379`.
These are editor-input measurements, not city-renderer performance or fresh Rust-suite validation.

Visible-LOD selection follow-up (2026-09-20): all six headless asset suites and rendered X11 /
Forward+ preview/selection suites pass. Tests cover distant-mesh clicks, automatic refinement,
part switching, replacement of the actual rendered tier, explicit clicks on an already highlighted
row, and failed-tier repair. Artifacts: `/tmp/metrum-visible-lod.3iHUKm/` (`*-final.log`,
`render-{preview,selection}.log`, `{preview,selection}-captures/`). No Rust or asset-format changes.
Highlight updates are O(1), do not rebuild the list, and leave idle policy evaluation at zero.
Two matched sequential unprofiled runs used the unchanged release library above, four Rayon workers,
isolated XDG profiles and `godot --headless --path /tmp/metrum-reference-label.wfAwYu/project
--script res://tests/asset_editor_preview_test.gd -- --asset-editor --benchmark-asset-preview`.
With one part/four LODs, 100 warm-up transitions then 2,000 alternating 32/900-pixel camera updates,
mean CPU update cost was 7.966 / 8.362 µs before and 9.389 / 9.467 µs after; the extra live row/button
synchronization adds about 1.3 µs per transition, with zero mesh imports and unchanged O(parts)
camera-update complexity. Logs: `before-{1,2}.log`, `after-{1,2}.log` in that directory. These measure
synchronous preview updates, not GPU frame time. Panel/controller/preview-test/selection-test digest:
`b5dbe3f7050e5c7be4128e5050898c8ac7ec937139b46ef737c7d8127b154da1`.

Temporary-inspection correction (2026-09-20): the previous follow-up still left Add/Replace and
row selection permanently forced, so zooming close could remain on LOD3. A generated four-tier
import/replacement regression reproduced this before the fix. Camera transform/projection changes
now resume Automatic, including on unselected parts, with no list rebuild or source mutation.
All six headless suites and rendered X11 / Forward+ preview/selection suites pass, including
close perspective picking after coarse-tier inspection, immediate inspection at a changed view,
and idle inspection retention. Artifacts: `/tmp/metrum-lod-resume.Gwl32p/` (`repro.log`,
`*-final.log`, `render-{preview,selection}.log`, `{preview,selection}-captures/`).
Two sequential unprofiled runs of the same command/workload above, unchanged release extension,
four Rayon workers and isolated XDG profiles measured 9.349 / 9.414 µs before and 9.679 / 9.534 µs
after per automatic transition, with zero imports; idle policy evaluations remain zero.
Logs: `before-{1,2}.log`, `after-{1,2}.log`. Complexity stays O(1) idle and O(parts) on camera
changes; this measures editor CPU updates, not GPU/city rendering. No Rust suite rerun.
Source digest (`sha256sum godot/scripts/editors/asset_editor.gd
godot/scripts/editors/asset_editor/preview_panel.gd godot/scripts/editors/asset_editor/preview_lod.gd
godot/tests/asset_editor_preview_test.gd godot/tests/asset_selection_test.gd | sha256sum`):
`39559648a416debfbedf9b5391632ed694e144b28631594aaa1f0194881bd2d7`.

LOD-details presentation follow-up (2026-09-20): headless preview/layout and rendered X11 /
Forward+ preview checks pass. Regression covers collapsed-by-default projected size, expansion,
live updates in both states and clearing the asset. Artifacts: `/tmp/metrum-lod-details.Ug1k2q/`
(`after-{1,2}.log`, `layout.log`, `render-preview.log`, `captures/`). Using the same benchmark
command, release extension, four workers, isolated profiles and sequential workload above,
automatic transitions measured 9.759 / 9.640 µs before and 10.027 / 10.228 µs after, with zero
imports and zero idle policy evaluations. The label split preserves O(1) selected-panel work;
LOD policy is unchanged. Panel/preview-test digest (`sha256sum` of those two files, then
`sha256sum`): `52bcdca47a71e2170c05d30e5fadcbc1ef4b216e25186594cbdf592eadf765fb`.

Source-material summary removal (2026-09-20): all six headless asset suites and rendered X11 /
Forward+ workspace/preview suites pass, including all-tier night emission and source-preserving
export. The unused UI, catalog and selection-time deep copy/formatting are removed; emission
surface capture is unchanged. Artifacts: `/tmp/metrum-remove-materials.cCbgJQ/` (`asset*.log`,
`render-*.log`, `*-captures/`). Two matched sequential unprofiled workspace runs used the unchanged
release extension above, `RAYON_NUM_THREADS=4`, isolated XDG profiles and command
`godot --headless --path /tmp/metrum-reference-label.wfAwYu/project --script
res://tests/asset_workspace_test.gd -- --asset-editor --benchmark-asset-workspace`.
For 100 metadata edits / 10,000 idle updates, before means were 289.410 / 286.040 µs per edit
and 1.565 / 1.614 µs idle; after means were 275.700 / 287.700 µs and 1.590 / 1.617 µs.
Zero imports and idle policy evaluations in every run (`before-{1,2}.log`, `after-{1,2}.log`).
This removes O(material surfaces) summary storage/copying; existing import traversal and idle
complexity are unchanged. No Rust or city-renderer changes; no Rust suite rerun.

Initial selection follow-up verification (2026-09-19, before the direct-click correction below): all six headless asset regressions pass, including
`asset_selection_test.gd`, now included in `run.sh --test`. Generated fixtures cover oblique roof
hits, empty space inside bounds, nested transforms, visible LODs, elevated yards/anchors, missing
and relinked sources, task/explicit filters, overlap cycling, filtered box and mixed selection,
zoom/UI-scaled handles, click jitter, drag/rotation/vertex undo, cancellation, inspector focus,
modal blocking, comparison meshes and cache invalidation. Native Wayland / Forward+ checks pass
with dark/light hover captures in `/tmp/metrum-selection.KzJu4V/final-captures/`; sibling logs hold
the six headless results and `native-final.log`. This follow-up does not change Rust or run the full
repository test suite; earlier Rust results remain historical evidence for the unchanged library.

`asset_selection.gd` owns gestures/hover state, `asset_picker.gd` collects deterministic hits, and
`picking_overlay.gd` draws visible screen-space handles; `building_preview.gd` draws depth-tested
world-space hover segments. `mesh_pick_geometry.gd` reuses Godot's native
`TriangleMesh` BVH inside the existing per-part/per-LOD preview cache, with no physics bodies or
new city index. Import builds O(T) cached triangle/BVH storage; transforms and warmed LOD swaps
reuse it. Idle hover checks are O(1), with no geometry queries or document copies. A changed query
visits this asset's mesh instances M, anchors A and yard vertices V, plus intersected BVH nodes;
ordering H hits costs O(H log H). BVH traversal is typically sublinear in triangles but O(T) in
the worst case. None of this iterates over city buildings or agents.

Handle occlusion reuses those same BVHs and caches one visibility result per fixed point. Camera,
projection, mesh/LOD and site-geometry changes invalidate it; pointer-only changes, theme changes
and moving the scale/comparison helpers do not. Rebuilding H handle visibilities costs H bounded
asset-local BVH queries; cached pointer redraw stays O(H), and idle remains O(1). No city index,
physics bodies or per-idle-frame rays are added.

Two sequential, unprofiled release acceptance runs use Godot 4.7.2, `RAYON_NUM_THREADS=4`, isolated
`XDG_DATA_HOME` / `XDG_CONFIG_HOME`, and the release library SHA-256 recorded above. Command:
`godot --headless --path godot --script res://tests/asset_selection_test.gd -- --asset-editor --benchmark-asset-selection`.
The overlapping two-mesh/one-anchor/one-yard fixture averaged **0.776 / 0.755 µs** per idle check
(10,000 iterations, zero geometry queries), and **15.329 / 14.997 µs** per All-filter query
(1,000 iterations, zero cache rebuilds). Dense native-BVH measurements separate setup from querying:

| Triangles per mesh | Instances | Cache build µs, runs 1 / 2 | Query mean µs, runs 1 / 2 |
|---:|---:|---:|---:|
| 4,224 | 1 | 1,257 / 1,243 | 3.680 / 3.642 |
| 4,224 | 16 | 1,215 / 1,229 | 7.134 / 7.216 |
| 66,048 | 1 | 21,373 / 21,532 | 5.677 / 5.698 |
| 66,048 | 16 | 21,090 / 21,603 | 9.187 / 9.264 |

These generated sphere instances share one mesh resource; one lies under the ray and the others
exercise AABB rejection. Each row has 1,000 queries. These are editor CPU acceptance measurements,
not before/after or GPU/city-scale comparisons. Logs: `/tmp/metrum-selection.KzJu4V/acceptance-{1,2}.log`.
Source digest: `38092abd8599810d5bb1c6003e79756ac6f941f1916316f37d3e97d15141be18`
(`sha256sum godot/scripts/editors/asset_editor/*.gd godot/scripts/editors/asset_editor.gd godot/scripts/core/editor_camera_input.gd godot/scripts/renderers/building_preview.gd godot/tests/asset_selection_test.gd | sha256sum`).

Direct-click correction (2026-09-19): task-linked filtering was removed from the default workflow.
Normal clicks select every object kind and open its settings, including reopening a collapsed
inspector. Guide/text priority matches their visible overlay layer; billboard text bounds are
queried from current Label3D nodes, excluding stale labels awaiting deletion. A deferred bounds-ready
invalidation updates stationary hover after glyph layout; idle frames still do no geometry queries.
Layout persistence moved from PREDELETE to tree exit, and detached callbacks reject unavailable windows.
All six headless asset regressions and the native Wayland / Forward+ selection regression pass.
The added fixture routes actual viewport mouse events through three parking rectangles, loading,
driveway and their labels over a yard, from Model/Overview/Site, at 100/200% UI scaling. It verifies
automatic settings navigation and collapsed-inspector expansion. Layout checks now detach/free the
editor both after settling and with a pending layout callback. Logs and dark/light click captures:
`/tmp/metrum-anchor-fix.8hM65b/` (`native-selection.log`, `captures/*-access-click.png`).
Two sequential unprofiled release runs of the same selection benchmark command above, same Godot,
four Rayon workers and unchanged release library, used separate isolated profiles. Idle checks:
0.733 / 0.741 µs; All-object queries: 20.221 / 20.610 µs, with zero idle queries/cache rebuilds.
The 66,048-triangle, 16-instance BVH case measured 9.550 / 9.322 µs per query; builds remained separate
from query timing. Logs: `acceptance-{1,2}.log` in that directory. Bounds remain O(1) idle and
O(asset instances + anchors + yard vertices + visited BVH nodes + hit sorting) on changed queries.
Source digest: `6d71e4800f38f5524abab529bf9b15c322435faf6d33086e8e9065fd79d164b0`
(`sha256sum godot/scripts/editors/asset_editor/*.gd godot/scripts/editors/asset_editor.gd godot/scripts/core/editor_camera_input.gd godot/scripts/renderers/building_preview.gd godot/tests/asset_selection_test.gd godot/tests/asset_layout_test.gd | sha256sum`).
No Rust changes or fresh full-repository test run in this correction.

Scale-reference follow-up (2026-09-19): removed the shell's empty-click relocation path and
duplicated human-toggle state. The shared picker/gesture controller now handles the reference
and comparison helper without document transactions. The capsule is actually 1.8 m tall (the old
configuration used 1.4 m); its cached geometry is created once, not during overlay rebuilds.
All six headless asset regressions pass. The selection regression adds actual mouse events for
figure/handle selection, top-down and oblique dragging, 100/200% UI scale, overlap priority, movement
outside the lot, cancellation, empty-click deselection, rebuild persistence and undo/redo isolation.
Native Wayland / Forward+ checks also pass; dark/light captures were inspected. Artifacts:
`/tmp/metrum-scale-reference.03ZOi3/` (`final-asset_*test.log`, `render-verified.log`,
`captures-verified/*-scale-reference.png`, `verified-acceptance-{1,2}.log`).

Two sequential, unprofiled release-library acceptance runs used Godot 4.7.2, four Rayon workers,
separate isolated XDG profiles, and `godot --headless --path godot --script
res://tests/asset_selection_test.gd -- --asset-editor --benchmark-asset-selection`.
With two parts, one anchor, one yard and the reference enabled: 10,000 idle checks averaged
0.730 / 0.788 µs; 1,000 All-object queries averaged 26.976 / 26.751 µs; 1,000 locked-target drag
updates averaged 8.237 / 8.524 µs. Idle checks and drag motion perform no pick queries; motion/queries
perform no BVH rebuilds. The reference adds fixed-size geometry/handle work to the existing query
bound; reference translation is O(1), with O(anchor count) existing handle redraw work on motion,
and no authored-geometry capture. Idle remains O(1). These are editor CPU measurements, not city/GPU
benchmarks. The unchanged release library SHA-256 is
`b50cba91e17a35d7d0257d77bd263e934593421d1b3a89ba5f88442507ab726b`; the same combined source-digest
command above yields `f5110cb6e1a848b95e438688e5bc7edae48f89143ae5fd5ead5995a720ca5a7a`.
No Rust changes or full-repository test run in this follow-up.

Guide-depth follow-up (2026-09-20): removed depth-test bypasses from guide materials/labels and
replaced canvas hover lines with depth-tested 3D segments. Ordinary clicks skip mesh-occluded
targets; Alt+click still cycles through them. All six asset headless suites pass freshly.
X11/Forward+ on Godot 4.7.2 / RX 7900 XTX passes the selection suite and day/night pixel checks:
four 16×16 regions behind a generated building are byte-identical with guides enabled or hidden.
Visible parking/loading/driveway clicks, label picking, LOD openings, moving occluders and cached
handle visibility are covered. Artifacts: `/tmp/metrum-guide-depth.IeWTwi/`, `final-*.log`,
`rendered-accepted.log`, `captures-accepted/`.

Matched unprofiled release runs (four Rayon workers, isolated XDG profile, the existing
`asset_selection_test.gd -- --asset-editor --benchmark-asset-selection` command) use two parts,
one anchor and one yard: 10,000 idle checks and 1,000 pointer queries/locked reference-drag steps.
Before/after All-query means are **23.646 / 23.648 → 28.193 / 27.645 µs**; reference-enabled queries
**28.511 / 29.123 → 34.027 / 33.500 µs**; reference drag **8.752 / 8.793 → 9.467 / 9.510 µs**.
Final idle is **0.748 / 0.744 µs**, with zero idle queries, drag visibility rays and BVH rebuilds.
The extra per-query depth ordering is editor-local; unchanged idle and cached reference dragging
avoid recurring occlusion work. Logs: `before-{1,2}.log`, `accepted-after-{1,2}.log`.
Baseline is `7339a7bdf46d1892` for the changed selection/renderer files; release library SHA256
prefix `5a5ddeb006068a581` is unchanged. Final combined SHA256 for `asset_picker.gd`,
`asset_selection.gd`, `picking_overlay.gd`, `building_preview.gd`, `asset_selection_test.gd`
(in that order, `sha256sum ... | sha256sum`) is `cf894331b6392ff001f8c1e6fec597856a4d93bc839b2de7d6154ffd78a8c009`.
No Rust, gameplay renderer or source asset changes are part of this follow-up.

### Working building previews and safe packaging — TOOLS-04

The building editor separates panel/control ownership (`asset_editor/editor_view.gd`),
preview controls (`preview_panel.gd`), editable mesh-part/LOD projections (`mesh_part.gd`), and
non-destructive emission overrides (`preview_materials.gd`). Rust owns the document, history,
drafts and package I/O; the superseded GDScript document, draft, dependency and package backends
are removed. The shell coordinates placement, anchors, yards and the pack browser, while Rust
owns applicability, LOD-chain validation and shared lot/polygon constraints.

Authoring backend consolidation (2026-09-19): pack creation and export use one TOML writer;
dependency planning checks every tier, URI-decoded texture/buffer paths, content collisions,
reserved filenames and directory links. Publication stages on the destination filesystem,
preserves unmanaged regular files, protects the prior asset before replacement, and retains its
backup if restoration fails. File work is O(files + bytes), with O(F log F) ordered planning.
The editor and manifest validator share allocation-free O(V²) polygon checks; the bridge reuses
its O(V) scratch buffer. Yard fill/highlight use Godot's native triangulation with no invalid
concave fan fallback. Mesh bounds use native AABB transforms instead of script-side corner
arrays; lot translation is O(1) in Rust. Numeric/vector preview projection, widget ownership and
active-LOD material ownership are consolidated. Legacy single-mesh loading, global preview
scaling, write-only state and unused editing entry points are removed. UI widgets, camera/input,
native mesh picking and rendering remain GDScript presentation code.

Fresh consolidation verification: `cd rust && cargo test` passes 1,832 tests (63 ignored);
`cargo check --benches` and the release build pass without warnings. All six headless asset-editor
suites and `economy_machinery_test.gd` pass against the deployed release library. Artifacts:
`/tmp/metrum-editor-cleanup.boDR12/` (`rust-full.log`, `bench-check.log`, `release-build.log`,
`final-asset_*test.log`, `economy-machinery.log`). Library SHA-256:
`4e9010ddbfca01be311cc31aaeea1a80bc5eeeb1869c13ece4264015ba487b68`.

Sequential unprofiled Godot 4.7.2 release-extension measurements used four Rayon workers and
isolated XDG data/config directories. Command: `godot --headless --path godot --script
res://tests/asset_selection_test.gd -- --asset-editor --benchmark-asset-selection`.
The unchanged two-part/one-anchor/one-yard fixture has 10,000 idle checks, 1,000 queries and
1,000 reference drag updates. Before / two after runs (`before.log`, `after-{1,2}.log`): idle
0.747 / 0.756 / 0.733 µs; query 21.771 / 20.304 / 20.182 µs; reference query
26.972 / 25.567 / 25.591 µs; drag 8.253 / 8.352 / 8.406 µs. BVH rebuilds remain zero.
Frontage submissions (500, ten cells) average 35.380 / 35.484 / 35.352 µs; anchor submissions
(100, one anchor) average 24.410 / 32.090 / 24.640 µs; the first after anchor run was higher.
These measure editor CPU work, not city/GPU performance. The existing document regression's
`--benchmark-asset-document` mode measures 100 edits and undos with 100 / 1,000 metadata entries:
edits 51.040 / 565.120 µs, undo 0.780 / 0.820 µs (`document-benchmark.log`). Boundary copying
scales with document size; Rust history navigation does not copy revisions. UI redraw is excluded.
Source digest (the earlier combined command, plus `asset_document_test.gd` and
`asset_editor_preview_test.gd`): `64e7d071fe092276892ef190696b36b75f94179b3cb1642c9b62112193e9db8e`.
Native Wayland / Forward+ preview assertions pass; inspected `captures/day.png` and `night.png`
show the framed mesh and emission correctly. The capture fixture now waits for deferred container
layout and asserts a visible projected size before framing/capture. These native checks cover
rendering assertions and captures only (`rendered-preview.log`).

Guide visual refresh (2026-09-19): frontage/access arrows share an eight-triangle filled glyph;
the lot outline is constant-size geometry and frontage retains O(frontage cells) tick generation
with at most three arrows. Existing anchor/yard edge generation and picking bounds are unchanged;
no new per-frame mesh builds, textures, scene nodes or draw calls were introduced. All six headless
asset suites pass, including cardinal frontage/lot-size and guide-height checks. Native Wayland /
Forward+ selection assertions and inspected close-up day/night captures pass in both themes;
test camera input is disabled so physical scrolling cannot disturb the fixed capture rig.
These native checks cover selection assertions and captures only.
Artifacts: `/tmp/metrum-guide-polish.nFhf7n/accepted-asset_*test.log`, `verified-render.log`, and
`verified-captures/*-guide-{detail,night}.png` in that directory.

Matched, sequential, unprofiled before/after runs used Godot 4.7.2, four Rayon workers, the same
release library SHA-256 recorded above, separate isolated XDG profiles, and the existing
`--benchmark-asset-selection` command. A 10-cell frontage (500 rebuild submissions) averaged
154.710 / 154.874 µs before and 34.990 / 36.012 µs after; one entrance anchor (100 rebuild
submissions) averaged 31.810 / 30.770 µs before and 23.540 / 24.380 µs after. Deferred label cleanup
is outside the measured loop. Idle still performs no geometry queries; final idle checks averaged
0.735 / 0.749 µs. Logs: `before-{1,2}.log`, `final-after-{1,2}.log` in the same artifact directory.
Renderer SHA-256 before: `64aaa923d5b39be8fdff9b3b25026f20b3245c6161f1b31155e481daaa8bcfb3`;
after: `2a8d5747908fc64f30f0f923109b5738db919972170ec8a0ada00d0b609e71fb`. The combined source-digest
command above yields `f9afce214d288df7d5b039a8dafb37b280ca654c2fab3e8bb92ce2aca3a28f6c`.
These are editor CPU submission timings, not GPU measurements. No Rust changes/full-repository run.

`editor_spacing.gd` owns asset-editor-wide spacing: 12 px content insets, 8 px control gaps,
roomier input/button text and multiline labels. Inspector/preview tabs and the browser/log
share it in both themes; action rows wrap when narrow. Shared gameplay/other-editor styles
are not modified.

Every part retains its complete `mesh_parts.lods` chain on load/save. Copy planning includes
all tiers' external images and glTF buffers, including different dependencies beside meshes
in the same source directory. Missing files, escaping paths and conflicting destination names
block publication. Export stages a complete candidate, validates it through Rust, then replaces
the asset directory with rollback on a reported rename failure. Unmanaged files are retained;
unreferenced tiers are not automatically deleted. Existing pack metadata is not overwritten.
This is not a crash-recovery journal: an interrupted directory replacement may leave the prior
asset in the pack's `.stage-*/previous` directory for recovery. Copy/Move remains an explicit user choice.

Verification uses generated four-tier box meshes and default-off external emission textures;
no separate content repository is required. `godot --headless --path godot --script
res://tests/asset_editor_preview_test.gd -- --asset-editor` covers the real editor/Rust
save-load-save path, placement preservation, automatic emission through the actual preset buttons,
manual-hour/midnight changes, persistent emission overrides, source isolation, dependency collisions,
failed validation and existing yard/anchor tools. The regression is included in `./run.sh --test`.
Initial headless, Xvfb/Compatibility software-rendered, and native Wayland/Forward+ (RX 7900 XTX)
runs passed on Godot 4.7.2. The economy-profile selector and day-cycle regressions also passed,
along with all 15 Rust `asset_export::tests` cases. Optional
`--capture-preview=<temporary-directory>` captures the generated day/night fixture in a rendered run.
Optional `--benchmark-asset-preview` now measures warmed switches rather than imports (see
`TOOLS-05`). Loading scales with mesh/dependency size, emission updates with preview material
surfaces, and export with the asset's copied bytes. No simulation tick or world-wide rendering
loop is added.

Night-preview follow-up (2026-09-20): all six asset-editor headless suites and `day_cycle_test`
pass freshly; X11/Forward+ (RX 7900 XTX, Godot 4.7.2) also passes, with inspected day/night
captures now exercising Automatic via the actual preset buttons. The sandboxed Xvfb attempt
could not open a display; the rendered check used approved desktop access instead.
Artifacts: `/tmp/metrum-night-preview.uKcymY/`. The isolated `project/emission_measure.gd`
extends the same fixture: 100 warmup Day/Night pairs, then 2,000 alternating preset changes,
one part/emissive surface. Two sequential unprofiled headless runs with `RAYON_NUM_THREADS=4`
measured **101.204 / 100.710 µs per change**, zero imports, using the existing release extension
(SHA256 prefix `5a5ddeb006068a581`) and panel source `bf10e33785c8af18`.
Run with `godot --headless --path <isolated-project> --script res://emission_measure.gd -- --asset-editor`
and isolated XDG data/config paths. These are input-handler timings, not GPU/frame timings:
the added solar lookup is O(1); applying emission remains O(parts + emissive surfaces) on input
only, with no idle-frame work, new material copies or simulation changes.

Comparison readability follow-up (2026-09-20): fresh `asset_editor_preview_test` and
`asset_selection_test` headless runs pass. The preview test generates a textured wall/roof GLB,
checks opacity/shading, source isolation, authored glass/cutouts, load-during-night emission and
clear/reload; X11/Forward+ captures pass in both UI themes at Day/Night. Artifacts:
`/tmp/metrum-comparison.STBDZK/{accepted.log,selection.log,render-accepted.log,captures-accepted/}`.
Using the isolated project, fresh XDG profiles, `RAYON_NUM_THREADS=4`, Godot 4.7.2 and the existing
release extension (`d8ce9d24c17ded8c`), `--benchmark-asset-preview` on i9-12900K measures 1.371 ms
mean comparison load (10 loads, two surfaces; import/material/pick-cache setup, excluding deferred
scene cleanup) and 80.678 µs per cached emission change (2,000 changes, one surface). Renderer
digest: `f4aff8e81fc3b960`. These are current-build CPU checks, not before/after or GPU timings;
no simulation code changed. The optional capture/benchmark flags extend the existing command above.

### Shared LOD policy and automatic inspection — TOOLS-05

`rust/src/assets/lod_policy.rs` owns the allocation-free projection and selection functions;
`AssetLodPolicy` is a stateless Godot bridge. `asset_editor/preview_lod.gd` coordinates editor
nodes, not game state. Future spatial building rendering must call the same Rust policy.

- Project all eight corners of each part's **LOD0** bounds through its placed transform and
  the camera matrix. The metric is the larger projected width/height in render pixels,
  including viewport resolution and 3D render scale. Perspective, orthographic, aspect/FOV
  and camera offsets come from the real projection matrix. Do not use plot bounds, current
  simplified-mesh bounds or clipped-to-viewport dimensions. Near-plane intersections or
  invalid projection inputs retain LOD0; visibility/culling is a separate responsibility.
- Provisional Balanced boundaries are 512 / 256 / 128 px for four tiers; each additional
  tier halves the boundary. Equality keeps the finer tier. Performance multiplies effective
  pixels by 0.5, Quality by 2.0. These are global preview presets, not asset-authored overrides
  or shipped player graphics settings. Screen extent measures size, **not mesh approximation
  error**; representative art still needs silhouette/material/emission transition review.
- Hysteresis uses a 10% margin: coarsen below 90% of a boundary, refine above 110% (equality
  refines). Large camera changes can skip directly to the appropriate tier. Explicit quality
  changes and entering Automatic reset history. Single-tier assets keep LOD0; short chains
  retain their last available tier. Selection never culls an asset or changes its shadows.
- Parts start in Automatic. Explicit tier inspection affects only that part, survives selection
  changes and does not enter the manifest. Camera transform/projection changes end temporary
  inspection on all parts and reset its hysteresis history; idle updates do not.
  Camera/quality/emission controls do not modify source assets.
  Failed tier imports retain the visible mesh with an inspector error; camera movement does
  not continually retry failed imports. Removing a tier/part clears its corresponding cache.
- Import and count geometry only on document/chain changes. All tiers are retained under the
  preview root, with only the active scene visible; material overrides are preallocated and
  source materials untouched. This editor-only cache trades memory for smooth inspection and
  is not a proposed city residency scheme. Idle checks are O(1); camera/placement changes cost
  O(P) for this document's P parts. Projection/selection are O(1) per part, allocation-free in
  Rust. No all-city scan or simulation work is introduced.

Schema scope: this step preserves the existing metre-band fields and does not silently reinterpret
them as pixels. Retire them in a coordinated manifest/editor/validator migration after art
calibration. Gameplay spatial batching/LOD groups, scene-scale GPU acceptance, culling and
shadow policy integration remain separate work. Existing meshes need not be remade for this step.

Verification commands:

```bash
cd rust
cargo test --lib assets:: -- --test-threads=1
cargo bench --bench asset_lod_benchmark
cd ..
# After deploying the rebuilt GDExtension:
godot --headless --path godot --script res://tests/asset_editor_preview_test.gd -- --asset-editor --benchmark-asset-preview
```

The generated regression exercises real editor controls, automatic/forced selection, independent
part state, hysteresis, quality/FOV/render-scale/placement changes, cached imports, failed tiers,
chain edits and unchanged export metadata. Optional `--capture-preview=<temporary-directory>`
captures day, night and the automatic LOD inspector. Fixtures require no external content.

Fresh verification (2026-09-19): 65 asset/policy Rust tests passed (one unrelated, opt-in
registry benchmark ignored), all 15 export tests passed, and `cargo doc --no-deps` completed
without warnings. Headless and native Wayland/Forward+ preview regressions passed on Godot
4.7.2; economy-profile and day-cycle regressions passed too. The rendered inspector was checked
on RX 7900 XTX. This validates the preview, not finished-art thresholds or gameplay GPU cost.

Isolated unprofiled release measurement: i9-12900K, single benchmark thread (no Rayon/world),
base `35f15a5b` plus this working tree; policy SHA-256 prefix `b63d5f2368675356`, benchmark prefix
`2895c0ca0a343a17`. Criterion uses a 20 m box at 100 m, 60° perspective, 1920×1080, four tiers,
Balanced quality and previous LOD1; 1 s warmup, 2 s measurement, 30 samples. Projection plus
selection: **37.54–37.73 ns**; selection alone: **3.94–3.99 ns** (95% intervals). Artifacts:
`rust/target/criterion/AssetLod/`. These exclude Godot calls, loading, uploads and rendering.
The headless generated-mesh diagnostic separately measured 40 warmed switches at **0.003 ms
mean, zero imports**. Neither result establishes city-scale performance or a before/after
gameplay improvement.

Implement the editor as a shared Godot-based tool mode with TOML manifests and `.glb` as the canonical asset format. Start implementation from the v1 contract in this document: canonical pack layout, scanner rules, `pack_id` / `asset_id` grammar, per-class `asset.toml` schemas, and the minimum preview-scene interactions required to author each asset class. Export manifest files directly into the pack output folder next to the asset files.

For buildings specifically, plot size is required asset metadata from day one. `3x3` remains only a default preset, not a design limit, and fixed `ZONING_DEPTH` is retired in favor of dynamic per-edge zoning extents. Compatibility metadata, redirect handling, workspaces, cross-pack sharing, signing, and similar ecosystem features stay in the later section until the base importer actually exists.
