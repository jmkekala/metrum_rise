# Asset Editor / Importer Design

## Purpose

Metrum Rise needs a dedicated asset-authoring tool for first-party content and modders. The tool should let creators import and validate game-ready assets without starting a live city simulation. It must remain consistent with the performance constraints of the main game: the importer can do expensive offline work, but the shipped runtime assets must be cheap to load and render.

The editor is not a replacement for Blender or other DCC tools. It is a constrained packaging, validation, preview, and metadata-authoring tool that turns external art assets into Metrum Rise content packs.

## Critical Assessment Of The Current Idea

The direction is good, but the first version should be narrower than "import every possible asset type with every possible behavior".

What is solid in the current idea:

- A separate tool is the right direction. Modders need a stable preview sandbox, not a live city with traffic and demand noise.
- A small preview map is correct. A `500 m x 500 m` sandbox is enough for scale validation, lighting checks, prop placement, and vehicle/citizen reference scenes.
- Asset metadata must be authored outside raw model files. Plot size, jobs, category, pack membership, author, and licensing do not belong in the mesh alone.
- Pack/set enable-disable is essential. Cities games live or die by content curation.

What needs to change:

- "Pick asset type, then get a cell area" is only partly right. Cells make sense for zoned buildings and parks, but not for citizens and cars. Vehicles need lane-width and turning-radius references; citizens need sidewalk, door, and camera-distance references.
- Roads should not be part of version one of the importer. A road is not just a mesh. It also defines lane count, widths, allowed modes, sidewalk offsets, markings, junction behavior, and compatibility with the lane/pathing system. That is a separate network authoring problem.
- Arbitrary skeletal animation at runtime is not compatible with the current long-term rendering plan. Godot `MultiMesh` does not support per-instance skeleton animation, and the project roadmap already points toward SDF or VAT-based crowds. The editor can ingest rigged source assets, but the shipped runtime representation for citizens must be static, billboarded, or VAT-baked.
- The editor should not treat `3 x 3` as a permanent rule. That is only the current default assumption in parts of the codebase and docs. The proper model is per-asset plot metadata: each building declares its own `lot_width_cells` and `lot_depth_cells`.
- The current fixed `ZONING_DEPTH` storage model is too restrictive for a real asset pipeline. It should be replaced by dynamic zoning extents so building size is limited by painted zoning and map space, not by a compile-time constant.

## Recommended Product Shape

Build a separate `asset_editor` executable or launch mode, but keep it inside the same Godot + Rust project family.

Why:

- Godot already gives scene importing, material preview, cameras, lighting, gizmos, thumbnails, and export workflows.
- The runtime and the editor can share the same asset schema and validation rules.
- This keeps the content pipeline consistent with the main game instead of creating a second rendering stack.

Recommended runtime shape:

- Launch into a stripped-down sandbox scene.
- No demographics, no active `AgentSystem`, no demand simulation, no immigration.
- Small map config, for example `MapConfig::new(500.0, 500.0, 1.0, 10.0)`.
- A few preview templates:
  - Flat studio scene
  - Zoned roadside scene
  - Sidewalk + lane scene
  - Day/night lighting scene

This is closer to the Cities: Skylines approach than the old SimCity 4 BAT approach. Cities-style tooling previews the asset in a constrained in-engine scene. SimCity-style tooling split model creation from lot metadata into separate authoring steps. Metrum Rise should combine both ideas: in-engine preview plus explicit metadata files.

## Scope Split

Version one should support four authoring targets:

1. Zoned buildings
2. Props / environment details
3. Vehicles
4. Citizen source assets for offline baking

Version one should not support:

- Full road authoring
- Junction authoring
- Rail/ship/air infrastructure authoring
- Runtime skeletal crowd animation
- Arbitrary simulation scripting inside assets

## Canonical Asset Format

Use `glTF 2.0`, preferably binary `.glb`, as the canonical 3D asset format.

Why:

- Godot recommends `glTF 2.0` for 3D scenes.
- It preserves meshes, materials, UVs, skeletons, and animation clips better than older interchange formats.
- It is more pipeline-friendly than OBJ and less awkward than keeping FBX as the canonical shipped format.

Recommended import policy:

- First-class supported source: `.glb`
- Acceptable source for authoring convenience: `.fbx`, but convert it to `.glb` during import or baking
- Do not make raw `.fbx` the canonical packaged format

Texture inputs:

- Required: base color / albedo
- Optional but recommended: normal
- Optional: ORM packed texture or separate roughness / metallic / AO
- Optional: emission
- Optional: opacity mask

Packaged runtime outputs should stay simple:

- `.glb` for static meshes
- `.png` or engine-imported textures for standard materials
- `.exr` only where VAT baking needs float texture data

