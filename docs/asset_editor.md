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

- Buildings are still loaded from hardcoded `res://assets/...` paths and use Godot project-imported assets.
- Cars and VAT pedestrians already use runtime GLTF loading APIs, but still from project-bundled paths.

Shipped design:

- Built-in first-party content can remain bundled with the game.
- V1 canonical mod install location: `user://mods/`

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

- `driveway`: connector intent from lot interior toward the road-facing edge; requires `width_m`
- `parking`: car stop/stand position inside the lot; requires `width_m` and `length_m`
- `loading_bay`: freight/service stop position inside the lot; requires `width_m` and `length_m`
- All building anchor positions must remain inside the authored lot footprint. For `driveway`,
  `parking`, and `loading_bay`, the whole authored footprint/rectangle must remain inside the lot;
  the anchor handle alone being inside is not enough. Driveway footprint length is derived by the
  editor from `width_m` in v1. All anchor `forward` vectors must be finite non-zero unit vectors.
- `entrance/main` cannot be deleted. Dragging any anchor moves it; right-click dragging or the
  shared yaw field rotates its `forward`.

Runtime use in v1:

- `entrance/main` remains the only anchor consumed by the live entrance/exit system.
- `driveway`, `parking`, and `loading_bay` anchors are semantic site-layout metadata only. They do
  not create asphalt, concrete, paths, pads, yards, parking markings, loading markings, or other
  visuals by themselves.
- Building yard polygons are authored explicitly through `[[site_surfaces]]`, but they are editor
  preview and asset metadata only in v1. Live gameplay does not render, clip terrain with, or query
  these surfaces until `EARTH-02` implements them as first-class engineered ground.
- For the `EARTH-02` target, driveway anchors become the preferred runtime connection points used
  to choose a single flat site height from the road/world surface. They still do not create
  surfaces by themselves; authored `[[site_surfaces]]` polygons provide the local material/layout
  regions on the flat lot.
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
- Site surfaces are authored visual-ground metadata and editor-preview geometry only in v1. Live
  gameplay ignores them until the parked `EARTH-02` engineered-ground work lands.
- In the `EARTH-02` target, the runtime terrain-ownership footprint is still the whole occupied lot
  rectangle. `[[site_surfaces]]` polygons are material/layout regions on top of that flat lot, not
  independent terrain-cut footprints.
- Site surfaces do not imply access, parking capacity, freight capacity, service eligibility,
  pedestrian paths, or vehicle routing.
- Anchors may sit on top of site surfaces, but anchors never create surfaces by themselves.
- If an asset exports no site surfaces, the editor previews no yard treatment. Live runtime behavior
  is currently the same whether site surfaces are present or absent.
- The editor can create rectangular starting surfaces, then authors can move the whole polygon,
  drag vertices, right-click an edge to add a vertex, and right-click an existing vertex to delete it
  while preserving the minimum three-vertex polygon.
- Painted decals, curbs, markings, and per-material texture selection are later extensions.

### Future Flat-Site Authoring Target

The `EARTH-02` authoring target is WYSIWYG for the local flat lot:

- The editor preview should show a flat lot plane with the authored `lot_width_cells` and
  `lot_depth_cells`, not an abstract infinite grid as the main authoring reference.
- The lot boundary is the future runtime site footprint.
- Mesh parts, anchors, and `[[site_surfaces]]` share the same local coordinate system.
- Authored site-surface materials preview on the flat lot exactly as they should appear once the
  runtime site client exists.
- The editor does not choose the world height of the lot. Runtime placement chooses the height from
  road/driveway connection, neighboring fixed sites, or explicit-site fallback according to
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

Building assets should use canonical `+Z` frontage when practical. The exported `main` entrance
anchor `forward` vector is still authoritative for runtime alignment, so imported meshes whose
frontage is not yet canonical can be represented without pack-specific runtime fixes.

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
- Selection and culling bounds may differ from the visual mesh and do not depend on exact triangle detail.
- Shadow proxies are separate from visual bounds.
- Anchor metadata uses one canonical `[[anchors]]` array-of-tables shape rather than a mix of one-off fields like `entrance_anchor`, `parking_slot_1`, etc.
- Single-anchor assets still use `[[anchors]]` with exactly one entry.

Anchor requirements by asset class:

- Buildings require exactly one `entrance` anchor named `main` that marks the main door or primary access point used by the generic entrance/exit system.
- Buildings may define optional `driveway`, `parking`, and `loading_bay` anchors. These are authored
  site-layout metadata in v1. They do not generate visual yard surfaces; vehicle parking and
  freight stop behavior remain later runtime hooks.
- Buildings may define optional `[[site_surfaces]]` polygons for editor-preview yard materials:
  asphalt and concrete. Live gameplay ignores these polygons in v1. In the `EARTH-02` target, these
  polygons become material/layout regions on the flat whole-lot building site.
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
  the imported mesh bounds; the runtime manifest validator does not independently load external mesh
  files just to recompute their bounds.
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

