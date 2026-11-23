# Metrum Rise - Buildings

This document owns buildings as things: the **model contract** the models repository builds
buildings to, as the game relies on it; the **facts sidecar** a building model ships; the ground
rules between a model and the game's ground; the lot and site layout conventions of the reference
set; and the catalogue of that set.

It does not own:

- the asset editor, the model draft command, the acceptance check procedure and the ledger
  mechanics; see [`asset_editor.md`](asset_editor.md#model-draft-pipeline--tools-33)
- the `asset.toml` schema of a building; see
  [`asset_editor.md`](asset_editor.md#v1-building-assettoml-schema)
- business types, economy profiles, shop sizes and levels; see
  [`economy.md`](economy.md#asset-binding-and-failure-states) and
  [`demand.md`](demand.md#level-changes)
- zoning cells, lots and how lots are derived along roads; see [`zoning.md`](zoning.md) and
  [`building_allocator.md`](building_allocator.md)
- access: entrances, walkers at doors, parking, driveways, docks, freight gates and kerb
  deliveries; see [`entrance_and_exit.md`](entrance_and_exit.md) and
  [`traffic.md`](traffic.md#anchor-lots-park-01)
- how buildings are drawn: tier batches, colour schemes and night windows; see
  [`asset_editor.md`](asset_editor.md#gameplay-building-lods--render-07) (`RENDER-07` to
  `RENDER-11`)

Every section says whether it describes the game **as built** or work that is **planned**, with
its roadmap row.

| Topic | State | Row |
| --- | --- | --- |
| Model contract (frame, bounds, tiers, materials, textures) | as built; the reference set is built to it | `TOOLS-33` |
| Facts sidecar version 1 and version 4, read by the draft command | as built | `TOOLS-33` |
| Door floors and storeys used by the game | planned | `BUILD-02` |
| Floor slabs in door recesses at grade | as built in the models; the pack holds them where swapped in | none |
| Corner lots | planned | `ZONE-15` |

## Model contract

**As built: every building of the reference set is built to it in the models repository, and the
draft command ([`asset_editor.md`](asset_editor.md#draft-command)) reads it.** The models
repository's own `CLAUDE.md`, section "Output Contract — Buildings", is the builders' full
contract (style, construction, texture painting, validation). This section lists what the game
relies on; a change on either side is a change on both.

### Frame and pivot

- **glTF frame:** right-handed, metres, `+Y` up, `+Z` front, `+X` right. Blender builds with `Z` up
  and the front toward `-Y`; the export turns that into `+Z`. No hidden turn at load: the game
  places a building with its mesh part's own position, yaw and scale, and its frontage is `+Z`
  (`frontage_forward = [0, 0, 1]` in every drafted asset).
- **Pivot:** on the ground (`y = 0`) at the centre of `LOD0`'s **plan bounds**: eaves, steps,
  porches, fences and ladders included. It is not the centre of the walls (the sidecar's
  `footprint`): in the reference set the two differ by up to 3.12 m in depth. The game never
  assumes the origin is the wall centre; the draft positions a model by its bounds
  ([`asset_editor.md`](asset_editor.md#derivation-rules), R2).
- **One origin for every tier.** A simplified tier is never recentred on its own bounds.

### Bounds and footprint

- **Bounds** are `LOD0`'s vertex bounds, `min.y = 0`. The game imports `LOD0`'s bounds only
  (`nodes/sim/bridge/assets/mesh_bounds.rs`) for the lot fit, structural support and the tier
  metric; far tiers are never read for size.
- **Far tiers may be smaller than `LOD0`, never larger, and never move the pivot.** A simplified
  roof may lose its eaves; nothing may stick out past `LOD0`'s box. (On 2026-11-23 three models
  were found with a far tier past `LOD0`: `rivitalo_two_storey` `LOD2` by 0.64 m,
  `rivitalo_brick_gable` `LOD2` by 0.86 m, `hiilenvastaanotto_brick_shed` `LOD3` by 5 cm. The game
  is not hurt by it, since it reads `LOD0` only, but such a tier can poke through a neighbour's
  surface; they are the models side's to fix.)
- **Footprint** is the outer wall outline at ground level, one simple polygon wound positively in
  `[x, z]`, without eaves, steps, fences or ladders. The game uses it to say when a site surface
  lies under the building ([stale report](asset_editor.md#draft-command), site layout check).
- Assets that meet (a gallery and the hall it enters) meet on a shared plan-bounds plane and never
  overlap: the game may take the whole plan bounds as solid.

### Tiers

Four tiers, `LOD0` to `LOD3`, every one a complete building. Triangle ceilings, binding, with no
floor: a simple building does not add geometry to reach a number.

| Tier | Ceiling | Hero or landmark ceiling |
| --- | --- | --- |
| `LOD0` | 12,000 | 20,000 |
| `LOD1` | 4,000 | 6,000 |
| `LOD2` | 1,500 | 2,500 |
| `LOD3` | 500 | 700 |

Each tier models the silhouette, the roofline and the major recesses; windows, facade repetition
and small trim are texture. The draft's acceptance check warns over a ceiling
(`model_draft/acceptance.rs`); the models validator is the gate. A zoned building needs all four tiers; the editor still accepts shorter
chains, `LOD0` alone included, for hand-made assets.

**The game switches tiers by screen size, not by distance bands.** The shared policy
(`assets/lod_policy.rs`, [`TOOLS-05`](asset_editor.md#shared-lod-policy-and-automatic-inspection--tools-05))
projects the eight corners of the placed `LOD0` bounds and switches at 512 / 256 / 128 render
pixels of the larger projected side, with 10 % hysteresis, scaled by the player's graphics quality;
the gameplay renderer uses the same function ([`RENDER-07`](asset_editor.md#gameplay-building-lods--render-07)).
A small building therefore changes tier much nearer than a large one. Measured in the game at
1080p and a 75 degree field of view (2026-11-23, Balanced):

| Building | Largest bounds side | `LOD1` from | `LOD2` from | `LOD3` from |
| --- | --- | --- | --- | --- |
| kiosk (`lippakioski_corner_glazed`) | 6.75 m | about 11 to 15 m | 22 to 25 m | 52 m |
| salon and barber (`kampaamo_tile_corner`, `parturi_brick_pyramid`) | 10.1 to 10.9 m | about 20 m | 42 m | 90 m |
| pharmacy (`liikekeskus_pharmacy`) | 26.0 m | 50 m | 100 m | 200 m |
| grocery (`lahikauppa_corner`) | 35.0 m | 65 m | 120 m | 240 m |

The distance depends on the view: the kiosk read 11 and 22 m from the front and 15 and 25 m
from the rear-left in two measurements of the same day.

As a rule of thumb the switches fall at about 1.8, 4 and 8 times the largest bounds side. What a
tier drops must not pop at those distances: counters, bins, benches and signs seen against the
wall stay, simplified, until the tier whose switch hides their loss.

The manifest's `distance_min_m` / `distance_max_m` fields are kept as written (a draft writes the
old provisional 35 / 70 / 120 m bands, R6) but nothing selects by them; removing them is a later
schema migration, not a reinterpretation of metres as pixels.

### Materials and textures

- **One mesh and one material per tier**, the same material on every tier. The draft refuses a
  tier with any other material count (R7). (The models contract allows a second slot for a real
  UV constraint; no reference building uses one, and the draft does not take it.)
- **External PNGs** beside the `.glb` files, 1024 square (2048 only for a documented landmark),
  64 texels per metre, fully opaque, no embedded images:
  - an **albedo per colour variant**, `<name>_albedo_<variant>.png`; variants change textures
    only, never geometry or UVs. The game turns each variant into a colour scheme
    ([`RENDER-08`](asset_editor.md#gameplay-colour-scheme-selection--render-08)); a set of
    buildings meant to stand together (a plant, a farm) shares variant names;
  - one **ORM** (occlusion, roughness, metallic) and one tangent-space **normal** map, shared by
    every variant;
  - an **emission** mask for a building with glazing or a lit sign.
- **Night lighting is the game's.** The models export the emission texture bound with the glTF
  emissive factor at zero; the game ignores that factor and drives emission itself from the mask,
  a per-building schedule and the asset's `window_brightness` (default 3)
  ([`RENDER-09`](asset_editor.md#gameplay-window-emission--render-09) to
  [`RENDER-11`](asset_editor.md#window-brightness-and-variation--render-11)). The mask marks every
  pane as the lit room behind it, in grey; colour, intensity and the on and off hours are the
  game's. No schedule or random occupancy is baked into a texture.
- **Sign lettering is Finnish** (the modellers' rule): the generic word for the business
  (`APTEEKKI`, `PARTURI`, `KAMPAAMO`, `KIOSKI`) or a fictional name, never a real brand.
- **No interiors, opaque glass.** Glass is an opaque closed surface, doors are closed, and no
  opening shows an unmodelled inside. Nothing in the game looks into a building.

## Facts sidecar

**As built: the models build writes one per building; the draft command reads versions 1 and 4
(`model_draft/sidecar.rs`).** One JSON file per model, `export/<name>.model.json`, beside the
`.glb` tiers. UTF-8, no comments; unknown keys are refused (`deny_unknown_fields`, as the game's
manifests are). It states geometry and files only: no zone, lot, capacity, anchor or other game
term.

Units and frame: metres, in the frame of the exported `.glb` files ([Frame and
pivot](#frame-and-pivot)). No second frame exists in the file: a consumer may add any sidecar
point to a `.glb` vertex. For the builder, a Blender point `(bx, by, bz)` with the build's pivot
offset `(px, py, pz)` (`asset_build.pivot_offset`) is `[bx - px, bz - pz, -(by - py)]`. Plan
points are `[x, z]` pairs of that frame.

### Versions

One version sequence for the one schema, whatever the kind: a number names one document shape.

| Version | Kind | Shape | Read by the game |
| --- | --- | --- | --- |
| 1 | buildings and boundary props | the fields below; doors without `floor_y_m` and `storey` | yes |
| 2 | vehicles (`"kind": "vehicle"`) | [`vehicles.md`](vehicles.md#facts-sidecar) | yes |
| 3 | vehicles | version 2 plus `doors` ([`TOOLS-43`](vehicles.md#doors-sidecar-version-3-tools-43)) | yes |
| 4 | buildings and boundary props | version 1 with `floor_y_m` and `storey` on every door | yes |

The models build writes version 4 for buildings and boundary props since models commit
`0f48233` (2026-11-23); the sidecars in the reference pack's ledger are still version 1 until the
regenerated models are swapped in. A building sidecar without a `kind` key is read as version 1
or 4; any other version is refused by name ("sidecar version 5 is not supported; building
sidecars are versions 1 and 4"), before any field is read, so a newer file is refused for its
version and not for the keys it adds. A version 1 door with either new key is refused; a version 4
door without either is refused, naming the door.

### Top-level fields

Keys in this order in both building versions.

| Field | Type | Meaning |
| --- | --- | --- |
| `schema` | string | `"metrum-model-facts"` |
| `version` | integer | `1` or `4` ([Versions](#versions)) |
| `name` | string | the asset folder name, e.g. `rintamamiestalo_porch` |
| `path` | string | repository path of the asset folder, e.g. `buildings/residential/rintamamiestalo_porch` |
| `source.commit` | string | 40-hex commit of the models repository whose build scripts reproduce the files |
| `source.dirty` | boolean | whether the asset folder or `scripts/` differed from that commit |
| `source.blender` | string | version of the Blender that wrote the sidecar (the `.glb` files may come from an earlier export) |
| `license` | string | SPDX id of the content, `CC-BY-4.0` |
| `bounds.min`, `bounds.max` | `[x, y, z]` | `LOD0` vertex bounds; `min.y` is `0` |
| `footprint` | array of `[x, z]` | outer wall outline at ground level, one simple polygon, no repeated end point, positive shoelace area in `[x, z]` from any start point; excludes eaves, steps, fences, ladders |
| `storeys` | array of `{floor_y_m, area_m2}` | one entry per occupied storey, lowest first; `area_m2` is the area inside the outer wall faces, and for a storey under a roof only the part with at least 1.6 m to the roof underside; empty for a model with no occupied storey |
| `dwellings` | integer | separate dwelling units with their own front door; `0` for a building that is not housing |
| `doors` | array | every door leaf or door pair in an outer wall ([Doors](#doors)); empty for a model without one |
| `lods` | array of `{file, triangles, sha256}` | nearest first; `file` is relative to the sidecar's folder |
| `textures.orm`, `.normal`, `.emission` | `{file, sha256}` | shared maps; `emission` absent for an unlit building |
| `variants` | array of `{id, albedo, sha256}` | every colour variant; `id` is the `<variant>` of `<name>_albedo_<variant>.png` |
| `default_variant` | string | the `id` the `.glb` files reference |
| `preview` | `{file, sha256}` | `../preview.png`, the catalog render |
| `hero` | boolean | whether the hero or landmark triangle ceilings apply |
| `passages` | array, optional | every lane a vehicle drives into or through the model ([Passages](#passages-and-vehicle-lanes)) |

The rule for the file (the models validator checks the same rule; the game's reader refuses a
sidecar that breaks it):

- `doors` may be empty. At most one door has `main: true`, and that door has `kind` `"entrance"`.
- `storeys` may be empty; the entries it has are listed lowest first, with a finite `floor_y_m`
  and a finite `area_m2` of 0 or more.
- Every listed file hashes to its `sha256`; the files lie in the export folder, except the
  preview (`../`).

A sidecar with no main door, or with no storey, is a valid file: it describes a piece that
honestly has none, such as a chimney, or one part of a multi-building placed asset (a power plant)
that is assembled by hand from several sidecars and never drafted alone. What a draft needs of a
model is the draft's precondition (a zoned building needs one main entrance; a residential one
needs storeys; [`asset_editor.md`](asset_editor.md#draft-command)). Lengths are rounded to 0.1 mm
and areas to 0.01 m².

### Doors

One entry per door leaf or door pair in an outer wall. Keys in this order: version 1 `id`, `kind`,
`position`, `facing`, `width_m`, `height_m`, `main`; version 4 `id`, `kind`, `position`, `facing`,
`width_m`, `height_m`, `floor_y_m`, `storey`, `main`.

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | string | unique in the asset, e.g. `front`, `back`, `unit_3` |
| `kind` | string | `entrance` (a pedestrian door from the ground into the occupied space), `service` (a staff or back pedestrian door at ground level), `vehicle` (loading and goods doors), `balcony` (any door not reachable from the ground, upper-floor doors included), `store` (sheds and unheated stores) |
| `position` | `[x, y, z]` | centre of the threshold on the outer wall plane; `y` is the threshold height above ground (a balcony door carries its upper floor's) |
| `facing` | `[x, 0, z]` | unit outward normal of that wall |
| `width_m`, `height_m` | number | clear opening of the leaf or pair, without fanlight |
| `floor_y_m` | number, version 4 | metres above grade of the walking surface at the threshold, `>= 0`, measured on `LOD0` just behind the wall plane where the leaf stands: 0.02 on a floor slab, 0.0 at bare grade (no door of the reference models reads 0.0 since every door at grade stands on a slab), the top of the step, landing, deck, sill or dock a raised door stands on, a sill's own height where nothing is under it |
| `storey` | integer or `null`, version 4 | the 0-based index into `storeys` of the storey the door opens to: the listed storey whose floor is nearest the door's `floor_y_m`, within 0.5 m (the lower on a tie); `null` when none is, or none is listed. The builder may state either value where the measurement cannot decide |
| `main` | boolean | at most one door is `true`, and that door is an `entrance`: the entrance a visitor uses |

The builder tags the main door; the reader never picks one. A door set back in a lobby or reveal
keeps its `position` on the outer wall plane; a door in the side wall of a pocket lies on a
`footprint` edge.

`position.y` and `floor_y_m` answer different questions. `position.y` is where the door opening
starts in the wall; `floor_y_m` is what a person stands on there. Examples from the reference
models:

- A raised entrance reads its landing: the machinery factory's main door
  (`konepaja_element_monitors`, `entrance`) has its threshold at 0.1562 m on a stepped landing,
  so its `floor_y_m` is 0.1562 and its `storey` 0.
- A door at grade in a recess stands on its slab: `floor_y_m` 0.02, `storey` 0 for a ground floor
  at 0.
- A balcony door reads the balcony deck: a door whose `position.y` is 3.4375 (the upper storey's
  floor, as on the terrace balconies) on a deck whose top is 3.5 has `floor_y_m` 3.5 and opens to
  that storey.
- A loft hatch in a gable opens to no listed storey (the loft is not occupied space): `storey`
  `null`.

The values above illustrate the rule; the regenerated sidecars state each model's own. Version 4
in full, one door (the schema's example):

```json
{"id": "front", "kind": "entrance", "position": [0.0, 0.1562, 13.015], "facing": [0.0, 0.0, 1.0], "width_m": 2.0, "height_m": 2.3438, "floor_y_m": 0.1562, "storey": 0, "main": true}
```

The reader refuses, naming the door: a missing `floor_y_m` or `storey` in version 4, a
`floor_y_m` that is not a finite number of 0 or more, a `storey` that is neither `null` nor a whole
number below the count of `storeys`, and either key in version 1.

### Passages and vehicle lanes

`passages` (`PARK-21`, added 2026-11-21; optional in both building versions): every lane a
vehicle drives into or through the model.

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | string | unique in the asset, e.g. `tunnel`; the passage's `name` in the manifest |
| `position` | `[x, 0, z]` | centre of the mouth on the outer wall plane, at ground level (`y` within 1 mm of 0) |
| `forward` | `[x, 0, z]` | unit direction into the model from the mouth: the opposite of the mouth's outward normal |
| `width_m` | number | clear width |
| `length_m` | number | depth from the mouth to the far end |
| `clear_height_m` | number | clear height; at least 4 m lets a truck through, at least 2 m a car |
| `through` | boolean | whether the far end is open too |

The fields are a mesh part passage's (`[[mesh_parts.passages]]`) in the sidecar's own frame, the
`.glb`'s, which is the drafted part's local frame: the draft copies each entry onto the part that
shows the model, and the part's position places it with the model. For a model whose lanes run
along its own axes, `forward` is one of `[0, 0, ±1]`, `[±1, 0, 0]`. The reader refuses an unknown
key in an entry, a repeated or empty `id`, a non-finite or off-ground mouth, a forward that is not
a horizontal unit vector (within 0.01), and a non-positive size; whether a passage lies inside the
model and runs along a lot axis is the drafted asset's validation. Absent, the model says nothing
about lanes; present (an empty array included), it is the whole list. Example, the coal bunker's
tunnel as the asset authors it on its part (a model build writes its own numbers):

```json
"passages": [
  {"id": "tunnel", "position": [-1.9, 0.0, 8.125], "forward": [0, 0, -1], "width_m": 5.0, "length_m": 16.25, "clear_height_m": 4.6, "through": true}
]
```

A loading point needs no field beyond a `vehicle` door. No loading bay, exit, driveway or way is
derived from a sidecar: docks are the site layout's, authored in the pack (freight yards owner
decision 8, [`traffic.md`](traffic.md#owner-decisions-freight-yards)).

### Worked example

`rintamamiestalo_porch` in version 1 (bounds, triangles, `LOD0` hash and commit read from the
files; door and footprint points computed from its `build.py`; storey areas illustrative;
`<sha256>` stands for a 64-hex digest). In version 4 the same document has `"version": 4` and each
door gains `floor_y_m` and `storey` before `main` (here `0.70` and `0` for both doors).

```json
{
  "schema": "metrum-model-facts",
  "version": 1,
  "name": "rintamamiestalo_porch",
  "path": "buildings/residential/rintamamiestalo_porch",
  "source": {"commit": "9c93f65ae0f3e78d40c9c86f3b65cc290965c35a", "dirty": false, "blender": "5.2.0"},
  "license": "CC-BY-4.0",
  "bounds": {"min": [-5.35, 0.0, -6.84], "max": [5.35, 9.5625, 6.84]},
  "footprint": [[-5.0, -5.1], [5.0, -5.1], [5.0, 2.9], [0.3, 2.9], [0.3, 5.1], [-2.7, 5.1], [-2.7, 2.9], [-5.0, 2.9]],
  "storeys": [{"floor_y_m": 0.70, "area_m2": 86.6}, {"floor_y_m": 3.80, "area_m2": 58.0}],
  "dwellings": 1,
  "doors": [
    {"id": "front", "kind": "entrance", "position": [-1.2, 0.70, 5.1], "facing": [0, 0, 1], "width_m": 0.90, "height_m": 2.10, "main": true},
    {"id": "back", "kind": "service", "position": [1.5, 0.70, -5.1], "facing": [0, 0, -1], "width_m": 0.90, "height_m": 2.10, "main": false}
  ],
  "lods": [
    {"file": "rintamamiestalo_porch_lod0.glb", "triangles": 2642, "sha256": "9729cbe0649a29cace4df2d3f04418d3e8033130c25acabca2cf53e9340b75fb"},
    {"file": "rintamamiestalo_porch_lod1.glb", "triangles": 2054, "sha256": "<sha256>"},
    {"file": "rintamamiestalo_porch_lod2.glb", "triangles": 682, "sha256": "<sha256>"},
    {"file": "rintamamiestalo_porch_lod3.glb", "triangles": 342, "sha256": "<sha256>"}
  ],
  "textures": {
    "orm": {"file": "rintamamiestalo_porch_orm.png", "sha256": "<sha256>"},
    "normal": {"file": "rintamamiestalo_porch_normal.png", "sha256": "<sha256>"},
    "emission": {"file": "rintamamiestalo_porch_emission.png", "sha256": "<sha256>"}
  },
  "variants": [
    {"id": "red", "albedo": "rintamamiestalo_porch_albedo_red.png", "sha256": "<sha256>"},
    {"id": "yellow", "albedo": "rintamamiestalo_porch_albedo_yellow.png", "sha256": "<sha256>"},
    {"id": "white", "albedo": "rintamamiestalo_porch_albedo_white.png", "sha256": "<sha256>"},
    {"id": "blue", "albedo": "rintamamiestalo_porch_albedo_blue.png", "sha256": "<sha256>"},
    {"id": "green", "albedo": "rintamamiestalo_porch_albedo_green.png", "sha256": "<sha256>"}
  ],
  "default_variant": "red",
  "preview": {"file": "../preview.png", "sha256": "<sha256>"},
  "hero": false
}
```

### What the game reads

Nothing at run time: a city loads the pack asset (`asset.toml` and its files), never a sidecar.
The draft command reads every field to validate the file, and uses:

| Fact | Used for |
| --- | --- |
| `bounds` | lot cells (R1), part position (R2), the lot fit and free space |
| `footprint` | the stale report's site layout check (a surface under the walls) |
| `storeys`, `dwellings` | household capacity and flat size (R5) |
| `doors` (`main`) | the `entrance/main` anchor (R4) and its distance check |
| `doors` (all, `kind`) | door sanity in the acceptance check, the doors listed in the lot frame for the author, the loading bays the editor offers at `vehicle` doors (`TOOLS-40`) |
| `lods`, `textures`, `variants`, `preview` | the published files, colour schemes (R7), thumbnail (R8), hashes and the stale report |
| `hero` | a hero model is refused for a zoned role |
| `passages` | the drafted part's passages |

**Stored for later: `floor_y_m` and `storey`.** No asset field holds a door: the draft turns the
main door into the entrance anchor and keeps the rest of the model's facts only in the pack's
ledger, which copies the sidecar as read (`model_sources/<asset_id>.json`,
[`asset_editor.md`](asset_editor.md#draft-command)). A version 4 sidecar's door floors and storeys
are therefore stored there, word for word, and the draft's report shows them in its door list
(each door's floor through the part, and its storey). Nothing in the simulation or the renderer
reads them yet. Uses planned under `BUILD-02`:

- a walker standing on a door's own floor instead of the ground, at a raised entrance or a slab;
- doors above grade (a balcony, a loading dock's sill) that no walker or truck is sent to as if
  at grade;
- lighting floor by floor: which storey's windows a door belongs to.

**Not in the data: walkable surfaces.** Stairs, ramps, landings and decks are geometry only; the
sidecar states where a door's floor is, not how a person gets up to it. A walker following the
steps needs walkable surfaces, which no sidecar version describes.

## Ground rules between model and game

**As built.**

- **The game's ground carries vehicles and walkers, never the mesh.** Their height is the graded
  terrain ([`terrain.md`](terrain.md)) plus the asset's site surfaces, drawn at about `y` 0.01
  (`y_m = 0.01` in every reference asset). A walker reaches a door at ground height, 2 cm under a
  slab's top; a car parks on the site surface.
- **Floor slabs in door recesses.** Because the ground shows wherever the model has no upward face,
  every opening recessed at grade has a concrete slab filling its plan in the models: top 0.02 m
  above grade (a face at grade fights the terrain), a nosing on the wall plane, nothing past it,
  on every tier that models the recess. None is needed where the opening is painted flat on a
  tier, stands on a step, landing, sill or plinth, opens off a floored lobby or porch, is a
  balcony door, or is an open bay or a drivable lane (the sidecar's passages), whose floor is the
  game's ground by design.
- **A slab changes nothing else.** It never changes bounds, pivot, footprint or door facts
  (`position`, size); version 4 reads its top as the door's `floor_y_m` (0.02). A pack asset takes
  a slab by a file swap alone.
- **A pivot move is compensated in the pack.** When a rebuilt model's pivot moves (its plan
  bounds grew on one side), the pack asset's mesh part position moves by the opposite amount, so
  walls, doors and anchors stay where they were in the lot. On 2026-11-23 the pharmacy
  (`liikekeskus_pharmacy`, +0.09 m in `x`) and the grocery (`lahikauppa_corner`, +0.09 m in `z`)
  moved 9 cm; their parts moved −0.09 m on the same axis, and every door and footprint corner was
  checked to stand within 0.00 mm of its old place in the lot.
- **Passages and truck lanes are declared in the pack against the model's opening, to the
  centimetre.** A part's passage must lie inside the opening it drives through: the power plant's
  coal shed lane was narrowed from 5.6 m to 5.575 m and its centre moved 1.25 cm when the shed's
  rebuilt opening narrowed (2026-11-23). The sidecar's `passages` are the reference.

## Lots and site layout

**As built, in the reference set (the Kuopio pack).** The rules a lot obeys are
[`zoning.md`](zoning.md)'s and the allocator's; this section records how the reference buildings
use them.

- **The zoning cell is 10 m** (`ZONE_CELL_M` in `assets/asset/validation.rs`, the editor's
  `EDITOR_LOT_CELL_M`, and `WorldConfig::zone_cell_m`, 10.0 in every constructor). A lot is
  `lot_width_cells` × `lot_depth_cells` cells, its front edge on the street.
- **Lot sizes in use:** kiosk 1 × 1; barber and salon 2 × 2; pharmacy 3 × 3; grocery 4 × 4;
  houses 2 × 3 (a back yard behind the 2 × 2 the draft gives); rowhouses 4 × 2; machinery factory
  5 × 3; food factory 6 × 3. The explicit (placed by the player) assets: grain farm 4 × 3, coal
  mine 5 × 4, power plant 9 × 5.
- **The building stands at the back of its lot with a forecourt in front.** The draft starts it
  flush with the back edge and centred across (R2); the shops keep that, and an asphalt or
  concrete forecourt runs from the front wall to the street.
- **Parking places sit at the street edge, away from the door,** opening straight onto the
  street (a bay on the street edge needs no driveway, [anchor
  lots](traffic.md#anchor-lots-park-01)), so the walk from the door to the street stays clear:
  the barber and the salon have two such places each on the half without the door, the pharmacy
  six on either side of its central door, the machinery factory six. The grocery's lot is deep
  enough for a car park: seventeen places in rows off an aisle, reached through its gate.
- **A 1 × 1 lot has no dock, driveway or parking.** The kiosk is delivered at the kerb: a truck
  stops at the kerb before the door and stands there ([kerb
  deliveries](traffic.md#kerb-deliveries-and-the-dock-queue-park-36-as-built), `PARK-36`).
- **Dock buildings have a freight gate**: a `freight` driveway at least 4 m wide on the street
  edge, and one `loading_bay` inside the lot reached through it ([freight
  yards](traffic.md#freight-yards-park-19-to-park-22)). In the reference set: the grocery (6 m
  gate, a bay backed into, `stand = "tail_in"`), the pharmacy (4 m), the food and machinery
  factories (6 m gates, bays backed into at their loading doors), the coal mine (separate 6 m
  `truck_in` and `truck_out` gates, a drive-through through the bunker's `tunnel` passage), the
  power plant (6 m `truck_gate`, coal tipped in the shed's lane) and the grain farm (4 m).
  A truck that finds the dock taken waits in line at the kerb before the gate (`PARK-36`).
- Houses have two places at the street edge and a 3.5 m car driveway; rowhouses have neither.

## Catalogue

**The reference set: the Kuopio pack**, the owner's own pack of Finnish buildings from the models
repository (CC BY 4.0), not shipped in this repository. Twenty assets, read from their
`asset.toml` files and ledger entries on 2026-11-23. "Ledger" names the models commit the files
were copied from; the three multi-model assets have no ledger entry (a ledger entry holds one
sidecar). All ledger sidecars are version 1 on that date.

| Asset id (`building.…`) | Source model(s) | Use (economy profile) | Lot | Colour schemes (default first) | Notable facts |
| --- | --- | --- | --- | --- | --- |
| `commercial.kiosk_1` | `lippakioski_corner_glazed` | grocery kiosk (`grocery_kiosk`) | 1 × 1 | ochre, cream, green, red boards | no parking or dock: kerb deliveries; lot-wide paving; ledger `dc0fd03` |
| `commercial.barber_1` | `parturi_brick_pyramid` | barber (`personal_service_small`) | 2 × 2 | yellow, brown, red, white brick | drafted by the tool; lobby slab in the entrance recess; sign PARTURI; ledger `dc0fd03` |
| `commercial.hair_salon_1` | `kampaamo_tile_corner` | hair salon (`personal_service_small`) | 2 × 2 | teal, blue, brown, rose tile | a corner shop on a mid-block lot (`ZONE-15`); shop-door slab; ledger `dc0fd03` |
| `commercial.pharmacy_1` | `liikekeskus_pharmacy` | pharmacy (`health_essentials_small`) | 3 × 3 | grey, beige, brown, red stone | part moved −0.09 m in `x` for the 9 cm pivot move; 6 places, dock; ledger `dc0fd03` |
| `commercial.grocery_store_1` | `lahikauppa_corner` | grocery (`grocery_basic`) | 4 × 4 | red, brown, grey, yellow brick | 45° corner door; part moved −0.09 m in `z`; lobby slab; 17 places, backed-into dock; ledger `dc0fd03` |
| `industrial.food_processing_factory_1` | `tehdas_brick_gable` | food processing (`food_processor_basic`) | 6 × 3 | red, brown, grey, yellow brick | eight doors; `forecourt` passage; backed-into dock at the loading door; the only `--check` failure: `entrance/main` 1.30 m from the `front` door |
| `industrial.machinery_factory_1` | `konepaja_element_monitors` | machinery (`machinery_factory_basic`) | 5 × 3 | grey, beige, red aggregate | raised main entrance, threshold 0.1562 m on its landing; `forecourt` passage; backed-into dock; ledger `dc0fd03` |
| `residential.house_1`, `_level_2` | `rintamamiestalo_balcony` | detached house, 1 household, 80 m² | 2 × 3 | red, blue, green, white, yellow | front and back doors at 0.70 m (raised floor), a balcony door; level 2 is the same model placed further forward |
| `residential.house_2`, `_level_2` | `rintamamiestalo_gable_entry` | detached house, 1 household, 80 m² | 2 × 3 | red, blue, green, white, yellow | gable entrance; balcony door at 3.9 m |
| `residential.house_3`, `_level_2` | `rintamamiestalo_porch` | detached house, 1 household, 80 m² | 2 × 3 | red, blue, green, white, yellow | the sidecar's worked example |
| `residential.house_4`, `_level_2` | `rintamamiestalo_side_entry` | detached house, 1 household, 80 m² | 2 × 3 | red, blue, green, white, yellow | side entrance facing `+X` |
| `residential.rowhouse_1` | `rivitalo_two_storey` | terrace, 6 households, 80 m² | 4 × 2 | yellow, brown, green, red, white | 18 doors (unit, back and store doors at 0.35 m); its part is named after the other terrace's file |
| `residential.rowhouse_2` | `rivitalo_brick_gable` | terrace, 4 households, 100 m² (hand value; R5 gives 90) | 4 × 2 | brown, green, grey, ochre, red | balcony doors at 3.4375 m |
| `agricultural.grain_farm_1` (explicit) | `rintamamiestalo_porch`, `navetta_board_gable`, `viljakuivuri_tower`, `konehalli_open_bays` | grain farm (`grain_farm_basic`) | 4 × 3 | red, grey, yellow ochre | four parts turned on the yard; farmyard surface; dock |
| `industrial.coal_mine_1` (explicit) | `kaivoskonttori_brick`, `hiilisiilo_drive_through`, `kaivostorni_concrete`, `nostokonehuone_brick` | coal mine (`coal_mine_basic`) | 5 × 4 | red, yellow, brown brick | drive-through dock through the bunker's `tunnel` (part turned 180°) |
| `power.power_plant_1` (explicit) | `voimalaitos_brick_hall`, `savupiippu_brick_round`, `hiilenvastaanotto_brick_shed`, `voimalan_konttori_brick`, `muuntajakentta_walled`, six `tiilimuuri_*` wall pieces | power plant (`power_plant_basic`, service `power`) | 9 × 5 | red, yellow, brown brick | 43 parts, the wall pieces placed 38 times round the lot; `truck_lane` and `under_gallery` passages |

## Open items

- **`BUILD-02`, planned, later:** the game uses `floor_y_m` and `storey` ([What the game
  reads](#what-the-game-reads)). Not started; nothing reads them.
- **The regenerated version 4 models** are not in the reference pack yet. At the swap every
  ledger entry is rewritten from its version 4 sidecar; before that, `--check` with a version 4
  sidecar against a version 1 ledger gives one information line, not a stale line
  ([`asset_editor.md`](asset_editor.md#draft-command)).
- **`ZONE-15`, corner lots:** the kiosk, the salon and the grocery are corner models
  (`*_corner*`) placed on ordinary lots, their corner door facing the street side.
- **`PARK-32`, both-sides lot access:** a car or truck cannot turn left into or out of a lot; it
  drives on to turn round. It decides how parking and gates sit on a lot only once built.
- **The food factory's entrance anchor** stands 1.30 m from its `front` door, the one acceptance
  failure of the reference set; the anchor is the owner's to move (no roadmap row).
- **Door recesses at grade** all stand on slabs since the models of 2026-11-23 (every pack asset
  takes them; the five doors where ground showed in the reveal are closed). One line of ground
  remains along the food factory's main facade: the pack's yard edge falls short of the wall
  there, the owner's to move.
- **Thumbnails**: the barber's, salon's and kiosk's are the tool's own; the other updated assets
  keep the owner's captures, with tool-made candidates saved.
- **Walkable surfaces** (stairs, ramps) are not in any data; later, after `BUILD-02`.