## Coordinate Conventions

The asset editor should document one canonical import convention instead of making modders guess at Blender/Godot axis conversions.

Current codebase reality:

- The simulation ground plane is `XZ`; `Y` is vertical/up.
- Many simulation helpers store planar positions as `(x, z)` in `Vector2`. Some legacy field names such as `center_y` and `pos_y` actually mean world `Z`, not vertical `Y`.
- Building transforms use `facing_dir` as the transform's local `+Z` axis, so building meshes must face local `+Z`.
- The pedestrian GLTF/VAT pipeline also expects the imported mesh to face local `+Z`. Existing code comments note that Blender-facing `-Y` becomes `+Z` after the current GLTF import path.
- Vehicles are a legacy exception today: the civilian car loader applies a `180°` yaw on load and the runtime car basis currently uses `basis_z = -fwd`. The asset editor should treat that as old compatibility behavior, not as the convention to preserve forever.

Canonical convention for newly imported Metrum Rise assets:

- Use a right-handed local asset basis.
- `+Y` = up
- `+Z` = front / forward
- `+X` = right

Per-asset pivot rules:

- Buildings: origin at ground level, centered on the footprint.
- Citizens: origin at ground level between the feet.
- Vehicles: origin at ground level on the vehicle centerline.
- Props: origin at the intended placement anchor.

Blender guidance:

- Author normally in Blender with `Z` up.
- For assets that should arrive facing `+Z` in Metrum Rise, make the model face Blender `-Y` before GLTF export.
- Export canonical source assets as `glTF/.glb`.
- Do not rely on per-renderer hidden rotations as part of the modding contract.

Editor-side validation:

- Show an axis gizmo and a front arrow on every imported mesh.
- Expose one import-time "fix orientation" step if needed, then bake the corrected result into the packaged asset.
- Show the placement pivot explicitly so creators can confirm the mesh sits on the ground plane instead of floating or sinking.
- For legacy built-in vehicles, allow a compatibility override until the runtime vehicle renderer is normalized to the same `+Z` convention as buildings and pedestrians.

## Texture Orientation

The editor should validate texture orientation explicitly so imported assets do not arrive upside down.

Current codebase reality:

- Standard building and vehicle `.glb` assets are loaded through Godot's GLTF pipeline and currently do not apply any general-purpose texture Y-flip in gameplay code.
- Existing built-in assets appear visually correct, so the baseline assumption for ordinary `.glb` imports should be: keep mesh UVs and texture images as authored unless the preview proves otherwise.
- The VAT pedestrian pipeline is a special case. Its baked `.exr` texture already includes explicit row reversal in the tooling to match the shader's expected sampling convention, so that path must keep its dedicated handling.

Importer/editor rules:

- Do not silently auto-flip all imported textures.
- Validate albedo, normal, ORM, emission, and mask textures in the preview scene.
- Provide an explicit per-texture vertical flip override only when the preview shows the texture is wrong.
- If a flip override is used, store it in asset metadata so the import result is deterministic and reproducible.

Recommended validation tools:

- A debug material mode that overlays a numbered UV test pattern.
- A "front/top" orientation test texture so upside-down or mirrored imports are obvious immediately.
- Side-by-side material preview for albedo, normal, and packed maps.
- A checker that warns when a normal map appears to be inverted or authored in the wrong convention.

Pipeline guidance:

- For ordinary static assets, prefer `.glb` with authored UV maps over loose mesh + loose texture assembly.
- For Blender-authored content, verify the material in Blender, then verify the exact exported `.glb` in the Metrum Rise editor before packaging.
- Treat VAT EXR textures as pipeline-generated data, not hand-authored art assets. Their orientation contract belongs to the bake tool and shader pair, not to manual editor flipping.

## Asset Classes

### Zoned Building

A zoned building asset needs:

- Visual mesh
- Materials/textures
- Metadata:
  - `zone_type`
  - `lot_width_cells`
  - `lot_depth_cells`
  - `residents_capacity` or `worker_capacity`
  - `employment_type` where relevant
  - `asset_set`
  - `author`
  - `license`
  - `tags`
  - `preview_scale`
  - `lod` references if present

Important constraint:

- `lot_width_cells` and `lot_depth_cells` must be first-class authored data, not inferred only from the mesh size. A skyscraper, row house, warehouse, and corner shop should be able to declare different footprints.
- The editor must validate the mesh against the lot footprint, not just allow free scaling until it "looks right".
- The current runtime already stores `width_cells`, `depth_cells`, and `variant`, so the editor should emit metadata that maps cleanly onto that system.
- If arbitrary-size lots are a design goal, footprint-related runtime fields cannot remain `u8` forever. The zoning/block/building/save schema should move to `u16` or `usize`-scale cell counts before the editor relies on very large assets.
- The current building renderer uses the building's `facing_dir` as local `+Z`, so authored building meshes must face `+Z` in packaged asset space.
- The renderer still uses a global building scale in places, so the editor should not assume that arbitrary per-model scale is already correct at runtime. That runtime gap should be fixed before relying on heavy import-time scale tweaking.