- Ordinary zoned building:
  - `LOD0`: `1,500-4,000` triangles
  - `LOD1`: `400-1,200` triangles
  - `LOD2`: `80-250` triangles
- Large skyline or landmark building:
  - `LOD0`: up to `6,000-8,000` triangles by exception
  - `LOD1`: `1,000-2,000` triangles
  - `LOD2`: `150-400` triangles
- Switch logic:
  - V1 uses distance bands in meters.
  - Derive default switch distances from asset bounds and height class.
- Default distance bands for ordinary zoned buildings:
  - `LOD0`: `0-150 m`
  - `LOD1`: `150-600 m`
  - `LOD2`: `600-2,000 m`
  - optional farther tier beyond `2,000 m`: skyline impostor, billboard, or coarse massing mesh
- Default distance bands for large skyline or landmark buildings:
  - `LOD0`: `0-300 m`
  - `LOD1`: `300-1,200 m`
  - `LOD2`: `1,200-4,000 m`
  - optional farther tier beyond `4,000 m`: skyline impostor or skyline-only massing mesh
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

Start flow:

1. Choose `New Pack`, `Open Pack`, or `Open Existing Asset`
2. Choose asset class template
3. Open the relevant sandbox scene

The editor does not use a one-asset-only wizard. One pack may contain and edit multiple assets in one session.

V1 editor shell:

- one shared editor shell scene
- one center viewport with shared camera controls
- one mode-specific inspector panel driven by the currently selected asset class
- one scene-template switcher that swaps reference environments inside that shell rather than launching a different application

Editor layout:

- Center: 3D viewport
- Left: asset browser and scene template switcher
- Right: inspector for metadata, materials, validation, and pack assignment
- Bottom: import log, warnings, and build output

V1 preview templates:

- zoned roadside lot
- empty ground grid
- lane + sidewalk scale reference
- traffic comparison scene with placeholder car/character
- night lighting scene

Minimum quality-of-life features:

- autosave of editor state
- recovery of unsaved drafts after crash or forced close
- one-click revalidate / rebuild for the current asset or whole pack

Thumbnail generation rules:

- Use locked per-asset-class thumbnail rigs with fixed focal length, lighting, background, and framing rules.
- The editor allows thumbnail regeneration.
- Thumbnail generation does not expose arbitrary camera, time-of-day, or scene-composition authoring.
- If asset classes need different framing, use a fixed preset set such as building, vehicle, prop, and character rigs.

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
- Building mode inspector edits shared asset fields (`asset_id`, `display_name`, `thumbnail`, `asset_set`, `tags`, optional attribution), building fields (`placement_mode`, conditional `zone_type` and `density`, `service_class`, `economy_profile`, `lot_width_cells`, `lot_depth_cells`, `min_zone_width_cells`, `min_zone_depth_cells`, `household_capacity`, `worker_capacity`, `flat_size_m2`), mesh part files/transforms, optional material paths, the required `entrance/main` anchor, and optional `driveway`/`parking`/`loading_bay` site anchors.
- Building mesh parts can be moved on the X/Z plane by left-dragging the part in the preview
  viewport and rotated freely around Y by right-dragging it horizontally, with a light snap when the
  rotation is close to a 90-degree cardinal angle; the clicked part becomes the selected part and
  the inspector transform fields mirror the live manipulation. Selected mesh parts use corner
  handles rather than full bounding-box cages. The editor clamps the transformed X/Z footprint of
  every mesh part to the authored lot rectangle during import, move, rotate, scale, and lot-size
  edits, and export must reject a mesh part that still cannot fit inside the lot.
- Dragging an empty area in the building preview draws a selection rectangle over the viewport and
  selects all mesh parts, the required main entrance, and site anchors whose projected bounds
  intersect it. Holding `Shift` while left-dragging forces rectangle selection even when the drag
  begins over a mesh part or anchor. Holding `Ctrl` while clicking toggles individual mesh parts,
  the main entrance, or site anchors into the current selection. Dragging any selected part,
  entrance, or site anchor moves the whole selected mesh/anchor group together on the X/Z plane. The
  main entrance can be moved but cannot be removed.
- The Asset tab provides `Remove Mesh Part`, and the keyboard `Delete` key performs the same action
  when mesh parts are selected and text input is not active. Removal deletes all selected mesh parts
  from the active asset draft and preview scene.
- The Anchors tab provides `Driveway`, `Parking`, and `Loading Bay` add actions plus a remove action
  for the selected site anchor. Site anchors are selectable in the list or viewport, movable on the
  X/Z plane with left-drag, and rotatable around Y with right-drag using the same light 90-degree
  snap as mesh-part rotation. `Delete` removes the selected site anchor when text input is not active.
