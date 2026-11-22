# Metrum Rise - Vehicles

This document owns vehicles as things: their models, sizes, lamps and glass, how they are
declared in packs, and how they are loaded and drawn. It holds the **model contract** that the
models repository builds to.

It does not own:

- how vehicles move: car following, junctions, lane changes, render smoothing, parking, parked
  cars and freight yards; see [`traffic.md`](traffic.md)
- attach and detach at buildings and the off-lane ground support solve; see
  [`entrance_and_exit.md`](entrance_and_exit.md)
- the pack format, the asset editor and the draft command in general; see
  [`asset_editor.md`](asset_editor.md)
- dated GPU measurement records of the car meshes (`RENDER-46`, `RENDER-48`); see
  [`terrain.md`](terrain.md#one-body-surface-per-car-mesh-street-lights-left-in-their-cells-render-46-2026-11-13)

Every section says whether it describes the game **as built** or work that is **planned**, with
its roadmap row. The owner accepted the plan on 2026-11-21.

| Topic | State | Row |
| --- | --- | --- |
| Model contract and pilot pair | as built; the pilot pair is the game's traffic | `TOOLS-41` |
| `[vehicle]` assets in packs, vehicle drafts | as built | `TOOLS-41` |
| Bundled `vehicles` pack, removal of the placeholder cars | as built | `TOOLS-42` |
| Vehicle renderer: catalogue, shader, tiers (stage 1a) | as built | `RENDER-65` |
| Brake lights, indicators, reverse lamp, steered and rolling wheels (stage 1b) | as built | `RENDER-66` |
| Low and high beam, hazard lights, later lamps, glass features | planned | `RENDER-67` |
| Lane length by body class | as built | `TRAFFIC-38` |
| Waiting position at junctions | planned | `TRAFFIC-39` |
| Night lamps (glows, beams, the headlight pool) | as built, from the sidecar lenses | `RENDER-26` |

## Model contract (`TOOLS-41`)

**As built: the pilot pair is built to it in the models repository ([Pilot pair](#pilot-pair)) and
drawn by the game ([renderer](#as-built-renderer-render-65)).** This is the authoritative copy; the models repository's own
`CLAUDE.md`, section "Vehicles", states the same contract for its builders. The game
turns the wheels, lights the lamps and paints the body itself, per vehicle, from ids stored in
the mesh: **no rig, no animation, no separate wheel files; one mesh and one material per tier.**
Lamp and glass groups the game does not use yet are still required, so that no model is
reworked later.

### Body classes

A model belongs to one body class and must fit that class's box. The game parks and queues
every vehicle of a class as the same box, so adding or removing models never changes simulation
results.

| Class | Box, length × width × height (m) | Why |
| --- | --- | --- |
| `car` | 4.50 × 1.80 × 1.95 | the car lot proof body, 1.8 × 4.5 m, clear height 2.0 m |
| `van` | 6.00 × 2.10 × 2.70 | a large panel van; uses freight gates and docks |
| `truck` | 8.00 × 2.55 × 3.80 | the freight lot proof body, 2.6 × 8.0 m, clear height 4.0 m |
| `bus` | 12.00 × 2.55 × 3.20 | a 12 m city bus (not ordered yet) |

- Width is without mirrors. Mirrors may add 0.12 m a side on a car and 0.25 m on the others.
  Nothing but the mirrors reaches past the width: the validator takes vertices outside it, above
  the lowest wheel centre and in the front half, for mirrors.
- The box applies to every tier as exported, wheels straight ahead. A steered wheel may leave it.
- Sizes are targets within ±0.05 m. A dimension whose target equals a class limit has the
  tolerance -0.05 to 0, so it is built slightly under: the pilot estate is 4.48 m long and the
  pilot truck 2.53 m wide, nominal.

### Fleet

| Model | Class | Length × width × height (m) | Wheelbase | Wheel radius | Suggested share in class | Order |
| --- | --- | --- | --- | --- | --- | --- |
| Compact estate | car | 4.50 × 1.80 × 1.47 | 2.65 | 0.32 | 25 % | pilot |
| Compact hatchback | car | 4.30 × 1.79 × 1.46 | 2.62 | 0.32 | 20 % | fleet |
| Small hatchback | car | 4.00 × 1.74 × 1.48 | 2.55 | 0.30 | 15 % | fleet |
| Sedan | car | 4.50 × 1.78 × 1.44 | 2.68 | 0.32 | 10 % | fleet |
| Compact SUV | car | 4.40 × 1.80 × 1.62 | 2.65 | 0.34 | 20 % | fleet |
| Mid SUV | car | 4.50 × 1.80 × 1.68 | 2.69 | 0.36 | 5 % | fleet |
| Small van | car | 4.45 × 1.80 × 1.83 | 2.75 | 0.32 | 5 % | fleet |
| Large panel van | van | 5.95 × 2.05 × 2.60 | 3.66 | 0.35 | 100 % | fleet |
| Rigid box truck, two axles | truck | 7.90 × 2.55 × 3.70 | 4.50 | 0.52 | 100 % | pilot |

The pilot pair proves the contract. The seven fleet models are ordered together, and only after
the pilot pair has been seen in the engine (the owner, 2026-11-21). The
pilot estate: five doors, roof rails, track 1.54 m. The pilot truck: cab over engine, twin rear
wheels, box body with a rear roller door, under-run bars, front track 2.05 m. No real makes.
Later, not ordered: a city bus, emergency and service vehicles, trailers.

### Frame

- Blender `Z` up, front toward `-Y`. Exported glTF: `+Y` up, nose at `+Z`; in that frame `+X` is
  the driver's left.
- Origin on the ground at the centre of the body's plan box (length × width without mirrors).
  One origin for all tiers. The simulation's lane position and a parking bay's centre are both
  the body centre.
- Tyres touch `y = 0` exactly; nothing is below it, on any tier.
- Symmetric about `x = 0`: wheel and lens pairs mirror within 1 mm. A lens of `reverse` (6),
  `fog_rear` (11), `plate` (12), `beacon` (13) or `sign` (14) may stand alone off the centre
  line; a car's high-mounted brake lamp is one lens on the centre line. The sidecar has no field
  for any other asymmetry.
- "Left" and "right" always mean the driver's. The sidecar names every side, so the game never
  infers one from a sign.

### Wheels

- In the source file each wheel is its own object with its origin at the wheel centre. At export
  everything is joined into one mesh.
- A wheel is a closed solid (tyre, rim face, inner face), so a turned wheel shows no hole. A
  truck's twin rear wheels are one part a side, and their axle says `twin: true`.
- Parts are numbered in sidecar order: front axle first, left before right.
- The arch is a dark recess. Every wheel straight ahead, and every front wheel turned 35 degrees
  either way about the vertical axis through its centre, keeps 10 mm from every body triangle.
- The game spins a wheel about the model `X` axis and steers it about `Y`, both through the
  wheel centre stated in the sidecar. Signs, the same in the models repository's tooling: a
  positive steer angle turns the wheel's nose toward `+X` (the driver's left, a left turn), a
  right-handed rotation about `+Y`; a positive spin rolls the wheel forward (the top of the tyre
  moves toward `+Z`), a right-handed rotation about `+X`.
- No brake discs, suspension or tread geometry. At most 8 wheel parts.

### Vertex ids

Every tier of every vehicle, pilot and fleet, carries its ids in one attribute: glTF `COLOR_0`
as a **normalised `UNSIGNED_BYTE` `VEC4`**. There is no float form and no second attribute.

| Channel | Meaning | Byte value |
| --- | --- | --- |
| R | part | `0` body; `1` front left wheel, `2` front right, `3` rear left, `4` rear right (up to `8`, in sidecar order) |
| G | surface | `0` plain; `1..15` a lamp; `32..47` glass; other values unused |
| B | paint | `255` on painted body panels, `0` elsewhere (glass, tyres, trim, lamps, plates) |
| A | unused | `255` |

- The three corners of every triangle carry identical values, R, G and B alike: ids never blend
  and paint never fades across a panel edge.
- A lamp or glass face is body: R = 0 and B = 0 wherever G is not 0.
- B is 0 or 255 at every vertex.
- A face is a lamp, or glass, or neither, so one channel holds both ranges.

How the bytes are produced (models side, tested 2026-11-21 with the Blender 5.3.0 exporter):

- Author a **linear float** colour attribute on the **face-corner** domain. A byte colour
  attribute in Blender is sRGB-encoded and bends the ids.
- The exporter writes `COLOR_0` as normalised `UNSIGNED_SHORT`. The models build converts it to
  normalised `UNSIGNED_BYTE` without loss and refuses any value more than 1e-3 off a whole byte.
- The exporter splits vertices where ids differ: no extra vertices on a surface of one id, up to
  33 to 48 % more on a mesh with an id per triangle.
- Draco and meshopt compression stay off.
- Blender's own importer is lossy for `COLOR_0`, so the validator reads the GLB accessors
  directly, never a re-import.

`COLOR_0` was chosen knowingly: standard glTF viewers multiply it into the base colour and show
these files dark. A custom attribute (`_METRUM_IDS`) was tried on the pilot and dropped from the
contract: Godot's stock glTF importer discards it. What the game does with the bytes, and what
was and was not tested there, is under
[the renderer's id decode](#as-built-renderer-render-65).

### Lamps

A lamp is ordinary faces of the mesh (a lens quad or a few triangles) with a lamp id in G. The
lens is painted unlit in the albedo and lit, in its own colour, in the emission texture. The
model has no glow, halo or beam geometry; the game adds those.

| Id | Group | Colour | Where | Lit by the game when |
| --- | --- | --- | --- | --- |
| 1 | `tail` | red | rear, both sides | lights on (night) |
| 2 | `brake` | red | rear, both sides, and a car's high-mounted lamp | braking |
| 3 | `tail_brake` | red | a rear lens that is both | dim at night, bright when braking |
| 4 | `indicator_left` | amber | front, rear and side repeater, left | blinking |
| 5 | `indicator_right` | amber | the same, right | blinking |
| 6 | `reverse` | white | rear | reversing |
| 7 | `low_beam` | white | front, both sides | lights on (night) |
| 8 | `high_beam` | white | front, both sides | high beam |
| 9 | `daytime` | white | front, both sides | driving by day; dim as position lamps at night |
| 10 | `fog_front` | white | optional | reserved |
| 11 | `fog_rear` | red | optional | reserved |
| 12 | `plate` | white | rear plate lamp, optional | reserved |
| 13 | `beacon` | not defined yet | roof (emergency, maintenance) | reserved |
| 14 | `sign` | not defined yet | taxi sign, bus destination board | reserved |
| 15 | `marker` | amber, white, red | truck and bus outline lamps | reserved |

- Every vehicle has 3 (or 1 and 2), 4, 5, 6, 7, 8 and 9. A car also has 2 on its high-mounted
  lamp, on the centre line, which is a lamp face and not glass. A truck also has 15.
- Low beam, high beam and daytime are separate faces even inside one headlamp.
- A lens is one connected group of faces of the same id, measured from LOD0. The sidecar lists
  each lens with its lit colour: `white`, `amber` or `red`. The colour is the group's own from
  the table; a marker's may be any of the three. The colours of `beacon` and `sign` are defined
  with the emergency-vehicle and bus orders; until then any of the three is accepted.
- LOD0 and LOD1 carry every lamp face. LOD2 keeps the ids on flat faces inside the same lens
  rectangles; a lamp id missing on LOD2 is a warning, not a failure. LOD3 has none.
- Emission is exported bound with factor zero.

### Glass

Glass is drawn opaque, dark and glossy, with no interior. Every pane is still its own tagged
region, so that tint, lit cabins and see-through glass can come later without touching a model.

| Id | Group | What it covers |
| --- | --- | --- |
| 32 | `glass_windscreen` | the windscreen |
| 33 | `glass_front_side` | front door windows, both sides; a truck or van cab's side windows |
| 34 | `glass_rear_side` | rear door and quarter windows, both sides |
| 35 | `glass_rear` | the rear window or tailgate glass; glazed rear doors of a van |
| 36 | `glass_roof` | a glass roof or sunroof (optional) |
| 37 | `glass_cab_rear` | the rear window of a truck or pickup cab |
| 38 | `glass_saloon` | passenger-row windows of a bus or minibus; load-area windows of a van |
| 39 | `glass_door` | glazed doors of a bus |
| 40..47 | reserved | |

- Glass faces are separate faces with paint 0. Mirror glass, lamp lenses and plates are not glass.
- A car has at least 32, 33, 34 and 35; a van and a truck at least 32 and 33; a bus at least 32.
- LOD0 and LOD1 carry every glass id. On LOD2 a face that is wholly glass keeps its id; front
  and rear side glass may merge into one band a side, tagged 33. LOD3 has no surface ids.
- In the emission texture, glass texels hold a dim, neutral picture of the lit cabin (the
  building night-room painter at a low level, no colour). The game keeps it dark until lit
  cabins are built.
- The texture generator writes a record of which texels are glass, `glass_texels.png`, beside
  the model's `notes.md`, so the validator can tell an unmarked glass face. It is not shipped and
  not in the sidecar.

### Interior

- A body mesh has **no interior and no seats**, in any tier, and its glass stays opaque (the
  owner, 2026-11-21).
- Reserved for later, and not built for the pilot: an **optional, separate** interior mesh per
  model, `<name>_interior.glb`, in the body's frame and origin. One mesh, one material, a
  ceiling of about 400 triangles: seat shapes, dashboard, steering wheel, optionally a driver's
  silhouette.
- The sidecar field `interior` is `null` until a model has one. Because the interior is its own
  file, adding it later needs no rework of a body mesh.
- The game's use of it is under [glass, later](#planned-beams-later-lamps-and-glass-render-67).

### Paint and textures

- Painted panels (B = 1) are neutral light grey in the albedo, about 0.75 linear, with panel
  gaps and light wear only. The game multiplies them by the vehicle's paint colour. No body
  colour is baked.
- The paint level is held so that every vehicle takes a paint alike. On LOD0 and LOD1 the panel
  field, the area-weighted median of the painted texels, lies within 0.70 to 0.80 linear. On
  every tier the area-weighted mean of the painted texels is within 10 % of LOD0's, so a vehicle
  keeps its brightness at a tier switch. Either miss fails the validator.
- Known and accepted for now: under dark paint a truck's far tiers render about 9 to 13 % darker
  overall, because its light unpainted parts shrink away there.
- Plates are blank white rectangles.
- Maps: albedo per variant, ORM and emission are required; a normal map is optional. The
  emission holds lit lamp lenses, the lit cabin on glass, and black everywhere else. External
  PNGs, one set for all tiers. One material slot.
- Size: 1024 for every class (car, van, truck, bus). That is about 128 texels per metre on a
  car body and about 64 on a truck, with denser rectangles for lamps and wheel faces. Mirrored
  left/right UVs are fine except where a livery needs lettering.
- Every vehicle has the variant `plain` (`"paint": true`). Liveries are further variants:
  another albedo on the same geometry. A livery that covers painted panels paints them in full
  colour and is marked `"paint": false` in the sidecar; the game then uses white as that
  vehicle's paint colour. A livery names no real company. The pilot truck has `plain` and
  `haulier`.

### Tiers

Triangle ceilings; no floors. All tiers share the origin, plan box and textures. The bands
assume a 4.5 m car on a 1080p screen at a 75 degree field of view: about 105 pixels long at 30 m,
32 at 100 m, 8 at 400 m.

| Tier | Seen at | Car | Van | Truck, bus | Must keep |
| --- | --- | --- | --- | --- | --- |
| `LOD0` | under 30 m | 3,000 | 3,500 | 4,500 | separate wheel parts (16 to 20 sides), every lamp and glass face, mirrors, arches |
| `LOD1` | 30 to 100 m | 900 | 1,000 | 1,400 | separate wheel parts (8 to 10 sides), every lamp and glass face |
| `LOD2` | 100 to 400 m | 220 | 260 | 350 | silhouette; wheels merged into the body as part 0; lamp ids on flat faces; glass ids on wholly glass faces; paint |
| `LOD3` | beyond 400 m | 40 | 40 | 60 | a box with the roofline; paint only (R = 0, G = 0); the glass band is in the texture |

These replace the [earlier provisional tier numbers](#earlier-provisional-tier-numbers).

### Facts sidecar

`export/<name>.model.json`, schema `metrum-model-facts`, **version 2** or **3**, with `"kind":
"vehicle"`. Version 3 adds door positions ([Doors](#doors-sidecar-version-3-tools-43)); the game
reads both. Building sidecars stay version 1 (see
[Model Draft Pipeline](asset_editor.md#model-draft-pipeline--tools-33)). Metres, in the exported
frame. Geometry and files only.

The example is the pilot estate's sidecar cut down to one entry per list, with the keys in the
order the models build writes them. It illustrates the form; a real sidecar states the values
measured from its own files.

```json
{
  "schema": "metrum-model-facts", "version": 2, "kind": "vehicle",
  "name": "estate_compact", "path": "vehicles/car/estate_compact",
  "source": {"commit": "…", "dirty": false, "blender": "5.3.0"},
  "license": "CC-BY-4.0",
  "bounds": {"min": [-1.01, 0.0, -2.24], "max": [1.01, 1.468, 2.24]},
  "body": {"class": "car", "length_m": 4.48, "width_m": 1.79, "width_mirrors_m": 2.02,
           "height_m": 1.468, "wheelbase_m": 2.65, "front_overhang_m": 0.86,
           "rear_overhang_m": 0.97, "ground_clearance_m": 0.19},
  "axles": [{"z": 1.38, "track_m": 1.54, "steer": true, "twin": false},
            {"z": -1.27, "track_m": 1.54, "steer": false, "twin": false}],
  "wheels": [{"id": "front_left", "part": 1, "axle": 0, "side": "left",
              "centre": [0.77, 0.32, 1.38], "radius_m": 0.32, "width_m": 0.207, "steer": true}],
  "attributes": {"set": "COLOR_0", "r": "part", "g": "surface", "b": "paint", "version": 1},
  "lamps": [{"group": "high_beam", "lamp": 8, "side": "left", "position": "front",
             "colour": "white", "centre": [0.6062, 0.7582, 2.0864],
             "facing": [0.2385, 0.5562, 0.7961], "width_m": 0.2286, "height_m": 0.1254}],
  "glass": [{"group": "glass_windscreen", "id": 32, "area_m2": 1.199}],
  "cargo": null,
  "coupling": null,
  "interior": null,
  "lods": [{"file": "estate_compact_lod0.glb", "triangles": 2906, "sha256": "…",
            "wheel_parts": true, "lamp_ids": true, "glass_ids": true}],
  "textures": {"orm": {"file": "…", "sha256": "…"}, "emission": {"file": "…", "sha256": "…"},
               "normal": {"file": "…", "sha256": "…"}},
  "variants": [{"name": "plain", "albedo": {"file": "…", "sha256": "…"}, "paint": true}],
  "default_variant": "plain",
  "preview": {"file": "preview.png", "sha256": "…"}
}
```

- The models build measures the values from the exported files, geometry from LOD0. A model's
  own build states `cargo`, and may state `lamps`, `glass`, `coupling` or `interior` instead of
  the measured ones.
- `bounds`: LOD0, mirrors included.
- `body.class` is a size word (`car`, `van`, `truck`, `bus`); the game maps it to its own class.
  `wheelbase_m` runs from the first to the last axle; `ground_clearance_m` is the height of the
  lowest body point.
- `axles`: front first; `twin` is true for an axle with twin wheels, one part a side.
- `wheels`: one entry per wheel part, in part order; `centre.y` equals `radius_m`.
- `attributes` names the id attribute and its channels; the byte form is fixed by
  [Vertex ids](#vertex-ids).
- `lamps`: a flat list with **one entry per lens**, keys in this order: `group`, `lamp` (the
  id), `side` (`left`, `right` or `centre`), `position` (`front`, `rear`, `side` or `roof`,
  from the facing's main axis), `colour` (the lit colour: `white`, `amber` or `red`), the lens
  `centre`, its unit outward `facing`, `width_m` (along up × facing) and `height_m` (across
  it). An indicator group with a front lamp, a rear lamp and a side repeater on each side has six
  entries. The game uses the entries for the far glow points and the origin of the light beams.
  `facing` is the lens surface's normal and carries the body's lean (the estate's headlamp
  lenses face upward by about 0.55 in `y`): it is not a beam direction, so beams are aimed from
  the vehicle's forward axis, and lamps are found by `group` and `facing`, not by `position`
  alone (a wrap-round headlamp reads `side`).
- `glass`: one entry per group present, with its id and area.
- `lods`: all four tiers.
- `cargo`: the load box of a van or truck, `{"min": [x, y, z], "max": [x, y, z], "door":
  "rear"}`; `null` for a car, and a failure on a van or truck.
- `coupling`: `null`; reserved for trailers (hitch point and kind).
- `interior`: `null`, or `{"file": "…", "triangles": n, "sha256": "…"}` once a model ships the
  optional interior mesh; `null` in the pilot.
- `textures.normal`: `null` when the model has no normal map.
- `preview.file` is relative to the asset folder, not to `export/`.
- Seats are not in the sidecar. Seating is a value of the pack asset on the game side. The
  family arrival does not read it yet: every member of a moving-in household steps out of the one
  car, more people than seats included, until a household can own a second car (`PARK-33`).
- The lists and LOD0 agree both ways: no listed group is missing from the mesh, and no id in
  the mesh is missing from the lists.

### Doors (sidecar version 3, `TOOLS-43`)

Agreed with the models repository on 2026-11-22 and built there for the two pilot models
(models commits 5eff594 and 0f4757e, on a branch awaiting the owner's review). The draft reads it
and writes the doors into the asset ([The `[vehicle]` pack class](#the-vehicle-pack-class),
`TOOLS-43`); the bundled pack was re-imported from 0f4757e with every tier, map and albedo
byte-identical.

Version 3 is version 2 plus one key, `doors`, directly after `cargo`; nothing else changes, and
no mesh, texture, vertex id or tier changes. Doors are painted seams of the body, not parts.

- `doors`: a flat list with one entry per door, keys in this order: `side` (`left`, `right` or
  `rear`; left and right are the driver's), `row` (`front`, `rear` or `cargo`), `centre` and
  `width_m`. List order: front row, rear row, cargo; left before right.
- `centre`: the door opening's centre on the door skin at ground level (`y = 0`), in the exported
  frame. A side door's centre lies on the plan box's side or up to 0.15 m inside it, never
  outside (the truck's cab is 0.103 m narrower a side than its load box). A rear door's centre is
  on the plan box's rear plane at `x = 0`, not on a raked tailgate. It is geometry only: the game
  adds its own offset outward for the point where a person stands.
- `width_m`: the opening's width along the body. The estate's rear door is measured at the belt
  (the wheel arch cuts its lower corner), its tailgate between the cut lines at the bumper.
- `row: "cargo"` is a load door or tailgate, on the rear or on a side.
- The values come from the shut-line constants each model's build states, which also draw the
  seams in the textures. A model's build without doors writes no sidecar.
- The drive side (left- or right-hand drive) is not model data: the body is symmetric and has no
  interior. It is a setting on the game side.

Validator checks, in the models repository: the front row exists on both sides and mirrors
within 1 mm; rear and cargo rows mirror where both sides have one (a van with one sliding door is
legal); a car, van or truck has both front doors, a bus at least one door; a van or truck has a
cargo door on the side `cargo.door` names, no wider than the load box.

| Model | `side` | `row` | `centre` | `width_m` |
| --- | --- | --- | --- | --- |
| `estate_compact` | `left` / `right` | `front` | [±0.895, 0, 0.3675] | 1.135 |
| `estate_compact` | `left` / `right` | `rear` | [±0.895, 0, −0.58] | 0.76 |
| `estate_compact` | `rear` | `cargo` | [0, 0, −2.24] | 1.15 |
| `rigid_box_18t` | `left` / `right` | `front` | [±1.162, 0, 3.0275] | 0.955 |
| `rigid_box_18t` | `rear` | `cargo` | [0, 0, −3.95] | 2.24 |

### Validator checks

The models repository runs these on the exported files:

1. Every tier, wheels straight ahead, fits its class box (mirrors taken as under
   [Body classes](#body-classes)), keeps LOD0's plan box (mirrors excluded), and has its lowest
   point at 0 within 1 mm; each wheel's lowest point is 0 within 1 mm and `centre.y` equals
   `radius_m`.
2. Wheel and lens pairs mirror in `x` within 1 mm, apart from the single lenses allowed under
   [Frame](#frame).
3. Ids, read from the GLB accessors: `COLOR_0` is a normalised `UNSIGNED_BYTE` `VEC4`; R and G
   are exact bytes from the allowed sets; the three corners of every triangle are identical, so
   R, G and B are uniform per triangle; R = 0 and B = 0 wherever G is not 0; B is 0 or 255; A is
   255. No Draco or meshopt extension.
4. Per tier: LOD0 and LOD1 have every wheel part (16 to 20 sides on LOD0, 8 to 10 on LOD1),
   every required lamp id and every required and listed glass id; LOD2 has no wheel parts, and a
   lamp id missing there is a warning; LOD3 has R = 0 and G = 0 everywhere.
5. On LOD0 and LOD1 each wheel part's vertices lie inside its sidecar cylinder (plus 5 mm) and
   no body vertex does.
6. Wheel clearance: every wheel straight ahead, and each steering wheel turned +35 and -35
   degrees, sampled at its vertices and at points along every edge, comes no closer than 10 mm
   to a body triangle.
7. Lamps: each lamp face lies within the rectangle of the nearest sidecar lens of its lamp id,
   to 2 mm in the lens plane and 50 mm along its facing; every lens has at least one face; lamp
   emission texels are not black. Each lens's `colour` is present, allowed for its group (see
   [Lamps](#lamps)) and matches the emission texels its LOD0 faces sample.
8. Glass, against `glass_texels.png`: on LOD0 and LOD1 a triangle sampling glass texels without
   a glass id fails, and a glass-id triangle sampling non-glass texels fails (LOD2: the second
   only); glass emission is not black. Over the glass texels of each albedo variant the mean
   sRGB value is 40-78 (8-bit, mean of the three channels), the mean roughness 0.05-0.12 and
   metallic below 0.02 (pilot: estate 52 and 0.080, truck 57 and 0.082).
9. Emission texels that faces sample and that are neither lamp nor glass are black. Painted
   faces are near neutral in the albedo. The paint level holds (see
   [Paint and textures](#paint-and-textures)): the panel field of LOD0 and LOD1 within 0.70 to
   0.80 linear, every tier's painted mean within 10 % of LOD0's.
10. Triangle ceilings, one material, external textures, hashes and a clean source, as for
    buildings; the sidecar's lists against the mesh both ways, and `cargo` present on a van or
    truck.

A review sheet (views, wheels turned 35 degrees, each lamp group lit, the cabin lit, a
false-colour view of the ids, three paint colours, tiers at their switch distances, a 1.8 m
figure and the class's bay outline: car 2.5 × 5.0 m, van and truck 3.0 × 8.0 m, bus 3.0 ×
13.0 m) is rendered for the owner's look. It is not a committed deliverable.

### Pilot pair

**Built in the models repository** (commit `22f26cb`, 2026-11-21, not yet pushed there). The
figures are those of the two sidecars.

| | Compact estate | Rigid box truck |
| --- | --- | --- |
| Folder | `vehicles/car/estate_compact` | `vehicles/truck/rigid_box_18t` |
| Length × width × height (m) | 4.480 × 1.790 × 1.468 | 7.900 × 2.530 × 3.700 |
| Width over mirrors (m) | 2.020 | 2.98 |
| Wheelbase (m) | 2.650 | 4.50 |
| Track (m) | 1.540 | front 2.05; rear twins 1.84 between centres |
| Wheel radius (m) | 0.320 | 0.52 |
| Triangles, LOD0 / 1 / 2 / 3 | 2,906 / 890 / 218 / 40 | 4,062 / 1,388 / 348 / 60 |
| Lenses | 20 | 27, 10 of them markers |
| Glass ids | 32 to 35 | 32, 33 |
| Variants | `plain` | `plain` (paint true), `haulier` (paint false) |
| Validator | 0 FAIL, 1 WARN: the high brake lamp's id is missing on LOD2, accepted | 0 FAIL, 0 WARN |

The game draws the pair as imported from models commit `428a413` (2026-11-22: new looks and
lens entries; the truck gained a front fog pair, 29 lenses), whose sidecars give the triangle
counts 2,990 / 880 / 216 / 40 (estate) and 4,490 / 1,398 / 348 / 60 (truck). Still to
do on the game side: wheel steer and spin, on the twin wheel parts as on single ones
([steered wheels](#as-built-brake-lights-indicators-reverse-lamp-and-wheels-render-66),
[wheel spin](#planned-beams-later-lamps-and-glass-render-67)), and the owner's look at the `car`
box with the estate beside a bay ([open points](#open-points-of-the-contract)).

### Open points of the contract

- The `car` box (4.50 × 1.80 m) is looked at once more with the pilot estate beside a parking
  bay, before the fleet is ordered. This is the only open point.

Closed 2026-11-21: the id attribute (`COLOR_0` as normalised unsigned bytes, see
[Vertex ids](#vertex-ids)) and the glass texel mask for check 8.

## Sizes

### Sizes the simulation uses today

**As built.** Since `TRAFFIC-38` (landed on `dev` 2026-11-22) lanes and parking use the same
real body lengths; before it a vehicle of any kind, and a walker, took the 2.6 m of the
placeholder cars (`CAR_LENGTH`) in a lane.

| What | Value | Where |
| --- | --- | --- |
| Length a vehicle takes in a lane, by body class | car 4.5 m (`CAR_BODY_LENGTH_M`), truck 8.0 m (`TRUCK_BODY_LENGTH_M`); van 6.0 m and bus 12.0 m have no simulated vehicle yet | `rust/src/config.rs`, read through `lane_half_length` |
| Length a walker takes in a foot lane | 0.5 m (`WALKER_BODY_LENGTH_M`) | `rust/src/config.rs` |
| Standstill gap | 0.1 m (`IDM_S_MIN`) | `rust/src/config.rs` |
| Time headway | 0.5 s (`IDM_T_HEAD`) | `rust/src/config.rs` |
| Car body for lot proofs | 1.8 × 4.5 m (the length is `CAR_BODY_LENGTH_M`), turn square 4.85 m, clear height 2.0 m, smallest bay 1.8 × 4.5 m | `CLASS_TABLE` in `rust/src/simulation/parking/validation.rs` |
| Truck body for lot proofs | 2.6 × 8.0 m (the length is `TRUCK_BODY_LENGTH_M`), turn square 8.45 m, bay 3.0 × 8.0 m, gate 4.0 m, clear height 4.0 m | the same table |
| Kerbside bay | 6.0 m long in a 2.5 m parking lane | `KERB_BAY_LENGTH_M` in `rust/src/simulation/parking/kerb.rs`, `PARKING_LANE_WIDTH_M` in `rust/src/simulation/network/graph/cross_section.rs` |
| Bays of the bundled lots | 2.5 × 5.0 m | `godot/bootstrap/mods/parking/assets/` |
| Driving lane | 3.5 m (`LANE_WIDTH`) | `rust/src/config.rs` |
| Walker | 1.80 m tall | `godot/assets/models/characters/Quaternius/VAT/` |

Which tests read the lengths, and how two bodies of different classes are spaced, is specified
with car following in [`traffic.md`](traffic.md#car-following); the measured effect is in
[lane length by body class](#lane-length-by-body-class-traffic-38) below.

### The placeholder cars, measured

**As built.** Bounds read from the `.glb` files (2026-11-21); the game draws them at this size.

| Model | Drawn length × width × height (m) | Wheelbase / track (m) | A real one (m) | Length ratio |
| --- | --- | --- | --- | --- |
| Sedan | 2.55 × 1.50 × 1.30 | 1.32 / 0.85 | 4.5 × 1.8 × 1.45 | 57 % |
| Sports sedan | 2.55 × 1.30 × 1.10 | 1.32 / 0.85 | 4.4 × 1.8 × 1.30 | 58 % |
| SUV | 2.70 × 1.50 × 1.30 | 1.32 / 0.85 | 4.5 × 1.85 × 1.65 | 60 % |
| Luxury SUV | 2.85 × 1.50 × 1.30 | 1.52 / 0.85 | 4.8 × 1.9 × 1.70 | 59 % |
| Delivery van | 3.25 × 1.50 × 1.65 | 1.62 / 0.90 | van 5.95 × 2.05 × 2.6; truck 8.0 × 2.55 × 3.7 | 55 % / 41 % |

Wheels are 0.60 m across on every model. Length is about 0.57 of real, width about 0.83, height
about 0.9. A 1.8 m walker is 0.5 m taller than a sedan. Each model is 2,032 to 2,476 triangles,
about 1,330 of them wheels.

### Lane length by body class (`TRAFFIC-38`)

**As built (2026-11-21, landed on `dev` 2026-11-22).** The rules, the
position convention and the hot-path cost are in [car following](traffic.md#car-following).

- A lane distance is the body's centre. Two bodies are clear when their centres differ by both
  half lengths plus `IDM_S_MIN`, plus the headway where a test adds it.
- Class from saved columns, no new state and no save change: a walker 0.5 m, a freight carrier
  (`vehicle_type` 4, the vehicle drawn as the truck) the `truck` class 8.0 m, every other vehicle
  the `car` class 4.5 m. The simulation has no van: door and dock deliveries are both made by the
  freight carrier, so both are `truck`. Van 6.0 m and bus 12.0 m wait for a simulated vehicle.
- The lot proof table reads the same two lengths; the proofs are unchanged.
- Walkers queue single file 0.6 m apart (0.5 m, a standing person's body depth, plus the
  standstill gap), where they kept the cars' 2.7 m.
- Standstill gap, headway and acceleration are not retuned.
- The populated-city fixture revision is `perf-fixture-revision-v12` (v11 for the lengths, v12
  for the border trunks below): every cached city is built again by the engine under test.

**Measured against the model's own formula.** Control `dev` `daab78d4c`, branch `ead1c18a3`'s
simulation (release test binaries, `measure_capacity` from
`tick/ring_capacity_tests.rs`, saturating sources, cars per minute per entry lane):

| | Formula | Measured |
| --- | --- | --- |
| Jam spacing, cars | 2.7 to 4.6 m: 41 % fewer cars per jammed km | enforced exactly by the slot and overlap tests (`traffic38_*` tests; the ring lock fills a 380 m lane with 82 cars, where it held 141) |
| Lane flow at 50 km/h | -16.5 % at the cruise spacing; the model's peak lane flow 4,230 to 3,441 an hour (-18.7 %) | straight road, 1+1 and 2+2 lanes: 55.1-56.3 to 44.1-47.4 a minute per lane (-15.8 % to -20.0 %) |
| Lane flow at 20 km/h | | -20.9 % to -24.1 % |
| Junction movements, 1+1 and 2+2 | | through, left, right: -16.6 % to -24.7 %; mixed: -15.7 % to -19.9 % |
| One-lane roundabout, 36 m | | -8.9 % to -28.2 % by movement |
| Trucks | 8.1 m jam spacing; peak lane flow 2,608 an hour (-38 % against the old 2.6 m) | not measured alone |

**The fixture cities** (one game day of each side's own prepared city at 32x, 8 workers, the
lane-entry audit `traffic07_lane_entry_audit` with its `TRAFFIC38_TRIPS` line; trip times are in
simulation seconds, a game minute each at 1x):

| 30k / 10k | Control | Branch |
| --- | --- | --- |
| Freight deliveries in the day | 4,458 / 1,461 | 3,205 / 1,063 (-28 % / -27 %) |
| Car trip, mean (p95) | 285 (677) / 208 (421) | 336 (870) / 240 (504): +18 % / +16 % |
| Commute by car, mean | 158 / 127 | 181 / 150: +14 % / +18 % |
| Freight trip, mean | 281 / 239 | 364 / 337: +29 % / +41 % |
| Car trips finished | 29,095 / 11,490 | 25,882 / 9,748 (-11 % / -15 %) |
| Lanes full at day end (spacings past the length) | 18 / 0 | 136 / 15 |
| Lanes overfilled, bodies stacked | 0 / 0 | 0 / 0 |
| Freight held at the border nodes at day end | 687 / 126 | 452 / 17 |
| Gridlock cycles, detours, saturation reports, trip failures | 0 | 0 |
| Departures that drove / walked because parking near the destination was full | 23,284 / 2,230 (30k) | 20,451 / 2,908 |

**Behaviour envelope** (`sim02_gate.py envelope`, fresh cities, 14 days; control the `dev`
binary's records): **16 notes outside the bands, no failure, no invariant broken.** The baseline
cities lose freight first and their economy follows:

| Baseline, 14-day mean | 10k control | 10k branch | 30k control | 30k branch |
| --- | --- | --- | --- | --- |
| Deliveries / 1k buildings / day | 172.3 | 129.1 (-25 %) | 175.4 | 142.6 (-19 %) |
| Households out of stock | 13.3 % | 19.6 % | 9.8 % | 16.1 % |
| Employment (final day) | 68.1 % (67.8 %) | 58.5 % (45.7 %) | 67.3 % (65.2 %) | 61.2 % (50.9 %) |
| Net income / resident / day | 11.20 | 5.19 (-54 %) | 11.00 | 7.11 (-35 %) |
| Bankruptcies / 1k buildings / day | 0.02 | 1.36 | 0.15 | 0.99 |

The fiscal-stress cities deliver more (+33 % to +51 %) and keep their other numbers within a few
per cent. Determinism is unchanged: the gate's repeat, 1- and 3-worker and save-reload checks are
identical, twice, and the 30k audit day ends in the same state at 1 and 8 workers.

**Round 1's open question** (the owner chose the fixture). No band was widened and no constant
tuned. An attribution run with freight carriers at the car's 4.5 m in lanes (not committed) still
left the bands (11 notes) but halved the loss, so about half came from the slower car traffic and
half from the 8 m trucks. The test cities' freight was the limit: every border was one
single-lane trunk per 10,000 residents.

**Flow check (round 2, 2026-11-22).** Is the slide traffic that flows slower, or vehicles stuck
for good? The lane-entry audit's flow check (`stuck.rs`, test-only: every agent on a trip
sampled each game minute, a vehicle stationary until it has moved 0.5 m; zero-throughput
lane-hours; per-day `TRAFFIC38_FLOW_DAY`) ran 15 days of each fresh city, `METRUM_AUDIT_FRESH=1`,
8 workers, 32x. A game minute is one simulated second. Days 7-18, mean (max) of day-end values:

| | `dev` 30k | Branch 30k | `dev` 10k | Branch 10k |
| --- | --- | --- | --- | --- |
| Vehicles stationary a whole game day (1,440 min) | 0 | 0 | 0 | 0 |
| Cars stationary 240 / 720 min | 12 / 0 | 36 / 0 | 0 / 0 | 3 / 0 |
| Trucks stationary 240 / 720 min | 190 / 40 | 434 / 91 | 63 / 9 | 76 / 17 |
| Of those over 240 min, also over 240 min a day earlier | 0 | 1 (3) | 0 | 0 (1) |
| Lanes full at day end | 22 | 169 | 2 | 34 |
| Zero-throughput lane-hours a day | 236 | 1,903 | 15 | 283 |
| Freight held at the border nodes at day end | 285 (922) | 623 (1,119) | 110 (320) | 118 (288) |
| Wait-for cycles (4-hourly), gridlock, trip failures | 0 | 0 | 0 | 0 |

Verdict: congested but flowing. The set of long waits is new every day, no cycle of lanes waiting
on each other forms, no agent leaves the trip accounting, and on the branch the counts stop
growing after day 8 while deliveries hold at about 3,270 (30k) and 1,110 (10k) a day. The only
vehicles that stand past 720 minutes are freight carriers held at the border nodes for their
first lane, off the map, in both builds; the longest wait chains end at the moving border trunk
lanes. Long-standing cars wait for a leader, for room on the next lane at a junction (spillback)
or for a gap to pull out of a driveway. Against the new lengths no road lane is shorter than
30 m and 3 of 7,577 connectors at 30k are shorter than a truck's 8.1 m spacing (none than a
car's); a short connector still takes one body at a time.

**Fixture change: one border trunk per 5,000 residents** (`RESIDENTS_PER_BORDER`, was 10,000;
fixture revision v12). Each trunk is the existing 1 + 1 row street continued to the world edge,
so 30k has 6 borders and 10k has 2, at the same lane count per border. Widening the trunks to
the 4-Lane Road instead was tried first: the wider trunk rows cost their lots' depth and the 30k
grid then housed 29,614 of 30,000 residents. With the extra borders the cities' business
failures stop altogether, so the 30k prepare's check that the snapshot's next day rebuilds a
building no longer holds; it is removed (the fiscal-stress fixture still checks churn every
hour). Flow on the new cities, days 7-18: no vehicle stationary 720 minutes at 10k and 5 trucks
(57) at 30k, all border-held; lanes full at day end 14 (30k) and 2 (10k); zero-throughput
lane-hours 98 and 2; border hold at day end 94 (607) and 31 (347); no cycle, no trip failure.

**Before and after** (one game day of each side's own day-12 snapshot at 32x, 8 workers; `dev`
`daab78d4c`, branch before the fixture change, branch after):

| 30k / 10k | `dev` | Branch, 1 trunk per 10k | Branch, 1 trunk per 5k |
| --- | --- | --- | --- |
| Freight deliveries in the day | 4,458 / 1,461 | 3,205 / 1,063 | 5,126 / 1,617 |
| Car trip, mean (p95) s | 285 (677) / 208 (421) | 336 (870) / 240 (504) | 286 (675) / 211 (430) |
| Commute by car, mean s | 158 / 127 | 181 / 150 | 158 / 128 |
| Freight trip, mean s | 281 / 239 | 364 / 337 | 225 / 210 |
| Lanes full at day end (peak of the 4-hourly checks) | 18 (25) / 0 (0) | 136 (160) / 15 (53) | 50 (68) / 4 (4) |
| Freight held at the border nodes at day end | 687 / 126 | 452 / 17 | 713 / 143 |
| Gridlock, wait cycles, trip failures | 0 | 0 | 0 |

| Envelope, baseline, 14-day mean | `dev` 10k | Before | After | `dev` 30k | Before | After |
| --- | --- | --- | --- | --- | --- | --- |
| Deliveries / 1k buildings / day | 172.3 | 129.1 | 175.0 | 175.4 | 142.6 | 182.6 |
| Households out of stock | 13.3 % | 19.6 % | 9.8 % | 9.8 % | 16.1 % | 7.8 % |
| Employment (final day) | 68.1 % (67.8 %) | 58.5 % (45.7 %) | 68.3 % (68.3 %) | 67.3 % (65.2 %) | 61.2 % (50.9 %) | 68.2 % (68.2 %) |
| Net income / resident / day | 11.20 | 5.19 | 11.51 | 11.00 | 7.11 | 11.59 |
| Bankruptcies / 1k buildings / day | 0.02 | 1.36 | 0 | 0.15 | 0.99 | 0 |

Against the old baseline the new cities leave 4 bands, all on the better side (30k deliveries,
final employment, out-of-stock; fiscal-stress 10k final employment, a note in round 1 too).
Employment, income and deliveries are flat over the last days, so the baseline is re-recorded on
this fixture (`tools/sim02_envelope_baseline.json`). Fiscal stress stays within a few per cent
of `dev`. Determinism: repeat, 1 and 3 workers and save-reload identical.

### Planned: waiting position at junctions (`TRAFFIC-39`)

**Planned check.** A vehicle lane ends on its junction mouth
([lane ends](traffic.md#junction-connectors)), and a waiting car stands with its centre, not its
nose, at the lane's end. Half its body is past the mouth: 1.3 m today, 2.25 m for a 4.5 m car,
4 m for a truck. This was read from the code, not seen in a still. With the pilot car, one still
at a junction with a crossing decides it: if the body stands on the crossing or in the
junction, queue heads hold half a body length before the lane end, compared against a control.

## As built today

### Models, loading and drawing

**As built (since `TOOLS-42` and `RENDER-65`, 2026-11-22).** The cars and trucks are the bundled
`vehicles` pack's models, drawn by the vehicle renderer; see
[the pack](#as-built-pack-assets-drafts-and-the-bundled-pack-tools-41-tools-42) and
[the renderer](#as-built-renderer-render-65). What stays as it was:

- Moving cars: one `MultiMeshInstance3D` per bucket in `godot/scripts/renderers/agents.gd`, made
  when the bucket is first drawn and remade when packs load. The simulation thread poses the
  cars in view into a snapshot (`rust/src/nodes/sim/core/snapshot/agents.rs`); a background job
  smooths the poses and grounds off-lane cars (`rust/src/nodes/sim/render/car_visual.rs`,
  `agent_frame.rs`); only buckets that changed are uploaded, and a car at rest is not uploaded
  again. See [Render Movement](traffic.md#render-movement).
- A car or truck pulling into or out of a lot place or a dock is posed on arcs round its route's
  bends (car 2.5 m, freight 6 m), not at its simulated point
  ([Drawn turns](traffic.md#drawn-turns-park-29-as-built), `PARK-29`); it still turns on the spot
  where it joins or leaves its lane (`PARK-31`).
- Cars draw out to 2,500 m, fading over 2,000 to 2,500 m (`CAR_FADE_START_M`,
  `CAR_DRAW_DISTANCE_M` in `rust/src/config.rs`); the tiers switch within that.
- The exact ground solve runs within 250 m of the camera and a cheaper one beyond
  (`rust/src/nodes/sim/render/vehicle_ground.rs`), now on each model's support from its wheels.
- Parked cars are drawn by `godot/scripts/renderers/parked_cars.gd` through the streaming core
  with the same meshes and shader and no lamps; see
  [Parked-car rendering](traffic.md#parked-car-rendering-park-01-park-02).
- Shadows: shadow-only twins near the camera take each bucket's mesh and material.
- The runtime points a car's `-Z` along travel; the models are turned at load.

### Vehicle models and lamps (`RENDER-26`)

Until `TOOLS-42` the cars were third-party placeholder models whose meshes carried no lamp data, so
`vehicle_lamps.gd` held a measured table of lamp anchors per model. The lamps now come from the
sidecar lenses ([Lamps at night](#lamps-at-night)); the layers below are unchanged. A car body is
one surface with one material (`RENDER-46`), now because every tier is one mesh with one material
by contract.

At night (sun below 4 to -2 degrees, as the street lights):

- `vehicle_lamps.gd` adds a camera-facing glow quad per
  lamp (`vehicle_lamp_glow.gdshader`) that keeps at least 1.6 pixels, so traffic shows across the
  map; headlights show from ahead, tail lights from behind;
- the 12 drawn cars nearest the camera within 100 m get a real `SpotLight3D` each
  (`vehicle_headlights.gd`, 60 m range, inverse-square decay, 35 degrees with a soft rim, no
  shadows; see below), chosen in Rust by
  `nearest_car_poses` (O(drawn cars · log 12) with a bounded heap). They light the road, trees,
  walls, walkers and the car ahead by their own colour;
- every car carries a light box (`vehicle_headlight_beam.gdshader`) that rebuilds each covered
  pixel from the depth buffer and adds a small red tail glow behind, and for cars from 70-100 m
  out (where the spot lights fade) the same cone the real headlight casts: Godot's SpotLight3D
  falloff with its 60 m range, inverse-square decay, 35° angle and rim attenuation 3, lit as level
  ground. This light is added blind to surface colour, acceptable only at that distance. Near the camera the box shrinks in the vertex shader
  to the tail region; by day and past 450 m it collapses and draws nothing.

Measured 2026-10-09 on the M2 Pro, `no-headlights` save (7 cars), release build, 120 frames,
old (Kenney models, no lamps) and new alternated three times: by day the new models cost under
0.1 ms. At 23:00, with real lights already on for the street lights: saved view +1.15 ms,
eye level +0.8 ms, 32 m overhead +0.4 ms with a 45 m spot range. Without the spot lights the
lamps cost 0.1-0.35 ms; the spot lights' cost follows their reach, not their count (6 or 12 the
same), and a 28 m range saved 0.45 ms in the saved view (about +0.85 ms over old in total), so
the range is 30 m. The 30 m setting itself was checked visually, not re-timed.

Near and far headlights did not match: up close the lighting shrank and dimmed. The real lights
were about 100 times weaker than the far light box on ordinary surfaces (their light is scaled by
albedo, the box's is added blind to it), fell off linearly instead of with the square of distance,
and reached 30 m against the box's 45. `tools/bake_headlight_projector.py` calibrates the
two against each other and bakes the box's own low-beam pattern into a spot-light projector
(`godot/assets/textures/vehicles/headlight_projector.png`). Godot 4.7 applies a projector only to
a shadowed light (`SpotLight3D` warns so; `storage_rd/light_storage.cpp` builds the projector
matrix only for shadowed lights), and four shadowed lights cost 1.9-2.9 ms a frame on the M2 Pro,
two 1.3-2.1 ms; hard shadows or an empty caster mask saved little, so the cost is per-pixel
shading. They are off (`SHADOWED := 0`), and every light is a plain cone: 60 m range,
inverse-square decay, 35 degrees with a steep rim falloff, energy 1200, matching the box's hot
spot near 20 m and overlighting the first few metres. Against the old 30 m lights (same save,
23:00, 120 frames, alternated twice): close view -0.1 ms, saved view +0.4 ms, overhead +0.15 ms.
Shots: `screenshots/2026-10-09-roads/headlights-4-shadowed.png` and
`headlights-shadowless-cone.png`.

Far and near headlights still did not match (2026-10-14): from 70 m out a car lit a wide white
block with straight edges, and close up a narrow soft cone. The box drew its own low-beam pattern,
still strong at the box's front face (45 m) and side faces (16 m), and lit each pixel with a
normal rebuilt from depth derivatives, which broke at every kerb and depth step into bright dashes
and straight-edged patches. The box now draws the real light's cone (above), lights everything as
level ground, and reaches 60 m ahead and 42 m (tan 35° × 60 m) to each side, where that cone is
spent; it fades out below the box's 8 m top. Its energy (62) and a ground tint (1.07, 1.0, 0.85)
were fitted to the mean light the real headlights add to the same views (`light-road` save at
01:00, cars 60 and 110 m away, all-real against all-box: 86-110 % per channel before the tint).
Frame cost was not measurable apart from the street-light change it shipped with (`ROAD-59`).

### Earlier provisional tier numbers

**Superseded by [Tiers](#tiers).** Moved from `asset_editor.md` unchanged, for the record.

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

## As built: pack assets, drafts and the bundled pack (`TOOLS-41`, `TOOLS-42`)

**As built (2026-11-22).**

### The `[vehicle]` pack class

A vehicle is a pack asset (`rust/src/assets/asset/vehicle.rs`). Its `asset.toml` holds the
model contract's facts, copied from the facts sidecar by the vehicle draft, plus the values only
the pack decides:

```toml
asset_id = "vehicle.civil.estate_compact"
display_name = "Estate Compact"
thumbnail = "thumbnail.webp"

[[lods]]                       # exactly four, LOD0 first; each band ends where the next begins
file = "estate_compact_lod0.glb"
distance_min_m = 0.0
distance_max_m = 30.0
# ... LOD1 30-100, LOD2 100-398, LOD3 from 398 (no distance_max_m)

[vehicle]
vehicle_class = "civil"        # civil, police, fire, ambulance, utility, bus
vehicle_family = "estate"      # sedan, hatchback, estate, suv, van, truck, bus; descriptive only
body_class = "car"             # car, van, truck, bus: the box it must fit
length_m = 4.48
width_m = 1.79
height_m = 1.468
seats = 5                      # a pack value, not a model fact
spawn_weight = 1.0             # optional, default 1: the share among its class's models
orm = "estate_compact_orm.png"
emission = "estate_compact_emission.png"
normal = "estate_compact_normal.png"   # optional

[[vehicle.variants]]           # the first is the default
name = "plain"
albedo = "estate_compact_albedo_plain.png"
paint = true                   # false: a livery, painted faces stay white

[[vehicle.paints]]             # optional, at most 64; none takes the class's default palette
colour = [0.92, 0.92, 0.91]    # sRGB
weight = 24.0

[[vehicle.wheels]]             # part order: part 1, 2, ...
part = 1
side = "left"
centre = [0.77, 0.32, 1.38]    # contract frame, nose at +Z
radius_m = 0.32
width_m = 0.207
steer = true
twin = false

[[vehicle.lamps]]              # one entry per lens, the sidecar's keys
group = "low_beam"
lamp = 7
side = "left"
position = "front"
colour = "white"
centre = [0.7307, 0.7487, 1.9913]
facing = [0.446, 0.6131, 0.6521]
width_m = 0.2036
height_m = 0.1707

[[vehicle.doors]]              # optional; version 3 sidecars: list order, the sidecar's keys
side = "left"                  # left, right (the driver's) or rear
row = "front"                  # front, rear or cargo
centre = [0.895, 0.0, 0.3675]  # on the door skin at ground level, contract frame
width_m = 1.135
```

Checked when a pack loads, O(wheels + lamps + paints) per asset: the size fits the body class
box (5 mm of rounding allowed); four contiguous tiers; 4 to 8 wheels, parts 1, 2, ... in order,
each resting on the ground (centre height equals its radius within 1 mm); every lamp id matches
its group and every facing is a unit vector; the required lamp groups (indicators, reverse, low
and high beam, daytime, and `tail_brake` or `tail` and `brake`; a car's `brake`, a truck's
`marker`); at least one variant, names unique; paints with channels 0 to 1 and a positive weight;
doors, when listed: each on the ground (`y` 0 within 1 mm) with a positive width, in list order
(front, rear, cargo; left before right; once each), a `rear` side only for a cargo door, a left
door at `+X` and a right one at `-X`, both front doors on a car, van or truck (any one on a bus),
and each row present on both sides mirrored within 1 mm. A vehicle without doors is legal.

**Doors in the game.** The asset's doors are for drawing and tooling. The simulation keeps its
own door points per body class (`rust/src/simulation/parking/doors.rs`): a simulated car has no
asset, and the look the catalogue picks for it changes with the loaded packs, so it never decides
where a person stands. The car class's points are the bundled estate's (a test holds them equal).
The drive side is the game setting `config::LEFT_HAND_DRIVE` (default left-hand drive: the
driver's door is `left`/`front`); the family arrival places its members by it
([Family arrival](traffic.md#family-arrival-park-35), `PARK-35`).
The unused `wheel` and `light` anchor types are removed. The asset browser lists vehicles under
"Vehicles"; share archives package the tiers, maps and albedos.

The `[[lods]]` bands are the tier switch distances (the renderer reads them, below). The draft
writes the contract's bands for a 4.5 m car, 0 / 30 / 100 / 400 m, scaled by the model's length
over 4.5 m and rounded to whole metres, so a tier switches at the same size on screen: the estate
switches at 30 / 100 / 398 m, the truck at 53 / 176 / 702 m.

### Vehicle drafts

`tools/model_draft.py` takes a version 2 or 3 `kind: "vehicle"` sidecar
(`rust/src/assets/authoring/model_draft/vehicle.rs`, the draft's flow in
`rust/src/nodes/asset_files/model_draft.rs`); see
[the draft command](asset_editor.md#draft-command):

- the reader is as strict as version 1's: unknown keys, another schema, version or kind, a
  non-null `coupling`, other than four `.glb` tiers, an id attribute other than `COLOR_0` with
  `part`, `surface`, `paint`, wheels out of part order or steering unlike their axle, a missing
  `cargo` on a van, truck or bus, and a file outside the export folder are refusals; a dirty
  source and a file whose SHA-256 differs from the sidecar's too; version 3 requires `doors` and
  version 2 may not have the key;
- the vehicle role gives `pack_id`, `asset_id`, `vehicle_class`, `vehicle_family`, `seats`, and
  optionally `display_name`, `spawn_weight` and `paints`;
- the draft publishes the tiers, maps, albedos and the thumbnail through the editor's file plan,
  writes `asset.toml` (the default variant first) and the ledger
  `model_sources/<asset_id>.json`, and checks that the pack scanner loads the asset as a vehicle;
- a rerun on an existing asset publishes nothing and reports stale lines: a role or body class
  the asset no longer carries, and every tier or map whose published file differs from the new
  sidecar's; `--check` reports the same and the load check;
- no lot, building-door or parking rule applies; the building rules are untouched.

### The bundled pack

The game's own vehicles ship in `godot/bootstrap/mods/vehicles/` (`pack.toml` licence CC BY 4.0),
seeded into a player's mods on first run and enabled by default like `parking`. A profile that
saved its pack selection before the pack shipped gets it enabled once
(`godot/scripts/core/mod_pack_config.gd`, `bundled_offered`); disabled afterwards, it stays
disabled. Every file is imported as `keep`, so an exported game reads them raw (`TOOLS-15`).

| Asset | Model | Variants |
| --- | --- | --- |
| `vehicles:vehicle.civil.estate_compact` | the pilot compact estate, seats 5 | `plain` |
| `vehicles:vehicle.utility.rigid_box_18t` | the pilot rigid box truck, seats 2 | `plain`, `haulier` (`paint: false`) |

The role files are `tools/vehicle_roles/<name>.json`. One command re-imports them from a commit
of the models repository, extracting that commit's `vehicles/` tree so uncommitted files are
never read, and records the commit in each ledger:

```bash
tools/vehicle_roles/import.sh <metrum_models checkout> <commit> --write
```

A draft never replaces an asset: remove `assets/<asset_id>/` and `model_sources/<asset_id>.json`
first. The pack was imported from models commit `0f4757e` (version 3 sidecars from clean trees,
with doors; the files are byte-identical to the earlier `428a413` import);
without `--write` the command is a dry run.

Other packs may add vehicles: the catalogue merges them by body class and weight. With no
vehicle model at all, a procedural box car the size of the car class box draws every vehicle.

The third-party placeholder cars (`godot/assets/models/vehicles/`, four cars and a delivery
van), their hand-written GLB reader and their lamp table left the game with this pack; their
measurements stay under [The placeholder cars, measured](#the-placeholder-cars-measured).

## As built: renderer (`RENDER-65`)

**As built (2026-11-22).** Stage 1a: models, paint, tiers and the night lamps; the data layout
and the shader are ready for brake lights, indicators and steered wheels (`RENDER-66`), which
only fill values.

### Catalogue

`rust/src/nodes/sim/render/vehicle_catalogue.rs`, rebuilt whenever packs load
(`SimCore::rebuild_vehicle_catalogue`, O(vehicle assets)): the models sorted by qualified asset
id, and the **looks**, one per model and albedo variant. A car's bucket key is `look * 4 + tier`
(a `u8` holds 64 looks). Per model: tier files and switch distances (the `[[lods]]` starts),
wheels and lenses, the palette (linear colour) and its cumulative weights. Presentation only:
the simulation never reads it, nothing of it is saved, and a revision number counts rebuilds.

GDScript reads it once a revision (`SimulationNode.get_vehicle_catalogue`): per look the tier
files, tier starts, maps, `paint`, palette, wheel centres, the right-hand low-beam and tail lenses
and the size.

### Which look a vehicle gets

`VehicleCatalogue::pick(freight, identity)`: freight (vehicle type 4) draws from the truck class
(then van, then car, whichever has models), any other vehicle from the car class; a splitmix64
hash of the identity picks the model by spawn weight, the variant uniformly, the paint by palette
weight (from a second mix) and a blink phase. The identity is the parking vehicle handle when the
car has one (moving and parked alike, so a car keeps its look across the handoff), else the
agent's render id. O(models of the class + paints), no allocation. With the pilot pair every car
is the estate in its own paint and every freight vehicle the truck, about half of them
`haulier`. The saved model and colour bytes are no longer read. The vehicle type names that meant
models are gone; type 4, freight, stays.

### Per-car data: 16 floats, 64 bytes

A row is the MultiMesh transform (12 floats) and one custom-data vector:

| Float | Content | Filled by |
| --- | --- | --- |
| `custom.x` | front wheel steer angle, radians, positive to the left | the snapshot's target, eased by the frame job ([`RENDER-66`](#as-built-brake-lights-indicators-reverse-lamp-and-wheels-render-66)) |
| `custom.y` | wheel roll angle, radians, positive rolling forward | the frame job, from the drawn travel |
| `custom.z` | one whole number: bits 0-7 lamps, 8-13 paint index, 14-17 blink phase | the snapshot |
| `custom.w` | reserved | 0 |

Lamp bits: 0 lights on (driving), 1 brake, 2 indicator left, 3 indicator right, 4 reverse,
5 high beam, 6 beacon, 7 spare. The snapshot writes the vector per posed car
(`RenderSnapshot::car_custom`, aligned with `car_transforms`); the smoothing carries it unchanged
into the drawn rows and compares it in its untouched-bucket test, so a car at rest uploads
nothing and a changed word uploads once. Parked rows carry the paint with every lamp bit zero.
Vehicle MultiMeshes keep `use_colors = false` and set `use_custom_data`; the shadow twins and
the parked slots take the same rows.

### Tiers

The snapshot already measures a posed car's distance to the eye for the draw distance; the same
squared distance picks the tier against the model's squared starts (`VehicleCatalogue::tier`,
one comparison chain per posed car, no new pass, no state). There is no hysteresis: the tier is a
pure function of the distance, and a car that changes bucket keeps its smoothed pose (the
smoothing keys cars by render id across buckets). The ground solve takes the bucket's model.

Parked cars (`RENDER-69`, [as built](#parked-cars-at-their-own-tier-render-69)): the parked
layer registers a look's four tier meshes as four buckets (`ParkedLayer`, `stream_node.rs`
`register_parked_mesh`) and writes a chunk's cars of that look into each. Each tier's material
keeps only the cars whose own distance from the main camera is in the tier's half-open band
(`tier_band_m` in the body shader; the moving cars' rule, so a parked and a moving car at the same
distance draw the same tier) and collapses the others in the vertex stage; each batch's visibility
range is the band widened by the batch's reach, so the engine skips only batches none of whose
cars can be in the band. The two near tiers split a chunk by 128 m cells. No work a frame; a
changed chunk uploads its rows four times.

### Mesh loading and the reader

GDScript loads each tier through Godot's glTF importer, as building meshes are
(`godot/scripts/renderers/vehicle_resources.gd`), after taking the images, textures and material
texture slots out of the GLB's JSON chunk: the body shader binds the maps itself, so the importer
decodes no texture and never tries to load a `keep` file as a resource. The tier is then turned
half a turn about `Y` (`x` and `z` negated exactly), and each `COLOR_0` id is written back as its
byte plus half a step, so the engine's float-to-byte truncation keeps it exact. The hand-written
GDScript GLB reader is gone (`RENDER-59`), and with it the `Color8` path the contract allowed
for it. `godot/tests/parking_test.gd` checks every tier of every bundled look: one body surface,
ids exact bytes in the allowed sets, wheel parts on LOD0 and LOD1 only, the low beams at `-Z`.

**Forward axis: not changed in 1a.** The runtime still points a car's `-Z` along travel, and the
models are turned at load, as the placeholder cars were. The planned `+Z` runtime touches the
snapshot, the off-lane and parked poses, the ground solve, the beam shader and the headlight pool
at once; 1a keeps all of them and converts once, in the catalogue (`turned`) and at load. The
contract's frame is unchanged; switching later is a contained render change.

### One body shader

`godot/scripts/shaders/vehicle_body.gdshader`, with the cloud shadow (`standard_light.gdshaderinc`),
rain, the street lamps' light and the draw fade (`agent_draw_fade.gdshaderinc`); bodies receive no
shadows, as before:

- vertex: `round(COLOR.rgb * 255.0)` gives part, surface and paint; a wheel vertex (part 1 to 8)
  and its normal are rotated about the wheel's centre (a uniform array per model), roll about
  the model's `X`, then steer about `Y` for a steering wheel; the lamp gain of the vertex's
  surface comes from the lamp bits, the night value (sun 4 to -2 degrees) and, for indicators,
  a blink of 90 flashes a minute on the wind clock (`weather_wind.x`, which stops while the game
  is paused and keeps normal pace above normal speed); the palette colour by the paint index,
  white for a `paint: false` variant;
- fragment: albedo, times the paint where `B` is 255; ORM; the optional normal map; emission
  times the lamp gain. Glass and every surface without a lit lamp have gain 0: glass draws
  opaque as painted, and the cabin picture stays dark.

A moving car at night lights its tail, tail-and-brake (half), low-beam and marker lenses; brake,
indicator and reverse lenses light by their bits at any hour; a parked car, and a truck standing in
its dock, none.

### Lamps at night

The night layers of [`RENDER-26`](#vehicle-models-and-lamps-render-26) stay, positioned from the
sidecar lenses instead of the measured table: the glow quads and the beam box take the right-hand
`low_beam` lens and the right-hand `tail_brake` lens (else `tail`), each the one of its group whose
facing points most along the nose (or the tail), mirrored for the left; the headlight pool takes
the same head lens. A lens's `facing` is a surface normal, not a beam: the glows face along the
vehicle's axis and the beams are aimed along it, as before.

### Ground support

Each model's ground support comes from its wheel facts (`VehicleCatalogue::ground_supports`): the
nine plan-box samples and a 3 x 3 grid over each wheel's contact patch (both tread edges and the
middle, under the centre and a quarter radius ahead and behind), 45 contacts for four wheels. The
GDScript vertex scan and the fixed five profiles are gone. `vehicle_ground_support_test.gd`
grounds both bundled models on a pitched and rolled grade with abrupt breaks: no tyre vertex is
buried, the lowest 0.3 mm under the 2 cm the solve lifts its support points (19.7 mm clearance).

### Cost

- CPU per frame: unchanged in kind, O(drawn cars). The snapshot adds the look pick and the tier
  comparison per posed car and fills one more bucket map; the smoothing copies four floats more
  per changed car and builds the 16-float rows only for buckets that changed. Rows are 64 bytes
  (48 before), a third more to upload for buckets that moved. Nothing allocates per frame once the
  buffers have grown.
- GPU per car: LOD0 2,990 triangles for the estate and 4,490 for the truck within its 53 m; 880 /
  1,398 to 100 / 176 m; 216 / 348 to 398 / 702 m; 40 / 60 beyond. The placeholder cars were
  2,032 to 2,476 triangles at every distance. The wheel and lamp work is a few ALU operations per
  vertex.
- Parked: four slots per chunk and look instead of one at first; since `RENDER-69` a far tier keeps
  one slot a chunk and look and a near tier one a 128 m cell with cars (1,758 slots on the 30k city
  instead of 256), and each car draws at its own distance's tier.
- Not timed in this round (the owner batches timing); the prepared session is level F, see
  `TOOLS-41` in the roadmap's notes.

### Known limits of 1a

- Lanes queued cars 2.7 m apart (`CAR_LENGTH`) while the estate is 4.48 m and the truck 7.90 m
  long, so queued cars overlapped nose to tail; fixed by `TRAFFIC-38`, which landed with 1a.
- One car shape in many paints until the fleet is built.
- The tier pops at a band edge, without a cross-fade. Seen in stills at the switch distances
  (2026-11-22, the 428a413 pair, Forward+ and Compatibility, 13:00): at 30 m the estate's mirrors
  go (LOD0 to LOD1), at 100 m its grey rims take the body colour (LOD1 to LOD2), the truck's
  steps at 53 and 176 m are as slight; at 398 m (estate) and 702 m (truck) LOD2 and LOD3 read
  alike at 1080p. LOD3 has no wheels and a dark lower strip, visible only near.
- Parked tiers switched per chunk, by the distance to the chunk's batch centre, so a parked car
  beside the camera could stay on a far tier; fixed by `RENDER-69`.

## As built: brake lights, indicators, reverse lamp and wheels (`RENDER-66`)

**As built (2026-11-22, stage 1b, landed on `dev` 2026-11-22).** The values 1a left at zero are filled
from the simulation's state, drawn only: nothing is saved, nothing is written back, and no speed,
route or position reads them. The lamp bits and the steer target are a pure function of the state
the snapshot reads, so two runs draw the same bits and angles for the same tick
(`rust/src/nodes/sim/render/vehicle_signals.rs`); the drawn steer and the wheel roll are the frame
job's, like the smoothed pose (`car_visual.rs`).

| Value | Rule | Computed in | Bound per posed car |
| --- | --- | --- | --- |
| Brake (bit 1) | below | the snapshot, LOD0 to LOD2 | the speed pass's own reads: O(log cars in the lane), near a lane end the planned connector and one entry-slot test |
| Indicators (bits 2, 3) | below | the snapshot, within 100 m of the eye | O(1) plus one or two lane samples, O(log lane points); near a lane end the planned connector |
| Reverse (bit 4) | a truck drawn tail first in a yard manoeuvre | the snapshot | O(1) |
| Steer target (`custom.x`) | below | the snapshot, LOD0 and LOD1 (the tiers with wheels) | two lane tangents, O(log lane points) each |
| Steer drawn, roll (`custom.x`, `custom.y`) | eased and integrated | the frame job's existing loop | O(1), two floats per drawn car |
| Blink | 90 flashes a minute on the wind clock | the shader | none on the CPU |

**Brake.** A car on a live lane brakes when the next tick's speed update would slow it or it
stands: its speed is at most 0.05 m/s, or one of the speed pass's braking caps (a junction
approach, a connector's blocked exit lane, a blocked planned lane change) is below its speed, or
its IDM acceleration is below -1.0 m/s². The pass's caps and IDM step are shared code
(`AgentSystem::speed_outlook`, `idm_acceleration`), read on the lane buckets the last tick rebuilt
after its movement, so they are what the next tick reads. The -1.0 m/s² margin keeps a car that
follows at a steady gap, whose acceleration hovers about zero, dark: a memoryless rule cannot flicker
there, and needs no hold time. A car off every lane brakes while it waits to join its lane (the
lane-attach hold, or an egress car within 0.5 m of its lane point). Braking is read from the state,
not from a speed change between snapshots, so it is not a snapshot late.

**Indicators.** Left or right, never both (no hazard state fits yet, below):

- during a lane change, toward the target lane;
- on a road lane, from 40 m before its lane end, or 3 s of its speed when that is farther, toward
  the bend of its planned connector, and on that connector to its end. A connector bends when its
  first and last tangent differ by 25 degrees or more; a U-turn takes its side from its chord;
- roundabouts: no signal entering the ring or circulating, the exit's bend side leaving it (a
  connector at a node with a ring edge that does not lead onto the ring);
- within the same lead of its detach point, toward the door of the building it drives to;
- pulling out (egress or the lane-attach hold), toward the way it turns onto its lane when that
  turn is 25 degrees or more, else toward the lane point when it lies 0.5 m or more to a side (out
  of a kerb bay);
- pulling in, toward the side of its lane it has left, until it faces 60 degrees from the lane;
- none while reversing.

**Reverse.** A truck whose yard pose is reversed (`yard_pose_reversed`: backing out of a dock, or
into a `tail_in` one) while it pulls in or out off the lane. Resident cars never back up.

**Steer.** On a lane, the signed angle between the path's tangents at the front and the rear axle
(the means of the steering and the fixed wheels' centres from the facts): an arc of radius `R`
gives `L / R` for a wheelbase `L`. Past its lane's end the front axle reads the lane the car takes
next (its planned connector, or a connector's exit lane), so the wheels turn in before the
connector, not when the car's centre reaches it. During a lane change both tangents are the
S-curve's. Off a lane, `atan(L κ)` of the drawn arc's curvature `κ` (`PARK-29`, 0 on a straight
leg), negated for a vehicle backing. Clamped to 35 degrees (the contract's wheel clearance) and
rounded to a quarter of a degree, with zero as `+0.0`: a car on a straight or a steady curve gives
one value tick after tick. The frame job eases the drawn steer toward the target at the body's
rotation rate (18 per second) and takes the target once within 0.002 rad, so the easing ends
exactly; a car is at rest only once its steer has settled.

**Roll.** The drawn origin's travel along the nose (the runtime's `-Z`) over the model's wheel
radius (the first wheel's; the pilot pair's wheels are one size each), added each frame and wrapped
to a turn: it stops with the drawn pose (paused, queued), runs backward when the car backs, and
follows any game speed. It carries on across tier and bucket switches and a change of height owner
(lane to off-lane); a car first drawn starts at zero, as a parked row has it. Not quantised: it
changes only while the pose does, and a resting car's row is copied, so it never churns on its
own.

**Parked and standing.** Parked rows keep every lamp bit zero, steer and roll zero, as in 1a: no
work a frame. A truck standing in its dock (`AtDock`) is drawn with every lamp bit zero and
straight wheels.

**Uploads.** The untouched-bucket test compares the snapshot's steer target and lamp word, and a
car's rest includes a settled steer, so a resting or steadily queued car uploads nothing; a lamp
bit that changes uploads its bucket once. The indicator blink needs no upload. Measured
(2026-11-22, the prepared 10k city, 1,190 frames at normal speed after 10 of warm-up, the release
library test `vehicle_rows_uploaded_on_a_prepared_city`, each frame drawn through the agents' frame
job twice, with 1b's values and with stage 1a's): a street view (eye 25 m up, a 300 m box) draws
28.7 cars a frame, of which 3.6 brake, 1.7 indicate and 1.1 steer; a district view (300 m up, 2 km)
826.6 cars, 8.6 braking, none indicating or steering (all on LOD2 and LOD3). Both views upload the
same buckets and rows as 1a, 5.2 and 6.0 buckets with 28.7 and 826.6 rows a frame: every drawn
bucket holds a moving car, so it uploads each frame either way, and the new values add no upload.

**Left out.**

- High beam stays off: an honest rule needs the oncoming lanes' buckets and on and off distances
  kept between frames (presentation memory), and only its lens would show without the beam layers;
  it stays with `RENDER-67`.
- Hazard lights: no state fits. A truck standing at a dock is parked (engine off, lamps dark), and
  a car stuck in gridlock is waiting, not broken down. Left for a breakdown or emergency state.
- No early indicator before a lane change starts: the simulation has no planned-change state.

**Tests.** `vehicle_signals/tests.rs` (brake for a braking, a standing and a queued car, dark for a
cruising and a steadily following one, and for an off-lane car in the attach hold; indicators on a
left and a right connector, a straight one and both lane changes, none beyond 100 m or on LOD3;
steer `L / R` with its sign on a known arc, zero on a straight, the lock, none on LOD2, the arc rule
forward and backing; the same state draws the same bits, angles and custom data);
`car_visual/tests.rs` (roll follows the drawn travel through speed changes, a tier switch and a
change of height owner, stops exactly and uploads nothing at rest, runs backward backing, restarts
when a car is drawn anew; the steer eases and settles); `docks/backs_in.rs` (a truck backing into
a `tail_in` dock shows the reverse lamp and no indicator, stands with every lamp dark, and leaves
forward without it).

## Snapshot cost of the vehicle stage (performance round, 2026-11-22)

The F-level sessions of stages 1a, 1b and `PARK-31` read the 30k city's mean snapshot time per
frame 0.17 to 0.18 ms over `dev` (1x 0.805 to 0.971 ms, 32x 1.252 to 1.431 ms). A round on branch
`vehicles-perf` (on `park-31`) split that cost and removed what could go without changing a drawn
value.

**Split.** Per-phase timers in diagnostic builds only (not committed), on the `PERF-07` workload
(the prepared 30k city, the 2 km camera box with no eye, the bundled pack's catalogue), 8 workers
on the P-cores, alternated with a `dev` build carrying the same timers; means of two or three runs
of 900 frames at 32x and 1,777 at 1x, on a machine another session also used, so read the columns
against each other, not against the timing rows. The camera has no eye, so every car is posed at
the nearest tier: the worst case of the signals' work. 1,475 cars and 2,593 agents are posed a
frame at 32x, 57 of the cars off their lanes.

| Phase, ms a frame | `dev` 32x | `park-31` 32x | `vehicles-perf` 32x | `dev` 1x | `vehicles-perf` 1x |
| --- | --- | --- | --- | --- | --- |
| Posing | 0.63 | 0.69 | 0.69 | 0.48 | 0.45 |
| Bucket fill | 0.15 | 0.07 | 0.04 | 0.12 | 0.03 |
| Ground solve of off-lane cars | 0.27 | 0.41 | 0.41 | 0.20 | 0.32 |
| Parked chunks | 0.27 | 0.38 | 0.34 | 0.04 | 0.05 |

- Posing: the look pick, the tier, the signals (0.045 ms at 32x, measured by switching them off),
  the custom data and 88-byte instances (72 before).
- Bucket fill: two car buckets where `dev` had ten, each of which scanned every posed agent.
- Ground solve: 45 support points a model (the nine plan-box points and a 3 x 3 patch a wheel),
  where the headless `dev` grounded every car on the nine-point fallback profile. In the game,
  `dev` grounded cars on their meshes' bottom vertices (up to 64), so the benchmark's `dev` row
  understated what the game paid.
- Parked chunks: three look picks for each car of a rebuilt chunk (about 12,900 picks a frame at
  32x), and the parked ground solves on the same 45 points.

The gate turn's test and the wider off-lane cull box measured nothing (switched off, the posing
time did not move); no off-lane car keeps its pose bit for bit from one frame to the next at
either speed, so a ground-solve cache would never hit.

**Changes** (drawn products bitwise equal, below):

- The look pick's three 64-bit divisions became a widening multiply by a reciprocal fixed when
  packs load, with at most two corrections (`Divisor`, exact for every divisor); a rebuilt parked
  chunk picks each car's look once, and its ground model alone (`ParkedLooks::ground_model`).
- Each posing task sorts its agents' bucket codes with their slots, and a bucket copies its run
  from each task in agent order: O(posed agents) plus a binary search a task and bucket, where every
  bucket scanned every posed agent. Memory: 4 bytes a posed agent, kept with the task's buffer.
- The steer samples only the lane tangent (`sample_lane_tangent_at`, bitwise the tangent of
  `sample_lane_pose`), one position sample less a tangent.

**Proof.** `vehicle_products_on_a_prepared_city` (release, ignored) hashes every snapshot's drawn
products (each car bucket's rows, custom data, render ids and ground flags, the walkers' rows, the
draw cuts and the newly published parked chunks' rows) for 300 frames at 32x from four views (the
`PERF-07` camera with no eye, a street eye, a district eye, play height), first with the city's
catalogue, then with none:

```bash
METRUM_PERF_RESIDENTS=30000 METRUM_PERF_CACHE=<snapshot cache> RAYON_NUM_THREADS=8 \
  "$TEST_BINARY" nodes::sim::core::populated_city::frame_budget::vehicle_rows::vehicle_products_on_a_prepared_city \
  --exact --ignored --nocapture --test-threads=1 2> "$ARTIFACTS/hash-30000.log"
```

Every hash of the 10k and 30k cities (2,400 a city) equals the one the `park-31` code gives with
this test added; two runs of one build agree. `SIM-02` envelope and determinism records equal the
`park-31` binary's.

**Timing** (level F, 2026-11-22 05:22, the same boot as the stage sessions, 8 workers on the
P-cores; control the `dev` build of daab78d4c, branch the `vehicles-perf` code):

```bash
./run.sh --timing --level F --control <dev $TEST_BINARY> --branch "$TEST_BINARY" --base dev \
  --k22 --artifacts "$ARTIFACTS/timing-F1"
./run.sh --timing ... --artifacts "$ARTIFACTS/timing-F1" --rerun p07-5-ctl   # the 1x control
```

The cached 1x control row was the noisy one of the stage 1a session (13 frames over 8 ms), so it
was timed afresh.

| 30k, mean a frame | `dev` | `park-31` (its session) | `vehicles-perf` |
| --- | --- | --- | --- |
| Snapshot, 32x | 1.260 ms | 1.431 ms | 1.402 ms (+11.3 %, WORSE) |
| Snapshot, 1x | 0.849 ms | 0.971 ms | 0.977 ms (+15.0 %, WORSE) |
| Frame, 32x | 4.306 ms | | 4.438 ms (+3.1 %) |
| Frame, 1x | 2.087 ms | | 2.224 ms (+6.6 %) |

Acceptance passes (1x: 5.05 / 5.31 ms worst unpreempted frame, p99.9 3.85 / 3.88 ms; 32x within
the `PERF-17` exception as `dev`); K22 commit 6.94 ms against 6.61. The changes take 0.03 ms off
the 32x snapshot in these rows (0.07 ms at 32x and 0.025 ms at 1x in the alternated diagnostic
runs). What is left is mostly the support points (0.13 ms at 32x and 0.07 ms at 1x in the
diagnostic runs, below), then the posing's own work for the signals at the nearest tier (about
0.05 ms at 32x).

**What remains, and why.** The support points. Each off-lane car's ground solve and each parked
ground solve reads its surface at every point, an exact road or graded-surface query, and the 45
points of 1a are five times the fallback's nine. Measured with the points per wheel switched in a
diagnostic build (the same workload, two runs each, means of the snapshot a frame):

| Points a model | 32x | 1x |
| --- | --- | --- |
| 45: nine plan-box points, 3 x 3 a wheel (as built) | 1.477 ms | 0.861 ms |
| 21: nine, three a wheel along its centre line | 1.387 ms | 0.819 ms |
| 13: nine, one a wheel under its centre | 1.344 ms | 0.790 ms |
| `dev`, the nine-point fallback (other runs) | 1.333 ms | 0.774 ms |

Fewer points per wheel lower the solve in proportion and change drawn heights where a tyre's patch
spans a kerb or a grade break, which is why the patch exists; the choice is the owner's. With no
eye the signals run for every car on the nearest tier; with the game's eye the brake rule runs only
within the LOD2 range and the indicators and steer within 100 m.

**Second round: the ground solve, exact** (2026-11-22, same branch). The 45 points stay and every
solved pose stays bit for bit; only the work per point changed. The solves a `PERF-07` run makes
at 32x (900 frames) were recorded in a diagnostic build and replayed on one thread against the
round's starting code:

| A solve, replayed serially | Off-lane car (57 a frame) | Parked car (8.3 a frame) |
| --- | --- | --- |
| Before | 25.2 µs | 29.9 µs |
| Graded-surface gather (`gather_near`, the 64 m cap's seams and sites) | about 10 µs, 0.92 a solve | 10.2 µs, 1.0 a solve |
| Points: on a road / on the ground | 11.6 / 33.5 | 2.9 / 46.1 |
| Lattice deltas evaluated, before -> after | 110.8 -> 36.2 | 138.4 -> 39.8 |
| After | 16.7 µs | 19.2 µs |

- The points of one solve fall on about a dozen `GL0` cells (a tyre's 3 x 3 patch on one or two),
  and each ground point evaluated its triangle's three lattice deltas afresh, about 0.1 µs each over
  the window's listed seams. The graded query scratch now keeps the deltas of its current
  preparation (`DeltaMemo` in `terrain/graded/query.rs`): 256 slots direct-mapped by the low four
  bits of each lattice index, so no two points of an 8 m block evict each other, stamped so that
  forgetting is O(1); forgotten wherever the prepared inputs are replaced. A delta is a pure function
  of the lattice point and those inputs, so a kept one is the value a fresh evaluation gives. 7 KB a
  query scratch (one a thread), no allocation; every graded consumer reads through it.
- The scratch also keeps the query index's last quiet-tile answer, by index identity, generation
  and tile: one ordered-map lookup a solve where it was one a ground point.
- The off-lane solve's parallel loop kept a minimum split of 32 cars from the nine-point profile;
  a bucket flags a few dozen of its cars, scattered in agent order, at microseconds each, so the
  minimum is gone (stolen halves split down to single cars). Products are unchanged: each car is
  solved alone.
- Not done: a parked-pose cache per (place, model). The recorded run made 7,474 parked solves on
  7,402 distinct places, so it would almost never hit. Snapping the gather to an 8 m grid so that
  neighbouring cars share it read the same values but cost more (a wider window, no sharing in
  practice): 0.06 ms worse at 32x.

The diagnostic phase timers (alternated runs, same build, means of two; snapshot a frame):

| `PERF-07` 30k | Off-lane ground | Parked chunks | Snapshot |
| --- | --- | --- | --- |
| 32x, before | 0.439 ms | 0.350 ms | 1.497 ms |
| 32x, kept deltas and tile | 0.343 ms | 0.294 ms | 1.374 ms |
| 32x, and no minimum split | 0.291 ms | 0.303 ms | 1.336 ms |
| 1x, before | 0.331 ms | 0.054 ms | 0.866 ms |
| 1x, all three | 0.217 ms | 0.037 ms | 0.752 ms |

The gather is now the largest part of a solve: about 0.6 ms of work a frame at 32x across the
workers, nearly all of it copying the sites and walking the road pieces' seams within the 64 m cap
of the car, which the graded surface needs for an exact value anywhere in the window. The signals'
0.05 ms at 32x is the brake, indicator and steer rules themselves (the speed pass's outlook, lane
samples, the planned connector), run for every car because `PERF-07` has no eye; nothing in it is
repeated work.

**Proof.** `vehicle_products_on_a_prepared_city` (above): all 2,400 hashes of the 10k and 30k
cities equal those of the first round's build. The replay compared every recorded solve with the
memo on and off: equal. `SIM-02` envelope and determinism records equal the `park-31` binary's.
The graded tests read two worlds in turn through one scratch at the same lattice points
(`kept_lattice_deltas_go_with_their_preparation`), and in debug builds every indexed query is
still compared with an index-free reference on its sampled points.

**Timing** (level F, 2026-11-22 06:36, the same boot, 8 workers on the P-cores; control the `dev`
build of daab78d4c as before, branch this round's code):

```bash
./run.sh --timing --level F --control <dev $TEST_BINARY> --branch "$TEST_BINARY" --base dev \
  --k22 --artifacts "$ARTIFACTS/timing-F2"
```

The 1x control row came from the cache: the fresh 05:43 rerun of the same `dev` binary in another
session of this boot (mean 2.098 ms, acceptance passing), not the noisy row the first round
replaced. The session's first 32x control row calibrated against the cache (+0.0 % mean).

| 30k, mean a frame | `dev` | `vehicles-perf`, first round | `vehicles-perf`, second round |
| --- | --- | --- | --- |
| Snapshot, 32x | 1.251 ms | 1.402 ms | 1.252 ms (+0.1 %, level) |
| Snapshot, 1x | 0.853 ms | 0.977 ms | 0.827 ms (-3.0 %, level) |
| Frame, 32x | 4.277 ms | 4.438 ms | 4.298 ms (+0.5 %, level) |
| Frame, 1x | 2.098 ms | 2.224 ms | 2.079 ms (-0.9 %, level) |

Acceptance passes (1x: 5.11 ms worst unpreempted frame, p99.9 3.66 ms; 32x within the `PERF-17`
exception as `dev`); K22 commit 6.58 ms against 6.80. The stack's snapshot is level with `dev` at
both speeds with the 45 support points, so the points table above is no longer a cost choice.

## Landing timing (level L, 2026-11-22)

`TOOLS-41`, `TOOLS-42`, `RENDER-65`, `RENDER-66` with the performance rounds, `PARK-31`,
`TRAFFIC-38` and `PARK-30` landed on `dev` together. One merge fix came with them: the parked
layer's bay paint keeps its 12-float rows beside the cars' 16-float rows with custom data.
Before the session, on the landing's head: `./run.sh --test` passes; the `SIM-02` envelope and
determinism records equal `TRAFFIC-38`'s own records (the vehicle stack and `PARK-30` are
simulation-neutral); `vehicle_products_on_a_prepared_city` gives the same hashes in two runs.

Session 2026-11-22 10:18-10:29, eight performance cores and eight workers, nothing else running,
no build during it. The control is the `dev` build of `daab78d4c` that `TRAFFIC-38` was timed
against, on its own city (one border trunk per 10,000 residents); the branch prepares and times
its own v12 city. Read the rows as the cost of the landing on its own city.

```bash
./run.sh --timing --level L --control "$CONTROL_TEST_BINARY" --branch "$TEST_BINARY" --base dev \
    --k22 --artifacts "$ARTIFACTS/timing-L"
./run.sh --timing --artifacts "$ARTIFACTS/timing-L" --rerun contract-1-ctl
```

Reused by the runner's rules from this boot's level F session of the same control: its 30k 1×
day and K22 row (calibration on the first control 32× run: +0.1 % mean, +0.6 % p99, +0.3 %
p99.9). The control's contract row is cache-only at L and was timed with the rerun above.

| Row | Control | Branch | Acceptance | Reading |
| --- | --- | --- | --- | --- |
| 30k 32×, C B B C: mean / p99 / p99.9 / worst unpreempted, ms | 4.283 / 6.364 / 7.365 / 8.225 | 4.235 / 6.193 / 6.817 / 7.361 | `PERF-07`, `PERF-17` the known exception | level on every metric; worst unpreempted frame and slot component better; both branch runs pass, the control one pass and one exception |
| 30k 32× snapshot / agent component, ms | 1.245 / 1.965 | 1.258 / 1.949 | | level |
| 30k 1×, control one day (reused), branch two: mean / p99 / p99.9 / worst unpreempted, ms | 2.088 / 3.290 / 3.625 / 5.286 | 2.073 / 3.278 / 3.673 / 4.428 | `PERF-07` | passes on every day; level, worst unpreempted better |
| 30k 1× snapshot / agent component, ms | 0.849 / 1.152 | 0.834 / 1.156 | | level |
| `load_s` (30k 32× and 1×) | 2.284, 2.260 | 2.282, 2.290 | | level |
| K22 30k: click-to-ack / commit, ms | 6.86 / 6.82 (reused, level F 03:43) | 7.50 / 7.47 | | one run a side, reported; `TRAFFIC-38`'s branch alone read 8.00 / 7.97 on its own city in its L session |
| Economy contract, hour means: baseline 10k / 30k, fiscal 10k / 30k, ms | 0.91 / 3.23 / 6.26 / 13.79 | 0.93 / 2.99 / 6.03 / 12.06 | the contract's limits | level or better; no time limit fails |
| Economy contract, work-count limits | baseline `buyer_sites_built`, `staffing_trials`, `supplier_searches`, `supplier_routes`, `job_search_keys` (the known five) | fiscal stress `job_search_keys` 7,707 -> 41,262 (limit 3.28×) | | the fixture's: the control's 30k count is the same (41,295) but its 10k city searched 15,353 keys, so its ratio stays inside; the v12 cities no longer fail businesses, which clears the baseline five. The branch's counts equal `TRAFFIC-38`'s own L session's except `cch_point_searches` (worker schedule) |

No verdict row reads worse.

## Parked cars at their own tier (`RENDER-69`)

**As built (2026-11-22, branch `park-tiers`, for review).** The owner saw parked cars drawn at a
coarse tier (wheels as flat boxes) with the camera right beside them, and zooming in never
helped; moving cars were fine. Cause: the parked layer drew a chunk's cars of a look in four
batches, one a tier, and gave each the tier's band as its engine visibility range. The engine
tests that range against the camera's distance to the centre of the batch's bounds, which for a
512 m parking chunk can be hundreds of metres from the car in front of the camera: the nearest
tier drew only within 30 m of that centre. In the 30k city a lot car 357 m from its batch's
centre drew LOD2 at 5, 20, 60, 200 and 600 m.

### The rule

- **Each car's own tier, in the vertex stage.** The body shader takes `tier_band_m`, `[begin,
  end)` metres of the car's own distance from the main camera (the fade's distance,
  `MAIN_CAM_INV_VIEW_MATRIX`, so the depth and shadow passes keep the tier the main view
  draws). A car outside the band collapses to its origin and rasterizes nothing in any pass. The
  parked cars' resources give each tier its own copy of the look's material with the tier's band
  (`vehicle_resources.gd` `tier_bands`), from the catalogue's tier starts, the last tier
  unbounded; the moving cars keep the default band, every distance. The bands tile the distances
  half-open in one expression evaluated alike for every tier, so exactly one tier draws a car:
  the one the moving cars' rule (`VehicleCatalogue::tier`, `d >= start`) gives it.
- **Batch ranges that never cut a car in its band.** Every car of a batch stands within the
  batch's reach (the farthest car origin from the bounds' centre) of that centre, so a car in the
  band has the centre within the band widened by the reach. The range is `[max(0, begin -
  reach), min(end, draw distance) + reach]`. The bay paint (one band, `[0, inf)`) keeps its range.
- **Near tiers by 128 m cells.** Shading every car of a chunk with the two detailed meshes
  whenever the camera is within the widened range would draw collapsed triangles for every car of
  the chunk (below), so the layer splits a near tier's cars by a fixed 128 m cell of the world
  grid the parking chunks share (four a side), a counting sort, stable, so a cell's cars keep the
  chunk's order; the far tiers keep one batch a chunk. A chunk still installs whole or not at all,
  through the same slot pool, size classes and refusal; `get_parked_drawn` names each batch's
  cell. Install `O(B log B + rows)` for a chunk's `B` batches (no allocation once the scratch has
  grown); no work a frame and no slot shown or hidden for the tiers.

### Choosing the split

`parked_tier_batches_on_a_prepared_city` (ignored, release) installs every parked chunk of the
prepared 30k city (64 chunks, 12,013 parked cars, three looks) into the layer against the recorder
once a split, and counts, over 124 cameras standing by parked cars at 2, 20, 60 and 200 m height,
the triangles the engine's ranges send to the vertex stage (every car of an admitted batch,
every direction):

| Cells a side, tiers 0 / 1 / 2 | Slots | Shaded triangles a camera | Batches admitted |
| --- | --- | --- | --- |
| 1 / 1 / 1 (no split) | 256 | 2.24 M | 60.5 |
| 4 / 4 / 1 (128 m) | 1,758 | 0.82 M | 62.1 |
| 8 / 4 / 1 | 3,808 | 0.77 M | 61.8 |
| 8 / 8 / 1 (64 m) | 5,858 | 0.74 M | 65.1 |
| 8 / 8 / 4 | 6,609 | 0.58 M | 95.6 |
| 16 / 16 / 1 | 13,708 | 0.72 M | 70.6 |

The cars' own tiers draw 0.44 M a camera; the old rule drew 0.48 M (cheap only because it drew
far tiers near). Then rendered: the 30k city paused, three poses round the car above (20 and 60 m
from it, a 200 m overview), gamescope headless at 1,000 Hz, eight performance cores, one row each
in the order control, none, 128 m, 64 m, 64 m, 128 m, none, control (2026-11-22 11:22-11:26;
the desktop was in use, 1-min load 7-10). GPU and render CPU are Godot's measured viewport times,
medians of 360 frames; install is the Godot side of installing all 64 chunks again with the same
rows (slots kept), from GDScript:

| Library | Slots | GPU ms (20 m / 60 m / 200 m) | Render CPU ms | Primitives (20 m) | Install all, ms |
| --- | --- | --- | --- | --- | --- |
| `dev` (control) | 256 | 2.43 / 2.65 / 2.71 | 0.44 / 0.54 / 0.62 | 20.7 M | 0.6 |
| no split | 256 | 3.43 / 3.54 / 3.32 | 0.49 / 0.78 / 0.76 | 39.2 M | 0.6-1.2 |
| 128 m cells | 1,758 | 2.56 / 2.75 / 2.77 | 0.47 / 0.62 / 0.71 | 22.9 M | 1.3-1.5 |
| 64 m cells | 5,860 | 2.50 / 2.67 / 2.73 | 0.51 / 0.73 / 0.76 | 21.8 M | 2.9-3.5 |

Each pair of rows of one library agreed within 0.03 ms GPU. The collapsed vertices are not free:
without the split the frame costs 0.6-1.0 ms more GPU. 128 m cells bring it to 0.05-0.13 ms over
`dev`, which drew far tiers near; 64 m cells save another 0.04-0.06 ms GPU but take 3.3 times the
slots (72 % of the pool's 8,192 at 30k, so a larger city could fill it and have chunks refused),
more render CPU (Godot culls per instance: `RENDER-60`'s caster cells cost render CPU for the same
reason) and twice the install time. 128 m cells are built. Installing one chunk again costs about
22 µs on the Godot side with them (9 µs on `dev`).

### Proof

- `parked/tests.rs`: 600 cars over a chunk, three past its edges, the estate's bands; for
  cameras on rings at 0 to 2,600 m about five points, every car in a tier's band within the draw
  distance has its batch admitted by the engine's range test (failing on the old range); the near
  tiers' batches each hold one cell, the far ones one batch, a second install sends the same rows;
  a split chunk one slot short of the pool is refused with nothing sent, then fits, and its batches
  that leave free their slots. The draw-distance test now builds 16-float car rows.
- Stills, 30k city, gamescope headless, isolated profile, Forward+ and Compatibility: the lot car
  357 m from its batch's centre at 5, 20, 60, 200 and 600 m by day, at 5 and 20 m at 17:30 and at
  22:00. `dev` drew LOD2 at every distance (the engine's read-back: only the LOD2 batch's range
  held the camera); the branch draws LOD0, LOD0, LOD1, LOD2, LOD3. With only the drawn tier's
  batches shown, the car's crop (240 px square) is unchanged (0 % of pixels, 0.8-1 % at 20 m in low
  sun, a neighbour's shadow); with them hidden the car and its shadow are gone (3-94 %): no other
  tier draws it. Shadows follow each tier by day and in low sun; night unchanged.
- `parking_test.gd`: each tier's batches sum to the look's cars, and the tier materials' bands
  tile every distance.

## Planned: beams, later lamps and glass (`RENDER-67`)

**Planned, after `RENDER-66`.** The lamp regions are in the models from the pilot on.

| Lamp | On when |
| --- | --- |
| `daytime` | the car is driving, by day; dimmed at night |
| `low_beam`, `tail` | driving, and the global night value is above zero |
| `high_beam` | lights on, and the road has no street lights, and no vehicle is ahead in its lane within 150 m, and none is oncoming on the road's opposite lanes within 300 m |
| hazard (both indicators) | a state that fits (a breakdown, an emergency stop); not a truck at a dock, which is parked (`RENDER-66`) |

- The high-beam rule is cosmetic: no speed, route or headway reads it. It is decided per posed
  car in the snapshot from the road's street-light flag and two lane-bucket lookups, O(log cars
  in a lane) ahead and O(opposite lanes × log) oncoming, only for posed cars on unlit roads at
  night. Vehicles beyond the current road are not seen.
- To stop flicker at the thresholds the frame job keeps one bit per drawn car, with on and off
  distances of 150 / 180 m ahead and 300 / 350 m oncoming.

How the light is shown, through the three existing layers with a per-car high flag and no new
light type:

| Layer | Low beam | High beam | Bound |
| --- | --- | --- | --- |
| Lit lens (emission) | yes | yes, its own faces | part of the body draw |
| Glow point per lamp entry | as today | larger and brighter | 2 triangles per lamp; collapses when off |
| Real spot lights, the 12 nearest within 100 m | 60 m range | 120 m range, narrower | a fixed pool; the cost follows reach |
| Beam box on the ground, 70 to 450 m | as today | stretched to 150 m only for cars whose bit is set | fill rate; high-beam cars are isolated by the rule, so few are in view at once |

Glass, later, on the ids and the cabin picture the models already carry:

- **Tint variety:** darker rear glass (ids 34, 35) on a share of cars, picked by the paint's
  hash. One multiply in the shader.
- **Lit cabin at night:** the emission on `glass_saloon`, `glass_door` or a cab's glass comes on
  from one more lamp bit. Same draw, no new texture.
- **See-through glass with an interior, nearest tier only:** for the few cars closest to the
  camera, on LOD0 only, the game splits the glass-id faces into a second, transparent surface at
  load and draws the model's optional interior mesh (`<name>_interior.glb`, the sidecar's
  `interior`, about 400 triangles) inside it. A model without an interior keeps opaque glass.
  It is not part of the first renderer because it breaks the one-draw rule (a second, blended
  draw per bucket, and the interior's own draw), transparent instances of one MultiMesh are not
  sorted against each other, and the fade and the shadow twins need a second material. Limited
  to the nearest cars the cost is bounded by a fixed count; anywhere else there is nothing to
  see.