Recommended building workflow:

1. Import the building mesh
2. Choose `zone_type`
3. Set `lot_width_cells` and `lot_depth_cells`
4. Snap the mesh into a preview plot of that exact size
5. Enter capacity and category metadata
6. Run validation for footprint overflow, frontage orientation, and sidewalk clearance

Required simulation-side redesign:

- Replace the fixed `ZONING_DEPTH` constant with per-edge-side dynamic `depth_cells` allocated on demand from actually painted zoning.
- Keep the road-aligned zoning coordinate system `(edge_idx, side, x, y)` because frontage and road-facing orientation still matter for building placement.
- Spawn logic should no longer assume a fixed depth budget. It should only ask: "does a contiguous `lot_width_cells x lot_depth_cells` rectangle of matching zoning exist here?"
- If the player paints five adjacent `1x5` columns, a `5x5` building is allowed to consume all five columns as one footprint.
- Fit checks must be rectangle-based, not area-based. A `3x3` building must fail on a `3x2` zoned area, and it must also fail on a row of three `1x2` parcels even though there are three adjacent columns. Every cell in the full required `3x3` rectangle must exist, be zoned correctly, be unobstructed, and be unoccupied.
- Three adjacent `1x3` columns are valid for a `3x3` building only because together they form a real contiguous `3x3` rectangle. Summing parcel counts or total painted area is never enough by itself.
- Obstruction caching and zoning rendering should iterate only to the local painted depth for that edge-side, not to a global maximum depth for all edges.

Recommended zoning storage shape:

- `cells_long` remains per edge.
- `depth_cells` becomes per edge-side and grows on demand.
- The painted, occupied, and blocked cell arrays are sized to `cells_long * depth_cells` for that edge-side only.
- Solid zoning should retain parcel/block IDs so the allocator can prefer coherent rectangles and the renderer can keep separate painted parcels visually distinct.

### Props / Parks / Detail Objects

These are mostly visual assets with placement metadata.

Required metadata:

- `category`
- `bounding_size`
- `snap_mode`
- `terrain_behavior`
- `asset_set`
- `tags`

Parks should eventually become a richer class than props, but version one can treat small decorative park objects as props plus grouping metadata.

### Vehicle

Vehicle assets should be static meshes in version one.

Required metadata:

- `vehicle_class`
- `length_m`
- `width_m`
- `height_m`
- `asset_set`
- `color_variants`
- `lod` references

Optional metadata:

- wheel marker nodes for shader or transform-based wheel spin later
- light marker nodes

Avoid skeletal rigs for normal traffic cars in the first pass. They add complexity with little value for the current rendering architecture.

### Citizen Source Asset

Citizen import must be designed around the actual crowd-rendering plan, not around ordinary character-game assumptions.

Version one editor support:

- import rest mesh
- import skeleton
- import source clips
- preview clips inside the editor
- bake runtime output

Runtime output target:

- SDF billboard asset, or
- VAT-ready rest mesh plus baked animation textures

Supported source clips in the first pass:

- `walk` required if using VAT pedestrians
- `idle` useful for editor preview and future close-range LoD
- `run` optional later only if gameplay introduces it

Do not require a large animation library yet. `idle` and `walk` are enough for first content support. If the runtime crowd system remains billboard-based for a while, even those clips may stay source-only and never be played directly in-game.

## Roads Are A Separate Editor

Road authoring should be split into its own later tool mode.

A road asset is not just:

- a mesh
- a texture
- a width value

It also needs:

- lane definitions
- sidewalk presence and width
- allowed transit mask
- junction clip behavior
- markings
- spline generation rules
- build cost / category
- topology compatibility

That work must stay aligned with `RegionGraph`, lane rebuilding, frontage rules, zoning obstruction, and future multi-modal routing. It should not block the first importer milestone.

## User Experience

The start flow should be:

1. Choose `New Pack`, `Open Pack`, or `Open Existing Asset`
2. Choose asset class template
3. Open the relevant sandbox scene

Do not trap the user in a one-asset-only wizard. Once inside the editor, they should be able to manage multiple assets in one pack.

Recommended layout:

- Center: 3D viewport
- Left: asset browser and scene template switcher
- Right: inspector for metadata, materials, validation, and pack assignment
- Bottom: import log, warnings, and build output