- The building inspector groups long forms into focused equal-width sub-tabs, with persistent
  top-level actions such as mesh import and export kept outside the tab scroll pages. Switching tabs
  must not resize the inspector sidebar.
- The Pack tab uses a single `Set Pack...` command plus a compact selected-pack summary instead of
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
- Building mode viewport shows the lot rectangle, frontage arrow, sidewalk/road reference,
  entrance anchor gizmo, site-anchor previews, orientation validation, and footprint overflow
  warnings. Frontage, entrance, and site anchors use theme-aware high-contrast colors plus short
  viewport labels, dashed guide borders, and contrast halos so they remain legible in both dark and
  light editor themes.
- The comparison ghost is explicit, not automatic. Right-clicking an asset browser row opens an
  asset context menu with `Use as Ghost`; that asset remains as the viewport ghost until it is
  replaced or cleared.
- The comparison ghost uses the selected asset's first mesh part and authored part scale.
  Loading another asset into the inspector must not replace the ghost. Assets from packs with
  different source-unit conventions must compare in exported/game-space meters.
- The comparison ghost can be repositioned in the viewport by dragging it with the left mouse
  button.
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
- `forward`: `[f32, f32, f32]`; for building `main` entrance anchors this is the asset-local
  frontage direction that the runtime aligns to the placed building's road-facing direction.
  Manifest validation rejects non-finite, zero, or non-unit vectors.
- `width_m`: optional positive float. Required for building `driveway`, `parking`, and
  `loading_bay` anchors.
- `length_m`: optional positive float. Required for building `parking` and `loading_bay` anchors.
  `driveway` anchors derive their v1 editor footprint length from `width_m`.
- `vehicle_class`: optional string, baseline values `car`, `freight`, or `service`. If present,
  validation rejects any other value.

Optional `[[site_surfaces]]` table for building visual yard polygons:

- `material`: enum, one of `asphalt` or `concrete`
- `name`: optional editor label
- `y_m`: optional finite vertical offset in asset-local metres, default `0.0`
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
- `mesh_parts.lods.distance_min_m` / `distance_max_m`: ordered distance switch band for that part
- `service_class`: enum, one of `none`, `police`, `fire`, `healthcare`, `education`, `power`, `water`, `waste`, `transit`, `parks`, `government`; default `none`. Non-`none` service classes are valid only for `placement_mode = "explicit"` in baseline `v1`.
- `economy_profile`: reference to an authored economy profile. Utility service assets require a resolved utility profile; the starter mappings are `power -> power_plant_basic`, `water -> water_plant_basic`, and `waste -> wastewater_treatment_basic` (`waste` is the asset-side service class for sewage treatment).
- `min_zone_width_cells`: integer, default `lot_width_cells`
- `min_zone_depth_cells`: integer, default `lot_depth_cells`
- `household_capacity`: integer, `>= 0`. Defines the number of distinct household slots (families). Required for residential.
- `worker_capacity`: integer, `>= 0`. Defines the total staffing capacity. Required for commercial/industrial. Note: if an `economy_profile` is selected, this value is read authoritatively from the profile and cannot be overridden at the asset level.
- `flat_size_m2`: float, `>= 0.0`. The average interior living area per household. Used to filter which household sizes (e.g. 1-person vs 5-person) are compatible with the building.

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
- Exactly one `[[anchors]]` entry with `type = "entrance"` and `name = "main"` is required.
- The `main` entrance anchor's `forward` vector defines the asset-local frontage direction used by
  building placement, rendering, and entrance-cache derivation.
- Additional building-side site points use `type = "driveway"`, `type = "parking"`, or
  `type = "loading_bay"`, not a second generic `entrance` anchor.
- Driveway, parking, and loading-bay footprints must fit fully inside the authored lot rectangle.
- In v1, driveway, parking, and loading-bay anchors are not rendered as ground treatment. Authored
  `[[site_surfaces]]` polygons own asphalt, concrete, walkways, parking pads, loading pads, and
  driveway-apron visuals only in the asset editor preview.
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

Implement the editor as a shared Godot-based tool mode with TOML manifests and `.glb` as the canonical asset format. Start implementation from the v1 contract in this document: canonical pack layout, scanner rules, `pack_id` / `asset_id` grammar, per-class `asset.toml` schemas, and the minimum preview-scene interactions required to author each asset class. Export manifest files directly into the pack output folder next to the asset files.

For buildings specifically, plot size is required asset metadata from day one. `3x3` remains only a default preset, not a design limit, and fixed `ZONING_DEPTH` is retired in favor of dynamic per-edge zoning extents. Compatibility metadata, redirect handling, workspaces, cross-pack sharing, signing, and similar ecosystem features stay in the later section until the base importer actually exists.