Useful preview templates:

- zoned roadside lot
- empty ground grid
- lane + sidewalk scale reference
- traffic comparison scene with placeholder car/citizen
- night lighting scene

## Cells And Reference Areas

Use different reference systems by asset class.

For buildings:

- show the zoning grid
- default to the current game cell size
- present a resizable plot rectangle, not a fixed `3 x 3` box
- allow explicit `lot_width_cells` and `lot_depth_cells`
- preview frontage, road-facing direction, and sidewalk clearance
- offer common presets such as `1x1`, `2x2`, `3x3`, `4x4`, `5x5`, `10x10`, but always allow manual width/depth entry within the currently painted zoning and map bounds

For cars:

- show lane width, parking bay, and a few turning circles
- do not use building-style lot cells as the main reference

For citizens:

- show sidewalk width, a standard doorway, bench, crosswalk, and camera-distance silhouettes
- again, do not use building-style lot cells as the main reference

## Metadata And Pack Files

Use explicit JSON manifests so the pipeline stays compatible with the current Rust `serde` + `serde_json` stack.

Recommended pack structure:

```text
content/
  packs/
    kenney_city_pack/
      pack.json
      assets/
        residential_low_01/
          asset.json
          mesh.glb
          albedo.png
          normal.png
          thumbnail.png
        sedan_01/
          asset.json
          mesh.glb
          thumbnail.png
        citizen_male_01/
          asset.json
          rest.glb
          walk.exr
          thumbnail.png
```

`pack.json` should contain:

- `pack_id`
- `display_name`
- `version`
- `author`
- `license`
- `description`
- `enabled_by_default`
- `dependencies`

`asset.json` should contain:

- stable `asset_id`
- asset class
- references to imported/generated files
- metadata fields specific to the asset class
- source attribution
- validation status / importer version

For buildings, the schema should explicitly include:

- `lot_width_cells`
- `lot_depth_cells`
- `min_zone_width_cells`
- `min_zone_depth_cells`

In the normal case, `min_zone_*` equals the footprint size. It exists so the game can later support assets that reserve extra yard, plaza, setback, or service space without changing the core format.

## Validation Rules

The editor should be strict. Invalid content should fail before it enters a playable build.

Hard errors:

- missing required textures or generated outputs
- lot size outside supported runtime bounds
- missing `zone_type` on zoned buildings
- missing canonical mesh file
- invalid axes / origin conventions
- citizen asset lacks required `walk` source clip for VAT mode

Warnings:

- excessive triangle counts
- too many materials
- missing LOD mesh
- bounding box exceeds declared footprint
- thumbnail missing
- no license / attribution metadata

## Recommended Implementation Plan

### Phase 0: Shared Schema And Registry

- Define a shared asset schema in Rust, serialized as JSON.
- Add a pack registry and enable-disable list.
- Add runtime loading that reads manifests, not hardcoded directory assumptions.
- Redesign zoning storage so plot size is bounded by painted area, not by a fixed global `ZONING_DEPTH`.

### Phase 1: Building And Prop Importer

- Create the separate editor scene/executable.
- Support `.glb` import, thumbnail generation, metadata editing, variable lot-size authoring, lot-size validation, and pack saving.
- Hook building metadata into the existing variant and footprint systems.
- Replace fixed-depth zoning assumptions in storage, obstruction passes, rendering, and spawning so authored lot dimensions and runtime lot dimensions agree.
- Fix stale runtime `3x3` assumptions and building scale handling so authored lot dimensions and rendered dimensions agree.

### Phase 2: Vehicle Importer

- Add vehicle-class templates and lane-scale preview scenes.
- Support static meshes, color variants, thumbnails, and pack membership.

### Phase 3: Citizen Source Pipeline

- Add character source import and clip preview.
- Add offline baking to the chosen runtime format:
  - SDF billboard descriptor, or
  - VAT outputs from source clips
- Keep the shipped runtime asset free of skeleton cost.

### Phase 4: Content Manager

- Add in-game pack enable-disable UI.
- Surface author, license, version, and dependency warnings.

### Phase 5: Road / Network Template Editor

- Separate milestone.
- Define road assets as lane/topology/material templates, not as ordinary imported meshes.

## Recommendation

Implement the editor as a shared Godot-based tool mode with JSON manifests and `.glb` as the canonical asset format. Start with zoned buildings and props, then vehicles, then citizen source baking. Keep roads out of the first milestone.

For buildings specifically, treat plot size as required asset metadata from day one. `3x3` remains only a default preset, not a design limit, and fixed `ZONING_DEPTH` should be retired in favor of dynamic per-edge zoning extents.
